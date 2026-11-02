use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tauri::ipc::Response;

const MAX_INPUT: usize = 20 * 1024 * 1024;

#[derive(Serialize)]
struct Part {
    name: String,
    size: usize,
}

fn extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case(expected))
}

fn projection_for(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("边界文件没有有效的所在目录")?;
    let stem = path.file_stem().ok_or("边界文件名无效")?.to_string_lossy();
    let mut candidates = Vec::new();
    for entry in fs::read_dir(parent).map_err(|e| format!("无法读取边界所在目录：{e}"))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let candidate = entry.path();
        if extension(&candidate, "prj")
            && candidate
                .file_stem()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(&stem))
            && candidate.is_file()
        {
            candidates.push(candidate);
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err("没有找到同目录下的同名 PRJ 文件，请将投影文件放在 SHP 旁后重新选择".into()),
        _ => Err("存在多个同名 PRJ 文件，无法确定投影定义，请保留唯一的配套文件".into()),
    }
}

fn read_part(path: &Path, remaining: usize) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| format!("无法读取边界文件：{e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("请选择边界文件".into());
    }
    if metadata.len() > remaining as u64 {
        return Err("边界文件总大小不能超过 20 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(remaining as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > remaining {
        return Err("边界文件总大小不能超过 20 MiB".into());
    }
    Ok(bytes)
}

fn boundary_payload(path: &Path) -> Result<Vec<u8>, String> {
    if !["geojson", "json", "kml", "kmz", "zip", "shp"]
        .iter()
        .any(|ext| extension(path, ext))
    {
        return Err("请选择 GeoJSON、KML、KMZ、Shapefile ZIP 或 SHP 文件".into());
    }
    let mut paths = vec![path.to_path_buf()];
    if extension(path, "shp") {
        paths.push(projection_for(path)?);
    }
    let mut parts = Vec::new();
    let mut data = Vec::new();
    for path in paths {
        let bytes = read_part(&path, MAX_INPUT - data.len())?;
        parts.push(Part {
            name: path
                .file_name()
                .ok_or("边界文件名无效")?
                .to_string_lossy()
                .into_owned(),
            size: bytes.len(),
        });
        data.extend_from_slice(&bytes);
    }
    let header = serde_json::to_vec(&parts).map_err(|e| e.to_string())?;
    // Length-prefixed metadata + raw file bytes avoids large JSON byte arrays.
    let mut response = Vec::with_capacity(4 + header.len() + data.len());
    response.extend_from_slice(&(header.len() as u32).to_le_bytes());
    response.extend_from_slice(&header);
    response.extend_from_slice(&data);
    Ok(response)
}

#[tauri::command]
pub async fn choose_boundary_file() -> Result<Response, String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("导入检索边界")
        .add_filter(
            "支持的边界文件",
            &["geojson", "json", "kml", "kmz", "zip", "shp"],
        )
        .add_filter("Shapefile（自动读取同名 PRJ）", &["shp"])
        .add_filter("GeoJSON", &["geojson", "json"])
        .add_filter("Google Earth", &["kml", "kmz"])
        .add_filter("Shapefile ZIP", &["zip"])
        .pick_file()
        .await;
    let Some(file) = file else {
        return Ok(Response::new(Vec::<u8>::new()));
    };
    let path = file.path().to_path_buf();
    tokio::task::spawn_blocking(move || boundary_payload(&path))
        .await
        .map_err(|e| e.to_string())?
        .map(Response::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "sar-boundary-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn write(&self, name: &str, data: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, data).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            for entry in fs::read_dir(&self.0).unwrap() {
                fs::remove_file(entry.unwrap().path()).unwrap();
            }
            fs::remove_dir(&self.0).unwrap();
        }
    }

    #[test]
    fn single_shp_reads_case_insensitive_same_name_prj_without_dbf() {
        let f = Fixture::new();
        let shp = f.write("边界.SHP", b"shape bytes");
        f.write("边界.PRJ", b"projection bytes");
        f.write("other.prj", b"unrelated");
        let payload = boundary_payload(&shp).unwrap();
        let len = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
        let header: serde_json::Value = serde_json::from_slice(&payload[4..4 + len]).unwrap();
        assert_eq!(header[0]["name"], "边界.SHP");
        assert_eq!(header[1]["name"], "边界.PRJ");
        assert_eq!(&payload[4 + len..], b"shape bytesprojection bytes");
    }

    #[test]
    fn missing_prj_does_not_use_other_projection_or_parent_directory() {
        let f = Fixture::new();
        let shp = f.write("area.shp", b"shape");
        f.write("different.prj", b"wrong");
        assert!(
            boundary_payload(&shp)
                .unwrap_err()
                .contains("同目录下的同名 PRJ")
        );
    }

    #[test]
    fn geojson_is_read_without_a_projection_sidecar() {
        let f = Fixture::new();
        let path = f.write("area.geojson", b"{}");
        let payload = boundary_payload(&path).unwrap();
        let len = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
        assert_eq!(&payload[4 + len..], b"{}");
        assert!(boundary_payload(&f.write("area.exe", b"bad")).is_err());
    }

    #[test]
    fn oversized_selection_is_rejected_before_ipc() {
        let f = Fixture::new();
        let path = f.write("large.geojson", b"");
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_INPUT as u64 + 1)
            .unwrap();
        assert!(boundary_payload(&path).unwrap_err().contains("20 MiB"));
    }
}
