use super::{
    bundle,
    manager::DownloadManager,
    options::{TaskOptions, safe_component},
    types::DownloadFile,
};
use crate::{
    common::atomic_file::write_atomic,
    modules::{
        sar_search::product::{SarProduct, allowed_source},
        settings::{self, Settings},
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub file: DownloadFile,
    #[serde(default)]
    pub companion: Option<DownloadFile>,
    #[serde(default)]
    pub active_file: Option<String>,
    pub product: String,
    pub save_dir: String,
    pub status: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub verified_bytes: u64,
    pub error: Option<String>,
    pub revision: u64,
}
type Session = Option<([u8; 32], Arc<DownloadManager>)>;
pub struct DownloadService {
    pub(super) root: PathBuf,
    pub(super) tasks: Mutex<BTreeMap<String, Task>>,
    pub(super) active: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub(super) cleanup_plan: Mutex<Option<super::cleanup::CleanupPlan>>,
    session: Mutex<Session>,
}
pub fn running(status: &str) -> bool {
    [
        "Queued",
        "Connecting",
        "Downloading",
        "Verifying",
        "Publishing",
    ]
    .contains(&status)
}
impl DownloadService {
    pub fn load(root: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let path = root.join("tasks.json");
        let mut tasks: BTreeMap<String, Task> = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("任务文件损坏：{e}"))?
        } else {
            BTreeMap::new()
        };
        for task in tasks.values_mut() {
            if task.product == "BURST" && task.companion.is_none() {
                task.companion = Some(bundle::burst_xml(&task.file)?);
                if task.status == "Completed" {
                    task.status = "Ready".into();
                    task.error = Some("此 Burst 需要补充匹配的 XML，请继续下载".into());
                }
                task.revision += 1;
            }
            if task.product == "BURST" && task.file.name.ends_with(".tiff") {
                if let Some(xml) = &mut task.companion {
                    xml.task_id = task_id(Path::new(&task.save_dir), &xml.name, &xml.url);
                }
            }
            if running(&task.status) {
                task.status = "Paused".into();
                task.error = Some("应用已重启，可继续下载".into());
                task.revision += 1;
            }
            if task.status == "Completed" && !outputs_present(task) {
                task.status = "Ready".into();
                task.error = Some("本地文件已移走，可重新下载".into());
                task.revision += 1;
            }
            task.speed_bps = 0.0;
        }
        // Only unfinished legacy jobs change format. A new task identity keeps
        // their ZIP checkpoints/staging bytes separate from the TIFF download.
        let legacy: Vec<_> = tasks
            .iter()
            .filter(|(_, task)| {
                task.product == "BURST"
                    && task.file.name.ends_with(".zip")
                    && task.status != "Completed"
            })
            .map(|(id, task)| (id.clone(), task.clone()))
            .collect();
        for (old_id, mut task) in legacy {
            task.file = bundle::burst_tiff(task.file)?;
            task.id = task_id(Path::new(&task.save_dir), &task.file.name, &task.file.url);
            task.file.task_id = task.id.clone();
            let mut xml = bundle::burst_xml(&task.file)?;
            xml.task_id = task_id(Path::new(&task.save_dir), &xml.name, &xml.url);
            task.companion = Some(xml);
            task.active_file = None;
            task.status = "Ready".into();
            task.downloaded_bytes = 0;
            task.verified_bytes = 0;
            task.total_bytes = task.file.size;
            task.error = Some("已切换为 TIFF 和分组 XML，原 ZIP 与续传文件保留".into());
            task.revision += 1;
            tasks.remove(&old_id);
            tasks.entry(task.id.clone()).or_insert(task);
        }
        let service = Self {
            root,
            tasks: Mutex::new(tasks),
            active: Mutex::new(HashMap::new()),
            session: Mutex::new(None),
            cleanup_plan: Mutex::new(None),
        };
        service.persist(&service.tasks.lock().unwrap())?;
        Ok(service)
    }
    fn persist(&self, tasks: &BTreeMap<String, Task>) -> Result<(), String> {
        write_atomic(
            &self.root.join("tasks.json"),
            &serde_json::to_vec(tasks).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("无法保存下载任务：{e}"))
    }
    fn update(&self, id: &str, edit: impl FnOnce(&mut Task)) -> Result<Task, String> {
        let mut tasks = self.tasks.lock().unwrap();
        let mut task = tasks.get(id).ok_or("任务不存在")?.clone();
        edit(&mut task);
        task.revision += 1;
        let previous = tasks.insert(id.into(), task.clone());
        if let Err(error) = self.persist(&tasks) {
            if let Some(previous) = previous {
                tasks.insert(id.into(), previous);
            }
            return Err(error);
        }
        Ok(task)
    }
    fn manager(&self, username: &str, password: &str, config: &Settings) -> Arc<DownloadManager> {
        let mut digest = Sha256::new();
        digest.update(username.len().to_le_bytes());
        digest.update(username);
        digest.update(password);
        let key: [u8; 32] = digest.finalize().into();
        let mut session = self.session.lock().unwrap();
        if session.as_ref().is_none_or(|(old, _)| old != &key) {
            *session = Some((key, Arc::new(DownloadManager::new())));
        }
        let manager = session.as_ref().unwrap().1.clone();
        manager.budget.set_files(config.parallel_files);
        manager.shared_limiter.set_rate(
            (config.speed_limit_mib > 0).then_some(config.speed_limit_mib as usize * 1024 * 1024),
        );
        manager
    }
}
fn task_id(directory: &Path, name: &str, url: &str) -> String {
    let mut hash = Sha256::new();
    let directory = directory.to_string_lossy().to_string();
    #[cfg(windows)]
    let directory = directory.to_lowercase();
    for value in [directory.as_str(), name, url] {
        hash.update(value.len().to_le_bytes());
        hash.update(value);
    }
    hex::encode(hash.finalize())[..32].into()
}

