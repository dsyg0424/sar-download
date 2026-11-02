use super::*;
static NEXT_TEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn exercise(catalog_size: u64, range_support: bool, wrong_range: bool) {
    let payload = Arc::new(
        (0..12 * 1024 * 1024)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/scene", listener.local_addr().unwrap());
    let source = payload.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let source = source.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 1024];
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..n]);
                    if request.windows(4).any(|v| v == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                let range = request
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="));
                let (start, end) = if range_support {
                    let (a, b) = range.unwrap().split_once('-').unwrap();
                    (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap())
                } else {
                    (0, source.len() - 1)
                };
                let header = if range_support {
                    let reported_start = if wrong_range && end > 0 {
                        start + 1
                    } else {
                        start
                    };
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\n",
                        reported_start,
                        end,
                        source.len()
                    )
                } else {
                    "HTTP/1.1 200 OK\r\n".to_string()
                };
                let header = format!(
                    "{header}ETag: \"stable\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    end - start + 1
                );
                if socket.write_all(header.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(&source[start..=end]).await;
                }
            });
        }
    });
    let root = std::env::temp_dir().join(format!(
        "sar-transfer-{}-{}",
        NEXT_TEST.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let file = DownloadFile {
        name: "scene.bin".into(),
        url,
        size: catalog_size,
        sha256: String::new(),
        hash_type: None,
        hash_value: None,
        task_id: "test".into(),
        mirrors: vec![],
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let metrics = Arc::new(TransferMetrics::default());
    tokio::time::timeout(
        Duration::from_secs(30),
        run_download_core(
            Arc::new(DownloadManager::new()),
            file,
            root.to_string_lossy().into_owned(),
            root.join("metadata"),
            String::new(),
            String::new(),
            16,
            None,
            None,
            Some(metrics.clone()),
            move |bytes, total, _, status, error| {
                captured
                    .lock()
                    .unwrap()
                    .push((bytes, total, status.to_string(), error))
            },
        ),
    )
    .await
    .unwrap();
    server.abort();
    let events = events.lock().unwrap();
    let last = events.last().unwrap();
    if wrong_range {
        assert_eq!(last.2, "Failed", "{last:?}");
        assert!(!root.join("scene.bin").exists());
    } else {
        assert_eq!(last.2, "Completed", "{last:?}");
        assert_eq!(
            metrics.received.load(Ordering::SeqCst),
            payload.len() as u64
        );
        assert_eq!(metrics.durable.load(Ordering::SeqCst), payload.len() as u64);
        assert_eq!(
            (last.0, last.1),
            (payload.len() as u64, payload.len() as u64)
        );
        assert_eq!(std::fs::read(root.join("scene.bin")).unwrap(), *payload);
        assert!(events.iter().all(|event| event.0 <= event.1));
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn parallel_transfer_corrects_both_catalog_size_directions() {
    exercise(1, true, false).await;
    exercise(20 * 1024 * 1024, true, false).await;
}
#[tokio::test]
async fn ignored_range_uses_single_full_object_transfer() {
    exercise(1, false, false).await;
}
#[tokio::test]
async fn mismatched_range_is_never_published() {
    exercise(1, true, true).await;
}
