use super::{
    engine::{TransferMetrics, run_download_core},
    integrity,
    manager::DownloadManager,
    options::safe_component,
    types::DownloadFile,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;

fn burst_endpoint(file: &DownloadFile) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(&file.url).map_err(|_| "Burst 下载地址无效")?;
    let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
    if !crate::modules::sar_search::product::allowed_source(&url)
        || url.host_str() != Some("sentinel1-burst.asf.alaska.edu")
        || parts.len() != 4
        || !parts[0].starts_with("S1")
        || !["IW1", "IW2", "IW3", "EW1", "EW2", "EW3", "EW4", "EW5"].contains(&parts[1])
        || !["VV", "VH", "HH", "HV"].contains(&parts[2])
        || !parts[3]
            .strip_suffix(if file.name.ends_with(".tiff") {
                ".tiff"
            } else {
                ".zip"
            })
            .and_then(|s| s.parse::<u32>().ok())
            .is_some_and(|i| i < 1500)
        || !safe_component(&file.name)
        || !(file.name.ends_with(".zip") || file.name.ends_with(".tiff"))
    {
        return Err("无法确定与 Burst 匹配的 TIFF/XML 地址".into());
    }
    Ok(url)
}

/// Normalize cached ZIP search results as well as new TIFF results before planning.
pub fn burst_tiff(mut file: DownloadFile) -> Result<DownloadFile, String> {
    let mut url = burst_endpoint(&file)?;
    if file.name.ends_with(".zip") {
        file.name = format!("{}.tiff", file.name.strip_suffix(".zip").unwrap());
        let path = format!("{}.tiff", url.path().strip_suffix(".zip").unwrap());
        url.set_path(&path);
        file.url = url.into();
        // ZIP lengths and checksums never describe the extracted TIFF.
        file.size = 0;
        file.sha256.clear();
        file.hash_type = None;
        file.hash_value = None;
        file.mirrors.clear();
    }
    Ok(file)
}

pub fn burst_xml(file: &DownloadFile) -> Result<DownloadFile, String> {
    let mut url = burst_endpoint(file)?;
    let parts: Vec<_> = url
        .path()
        .trim_start_matches('/')
        .split('/')
        .map(str::to_owned)
        .collect();
    let (path, name) = if file.name.ends_with(".tiff") {
        // Extractor XML contains all swaths for one source SLC and polarization.
        // burst2safe expects exactly <source SLC>_<polarization>.xml. Canonicalize
        // the endpoint too, so different burst indexes/swaths share one resource.
        let swath = format!("{}1", &parts[1][..2]);
        (
            format!("/{}/{}/{}/0.xml", parts[0], swath, parts[2]),
            format!("{}_{}.xml", parts[0], parts[2]),
        )
    } else {
        // Preserve completed legacy ZIP history and its original companion name.
        (
            format!("{}.xml", url.path().strip_suffix(".zip").unwrap()),
            format!("{}.xml", file.name.strip_suffix(".zip").unwrap()),
        )
    };
    if !safe_component(&name) {
        return Err("Burst 元数据文件名无效".into());
    }
    url.set_path(&path);
    Ok(DownloadFile {
        name,
        url: url.into(),
        size: 0,
        sha256: String::new(),
        hash_type: None,
        hash_value: None,
        task_id: format!("{}-xml", file.task_id),
        mirrors: vec![],
    })
}

