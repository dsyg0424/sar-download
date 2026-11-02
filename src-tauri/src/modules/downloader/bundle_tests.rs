use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn file(name: &str, url: String, id: &str) -> DownloadFile {
    DownloadFile {
        name: name.into(),
        url,
        task_id: id.into(),
        size: 0,
        sha256: String::new(),
        hash_type: None,
        hash_value: None,
        mirrors: vec![],
    }
}

#[test]
fn burst_tiff_metadata_names_match_burst2safe_and_share_a_canonical_endpoint() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../../tests/fixtures/cmr/BURST.json")).unwrap();
    let item = &fixture["items"][0];
    let product = crate::modules::sar_search::product::normalize(item, "BURST", None).unwrap();
    let mut tiff = file(&product.name, product.url, "data");
    tiff.hash_type = Some("sha256".into());
    tiff.hash_value = Some("a".repeat(64));
    tiff.size = 100;
    let xml = burst_xml(&tiff).unwrap();
    let source = item["umm"]["InputGranules"][0]
        .as_str()
        .unwrap()
        .strip_suffix("-SLC")
        .unwrap();
    assert_eq!(
        xml.url,
        format!("https://sentinel1-burst.asf.alaska.edu/{source}/IW1/VV/0.xml")
    );
    assert_eq!(xml.name, format!("{source}_VV.xml"));
    assert_eq!(xml.size, 0);
    assert!(xml.hash_type.is_none() && xml.hash_value.is_none() && xml.sha256.is_empty());
    tiff.url = tiff.url.replace("/IW3/VV/0.tiff", "/IW2/VV/9.tiff");
    assert_eq!(burst_xml(&tiff).unwrap().url, xml.url);
    assert_eq!(burst_xml(&tiff).unwrap().name, xml.name);
    tiff.url = tiff.url.replace("/VV/", "/VH/");
    assert_ne!(burst_xml(&tiff).unwrap().name, xml.name);
    for url in [
        "https://datapool.asf.alaska.edu/scene.zip",
        "https://sentinel1-burst.asf.alaska.edu/S1A/IW1/VV/1500.tiff",
        "https://sentinel1-burst.asf.alaska.edu/S1A/IW9/VV/0.tiff",
    ] {
        tiff.url = url.into();
        assert!(burst_xml(&tiff).is_err());
    }
}

#[test]
fn cached_zip_search_results_request_tiff_without_zip_size_or_checksum() {
    let mut zip = file(
        "scene.zip",
        "https://sentinel1-burst.asf.alaska.edu/S1A/IW1/VV/3.zip".into(),
        "old",
    );
    zip.size = 123;
    zip.hash_type = Some("md5".into());
    zip.hash_value = Some("a".repeat(32));
    let tiff = burst_tiff(zip.clone()).unwrap();
    assert_eq!(tiff.name, "scene.tiff");
    assert!(tiff.url.ends_with("/3.tiff"));
    assert_eq!(tiff.size, 0);
    assert!(tiff.hash_type.is_none() && tiff.hash_value.is_none());
    assert_eq!(
        burst_xml(&zip).unwrap().name,
        "scene.xml",
        "legacy completed history retains its original XML"
    );
}

