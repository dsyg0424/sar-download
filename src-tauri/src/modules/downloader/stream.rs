//! Full-response streaming without remote length; interruptions never append or resume.
#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
use super::{
    engine::TransferMetrics, integrity, manager::DownloadManager, space::SpaceMonitor,
    types::DownloadFile,
};
use futures_util::StreamExt;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

pub(super) async fn run<F>(
    manager: &DownloadManager,
    response: reqwest::Response,
    file: &DownloadFile,
    temporary: &Path,
    target: &Path,
    task_dir: &Path,
    cancel: Arc<AtomicBool>,
    metrics: Arc<TransferMetrics>,
    report: F,
) -> Result<u64, String>
where
    F: Fn(u64, u64, f64, &str, Option<String>) + Send + Sync + Clone + 'static,
{
    let directory = target.parent().ok_or("INVALID_OUTPUT_PATH")?.to_path_buf();
    let probe_path = directory.clone();
    let mut space = SpaceMonitor::new(&manager.disk_budget, None, move || {
        crate::common::disk_space::probe(&probe_path).map(|s| (s.volume, s.available))
    })?;
    if temporary.exists() {
        integrity::isolate(temporary, &task_dir.join("state.json"))?;
    }
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = response.bytes_stream();
    let mut bytes = 0u64;
    let mut synced = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut previous = Instant::now();
    let mut previous_bytes = 0;
    report(0, 0, 0.0, "Downloading", None);
    loop {
        let next = tokio::select! {
            _=integrity::cancelled(&cancel)=>return Err("TASK_CANCELLED".into()),
            _=tokio::time::sleep_until(deadline)=>return Err("STREAM_TIMEOUT: 响应中断，重试将从头下载".into()),
            _=tick.tick()=>{
                if bytes>synced { output.sync_all().await.map_err(|e|e.to_string())?; synced=bytes; metrics.durable.store(bytes,Ordering::SeqCst); }
                let elapsed=previous.elapsed().as_secs_f64();
                report(bytes,0,bytes.saturating_sub(previous_bytes) as f64/elapsed.max(0.001),"Downloading",None);
                previous=Instant::now(); previous_bytes=bytes;
                space.before_write(0)?;
                continue;
            },
            next=stream.next()=>next,
        };
        let Some(next) = next else {
            break;
        };
        let data = next.map_err(|_| "STREAM_INTERRUPTED: 响应未完整结束，重试将从头下载")?;
        metrics
            .received
            .fetch_add(data.len() as u64, Ordering::SeqCst);
        for block in data.chunks(64 * 1024) {
            tokio::select! { _=manager.shared_limiter.consume(block.len())=>{}, _=integrity::cancelled(&cancel)=>return Err("TASK_CANCELLED".into()) }
            space.before_write(block.len() as u64)?;
            output
                .write_all(block)
                .await
                .map_err(|e| format!("STREAM_WRITE_FAILED: {e}"))?;
            bytes = bytes
                .checked_add(block.len() as u64)
                .ok_or("STREAM_SIZE_OVERFLOW")?;
            space.after_write(block.len() as u64);
        }
        // Local throttling and disk writes are not network inactivity.
        deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    }
    output.sync_all().await.map_err(|e| e.to_string())?;
    drop(output);
    metrics.durable.store(bytes, Ordering::SeqCst);
    if bytes == 0 {
        return Err("EMPTY_RESPONSE: 远端未返回文件内容".into());
    }
    report(bytes, bytes, 0.0, "Verifying", None);
    let path = temporary.to_path_buf();
    let expected = file.clone();
    let flag = cancel.clone();
    let verify_report = report.clone();
    let mut verified = tokio::task::spawn_blocking(move || {
        integrity::verify_with_progress(&path, bytes, &expected, &flag, |done| {
            metrics.verified.store(done, Ordering::SeqCst);
            verify_report(bytes, bytes, 0.0, "Verifying", None);
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    // EOF is not a remotely declared size. Never claim a remote-length verification.
    verified.checks.retain(|check| check != "remote_length");
    verified.checks.push("response_eof".into());
    crate::common::atomic_file::write_atomic(
        &task_dir.join("verification.json"),
        &serde_json::to_vec_pretty(&verified).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::SeqCst) {
        return Err("TASK_CANCELLED".into());
    }
    report(bytes, bytes, 0.0, "Publishing", None);
    super::storage::publish(temporary, target).map_err(|e| format!("PUBLISH_FAILED: {e}"))?;
    if let Some(parent) = temporary.parent() {
        super::storage::cleanup_staging(parent, &directory);
    }
    report(bytes, bytes, 0.0, "Completed", None);
    Ok(bytes)
}