type OutputLock = tokio::sync::Mutex<()>;
fn output_lock(target: &Path) -> Arc<OutputLock> {
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<PathBuf, Weak<OutputLock>>>> =
        OnceLock::new();
    let mut locks = LOCKS.get_or_init(Mutex::default).lock().unwrap();
    locks.retain(|_, lock| lock.strong_count() > 0);
    // Service paths are canonical. Normalize casing for Windows' file identity.
    #[cfg(windows)]
    let key = PathBuf::from(target.to_string_lossy().to_lowercase());
    #[cfg(not(windows))]
    let key = target.to_path_buf();
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(OutputLock::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

#[derive(Clone)]
pub struct Progress {
    pub file: String,
    pub received: u64,
    pub total: u64,
    pub verified: u64,
    pub speed: f64,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Default, Clone)]
struct FileProgress {
    received: u64,
    total: u64,
    verified: u64,
    speed: f64,
    status: String,
    error: Option<String>,
}

/// Start TIFF and shared XML together, within the existing file/request budgets.
pub async fn run<F>(
    manager: Arc<DownloadManager>,
    files: Vec<(DownloadFile, PathBuf)>,
    directory: String,
    username: String,
    password: String,
    concurrency: u32,
    cancel: Arc<AtomicBool>,
    validate_burst: bool,
    report: F,
) where
    F: Fn(Progress) + Send + Sync + Clone + 'static,
{
    let progress = Arc::new(Mutex::new(vec![FileProgress::default(); files.len()]));
    let inputs = files.clone();
    let work = files
        .into_iter()
        .enumerate()
        .map(|(index, (file, checkpoint))| {
            let (manager, directory, username, password, cancel, states, report) = (
                manager.clone(),
                directory.clone(),
                username.clone(),
                password.clone(),
                cancel.clone(),
                progress.clone(),
                report.clone(),
            );
            async move {
                let name = file.name.clone();
                let metrics = Arc::new(TransferMetrics::default());
                let observed = metrics.clone();
                run_file(
                    manager,
                    file,
                    directory,
                    checkpoint,
                    username,
                    password,
                    concurrency,
                    cancel,
                    metrics,
                    move |received, total, speed, status, error| {
                        let mut states = states.lock().unwrap();
                        states[index] = FileProgress {
                            received,
                            total,
                            verified: observed.verified.load(Ordering::SeqCst),
                            speed,
                            status: status.into(),
                            error: error.clone(),
                        };
                        // A failed XML must not discard an independent TIFF
                        // transfer. Preserve completed work for a metadata retry.
                        let received = states.iter().map(|s| s.received).sum();
                        let total = states.iter().map(|s| s.total).sum();
                        let verified = states
                            .iter()
                            .map(|s| {
                                if s.status == "Completed" {
                                    s.total
                                } else {
                                    s.verified
                                }
                            })
                            .sum();
                        let speed = states
                            .iter()
                            .filter(|s| s.status == "Downloading")
                            .map(|s| s.speed)
                            .sum();
                        report(Progress {
                            file: name.clone(),
                            received,
                            total,
                            verified,
                            speed,
                            status: if status == "Failed"
                                && states.iter().any(|state| {
                                    !matches!(
                                        state.status.as_str(),
                                        "Completed" | "Failed" | "Paused"
                                    )
                                }) {
                                states
                                    .iter()
                                    .find(|state| {
                                        !state.status.is_empty()
                                            && !matches!(
                                                state.status.as_str(),
                                                "Completed" | "Failed" | "Paused"
                                            )
                                    })
                                    .map(|state| state.status.clone())
                                    .unwrap_or_else(|| "Queued".into())
                            } else if status == "Completed" {
                                "Publishing".into()
                            } else {
                                status.into()
                            },
                            error: error.map(|error| format!("{name}: {error}")),
                        });
                    },
                )
                .await;
            }
        });
    futures_util::future::join_all(work).await;
    let states = progress.lock().unwrap().clone();
    let total = states.iter().map(|s| s.total).sum();
    let received = states.iter().map(|s| s.received).sum();
    let file = inputs
        .first()
        .map(|(file, _)| file.name.clone())
        .unwrap_or_default();
    if let Some(failed) = states.iter().find(|s| s.status == "Failed") {
        report(Progress {
            file,
            received,
            total,
            verified: 0,
            speed: 0.0,
            status: "Failed".into(),
            error: failed.error.clone(),
        });
        return;
    }
    if cancel.load(Ordering::SeqCst) || states.iter().any(|s| s.status != "Completed") {
        report(Progress {
            file,
            received,
            total,
            verified: 0,
            speed: 0.0,
            status: "Paused".into(),
            error: None,
        });
        return;
    }
    if validate_burst {
        report(Progress {
            file: file.clone(),
            received,
            total,
            verified: total,
            speed: 0.0,
            status: "Verifying".into(),
            error: None,
        });
        let validation_cancel = cancel.clone();
        let target = PathBuf::from(&directory);
        let pair = inputs.clone();
        let result = tokio::task::spawn_blocking(move || {
            let (data, checkpoint) = pair.first().ok_or("Burst 缺少 TIFF")?;
            let (metadata, _) = pair.get(1).ok_or("Burst 缺少 XML")?;
            let mapping =
                super::structure::burst_pair(&target, data, metadata, &validation_cancel)?;
            crate::common::atomic_file::write_atomic(
                &checkpoint.join("structure.json"),
                &serde_json::to_vec(&mapping).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result);
        if let Err(error) = result {
            report(Progress {
                file,
                received,
                total,
                verified: total,
                speed: 0.0,
                status: if error.contains("CANCEL") {
                    "Paused"
                } else {
                    "Failed"
                }
                .into(),
                error: Some(error),
            });
            return;
        }
    }
    report(Progress {
        file,
        received: total,
        total,
        verified: total,
        speed: 0.0,
        status: "Completed".into(),
        error: None,
    });
}

// Recover an already published output using this task's persisted digest. This
// lets an XML retry preserve a completed ZIP without trusting arbitrary files.
async fn run_file<F>(
    manager: Arc<DownloadManager>,
    file: DownloadFile,
    directory: String,
    checkpoint: PathBuf,
    username: String,
    password: String,
    concurrency: u32,
    cancel: Arc<AtomicBool>,
    metrics: Arc<TransferMetrics>,
    report: F,
) where
    F: Fn(u64, u64, f64, &str, Option<String>) + Send + Sync + Clone + 'static,
{
    let target = Path::new(&directory).join(&file.name);
    // Shared metadata has one writer/checkpoint. Waiting tasks remain cancellable
    // and recheck the published file against that checkpoint before reusing it.
    let lock = output_lock(&target);
    let mut announced_wait = false;
    let _guard = loop {
        if cancel.load(Ordering::SeqCst) {
            report(0, 0, 0.0, "Paused", None);
            return;
        }
        match tokio::time::timeout(std::time::Duration::from_millis(100), lock.lock()).await {
            Ok(guard) => break guard,
            Err(_) => {
                if !announced_wait {
                    report(0, file.size, 0.0, "Queued", None);
                    announced_wait = true;
                }
            }
        }
    };
    if !target.exists() {
        run_download_core(
            manager,
            file,
            directory,
            checkpoint,
            username,
            password,
            concurrency,
            None,
            Some(cancel),
            Some(metrics),
            report,
        )
        .await;
        return;
    }
    let result = async {
        let previous: integrity::Verification = serde_json::from_slice(
            &tokio::fs::read(checkpoint.join("verification.json"))
                .await
                .map_err(|_| "目标文件已存在且没有本任务的校验记录；不会覆盖已有文件")?,
        )
        .map_err(|_| "已有文件的校验记录损坏")?;
        let bytes = previous.bytes;
        let digest = previous.digest();
        let mut expected = file;
        expected.sha256.clear();
        expected.hash_type = Some(digest.algorithm);
        expected.hash_value = Some(digest.value);
        let progress = report.clone();
        tokio::task::spawn_blocking(move || {
            integrity::verify_with_progress(&target, bytes, &expected, &cancel, |done| {
                metrics.verified.store(done, Ordering::SeqCst);
                progress(done, bytes, 0.0, "Verifying", None);
            })
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok::<_, String>(bytes)
    }
    .await;
    match result {
        Ok(bytes) => report(bytes, bytes, 0.0, "Completed", None),
        Err(error) => report(
            0,
            0,
            0.0,
            if error.contains("CANCEL") {
                "Paused"
            } else {
                "Failed"
            },
            Some(error),
        ),
    }
}
