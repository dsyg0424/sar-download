use super::*;
use crate::modules::downloader::{
    engine::run_download_core,
    manager::DownloadManager,
    types::{DownloadFile, DownloadState},
};
use std::sync::{Arc, Mutex, atomic::AtomicU64};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn xml_metadata_requires_a_complete_xml_document_instead_of_an_error_response() {
    let root = directory();
    let path = root.join("burst.xml");
    let file = DownloadFile {
        name: "burst.xml".into(),
        url: "https://example.test/0.xml".into(),
        size: 0,
        sha256: String::new(),
        hash_type: None,
        hash_value: None,
        task_id: "xml".into(),
        mirrors: vec![],
    };
    for invalid in [
        "",
        "{\"message\":\"Accepted\"}",
        "<html>login</html>",
        "<error>denied</error>",
        "<burst><annotation>",
        "<burst></other>",
        "<burst/><extra/>",
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(
            verify(&path, invalid.len() as u64, &file, &AtomicBool::new(false))
                .unwrap_err()
                .contains("XML_INVALID"),
            "{invalid}"
        );
    }
    let xml = "<?xml version=\"1.0\"?><burst><manifest/><annotation>data</annotation></burst>";
    std::fs::write(&path, xml).unwrap();
    let verified = verify(&path, xml.len() as u64, &file, &AtomicBool::new(false)).unwrap();
    assert!(verified.checks.contains(&"xml_document".into()));
    assert!(
        verify(&path, xml.len() as u64, &file, &AtomicBool::new(true))
            .unwrap_err()
            .contains("CANCEL")
    );
    assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
    std::fs::remove_dir_all(root).unwrap();
}

