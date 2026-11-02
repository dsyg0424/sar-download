use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};
use tauri::{State, ipc::Response};

const TILE_LIMIT: usize = 4 * 1024 * 1024;
const CACHE_LIMIT: u64 = 256 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub struct MapTiles {
    client: reqwest::Client,
    root: PathBuf,
    slots: Arc<tokio::sync::Semaphore>,
    writes: AtomicU64,
    requests: Mutex<HashMap<String, (Arc<AtomicBool>, bool)>>,
}
pub fn tile_url(provider: &str, z: u32, x: u32, y: u32) -> Result<String, String> {
    let (base, max_zoom) = match provider {
        "imagery" => ("World_Imagery", 17),
        "streets" => ("World_Street_Map", 19),
        "labels" => ("Reference/World_Boundaries_and_Places", 17),
        "osm" => ("osm", 19),
        _ => return Err("不支持的底图来源".into()),
    };
    if z > max_zoom || x >= (1u32 << z) || y >= (1u32 << z) {
        return Err("瓦片坐标无效".into());
    }
    Ok(if base == "osm" {
        format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png")
    } else {
        format!(
            "https://server.arcgisonline.com/ArcGIS/rest/services/{base}/MapServer/tile/{z}/{y}/{x}"
        )
    })
}
fn image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}
const VECTOR_SERVICE: &str =
    "https://basemaps.arcgis.com/arcgis/rest/services/World_Basemap_v2/VectorTileServer";