struct Fixture {
    root: PathBuf,
    base: String,
    server: tokio::task::JoinHandle<()>,
    bad_xml: Arc<AtomicBool>,
    tiff_requests: Arc<AtomicUsize>,
    xml_requests: Arc<AtomicUsize>,
    require_overlap: Arc<AtomicBool>,
    overlap_detected: Arc<AtomicBool>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sar-burst-pair-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let bad_xml = Arc::new(AtomicBool::new(false));
        let tiff_requests = Arc::new(AtomicUsize::new(0));
        let xml_requests = Arc::new(AtomicUsize::new(0));
        let require_overlap = Arc::new(AtomicBool::new(false));
        let overlap_detected = Arc::new(AtomicBool::new(false));
        let (require, detected) = (require_overlap.clone(), overlap_detected.clone());
        let (bad, seen, xml_seen) = (bad_xml.clone(), tiff_requests.clone(), xml_requests.clone());
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (bad, seen, xml_seen) = (bad.clone(), seen.clone(), xml_seen.clone());
                let (require, detected) = (require.clone(), detected.clone());
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    loop {
                        let mut block = [0; 1024];
                        let Ok(n) = socket.read(&mut block).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        request.extend_from_slice(&block[..n]);
                        if request.windows(4).any(|s| s == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request_line = String::from_utf8_lossy(&request);
                    let probe = request_line
                        .to_ascii_lowercase()
                        .contains("range: bytes=0-0\r\n");
                    let tiff = request_line.lines().next().unwrap_or("").contains(".tiff ");
                    if !tiff && !probe && request_line.starts_with("GET ") {
                        xml_seen.fetch_add(1, Ordering::SeqCst);
                    }
                    if tiff && !probe && require.load(Ordering::SeqCst) {
                        let started = std::time::Instant::now();
                        while xml_seen.load(Ordering::SeqCst) == 0
                            && started.elapsed() < std::time::Duration::from_millis(800)
                        {
                            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                        }
                        detected.store(xml_seen.load(Ordering::SeqCst) > 0, Ordering::SeqCst);
                    }
                    let tiff_bytes = super::super::structure::test_tiff(1, 1);
                    let body = if tiff {
                        if !probe {
                            seen.fetch_add(1, Ordering::SeqCst);
                        }
                        tiff_bytes.as_slice()
                    } else if bad.load(Ordering::SeqCst) {
                        b"<html>not metadata</html>".as_slice()
                    } else {
                        b"<?xml version=\"1.0\"?><burst><manifest/><annotation>valid</annotation></burst>".as_slice()
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    if socket.write_all(header.as_bytes()).await.is_ok() {
                        let _ = socket.write_all(body).await;
                    }
                });
            }
        });
        Self {
            root,
            base,
            server,
            bad_xml,
            tiff_requests,
            xml_requests,
            require_overlap,
            overlap_detected,
        }
    }
    async fn run(&self, cancel: Arc<AtomicBool>, pause_after_tiff: bool) -> Vec<Progress> {
        self.run_named("scene", cancel, pause_after_tiff).await
    }
    async fn run_named(
        &self,
        name: &str,
        cancel: Arc<AtomicBool>,
        pause_after_tiff: bool,
    ) -> Vec<Progress> {
        let filename = format!("{name}.tiff");
        let files = vec![
            (
                file(&filename, format!("{}/{filename}", self.base), name),
                self.root.join("checkpoint").join(name),
            ),
            (
                file("scene.xml", format!("{}/scene.xml", self.base), "metadata"),
                self.root.join("checkpoint/xml"),
            ),
        ];
        for (_, path) in &files {
            crate::common::atomic_file::write_atomic(
                &path.join("options.json"),
                &serde_json::to_vec(&super::super::options::TaskOptions {
                    schema_version: 1,
                    save_dir: self.root.to_string_lossy().into(),
                    concurrency: 2,
                    auto_orbit: false,
                })
                .unwrap(),
            )
            .unwrap();
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let capture = events.clone();
        let flag = cancel.clone();
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            run(
                Arc::new(DownloadManager::new()),
                files,
                self.root.to_string_lossy().into(),
                String::new(),
                String::new(),
                2,
                cancel,
                false,
                move |event| {
                    if pause_after_tiff && event.file == filename && event.status == "Publishing" {
                        flag.store(true, Ordering::SeqCst);
                    }
                    capture.lock().unwrap().push(event);
                },
            ),
        )
        .await
        .unwrap();
        Arc::try_unwrap(events).ok().unwrap().into_inner().unwrap()
    }
}