static NEXT: AtomicU64 = AtomicU64::new(0);
fn directory() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sar-a2-{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
#[derive(Clone, Copy)]
enum Mode {
    Stable,
    Changed,
    Weak,
    Missing,
    Unauthorized,
    RateLimited,
    Interrupted,
    Html,
    Redirect,
    Unsatisfiable,
    Backoff,
    ServerError,
    DateValidator,
    RecentDate,
    UnverifiedInterrupted,
}
struct Fixture {
    root: std::path::PathBuf,
    file: DownloadFile,
    payload: Arc<Vec<u8>>,
    requests: Arc<Mutex<Vec<String>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/scene", listener.local_addr().unwrap());
        let payload = Arc::new(
            (0..3 * 1024 * 1024)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let (source, seen) = (payload.clone(), requests.clone());
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (source, seen) = (source.clone(), seen.clone());
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    loop {
                        let mut block = [0; 1024];
                        let Ok(n) = socket.read(&mut block).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&block[..n]);
                        if buffer.windows(4).any(|s| s == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8(buffer).unwrap();
                    let index = {
                        let mut seen = seen.lock().unwrap();
                        seen.push(request.clone());
                        seen.len()
                    };
                    let range = request
                        .lines()
                        .find_map(|line| line.strip_prefix("range: bytes="))
                        .unwrap();
                    let (start, end) = range.split_once('-').unwrap();
                    let (start, end) = (
                        start.parse::<usize>().unwrap(),
                        end.parse::<usize>().unwrap(),
                    );
                    let probe = start == 0 && end == 0;
                    let mut tag = "ETag: \"stable\"\r\n";
                    if matches!(mode, Mode::Weak) {
                        tag = "ETag: W/\"weak\"\r\n";
                    }
                    if matches!(mode, Mode::Missing | Mode::UnverifiedInterrupted) {
                        tag = "";
                    }
                    if matches!(mode, Mode::DateValidator) {
                        tag = "Last-Modified: Mon, 21 Sep 2026 00:00:00 GMT\r\nDate: Mon, 21 Sep 2026 00:01:00 GMT\r\n";
                    }
                    if matches!(mode, Mode::RecentDate) {
                        tag = "Last-Modified: Mon, 21 Sep 2026 00:00:00 GMT\r\nDate: Mon, 21 Sep 2026 00:00:59 GMT\r\n";
                    }
                    if matches!(mode, Mode::Changed) && !probe {
                        tag = "ETag: \"changed\"\r\n";
                    }
                    let special = if matches!(mode, Mode::Backoff) {
                        Some("429 Too Many Requests\r\nRetry-After: 20")
                    } else if matches!(mode, Mode::ServerError) && !probe {
                        Some("503 Service Unavailable\r\nRetry-After: 0")
                    } else if matches!(mode, Mode::RateLimited) && index == 2 {
                        Some("429 Too Many Requests\r\nRetry-After: 0")
                    } else if matches!(mode, Mode::Unauthorized) && !probe {
                        Some("401 Unauthorized")
                    } else if matches!(mode, Mode::Unsatisfiable) && !probe {
                        Some("416 Range Not Satisfiable")
                    } else if matches!(mode, Mode::Redirect)
                        && !probe
                        && !request.starts_with("GET /other ")
                    {
                        Some("302 Found\r\nLocation: /other")
                    } else {
                        None
                    };
                    if let Some(status) = special {
                        let _=socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await;
                        return;
                    }
                    let content_type = if matches!(mode, Mode::Html) && !probe {
                        "text/html"
                    } else {
                        "application/octet-stream"
                    };
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\n{tag}Content-Type: {content_type}\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        source.len(),
                        end - start + 1
                    );
                    if socket.write_all(header.as_bytes()).await.is_err() {
                        return;
                    }
                    let send_end =
                        if matches!(mode, Mode::Interrupted | Mode::UnverifiedInterrupted)
                            && index == 2
                        {
                            start + 1536 * 1024 - 1
                        } else {
                            end
                        };
                    let _ = socket.write_all(&source[start..=send_end]).await;
                });
            }
        });
        Self {
            root: directory(),
            file: DownloadFile {
                name: "scene.bin".into(),
                url,
                size: 1,
                sha256: String::new(),
                hash_type: None,
                hash_value: None,
                task_id: "test".into(),
                mirrors: vec![],
            },
            payload,
            requests,
            server,
        }
    }
    async fn run(&self, cancel: Option<Arc<AtomicBool>>) -> (String, Option<String>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        tokio::time::timeout(
            Duration::from_secs(15),
            run_download_core(
                Arc::new(DownloadManager::new()),
                self.file.clone(),
                self.root.to_string_lossy().into(),
                self.root.join("metadata"),
                String::new(),
                String::new(),
                4,
                None,
                cancel,
                None,
                move |_, _, _, status, error| {
                    seen.lock().unwrap().push((status.to_owned(), error));
                },
            ),
        )
        .await
        .unwrap();
        let last = events.lock().unwrap().last().unwrap().clone();
        last
    }
    async fn checkpoint(&self, changed: bool) {
        let response = reqwest::Client::new()
            .get(&self.file.url)
            .header(RANGE, "bytes=0-0")
            .send()
            .await
            .unwrap();
        let mut identity =
            RemoteIdentity::from_response(&self.file.url, &response, self.payload.len() as u64);
        if changed {
            identity.etag = Some("\"old\"".into());
        }
        let mut state = DownloadState::new_fresh(
            "test",
            "scene.bin",
            &self.file.url,
            self.payload.len() as u64,
            &self.file,
            4,
        );
        state.remote_identity = Some(identity);
        state.chunks[0].current = 1024 * 1024;
        crate::common::atomic_file::write_atomic(
            &self.root.join("metadata/state.json"),
            &serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        std::fs::write(
            self.root.join("scene.bin.downloading"),
            &self.payload[..1024 * 1024],
        )
        .unwrap();
        self.requests.lock().unwrap().clear();
    }
}

#[tokio::test]
async fn date_validator_requires_sixty_second_margin() {
    for mode in [Mode::DateValidator, Mode::RecentDate] {
        let fixture = Fixture::new(mode).await;
        fixture.checkpoint(false).await;
        assert_eq!(fixture.run(None).await.0, "Completed");
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(
            requests[1].contains("if-unmodified-since:"),
            matches!(mode, Mode::DateValidator)
        );
        assert_eq!(
            requests[1].contains("range: bytes=1048576-"),
            matches!(mode, Mode::DateValidator)
        );
    }
}