fn vector_url(resource: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(resource).map_err(|_| "矢量资源地址无效")?;
    if url.scheme() != "sar-vector"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("矢量资源地址无效".into());
    }
    let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
    match (url.host_str(), parts.as_slice()) {
        (Some("tiles"), [z, y, x]) => {
            let z: u32 = z.parse().map_err(|_| "矢量瓦片级别无效")?;
            let y: u32 = y.parse().map_err(|_| "矢量瓦片坐标无效")?;
            let x: u32 = x
                .strip_suffix(".pbf")
                .ok_or("矢量瓦片格式无效")?
                .parse()
                .map_err(|_| "矢量瓦片坐标无效")?;
            if z > 22 || x >= (1u32 << z) || y >= (1u32 << z) {
                return Err("矢量瓦片坐标无效".into());
            }
            Ok(format!("{VECTOR_SERVICE}/tile/{z}/{y}/{x}.pbf"))
        }
        (Some("fonts"), [font, range]) => {
            let font = percent_encoding::percent_decode_str(font)
                .decode_utf8()
                .map_err(|_| "字体名称无效")?;
            if font.is_empty()
                || font.len() > 200
                || font.contains(['/', '\\'])
                || font.contains("..")
                || font.chars().any(char::is_control)
            {
                return Err("字体名称无效".into());
            }
            let (start, end) = range
                .strip_suffix(".pbf")
                .and_then(|v| v.split_once('-'))
                .ok_or("字形范围无效")?;
            let start: u32 = start.parse().map_err(|_| "字形范围无效")?;
            let end: u32 = end.parse().map_err(|_| "字形范围无效")?;
            if start % 256 != 0 || end > 0x10ffff || start.checked_add(255) != Some(end) {
                return Err("字形范围无效".into());
            }
            let font =
                percent_encoding::utf8_percent_encode(&font, percent_encoding::NON_ALPHANUMERIC);
            Ok(format!(
                "{VECTOR_SERVICE}/resources/fonts/{font}/{start}-{end}.pbf"
            ))
        }
        (Some("sprites"), [name])
            if [
                "sprite.json",
                "sprite.png",
                "sprite@2x.json",
                "sprite@2x.png",
            ]
            .contains(name) =>
        {
            Ok(format!(
                "https://cdn.arcgis.com/sharing/rest/content/items/30d6b8271e1849cd9c3042060001f425/resources/sprites/{name}"
            ))
        }
        _ => Err("不支持的矢量资源".into()),
    }
}
fn valid_resource(bytes: &[u8], url: &str) -> bool {
    if url.ends_with(".png") {
        return image(bytes);
    }
    if url.ends_with(".json") {
        return serde_json::from_slice::<serde_json::Value>(bytes)
            .is_ok_and(|v| v.is_object() && v.get("error").is_none());
    }
    // PBF tiles/glyphs may be empty for an unpopulated area/range, but never HTML/JSON.
    !bytes.starts_with(&[0x1f, 0x8b])
        && !matches!(
            bytes.iter().find(|b| !b.is_ascii_whitespace()),
            Some(b'<' | b'{')
        )
}
fn decode_vector(bytes: Vec<u8>, url: &str) -> Result<Vec<u8>, String> {
    if !url.ends_with(".pbf") || !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes);
    }
    use std::io::Read;
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_slice())
        .take((TILE_LIMIT + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|_| "矢量瓦片解压失败")?;
    if decoded.len() > TILE_LIMIT {
        return Err("矢量瓦片解压后过大".into());
    }
    Ok(decoded)
}
impl MapTiles {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent("SAR-download/0.1 desktop map viewer")
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(8)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            root: root.join("map-cache"),
            slots: Arc::new(tokio::sync::Semaphore::new(8)),
            writes: AtomicU64::new(0),
            requests: Mutex::new(HashMap::new()),
        })
    }
    fn cancel(&self, id: &str) {
        if id.len() > 100 {
            return;
        }
        let mut requests = self.requests.lock().unwrap();
        // Keep early cancellations until their matching IPC request arrives.
        // Late cancellation tombstones are bounded; active requests are retained.
        if requests.len() >= 512 {
            requests.retain(|_, (_, active)| *active);
        }
        let (flag, _) = requests
            .entry(id.into())
            .or_insert_with(|| (Arc::new(AtomicBool::new(false)), false));
        flag.store(true, Ordering::SeqCst);
    }
    async fn request(
        &self,
        id: &str,
        provider: &str,
        z: u32,
        x: u32,
        y: u32,
    ) -> Result<Vec<u8>, String> {
        self.request_bytes(id, self.load(provider, z, x, y)).await
    }
    async fn request_bytes(
        &self,
        id: &str,
        operation: impl std::future::Future<Output = Result<Vec<u8>, String>>,
    ) -> Result<Vec<u8>, String> {
        if id.is_empty()
            || id.len() > 100
            || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err("瓦片请求标识无效".into());
        }
        let flag = {
            let mut requests = self.requests.lock().unwrap();
            let entry = requests
                .entry(id.into())
                .or_insert_with(|| (Arc::new(AtomicBool::new(false)), false));
            if entry.1 {
                return Err("重复的瓦片请求".into());
            }
            entry.1 = true;
            entry.0.clone()
        };
        let result = tokio::select! {biased;
            _=crate::common::download_budget::cancelled(&flag)=>Err("MAP_TILE_ABORTED".into()),
            result=operation=>result,
        };
        self.requests.lock().unwrap().remove(id);
        result
    }
    async fn fetch(&self, url: &str, raster: bool) -> Result<Vec<u8>, String> {
        let _permit = self.slots.acquire().await.map_err(|e| e.to_string())?;
        for attempt in 0..2 {
            let response = match self.client.get(url).send().await {
                Ok(r) => r,
                Err(_) if attempt == 0 => continue,
                Err(_) => return Err("底图连接超时或网络不可用".into()),
            };
            let status = response.status();
            if attempt == 0 && (status.as_u16() == 429 || status.is_server_error()) {
                let delay = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(1)
                    .min(5);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
            if !status.is_success() {
                return Err(format!("底图服务器返回 HTTP {status}"));
            }
            if response
                .content_length()
                .is_some_and(|n| n > TILE_LIMIT as u64)
            {
                return Err("瓦片文件过大".into());
            }
            let mut stream = response.bytes_stream();
            let mut bytes = vec![];
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| "瓦片传输中断")?;
                if bytes.len() + chunk.len() > TILE_LIMIT {
                    return Err("瓦片文件过大".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            if !raster {
                bytes = decode_vector(bytes, url)?;
            }
            if !(if raster {
                image(&bytes)
            } else {
                valid_resource(&bytes, url)
            }) {
                return Err("地图服务器返回的资源格式无效".into());
            }
            return Ok(bytes);
        }
        Err("底图请求失败".into())
    }
    pub async fn load(&self, provider: &str, z: u32, x: u32, y: u32) -> Result<Vec<u8>, String> {
        let url = tile_url(provider, z, x, y)?;
        let path = self
            .root
            .join(provider)
            .join(z.to_string())
            .join(x.to_string())
            .join(format!("{y}.tile"));
        self.load_cached(&url, path, true).await
    }
    async fn load_vector(&self, resource: &str) -> Result<Vec<u8>, String> {
        let url = vector_url(resource)?;
        let key = hex::encode(Sha256::digest(url.as_bytes()));
        self.load_cached(
            &url,
            self.root.join("vector").join(format!("{key}.tile")),
            false,
        )
        .await
    }
    async fn load_cached(&self, url: &str, path: PathBuf, raster: bool) -> Result<Vec<u8>, String> {
        let cached = tokio::fs::read(&path).await.ok().filter(|v| {
            v.len() <= TILE_LIMIT
                && if raster {
                    image(v)
                } else {
                    valid_resource(v, url)
                }
        });
        let fresh = tokio::fs::metadata(&path)
            .await
            .ok()
            .and_then(|v| v.modified().ok())
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age < CACHE_TTL);
        if fresh {
            if let Some(bytes) = &cached {
                return Ok(bytes.clone());
            }
        }
        match self.fetch(url, raster).await {
            Ok(bytes) => {
                let disk_path = path.clone();
                let disk_bytes = bytes.clone();
                // A full/read-only cache must not prevent the map from displaying.
                let _ = tokio::task::spawn_blocking(move || {
                    crate::common::atomic_file::write_atomic(&disk_path, &disk_bytes)
                })
                .await;
                if self.writes.fetch_add(1, Ordering::Relaxed) % 64 == 63 {
                    let root = self.root.clone();
                    tokio::task::spawn_blocking(move || prune(&root));
                }
                Ok(bytes)
            }
            Err(_) if cached.is_some() => Ok(cached.unwrap()),
            Err(e) => Err(e),
        }
    }
}
fn prune(root: &Path) {
    prune_to(root, CACHE_LIMIT);
}
fn prune_to(root: &Path, limit: u64) {
    let mut stack = vec![root.to_path_buf()];
    let mut files = vec![];
    let mut size = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|v| v == "tile") {
                size += meta.len();
                files.push((
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    meta.len(),
                    path,
                ));
            }
        }
    }
    files.sort_by_key(|v| v.0);
    for (_, bytes, path) in files {
        if size <= limit {
            break;
        }
        if path.starts_with(root) && std::fs::remove_file(path).is_ok() {
            size = size.saturating_sub(bytes);
        }
    }
}
#[tauri::command]
pub async fn load_map_tile(
    state: State<'_, MapTiles>,
    request_id: String,
    provider: String,
    z: u32,
    x: u32,
    y: u32,
) -> Result<Response, String> {
    state
        .request(&request_id, &provider, z, x, y)
        .await
        .map(Response::new)
}
#[tauri::command]
pub async fn load_map_resource(
    state: State<'_, MapTiles>,
    request_id: String,
    resource: String,
) -> Result<Response, String> {
    state
        .request_bytes(&request_id, state.load_vector(&resource))
        .await
        .map(Response::new)
}
#[tauri::command]
pub fn cancel_map_tile(state: State<'_, MapTiles>, request_id: String) {
    state.cancel(&request_id);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vector_resources_include_tiles_glyphs_and_sprites_without_arbitrary_urls() {
        assert_eq!(
            vector_url("sar-vector://tiles/3/2/5.pbf").unwrap(),
            format!("{VECTOR_SERVICE}/tile/3/2/5.pbf")
        );
        assert!(
            vector_url("sar-vector://fonts/Arial%20Regular/0-255.pbf")
                .unwrap()
                .ends_with("/fonts/Arial%20Regular/0-255.pbf")
        );
        assert!(
            vector_url("sar-vector://sprites/sprite@2x.png")
                .unwrap()
                .ends_with("/sprites/sprite@2x.png")
        );
        for url in [
            "https://example.com/tile",
            "sar-vector://tiles/23/0/0.pbf",
            "sar-vector://tiles/2/0/4.pbf",
            "sar-vector://fonts/Arial/1-256.pbf",
            "sar-vector://fonts/%2e%2e%2fprivate/0-255.pbf",
            "sar-vector://sprites/anything.png",
            "sar-vector://sprites/sprite.png?token=1",
        ] {
            assert!(vector_url(url).is_err(), "{url}");
        }
        assert!(!valid_resource(b"<html>denied</html>", "tile.pbf"));
        assert!(!valid_resource(br#"{"error":{"code":403}}"#, "sprite.json"));
    }
    #[test]
    fn gzipped_vector_tiles_are_decoded_before_caching_and_rendering() {
        use std::io::Write;
        let raw = b"\x1a\x03\x0a\x01x";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(raw).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(!valid_resource(&compressed, "tile.pbf"));
        assert_eq!(decode_vector(compressed, "tile.pbf").unwrap(), raw);
        assert_eq!(decode_vector(raw.to_vec(), "tile.pbf").unwrap(), raw);
        assert!(decode_vector(vec![0x1f, 0x8b, 0], "tile.pbf").is_err());
    }
    #[tokio::test]
    #[ignore = "public ArcGIS vector tiles, glyphs and sprite integration; requires network"]
    async fn live_vector_resources_and_cache() {
        let root = std::env::temp_dir().join(format!(
            "sar-map-vector-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let service = MapTiles::new(root.clone()).unwrap();
        for resource in [
            "sar-vector://tiles/2/1/3.pbf",
            "sar-vector://fonts/Arial%20Regular/0-255.pbf",
            "sar-vector://fonts/Arial%20Unicode%20MS%20Bold/19968-20223.pbf",
            "sar-vector://sprites/sprite.json",
            "sar-vector://sprites/sprite.png",
            "sar-vector://sprites/sprite@2x.json",
            "sar-vector://sprites/sprite@2x.png",
        ] {
            let bytes = service.load_vector(resource).await.unwrap();
            assert!(!bytes.is_empty(), "{resource}");
            assert!(
                !bytes.starts_with(&[0x1f, 0x8b]),
                "gzip content must be decoded: {resource}"
            );
            assert_eq!(bytes, service.load_vector(resource).await.unwrap());
            println!("{resource}: {} bytes, cache verified", bytes.len());
        }
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn tile_sources_and_coordinates_are_bounded() {
        assert!(
            tile_url("imagery", 3, 5, 2)
                .unwrap()
                .ends_with("/tile/3/2/5")
        );
        assert!(
            tile_url("labels", 17, 1, 1)
                .unwrap()
                .contains("Reference/World_Boundaries_and_Places")
        );
        for (p, z, x, y) in [
            ("../bad", 1, 0, 0),
            ("osm", 20, 0, 0),
            ("imagery", 2, 4, 0),
            ("streets", 0, 0, 1),
        ] {
            assert!(tile_url(p, z, x, y).is_err());
        }
        assert!(!image(b"<html>access denied</html>"));
        assert!(image(b"\x89PNG\r\n\x1a\nfixture"));
    }
    #[tokio::test]
    async fn cancelled_tiles_leave_the_queue_and_do_not_require_network() {
        let mut service =
            MapTiles::new(std::env::temp_dir().join("sar-map-cancel-unused")).unwrap();
        service.slots = Arc::new(tokio::sync::Semaphore::new(0));
        service.cancel("early");
        assert_eq!(
            service.request("early", "osm", 0, 0, 0).await.unwrap_err(),
            "MAP_TILE_ABORTED"
        );
        let service = Arc::new(service);
        let worker = service.clone();
        let job = tokio::spawn(async move { worker.request("queued", "osm", 0, 0, 0).await });
        tokio::task::yield_now().await;
        service.cancel("queued");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), job)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err(),
            "MAP_TILE_ABORTED"
        );
        assert!(service.requests.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn valid_expired_tiles_survive_an_offline_refresh() {
        let root = std::env::temp_dir().join(format!(
            "sar-map-offline-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut service = MapTiles::new(root.clone()).unwrap();
        service.client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .connect_timeout(Duration::from_millis(50))
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();
        let path = service.root.join("imagery/2/3/1.tile");
        let bytes = include_bytes!("../../icons/32x32.png");
        crate::common::atomic_file::write_atomic(&path, bytes).unwrap();
        let old = SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        assert_eq!(service.load("imagery", 2, 3, 1).await.unwrap(), bytes);
        std::fs::write(&path, b"<html>bad tile</html>").unwrap();
        assert!(service.load("imagery", 2, 3, 1).await.is_err());
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cache_budget_evicts_old_tiles_without_touching_unrelated_files() {
        let root = std::env::temp_dir().join(format!(
            "sar-map-budget-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        for (name, time) in [("old.tile", 1), ("new.tile", 2)] {
            let path = root.join(name);
            std::fs::write(&path, b"tile").unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(
                    std::fs::FileTimes::new()
                        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(time)),
                )
                .unwrap();
        }
        std::fs::write(root.join("keep.txt"), b"user note").unwrap();
        prune_to(&root, 4);
        assert!(!root.join("old.tile").exists());
        assert!(root.join("new.tile").exists());
        assert!(root.join("keep.txt").exists());
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore = "public map tile integration; requires network"]
    async fn live_basemaps_and_cache() {
        let root = std::env::temp_dir().join(format!(
            "sar-map-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let service = MapTiles::new(root.clone()).unwrap();
        for provider in ["imagery", "streets", "labels", "osm"] {
            let bytes = service.load(provider, 2, 3, 1).await.unwrap();
            assert!(image(&bytes));
            let cached = service.load(provider, 2, 3, 1).await.unwrap();
            assert_eq!(bytes, cached);
            println!("{provider}: {} bytes, cache verified", bytes.len());
        }
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