#[tokio::test]
async fn metadata_transfer_starts_before_tiff_has_finished() {
    let fixture = Fixture::new().await;
    fixture.require_overlap.store(true, Ordering::SeqCst);
    let events = fixture.run(Arc::new(AtomicBool::new(false)), false).await;
    assert_eq!(events.last().unwrap().status, "Completed");
    assert!(fixture.overlap_detected.load(Ordering::SeqCst));
    assert_eq!(fixture.xml_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn concurrent_bursts_download_their_own_tiffs_but_transfer_shared_metadata_once() {
    let fixture = Fixture::new().await;
    let (a, b) = tokio::join!(
        fixture.run_named("scene", Arc::new(AtomicBool::new(false)), false),
        fixture.run_named("other", Arc::new(AtomicBool::new(false)), false),
    );
    assert_eq!(a.last().unwrap().status, "Completed");
    assert_eq!(b.last().unwrap().status, "Completed");
    assert_eq!(fixture.tiff_requests.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.xml_requests.load(Ordering::SeqCst), 1);
    assert!(fixture.root.join("scene.tiff").exists() && fixture.root.join("other.tiff").exists());
    assert!(fixture.root.join("scene.xml").exists());
    assert!(!fixture.root.join("scene.zip").exists());
    // Reuse still checks the saved digest: a locally modified XML cannot pass.
    std::fs::write(fixture.root.join("scene.xml"), b"<burst>changed</burst>").unwrap();
    let events = fixture.run(Arc::new(AtomicBool::new(false)), false).await;
    assert_eq!(events.last().unwrap().status, "Failed");
    assert_eq!(fixture.xml_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn waiting_for_shared_metadata_can_be_cancelled_without_releasing_the_writer() {
    let fixture = Fixture::new().await;
    let lock = output_lock(&fixture.root.join("scene.xml"));
    let guard = lock.lock().await;
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let capture = events.clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        run_file(
            Arc::new(DownloadManager::new()),
            file(
                "scene.xml",
                format!("{}/scene.xml", fixture.base),
                "metadata",
            ),
            fixture.root.to_string_lossy().into(),
            fixture.root.join("checkpoint/xml"),
            String::new(),
            String::new(),
            2,
            cancel,
            Arc::new(TransferMetrics::default()),
            move |_, _, _, status, _| {
                capture.lock().unwrap().push(status.to_owned());
                flag.store(true, Ordering::SeqCst);
            },
        ),
    )
    .await
    .unwrap();
    assert_eq!(events.lock().unwrap().last().unwrap(), "Paused");
    assert!(lock.try_lock().is_err());
    assert_eq!(fixture.xml_requests.load(Ordering::SeqCst), 0);
    drop(guard);
}

#[tokio::test]
async fn tiff_and_xml_complete_together_and_xml_failure_reuses_verified_tiff() {
    let fixture = Fixture::new().await;
    fixture.bad_xml.store(true, Ordering::SeqCst);
    let events = fixture.run(Arc::new(AtomicBool::new(false)), false).await;
    assert_eq!(events.last().unwrap().status, "Failed");
    assert!(
        events
            .last()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("XML_INVALID")
    );
    assert!(!events.iter().any(|e| e.status == "Completed"));
    assert!(fixture.root.join("scene.tiff").is_file());
    assert!(!fixture.root.join("scene.xml").exists());
    let requests = fixture.tiff_requests.load(Ordering::SeqCst);
    fixture.bad_xml.store(false, Ordering::SeqCst);
    let events = fixture.run(Arc::new(AtomicBool::new(false)), false).await;
    assert_eq!(events.last().unwrap().status, "Completed");
    assert!(fixture.root.join("scene.xml").is_file());
    assert_eq!(fixture.tiff_requests.load(Ordering::SeqCst), requests);
    let total = std::fs::metadata(fixture.root.join("scene.tiff"))
        .unwrap()
        .len()
        + std::fs::metadata(fixture.root.join("scene.xml"))
            .unwrap()
            .len();
    assert_eq!(events.last().unwrap().received, total);
    assert_eq!(events.last().unwrap().total, total);
}

#[tokio::test]
async fn pause_during_parallel_pair_never_completes_and_resume_preserves_tiff() {
    let fixture = Fixture::new().await;
    let events = fixture.run(Arc::new(AtomicBool::new(false)), true).await;
    assert_eq!(events.last().unwrap().status, "Paused");
    assert!(!events.iter().any(|e| e.status == "Completed"));
    // XML may have completed before the TIFF because the files are parallel.
    let requests = fixture.tiff_requests.load(Ordering::SeqCst);
    assert_eq!(
        fixture
            .run(Arc::new(AtomicBool::new(false)), false)
            .await
            .last()
            .unwrap()
            .status,
        "Completed"
    );
    assert_eq!(fixture.tiff_requests.load(Ordering::SeqCst), requests);
}