#[tokio::test]
async fn retry_budget_is_bounded_and_unverified_stream_cannot_be_appended() {
    for mode in [Mode::ServerError, Mode::UnverifiedInterrupted] {
        let fixture = Fixture::new(mode).await;
        assert_eq!(fixture.run(None).await.0, "Failed");
        assert!(!fixture.root.join("scene.bin").exists());
        assert_eq!(
            fixture.requests.lock().unwrap().len(),
            if matches!(mode, Mode::ServerError) {
                5
            } else {
                2
            }
        );
    }
}

#[tokio::test]
async fn stable_identity_resumes_and_changed_same_length_is_isolated() {
    for changed in [false, true] {
        let fixture = Fixture::new(Mode::Stable).await;
        fixture.checkpoint(changed).await;
        assert_eq!(fixture.run(None).await.0, "Completed");
        assert_eq!(
            std::fs::read(fixture.root.join("scene.bin")).unwrap(),
            *fixture.payload
        );
        let requests = fixture.requests.lock().unwrap();
        assert!(requests[1].contains(if changed {
            "range: bytes=0-"
        } else {
            "range: bytes=1048576-"
        }));
        assert!(requests[1].contains("if-match: \"stable\""));
        let rejected = std::fs::read_dir(&fixture.root)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with("rejected-"));
        if changed {
            assert_eq!(
                std::fs::read(rejected.unwrap().path().join("payload.part")).unwrap(),
                fixture.payload[..1024 * 1024]
            );
        } else {
            assert!(rejected.is_none());
        }
        let report: Verification = serde_json::from_slice(
            &std::fs::read(fixture.root.join("metadata/verification.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report.level, "local_hash");
    }
}
#[tokio::test]
async fn unsafe_response_never_publishes_and_auth_range_errors_do_not_retry() {
    for mode in [
        Mode::Changed,
        Mode::Unauthorized,
        Mode::Html,
        Mode::Redirect,
        Mode::Unsatisfiable,
    ] {
        let fixture = Fixture::new(mode).await;
        let result = fixture.run(None).await;
        assert_eq!(result.0, "Failed", "{result:?}");
        assert!(!fixture.root.join("scene.bin").exists());
        if matches!(mode, Mode::Unauthorized | Mode::Unsatisfiable) {
            assert_eq!(fixture.requests.lock().unwrap().len(), 2);
        }
    }
}
#[tokio::test]
async fn missing_or_weak_identity_restarts_whole_and_never_reuses_prefix() {
    for mode in [Mode::Weak, Mode::Missing] {
        let fixture = Fixture::new(mode).await;
        fixture.checkpoint(false).await;
        assert_eq!(fixture.run(None).await.0, "Completed");
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains("range: bytes=0-3145727"));
        assert!(!requests[1].contains("if-match:"));
    }
}
#[tokio::test]
async fn rate_limit_and_interrupted_body_retry_with_identity() {
    for mode in [Mode::RateLimited, Mode::Interrupted] {
        let fixture = Fixture::new(mode).await;
        assert_eq!(fixture.run(None).await.0, "Completed");
        assert_eq!(
            std::fs::read(fixture.root.join("scene.bin")).unwrap(),
            *fixture.payload
        );
        assert_eq!(fixture.requests.lock().unwrap().len(), 3);
    }
}
#[tokio::test]
async fn cancellation_interrupts_retry_after_wait() {
    let fixture = Fixture::new(Mode::Backoff).await;
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let requests = fixture.requests.clone();
    let trigger = tokio::spawn(async move {
        while requests.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        flag.store(true, Ordering::SeqCst);
    });
    let result = tokio::time::timeout(Duration::from_secs(2), fixture.run(Some(cancel)))
        .await
        .unwrap();
    trigger.await.unwrap();
    assert_eq!(result.0, "Paused");
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn supplier_hash_is_verified_before_publication() {
    for valid in [false, true] {
        let mut fixture = Fixture::new(Mode::Stable).await;
        fixture.file.hash_type = Some("MD5".into());
        fixture.file.hash_value = Some(if valid {
            format!("{:x}", md5::compute(&*fixture.payload))
        } else {
            "0".repeat(32)
        });
        let result = fixture.run(None).await;
        assert_eq!(result.0, if valid { "Completed" } else { "Failed" });
        assert_eq!(fixture.root.join("scene.bin").exists(), valid);
        if valid {
            let report: Verification = serde_json::from_slice(
                &std::fs::read(fixture.root.join("metadata/verification.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(report.level, "supplier_checksum");
        }
    }
}
#[test]
fn checksum_size_and_retry_policy_are_explicit() {
    let root = directory();
    let path = root.join("payload");
    std::fs::write(&path, b"abc").unwrap();
    let mut file = DownloadFile {
        name: "scene".into(),
        url: String::new(),
        size: 0,
        sha256: String::new(),
        hash_type: None,
        hash_value: None,
        task_id: String::new(),
        mirrors: vec![],
    };
    let cancel = AtomicBool::new(false);
    file.sha256 = digest(b"abc");
    assert_eq!(
        verify(&path, 3, &file, &cancel).unwrap().level,
        "supplier_checksum"
    );
    file.sha256 = digest(b"abd");
    assert!(
        verify(&path, 3, &file, &cancel)
            .unwrap_err()
            .contains("HASH_MISMATCH")
    );
    file.sha256.clear();
    file.hash_type = Some("md5".into());
    file.hash_value = Some(format!("{:x}", md5::compute(b"abc")));
    let mut progress = Vec::new();
    let md5_report = verify_with_progress(&path, 3, &file, &cancel, |n| progress.push(n)).unwrap();
    assert!(md5_report.sha256.is_empty());
    assert_eq!(md5_report.digest().algorithm, "md5");
    assert_eq!(progress, vec![0, 3]);
    file.hash_value = Some("0".repeat(32));
    progress.clear();
    assert!(verify_with_progress(&path, 3, &file, &cancel, |n| progress.push(n)).is_err());
    assert!(!progress.contains(&3));
    // Stronger supplied SHA-256 takes precedence over the MD5 field.
    file.sha256 = digest(b"abc");
    assert_eq!(
        verify(&path, 3, &file, &cancel).unwrap().digest().algorithm,
        "sha256"
    );
    file.sha256.clear();
    file.hash_type = None;
    file.hash_value = None;
    assert_eq!(
        verify(&path, 3, &file, &cancel).unwrap().digest().value,
        digest(b"abc")
    );
    assert!(verify(&path, 4, &file, &cancel).is_err());
    progress.clear();
    let interrupted = verify_with_progress(&path, 3, &file, &cancel, |n| {
        progress.push(n);
        cancel.store(true, Ordering::SeqCst);
    });
    assert_eq!(interrupted.unwrap_err(), "TASK_CANCELLED");
    assert_eq!(progress, vec![0]);
    cancel.store(false, Ordering::SeqCst);
    file.hash_type = Some("unsupported".into());
    assert!(verify(&path, 3, &file, &cancel).is_err());
    let mut headers = HeaderMap::new();
    headers.insert(RETRY_AFTER, "2".parse().unwrap());
    assert!(retry_delay(&headers, 0).unwrap() >= Duration::from_secs(2));
    headers.insert(RETRY_AFTER, "61".parse().unwrap());
    assert!(retry_delay(&headers, 0).is_err());
    headers.insert(
        RETRY_AFTER,
        (chrono::Utc::now() + chrono::Duration::seconds(5))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string()
            .parse()
            .unwrap(),
    );
    assert!(retry_delay(&headers, 0).unwrap() >= Duration::from_secs(4));
    std::fs::remove_dir_all(root).unwrap();
}
