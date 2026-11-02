use super::*;
use crate::modules::downloader::engine::run_download_core;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn exercise(mode: &str) {
    let data = Arc::new(vec![71u8; 2 * 1024 * 1024]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/scene", listener.local_addr().unwrap());
    let sent = data.clone();
    let server_mode = mode.to_string();
    let requests = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let count = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            let data = sent.clone();
            let mode = server_mode.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                loop {
                    let mut b = [0u8; 1024];
                    let n = socket.read(&mut b).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&b[..n]);
                    if request.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                }
                let header = match mode.as_str() {
                    "html" => "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n",
                    "range" => "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/*\r\n",
                    _ => "HTTP/1.1 200 OK\r\n",
                };
                if socket
                    .write_all(
                        format!("{header}Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
                            .as_bytes(),
                    )
                    .await
                    .is_err()
                {
                    return;
                }
                if mode != "empty" {
                    for bytes in data.chunks(65536) {
                        if socket
                            .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                        if socket.write_all(bytes).await.is_err() {
                            return;
                        }
                        if socket.write_all(b"\r\n").await.is_err() {
                            return;
                        }
                        if mode == "cancel" {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
                if mode != "truncated" {
                    let _ = socket.write_all(b"0\r\n\r\n").await;
                }
            });
        }
    });
    let root = std::env::temp_dir().join(format!(
        "sar-stream-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    std::fs::create_dir_all(root.join("metadata")).unwrap();
    std::fs::write(root.join("scene.bin.downloading"), b"untrusted-old-partial").unwrap();
    let file = DownloadFile {
        name: "scene.bin".into(),
        url,
        size: 999,
        sha256: if mode == "hash" {
            "0".repeat(64)
        } else {
            String::new()
        },
        hash_type: None,
        hash_value: None,
        task_id: "stream_test".into(),
        mirrors: vec![],
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let metrics = Arc::new(TransferMetrics::default());
    let flag = Arc::new(AtomicBool::new(false));
    let stop = flag.clone();
    let cancelling = mode == "cancel";
    tokio::time::timeout(
        Duration::from_secs(20),
        run_download_core(
            Arc::new(DownloadManager::new()),
            file,
            root.to_string_lossy().into(),
            root.join("metadata"),
            String::new(),
            String::new(),
            16,
            None,
            Some(flag),
            Some(metrics),
            move |bytes, total, _, status, error| {
                captured
                    .lock()
                    .unwrap()
                    .push((bytes, total, status.to_string(), error));
                if cancelling && bytes > 0 {
                    stop.store(true, Ordering::SeqCst);
                }
            },
        ),
    )
    .await
    .unwrap();
    server.abort();
    let events = events.lock().unwrap();
    let last = events.last().unwrap();
    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "stream must reuse probe response"
    );
    if mode == "ok" {
        assert_eq!(last.2, "Completed", "{last:?}");
        assert_eq!((last.0, last.1), (data.len() as u64, data.len() as u64));
        assert_eq!(std::fs::read(root.join("scene.bin")).unwrap(), *data);
        let report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("metadata/verification.json")).unwrap(),
        )
        .unwrap();
        assert!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == "response_eof")
        );
        assert!(
            !report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == "remote_length")
        );
        assert!(
            events
                .iter()
                .filter(|e| e.2 == "Downloading")
                .all(|e| e.1 == 0)
        );
        assert!(
            std::fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|e| e.file_name().to_string_lossy().starts_with("rejected-"))
        );
    } else {
        assert_eq!(
            last.2,
            if cancelling { "Paused" } else { "Failed" },
            "{last:?}"
        );
        assert!(!root.join("scene.bin").exists());
    }
    std::fs::remove_dir_all(&root).unwrap();
}
#[tokio::test]
async fn unknown_length_stream_uses_real_eof_size_and_never_appends() {
    exercise("ok").await;
}
#[tokio::test]
async fn truncated_empty_and_bad_hash_streams_never_publish() {
    exercise("truncated").await;
    exercise("empty").await;
    exercise("hash").await;
}
#[tokio::test]
async fn unknown_length_does_not_accept_html_or_malformed_ranges() {
    exercise("html").await;
    exercise("range").await;
}
#[tokio::test]
async fn unknown_length_transfer_cancels_without_publishing() {
    exercise("cancel").await;
}