fn outputs_present(task: &Task) -> bool {
    std::iter::once(&task.file)
        .chain(task.companion.iter())
        .all(|file| Path::new(&task.save_dir).join(&file.name).is_file())
}
fn reserves_name(task: &Task, directory: &Path, name: &str) -> bool {
    Path::new(&task.save_dir) == directory
        && std::iter::once(&task.file)
            .chain(task.companion.iter())
            .any(|file| file.name.eq_ignore_ascii_case(name))
}

fn shares_metadata(task: &Task, directory: &Path, file: &DownloadFile) -> bool {
    task.product == "BURST"
        && task.file.name.ends_with(".tiff")
        && Path::new(&task.save_dir) == directory
        && task
            .companion
            .as_ref()
            .is_some_and(|xml| xml.name == file.name && xml.url == file.url)
}

fn companion_checkpoint(root: &Path, task: &Task, xml: &DownloadFile) -> PathBuf {
    if task.product == "BURST" && task.file.name.ends_with(".tiff") {
        root.join("downloading").join("metadata").join(&xml.task_id)
    } else {
        root.join("downloading").join(&task.id).join("xml")
    }
}

#[derive(Serialize)]
pub struct AddResult {
    pub added: Vec<String>,
    pub skipped: usize,
}
#[tauri::command]
pub fn add_downloads(
    service: State<'_, Arc<DownloadService>>,
    products: Vec<SarProduct>,
    settings: Settings,
) -> Result<AddResult, String> {
    add_products(service.inner(), products, settings)
}
fn add_products(
    service: &DownloadService,
    products: Vec<SarProduct>,
    settings: Settings,
) -> Result<AddResult, String> {
    settings::validate(&settings)?;
    if products.is_empty() {
        return Err("请先选择影像".into());
    }
    let output = Path::new(&settings.output_dir);
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let output = output.canonicalize().map_err(|e| e.to_string())?;
    let mut tasks = service.tasks.lock().unwrap();
    let mut planned = tasks.clone();
    let mut added = vec![];
    let mut skipped = 0;
    for product in products {
        if !safe_component(&product.name)
            || !["SLC", "BURST", "GSLC", "RSLC"].contains(&product.product.as_str())
        {
            return Err("影像文件名或产品类型无效".into());
        }
        let url = reqwest::Url::parse(&product.url).map_err(|_| "影像下载地址无效")?;
        if !allowed_source(&url) {
            return Err("只支持 ASF 官方 HTTPS 下载地址".into());
        }
        let directory = if settings.organize {
            PathBuf::from(
                super::storage::plan_download_storage(
                    output.to_string_lossy().into_owned(),
                    vec![product.product.clone()],
                )?
                .remove(&product.product)
                .ok_or("无法规划下载目录")?,
            )
        } else {
            output.clone()
        };
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        let mut file = DownloadFile {
            task_id: String::new(),
            name: product.name,
            url: product.url,
            size: product.size,
            sha256: String::new(),
            hash_type: product.download_hash.as_ref().map(|h| h.r#type.clone()),
            hash_value: product.download_hash.map(|h| h.value),
            mirrors: vec![],
        };
        if product.product == "BURST" {
            file = bundle::burst_tiff(file)?;
        }
        let id = task_id(&directory, &file.name, &file.url);
        file.task_id = id.clone();
        if let Some(existing) = planned.get(&id) {
            if existing.status == "Completed" && !outputs_present(existing) {
                let mut existing = existing.clone();
                existing.status = "Ready".into();
                existing.error = None;
                existing.downloaded_bytes = 0;
                existing.revision += 1;
                planned.insert(id.clone(), existing);
                added.push(id);
                continue;
            }
            skipped += 1;
            continue;
        }
        let companion = if product.product == "BURST" {
            let mut xml = bundle::burst_xml(&file)?;
            xml.task_id = task_id(&directory, &xml.name, &xml.url);
            Some(xml)
        } else {
            None
        };
        for output_file in std::iter::once(&file).chain(companion.iter()) {
            let shared = companion
                .as_ref()
                .is_some_and(|xml| xml.name == output_file.name);
            let owners: Vec<_> = planned
                .values()
                .filter(|task| reserves_name(task, &directory, &output_file.name))
                .collect();
            let conflict = owners
                .iter()
                .any(|task| !shared || !shares_metadata(task, &directory, output_file));
            let recorded = shared && metadata_record_exists(service, output_file);
            let reusable = shared && (!owners.is_empty() || recorded);
            if conflict || (directory.join(&output_file.name).exists() && !reusable) {
                return Err(format!(
                    "目标文件已存在或被其他任务占用：{}。请选择其他目录。",
                    output_file.name
                ));
            }
        }
        super::integrity::checksum_metadata(&file)?;
        let total_bytes = file.size;
        planned.insert(
            id.clone(),
            Task {
                id: id.clone(),
                file,
                companion,
                active_file: None,
                product: product.product,
                save_dir: directory.to_string_lossy().into_owned(),
                status: "Ready".into(),
                downloaded_bytes: 0,
                total_bytes,
                speed_bps: 0.0,
                verified_bytes: 0,
                error: None,
                revision: 1,
            },
        );
        added.push(id);
    }
    service.persist(&planned)?;
    *tasks = planned;
    Ok(AddResult { added, skipped })
}

fn metadata_record_exists(service: &DownloadService, file: &DownloadFile) -> bool {
    // A removed task may leave a verified shared XML. Only this app's persisted
    // digest permits reuse; run_file rehashes it before reporting success.
    service
        .root
        .join("downloading")
        .join("metadata")
        .join(&file.task_id)
        .join("verification.json")
        .is_file()
}

#[tauri::command]
pub fn list_downloads(service: State<'_, Arc<DownloadService>>) -> Vec<Task> {
    service.tasks.lock().unwrap().values().cloned().collect()
}

#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    service: State<'_, Arc<DownloadService>>,
    id: String,
    username: String,
    password: String,
    settings: Settings,
) -> Result<(), String> {
    settings::validate(&settings)?;
    if username.trim().is_empty() || password.is_empty() {
        return Err("请先填写 Earthdata 账号和密码".into());
    }
    let mut active = service.active.lock().unwrap();
    if active.contains_key(&id) {
        return Err("任务正在运行或正在暂停，请稍后重试".into());
    }
    let task = service
        .tasks
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("任务不存在")?;
    if task.status == "Completed" {
        return Err("任务已完成".into());
    }
    let checkpoint = service.root.join("downloading").join(&id);
    let mut files = vec![(task.file.clone(), checkpoint.clone())];
    if let Some(xml) = &task.companion {
        files.push((xml.clone(), companion_checkpoint(&service.root, &task, xml)));
    }
    let options = TaskOptions {
        schema_version: 1,
        save_dir: task.save_dir.clone(),
        concurrency: settings.concurrency,
        auto_orbit: false,
    };
    for (_, directory) in &files {
        write_atomic(
            &directory.join("options.json"),
            &serde_json::to_vec(&options).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    let manager = service.manager(&username, &password, &settings);
    let task = service.update(&id, |t| {
        t.status = "Queued".into();
        t.active_file = Some(task.file.name.clone());
        t.error = None;
        t.speed_bps = 0.0;
    })?;
    let cancel = Arc::new(AtomicBool::new(false));
    active.insert(id.clone(), cancel.clone());
    drop(active);
    let service = service.inner().clone();
    let _ = app.emit("download-progress", &task);
    tauri::async_runtime::spawn(async move {
        let events = service.clone();
        let event_app = app.clone();
        let event_id = id.clone();
        let failed = Arc::new(AtomicBool::new(false));
        let event_failed = failed.clone();
        let event_cancel = cancel.clone();
        let reporter = move |progress: bundle::Progress| {
            if event_failed.load(Ordering::SeqCst) {
                return;
            }
            let update = events.update(&event_id, |t| {
                t.downloaded_bytes = progress.received;
                t.total_bytes = progress.total;
                t.speed_bps = if running(&progress.status) {
                    progress.speed
                } else {
                    0.0
                };
                t.status = progress.status;
                t.error = progress.error;
                t.active_file = Some(progress.file);
                t.verified_bytes = progress.verified;
            });
            match update {
                Ok(task) => {
                    let _ = event_app.emit("download-progress", task);
                }
                Err(error) => {
                    event_failed.store(true, Ordering::SeqCst);
                    event_cancel.store(true, Ordering::SeqCst);
                    let _ = event_app.emit("download-error", error);
                }
            }
        };
        bundle::run(
            manager,
            files,
            task.save_dir,
            username,
            password,
            settings.concurrency,
            cancel,
            task.product == "BURST" && task.file.name.ends_with(".tiff"),
            reporter,
        )
        .await;
        service.active.lock().unwrap().remove(&id);
        if failed.load(Ordering::SeqCst) {
            // Do not silently resume when the task catalog could not be committed.
            let mut tasks = service.tasks.lock().unwrap();
            if let Some(t) = tasks.get_mut(&id) {
                t.status = "Failed".into();
                t.error = Some("任务持久化失败，请检查磁盘后重启应用".into());
                t.revision += 1;
                let _ = app.emit("download-progress", t.clone());
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn pause_download(service: State<'_, Arc<DownloadService>>, id: String) -> Result<(), String> {
    if let Some(cancel) = service.active.lock().unwrap().get(&id) {
        cancel.store(true, Ordering::SeqCst);
        return Ok(());
    }
    Err("任务未运行".into())
}
#[tauri::command]
pub fn remove_download(service: State<'_, Arc<DownloadService>>, id: String) -> Result<(), String> {
    // Remove the record only. Completed outputs and resume data stay on disk.
    let active = service.active.lock().unwrap();
    if active.contains_key(&id) {
        return Err("请先暂停任务并等待暂停完成".into());
    }
    let mut tasks = service.tasks.lock().unwrap();
    let mut next = tasks.clone();
    next.remove(&id);
    service.persist(&next)?;
    *tasks = next;
    Ok(())
}

async fn remove_many_records(
    service: &DownloadService,
    ids: Vec<String>,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let ids: std::collections::HashSet<String> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(());
    }
    let started = tokio::time::Instant::now();
    loop {
        {
            // Same lock order as start_download; hold active until the catalog commit
            // so a removed task cannot restart between the stop check and removal.
            let active = service.active.lock().unwrap();
            let mut stopping = false;
            for id in &ids {
                if let Some(cancel) = active.get(id) {
                    cancel.store(true, Ordering::SeqCst);
                    stopping = true;
                }
            }
            if !stopping {
                let mut tasks = service.tasks.lock().unwrap();
                let mut next = tasks.clone();
                for id in &ids {
                    next.remove(id);
                }
                service.persist(&next)?;
                *tasks = next;
                return Ok(());
            }
        }
        if started.elapsed() >= timeout {
            return Err("部分任务尚未停止，未删除任务记录，请稍后重试".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tauri::command]
pub async fn remove_downloads(
    service: State<'_, Arc<DownloadService>>,
    ids: Vec<String>,
) -> Result<(), String> {
    // Remove only the requested snapshot, preserving new tasks, outputs and checkpoints.
    remove_many_records(service.inner(), ids, std::time::Duration::from_secs(60)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slc_md5sum_survives_task_persistence_and_verifies_supplier_content() {
        let root = std::env::temp_dir().join(format!(
            "sar-md5sum-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let service = DownloadService::load(root.clone()).unwrap();
        let mut fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/cmr/SLC.json")).unwrap();
        let content = super::super::structure::test_zip();
        let expected = format!("{:x}", md5::compute(&content));
        fixture["items"][0]["umm"]["AdditionalAttributes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|attribute| attribute["Name"] == "MD5SUM")
            .unwrap()["Values"] = serde_json::json!([expected.to_uppercase()]);
        let product =
            crate::modules::sar_search::product::normalize(&fixture["items"][0], "SLC", None)
                .unwrap();
        // Follow the real frontend IPC serialization round trip as well.
        let product: SarProduct =
            serde_json::from_value(serde_json::to_value(product).unwrap()).unwrap();
        let settings = Settings {
            output_dir: root.join("output").to_string_lossy().into(),
            ..Settings::default()
        };
        let result = add_products(&service, vec![product], settings).unwrap();
        drop(service);
        let service = DownloadService::load(root.clone()).unwrap();
        let task = service.tasks.lock().unwrap()[&result.added[0]].clone();
        assert_eq!(task.file.hash_type.as_deref(), Some("md5"));
        assert_eq!(task.file.hash_value.as_deref(), Some(expected.as_str()));
        let path = Path::new(&task.save_dir).join(&task.file.name);
        std::fs::write(&path, &content).unwrap();
        let cancel = AtomicBool::new(false);
        let verification =
            super::super::integrity::verify(&path, content.len() as u64, &task.file, &cancel)
                .unwrap();
        assert_eq!(verification.level, "supplier_checksum");
        assert_eq!(verification.supplier_algorithm.as_deref(), Some("md5"));
        assert_eq!(
            verification.supplier_value.as_deref(),
            Some(expected.as_str())
        );
        assert!(
            verification
                .checks
                .iter()
                .any(|check| check == "supplier_checksum")
        );
        let mut changed = content.clone();
        changed[0] ^= 1;
        std::fs::write(&path, changed).unwrap();
        assert!(
            super::super::integrity::verify(&path, content.len() as u64, &task.file, &cancel)
                .unwrap_err()
                .contains("HASH_MISMATCH")
        );
        drop(service);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unfinished_zip_history_migrates_without_relabeling_zip_bytes_or_completed_files() {
        let root = std::env::temp_dir().join(format!(
            "sar-burst-legacy-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let service = DownloadService::load(root.clone()).unwrap();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/cmr/BURST.json"))
                .unwrap();
        let product =
            crate::modules::sar_search::product::normalize(&fixture["items"][0], "BURST", None)
                .unwrap();
        let settings = Settings {
            output_dir: root.join("output").to_string_lossy().into(),
            ..Settings::default()
        };
        let result = add_products(&service, vec![product], settings).unwrap();
        let mut task = service.tasks.lock().unwrap()[&result.added[0]].clone();
        task.file.name = task.file.name.replace(".tiff", ".zip");
        task.file.url = task.file.url.replace(".tiff", ".zip");
        task.id = task_id(Path::new(&task.save_dir), &task.file.name, &task.file.url);
        task.file.task_id = task.id.clone();
        task.companion = Some(bundle::burst_xml(&task.file).unwrap());
        task.status = "Completed".into();
        let zip_path = Path::new(&task.save_dir).join(&task.file.name);
        let xml_path = Path::new(&task.save_dir).join(&task.companion.as_ref().unwrap().name);
        std::fs::write(&zip_path, b"original ZIP").unwrap();
        std::fs::write(&xml_path, b"original XML").unwrap();
        let old_checkpoint = root.join("downloading").join(&task.id).join("state.json");
        write_atomic(&old_checkpoint, b"original checkpoint").unwrap();
        write_atomic(
            &root.join("tasks.json"),
            &serde_json::to_vec(&serde_json::json!({ &task.id: &task })).unwrap(),
        )
        .unwrap();
        drop(service);
        let service = DownloadService::load(root.clone()).unwrap();
        assert_eq!(service.tasks.lock().unwrap()[&task.id].status, "Completed");
        assert!(
            service.tasks.lock().unwrap()[&task.id]
                .file
                .name
                .ends_with(".zip")
        );
        service
            .update(&task.id, |task| task.status = "Paused".into())
            .unwrap();
        drop(service);
        let service = DownloadService::load(root.clone()).unwrap();
        let tasks = service.tasks.lock().unwrap();
        assert!(!tasks.contains_key(&task.id));
        let migrated = tasks.values().next().unwrap();
        assert_eq!(migrated.status, "Ready");
        assert!(migrated.file.name.ends_with(".tiff") && migrated.file.url.ends_with(".tiff"));
        assert_ne!(migrated.id, task.id);
        assert!(
            !Path::new(&migrated.save_dir)
                .join(&migrated.file.name)
                .exists()
        );
        assert_eq!(std::fs::read(zip_path).unwrap(), b"original ZIP");
        assert_eq!(std::fs::read(xml_path).unwrap(), b"original XML");
        assert_eq!(
            std::fs::read(old_checkpoint).unwrap(),
            b"original checkpoint"
        );
        drop(tasks);
        drop(service);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn burst_planning_shares_metadata_by_source_slc_and_polarization() {
        let root = std::env::temp_dir().join(format!(
            "sar-burst-shared-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let service = DownloadService::load(root.clone()).unwrap();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/cmr/BURST.json"))
                .unwrap();
        let first =
            crate::modules::sar_search::product::normalize(&fixture["items"][0], "BURST", None)
                .unwrap();
        let mut second = first.clone();
        second.name = second.name.replace("301495", "301496");
        second.url = second.url.replace("/IW3/VV/0.tiff", "/IW2/VV/1.tiff");
        let mut vh = first.clone();
        vh.name = vh.name.replace("_VV_", "_VH_");
        vh.url = vh.url.replace("/VV/", "/VH/");
        let settings = Settings {
            output_dir: root.join("output").to_string_lossy().into(),
            ..Settings::default()
        };
        let result =
            add_products(&service, vec![first.clone(), second, vh], settings.clone()).unwrap();
        assert_eq!(result.added.len(), 3);
        let tasks = service.tasks.lock().unwrap();
        let a = &tasks[&result.added[0]];
        let b = &tasks[&result.added[1]];
        let c = &tasks[&result.added[2]];
        let xml = a.companion.as_ref().unwrap();
        assert!(a.file.name.ends_with(".tiff") && b.file.name.ends_with(".tiff"));
        assert_eq!(xml.name, b.companion.as_ref().unwrap().name);
        assert_eq!(xml.task_id, b.companion.as_ref().unwrap().task_id);
        assert_eq!(
            companion_checkpoint(&root, a, xml),
            companion_checkpoint(&root, b, b.companion.as_ref().unwrap())
        );
        assert_ne!(xml.name, c.companion.as_ref().unwrap().name);
        assert_ne!(xml.task_id, c.companion.as_ref().unwrap().task_id);
        drop(tasks);
        // Cached pre-change results must resolve to the existing TIFF task too.
        let mut cached_zip = first;
        cached_zip.name = cached_zip.name.replace(".tiff", ".zip");
        cached_zip.url = cached_zip.url.replace(".tiff", ".zip");
        assert_eq!(
            add_products(&service, vec![cached_zip], settings)
                .unwrap()
                .skipped,
            1
        );
        drop(service);
        let service = DownloadService::load(root.clone()).unwrap();
        assert_eq!(service.tasks.lock().unwrap().len(), 3);
        drop(service);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn burst_planning_migrates_old_history_and_protects_both_output_names() {
        let root = std::env::temp_dir().join(format!(
            "sar-burst-plan-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let service = DownloadService::load(root.clone()).unwrap();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/cmr/BURST.json"))
                .unwrap();
        let product =
            crate::modules::sar_search::product::normalize(&fixture["items"][0], "BURST", None)
                .unwrap();
        let settings = Settings {
            output_dir: root.join("output").to_string_lossy().into(),
            ..Settings::default()
        };
        let added = add_products(&service, vec![product.clone()], settings.clone()).unwrap();
        assert_eq!(added.added.len(), 1);
        assert_eq!(
            service.tasks.lock().unwrap().len(),
            1,
            "TIFF and its shared XML are a single task"
        );
        let id = &added.added[0];
        let task = service.tasks.lock().unwrap()[id].clone();
        let xml = task.companion.as_ref().unwrap();
        assert!(xml.name.ends_with(".xml"));
        assert_eq!(
            add_products(&service, vec![product.clone()], settings.clone())
                .unwrap()
                .skipped,
            1
        );
        let zip_path = Path::new(&task.save_dir).join(&task.file.name);
        std::fs::write(&zip_path, b"existing TIFF").unwrap();
        // Old JSON did not have companion/active_file. Upgrade without starting a download.
        let mut old_task = serde_json::to_value(&task).unwrap();
        old_task.as_object_mut().unwrap().remove("companion");
        old_task.as_object_mut().unwrap().remove("active_file");
        old_task["status"] = "Completed".into();
        write_atomic(
            &root.join("tasks.json"),
            &serde_json::to_vec(&serde_json::json!({ id: old_task })).unwrap(),
        )
        .unwrap();
        drop(service);
        let service = DownloadService::load(root.clone()).unwrap();
        let migrated = service.tasks.lock().unwrap()[id].clone();
        assert_eq!(migrated.status, "Ready");
        assert_eq!(migrated.companion.as_ref().unwrap().url, xml.url);
        assert!(service.active.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&zip_path).unwrap(), b"existing TIFF");
        service
            .update(id, |t| t.status = "Completed".into())
            .unwrap();
        drop(service);
        let service = DownloadService::load(root.clone()).unwrap();
        assert_eq!(
            service.tasks.lock().unwrap()[id].status,
            "Ready",
            "missing XML invalidates a completed pair"
        );
        // An unowned XML blocks planning before the ZIP transfer can begin.
        std::fs::remove_file(&zip_path).unwrap();
        let xml_path = Path::new(&task.save_dir).join(&xml.name);
        std::fs::write(&xml_path, b"user metadata").unwrap();
        service.tasks.lock().unwrap().clear();
        let result = add_products(&service, vec![product], settings);
        assert!(result.is_err());
        assert!(service.tasks.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&xml_path).unwrap(), b"user metadata");
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn task_identity_includes_destination_and_url() {
        let path = Path::new("/data");
        assert_ne!(
            task_id(path, "x.zip", "https://a"),
            task_id(path, "x.zip", "https://b")
        );
        assert_ne!(
            task_id(path, "x.zip", "https://a"),
            task_id(Path::new("/other"), "x.zip", "https://a")
        );
    }
    #[test]
    fn credentials_are_not_part_of_persisted_settings() {
        let value = serde_json::to_value(Settings::default()).unwrap();
        assert!(value.get("password").is_none());
    }
    #[test]
    fn restart_pauses_active_tasks_and_keeps_resume_metadata() {
        let root = std::env::temp_dir().join(format!(
            "sar-download-service-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let service = DownloadService::load(root.clone()).unwrap();
        let file = DownloadFile {
            name: "sample.zip".into(),
            url: "https://datapool.asf.alaska.edu/sample.zip".into(),
            size: 100,
            sha256: String::new(),
            hash_type: None,
            hash_value: None,
            task_id: "test".into(),
            mirrors: vec![],
        };
        service.tasks.lock().unwrap().insert(
            "test".into(),
            Task {
                id: "test".into(),
                file,
                companion: None,
                active_file: None,
                product: "SLC".into(),
                save_dir: root.to_string_lossy().into(),
                status: "Downloading".into(),
                downloaded_bytes: 42,
                total_bytes: 100,
                speed_bps: 9.0,
                verified_bytes: 0,
                error: None,
                revision: 2,
            },
        );
        service.persist(&service.tasks.lock().unwrap()).unwrap();
        drop(service);
        let recovered = DownloadService::load(root.clone()).unwrap();
        let task = recovered.tasks.lock().unwrap()["test"].clone();
        assert_eq!(task.status, "Paused");
        assert_eq!(task.downloaded_bytes, 42);
        assert_eq!(task.speed_bps, 0.0);
        assert_eq!(task.revision, 3);
        assert!(recovered.active.lock().unwrap().is_empty());
        // A failed atomic commit must not change the in-memory catalog.
        std::fs::remove_file(root.join("tasks.json")).unwrap();
        std::fs::create_dir(root.join("tasks.json")).unwrap();
        assert!(
            recovered
                .update("test", |t| t.status = "Completed".into())
                .is_err()
        );
        assert_eq!(recovered.tasks.lock().unwrap()["test"].status, "Paused");
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod remove_many_tests {
    use super::*;
    use std::{fs, time::Duration};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "sar-remove-many-test-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn service(&self) -> DownloadService {
            DownloadService::load(self.0.clone()).unwrap()
        }
        fn task(&self, id: &str, status: &str) -> Task {
            Task {
                id: id.into(),
                file: DownloadFile {
                    name: format!("{id}.h5"),
                    url: format!("https://example.invalid/{id}"),
                    size: 4,
                    sha256: String::new(),
                    hash_type: None,
                    hash_value: None,
                    task_id: id.into(),
                    mirrors: vec![],
                },
                companion: None,
                active_file: None,
                product: "GSLC".into(),
                save_dir: self.0.to_string_lossy().into_owned(),
                status: status.into(),
                downloaded_bytes: 4,
                total_bytes: 4,
                speed_bps: 0.0,
                verified_bytes: 0,
                error: None,
                revision: 1,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            assert!(
                self.0
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("sar-remove-many-test-")
            );
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn seed(service: &DownloadService, tasks: Vec<Task>) {
        let mut values = service.tasks.lock().unwrap();
        for task in tasks {
            values.insert(task.id.clone(), task);
        }
        service.persist(&values).unwrap();
    }

    #[tokio::test]
    async fn remove_many_preserves_outputs_checkpoints_and_new_tasks() {
        let f = Fixture::new();
        let service = f.service();
        seed(
            &service,
            vec![
                f.task("done", "Completed"),
                f.task("paused", "Paused"),
                f.task("new", "Ready"),
            ],
        );
        fs::write(f.0.join("done.h5"), b"data").unwrap();
        let resume = f.0.join("downloading/paused");
        fs::create_dir_all(&resume).unwrap();
        fs::write(resume.join("resume.json"), b"checkpoint").unwrap();
        remove_many_records(
            &service,
            vec!["done".into(), "paused".into()],
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(
            service
                .tasks
                .lock()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["new"]
        );
        assert_eq!(fs::read(f.0.join("done.h5")).unwrap(), b"data");
        assert_eq!(fs::read(resume.join("resume.json")).unwrap(), b"checkpoint");
        assert_eq!(f.service().tasks.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn remove_many_waits_for_cancellation_and_does_not_stop_unrelated_tasks() {
        let f = Fixture::new();
        let service = Arc::new(f.service());
        seed(
            &service,
            vec![
                f.task("active", "Downloading"),
                f.task("new", "Downloading"),
            ],
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let keep = Arc::new(AtomicBool::new(false));
        service
            .active
            .lock()
            .unwrap()
            .insert("active".into(), cancel.clone());
        service
            .active
            .lock()
            .unwrap()
            .insert("new".into(), keep.clone());
        let worker = service.clone();
        let stopped = cancel.clone();
        let exit = tokio::spawn(async move {
            while !stopped.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            worker.active.lock().unwrap().remove("active");
        });
        remove_many_records(&service, vec!["active".into()], Duration::from_secs(2))
            .await
            .unwrap();
        exit.await.unwrap();
        assert!(cancel.load(Ordering::SeqCst));
        assert!(!keep.load(Ordering::SeqCst));
        assert!(!service.tasks.lock().unwrap().contains_key("active"));
        assert!(service.tasks.lock().unwrap().contains_key("new"));
    }
    #[tokio::test]
    async fn remove_many_timeout_keeps_all_records() {
        let f = Fixture::new();
        let service = f.service();
        seed(
            &service,
            vec![f.task("active", "Downloading"), f.task("done", "Completed")],
        );
        service
            .active
            .lock()
            .unwrap()
            .insert("active".into(), Arc::new(AtomicBool::new(false)));
        let previous = fs::read(f.0.join("tasks.json")).unwrap();
        let result = remove_many_records(
            &service,
            vec!["active".into(), "done".into()],
            Duration::ZERO,
        )
        .await;
        assert!(result.unwrap_err().contains("未删除任务记录"));
        assert_eq!(service.tasks.lock().unwrap().len(), 2);
        assert_eq!(fs::read(f.0.join("tasks.json")).unwrap(), previous);
    }
    #[tokio::test]
    async fn remove_many_catalog_write_failure_keeps_memory_records() {
        let f = Fixture::new();
        let service = f.service();
        seed(&service, vec![f.task("done", "Completed")]);
        let catalog = f.0.join("tasks.json");
        fs::remove_file(&catalog).unwrap();
        fs::create_dir(&catalog).unwrap();
        assert!(
            remove_many_records(&service, vec!["done".into()], Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(service.tasks.lock().unwrap().contains_key("done"));
    }
}
