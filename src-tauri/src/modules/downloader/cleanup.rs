//! Preview-first cleanup of unreferenced transfer bytes, never downloaded products.
use super::{
    options::{TaskOptions, safe_component},
    service::DownloadService,
    types::DownloadState,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::State;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub id: String,
    pub path: String,
    pub bytes: u64,
    pub category: String,
    #[serde(skip)]
    owner: String,
    #[serde(skip)]
    stamp: SystemTime,
    #[serde(skip)]
    boundary: PathBuf,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPlan {
    pub token: String,
    pub items: Vec<CleanupItem>,
    pub total_bytes: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub removed_files: usize,
    pub freed_bytes: u64,
    pub errors: Vec<String>,
}

fn protected(service: &DownloadService) -> HashSet<String> {
    let mut result = HashSet::new();
    for task in service.tasks.lock().unwrap().values() {
        result.insert(task.id.clone());
        result.insert(task.file.task_id.clone());
        if let Some(xml) = &task.companion {
            result.insert(xml.task_id.clone());
        }
    }
    result.extend(service.active.lock().unwrap().keys().cloned());
    result
}
fn plain(path: &Path, boundary: &Path) -> Result<(), String> {
    if !path.starts_with(boundary) {
        return Err("清理路径超出允许目录".into());
    }
    let mut current = path;
    loop {
        let metadata = fs::symlink_metadata(current).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        let link = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let link = metadata.file_type().is_symlink();
        if link {
            return Err("不清理链接或重解析点中的文件".into());
        }
        if current == boundary {
            break;
        }
        current = current.parent().ok_or("清理路径无效")?;
    }
    Ok(())
}
fn add(
    items: &mut Vec<CleanupItem>,
    path: PathBuf,
    boundary: &Path,
    owner: &str,
    category: &str,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    plain(&path, boundary)?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Ok(());
    }
    let stamp = metadata.modified().map_err(|e| e.to_string())?;
    let id = hex::encode(Sha256::digest(format!(
        "{}:{}:{stamp:?}",
        path.display(),
        metadata.len()
    )));
    if items.iter().any(|item| item.id == id) {
        return Ok(());
    }
    items.push(CleanupItem {
        id,
        path: path.to_string_lossy().into(),
        bytes: metadata.len(),
        category: category.into(),
        owner: owner.into(),
        stamp,
        boundary: boundary.into(),
    });
    Ok(())
}
fn leaves(root: &Path, depth: usize, result: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > 4 || !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        // Check before descending; read_dir does not follow reparse points here.
        if plain(&path, root).is_err() {
            continue;
        }
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            if path.join("options.json").is_file() {
                result.push(path.clone());
            }
            leaves(&path, depth + 1, result)?;
        }
    }
    Ok(())
}
pub(super) fn preview(service: &DownloadService, output: &Path) -> Result<CleanupPlan, String> {
    if !output.is_absolute() {
        return Err("请先设置有效的下载目录".into());
    }
    let output = output.canonicalize().map_err(|_| "下载目录不存在")?;
    plain(&output, &output)?;
    let reserved = protected(service);
    let checkpoints = service.root.join("downloading");
    let mut folders = vec![];
    leaves(&checkpoints, 0, &mut folders)?;
    let mut items = vec![];
    for folder in folders {
        plain(&folder, &service.root)?;
        let options: TaskOptions = match fs::read(folder.join("options.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(value) => value,
            None => continue,
        };
        let directory = match Path::new(&options.save_dir).canonicalize() {
            Ok(path) if path.starts_with(&output) => path,
            _ => continue,
        };
        let state: DownloadState = match fs::read(folder.join("state.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(value) => value,
            None => continue,
        };
        if !safe_component(&state.task_id)
            || !safe_component(&state.file_name)
            || reserved.contains(&state.task_id)
        {
            continue;
        }
        // Checkpoint and task identity must agree; never infer arbitrary paths.
        if folder.file_name().and_then(|name| name.to_str()) != Some(state.task_id.as_str())
            && folder.file_name().and_then(|name| name.to_str()) != Some("xml")
        {
            continue;
        }
        let stage = directory
            .join(".sar-download")
            .join("staging")
            .join(&state.task_id)
            .join("payload.part");
        add(
            &mut items,
            stage.clone(),
            &output,
            &state.task_id,
            "未完成下载",
        )?;
        if let Some(parent) = stage.parent().filter(|parent| parent.is_dir()) {
            plain(parent, &output)?;
            for entry in fs::read_dir(parent).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                if !entry.file_name().to_string_lossy().starts_with("rejected-") {
                    continue;
                }
                let rejected = entry.path();
                plain(&rejected, &output)?;
                if !entry
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_dir()
                {
                    continue;
                }
                let checkpoint = rejected.join("state.json");
                let old: DownloadState = match fs::read(&checkpoint)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                {
                    Some(state) => state,
                    None => continue,
                };
                if old.task_id != state.task_id || old.file_name != state.file_name {
                    continue;
                }
                add(
                    &mut items,
                    rejected.join("payload.part"),
                    &output,
                    &state.task_id,
                    "失效下载",
                )?;
                add(
                    &mut items,
                    checkpoint,
                    &output,
                    &state.task_id,
                    "失效续传记录",
                )?;
            }
        }
        if options.schema_version != 1 {
            add(
                &mut items,
                directory.join(format!("{}.downloading", state.file_name)),
                &output,
                &state.task_id,
                "旧版未完成下载",
            )?;
        }
        // Keep verification.json and structure.json: they describe retained data.
        for name in ["state.json", "options.json"] {
            add(
                &mut items,
                folder.join(name),
                &service.root,
                &state.task_id,
                "失效续传记录",
            )?;
        }
    }
    items.sort_by(|a, b| a.path.cmp(&b.path));
    let total_bytes = items.iter().map(|item| item.bytes).sum();
    let token = hex::encode(Sha256::digest(format!(
        "{:?}:{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?,
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>()
            .join("|")
    )));
    let plan = CleanupPlan {
        token,
        items,
        total_bytes,
    };
    *service.cleanup_plan.lock().unwrap() = Some(plan.clone());
    Ok(plan)
}
pub(super) fn execute(
    service: &DownloadService,
    token: &str,
    ids: Vec<String>,
) -> Result<CleanupResult, String> {
    // Same lock order as starting/clearing tasks. Freeze ownership while deleting.
    let active = service.active.lock().unwrap();
    let tasks = service.tasks.lock().unwrap();
    let mut reserved: HashSet<_> = active.keys().cloned().collect();
    for task in tasks.values() {
        reserved.insert(task.id.clone());
        reserved.insert(task.file.task_id.clone());
        if let Some(xml) = &task.companion {
            reserved.insert(xml.task_id.clone());
        }
    }
    let mut saved = service.cleanup_plan.lock().unwrap();
    let plan = saved
        .as_ref()
        .filter(|plan| plan.token == token)
        .ok_or("清理预览已失效，请重新扫描")?;
    let ids: BTreeSet<_> = ids.into_iter().collect();
    if ids.is_empty() {
        return Err("请选择要清理的项目".into());
    }
    let mut selected = plan
        .items
        .iter()
        .filter(|item| ids.contains(&item.id))
        .collect::<Vec<_>>();
    if selected.len() != ids.len() {
        return Err("清理项目不属于本次预览".into());
    }
    for item in &selected {
        if item.category == "失效续传记录"
            && plan
                .items
                .iter()
                .any(|other| other.owner == item.owner && !ids.contains(&other.id))
        {
            return Err("续传记录需与该任务的残留一并清理，也可仅选择临时文件".into());
        }
    }
    // Remove payloads first. If one fails, keep its locator/state records so a
    // later scan can still find the remaining large temporary file.
    selected.sort_by_key(|item| item.category == "失效续传记录");
    // Validate the entire selection before deleting any file.
    for item in &selected {
        if reserved.contains(&item.owner) {
            return Err("项目已被下载任务使用，请重新扫描".into());
        }
        let path = Path::new(&item.path);
        plain(path, &item.boundary)?;
        let metadata = fs::metadata(path).map_err(|_| "文件已变化，请重新扫描")?;
        if !metadata.is_file()
            || metadata.len() != item.bytes
            || metadata.modified().ok() != Some(item.stamp)
        {
            return Err("文件已变化，请重新扫描".into());
        }
    }
    let mut result = CleanupResult {
        removed_files: 0,
        freed_bytes: 0,
        errors: vec![],
    };
    for item in selected {
        if item.category == "失效续传记录"
            && plan.items.iter().any(|other| {
                other.owner == item.owner
                    && other.category != "失效续传记录"
                    && Path::new(&other.path).exists()
            })
        {
            result
                .errors
                .push(format!("{}：对应临时文件尚未移除，保留续传记录", item.path));
            continue;
        }
        match fs::remove_file(&item.path) {
            Ok(()) => {
                result.removed_files += 1;
                result.freed_bytes += item.bytes;
            }
            Err(error) => result.errors.push(format!("{}：{error}", item.path)),
        }
    }
    *saved = None;
    Ok(result)
}
#[tauri::command]
pub async fn preview_download_cleanup(
    service: State<'_, Arc<DownloadService>>,
    output_dir: String,
) -> Result<CleanupPlan, String> {
    let service = service.inner().clone();
    tokio::task::spawn_blocking(move || preview(&service, Path::new(&output_dir)))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn execute_download_cleanup(
    service: State<'_, Arc<DownloadService>>,
    token: String,
    ids: Vec<String>,
) -> Result<CleanupResult, String> {
    let service = service.inner().clone();
    tokio::task::spawn_blocking(move || execute(&service, &token, ids))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
