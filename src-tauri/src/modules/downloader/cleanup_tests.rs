use super::super::{options::TaskOptions, service::Task, types::DownloadFile};
use super::*;
fn fixture() -> (PathBuf, DownloadService, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "sar-cleanup-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let service = DownloadService::load(root.join("app")).unwrap();
    let output = root.join("output");
    fs::create_dir_all(&output).unwrap();
    (root, service, output)
}
fn orphan(service: &DownloadService, output: &Path, id: &str) -> PathBuf {
    let folder = service.root.join("downloading").join(id);
    fs::create_dir_all(&folder).unwrap();
    let file = DownloadFile {
        task_id: id.into(),
        name: "scene.tiff".into(),
        url: "https://example.invalid/scene.tiff".into(),
        size: 100,
        sha256: String::new(),
        hash_type: None,
        hash_value: None,
        mirrors: vec![],
    };
    let state = DownloadState::new_fresh(id, &file.name, &file.url, 100, &file, 1);
    fs::write(
        folder.join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    fs::write(
        folder.join("options.json"),
        serde_json::to_vec(&TaskOptions {
            schema_version: 1,
            save_dir: output.to_string_lossy().into(),
            concurrency: 1,
            auto_orbit: false,
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(folder.join("verification.json"), b"retained digest").unwrap();
    let stage = output
        .join(".sar-download/staging")
        .join(id)
        .join("payload.part");
    fs::create_dir_all(stage.parent().unwrap()).unwrap();
    fs::write(&stage, b"partial bytes").unwrap();
    stage
}
#[test]
fn cleanup_preview_and_selected_execution_preserve_products_and_digests() {
    let (root, service, output) = fixture();
    let stage = orphan(&service, &output, "orphan");
    fs::write(output.join("scene.tiff"), b"raw product").unwrap();
    fs::write(output.join("source_VV.xml"), b"shared metadata").unwrap();
    let plan = preview(&service, &output).unwrap();
    assert_eq!(plan.items.len(), 3);
    assert!(stage.exists());
    let item = plan
        .items
        .iter()
        .find(|item| item.category == "未完成下载")
        .unwrap();
    let result = execute(&service, &plan.token, vec![item.id.clone()]).unwrap();
    assert_eq!(result.removed_files, 1);
    assert!(!stage.exists());
    assert_eq!(fs::read(output.join("scene.tiff")).unwrap(), b"raw product");
    assert!(output.join("source_VV.xml").exists());
    assert!(
        service
            .root
            .join("downloading/orphan/verification.json")
            .exists()
    );
    assert!(execute(&service, &plan.token, vec![item.id.clone()]).is_err());
    drop(service);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn changed_or_newly_owned_files_invalidate_cleanup_before_any_deletion() {
    let (root, service, output) = fixture();
    let stage = orphan(&service, &output, "orphan");
    let plan = preview(&service, &output).unwrap();
    fs::write(&stage, b"changed file").unwrap();
    assert!(
        execute(
            &service,
            &plan.token,
            plan.items.iter().map(|item| item.id.clone()).collect()
        )
        .is_err()
    );
    assert!(service.root.join("downloading/orphan/state.json").exists());
    let plan = preview(&service, &output).unwrap();
    service.active.lock().unwrap().insert(
        "orphan".into(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    assert!(
        execute(
            &service,
            &plan.token,
            plan.items.iter().map(|item| item.id.clone()).collect()
        )
        .is_err()
    );
    assert!(stage.exists());
    assert!(preview(&service, &output).unwrap().items.is_empty());
    drop(service);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn shared_metadata_referenced_by_any_task_is_not_cleanup_candidate() {
    let (root, service, output) = fixture();
    let stage = orphan(&service, &output, "metadata");
    let file = DownloadFile {
        task_id: "main".into(),
        name: "burst.tiff".into(),
        url: "https://example.invalid/0.tiff".into(),
        size: 0,
        sha256: String::new(),
        hash_type: None,
        hash_value: None,
        mirrors: vec![],
    };
    service.tasks.lock().unwrap().insert(
        "main".into(),
        Task {
            id: "main".into(),
            file: file.clone(),
            companion: Some(DownloadFile {
                task_id: "metadata".into(),
                name: "source.xml".into(),
                ..file
            }),
            active_file: None,
            product: "BURST".into(),
            save_dir: output.to_string_lossy().into(),
            status: "Paused".into(),
            downloaded_bytes: 0,
            total_bytes: 0,
            speed_bps: 0.0,
            verified_bytes: 0,
            error: None,
            revision: 1,
        },
    );
    assert!(preview(&service, &output).unwrap().items.is_empty());
    assert!(stage.exists());
    drop(service);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_cannot_drop_locator_records_while_retaining_their_payload() {
    let (root, service, output) = fixture();
    let stage = orphan(&service, &output, "orphan");
    let plan = preview(&service, &output).unwrap();
    let state = plan
        .items
        .iter()
        .find(|item| item.path.ends_with("state.json"))
        .unwrap();
    assert!(execute(&service, &plan.token, vec![state.id.clone()]).is_err());
    assert!(
        stage.exists()
            && service
                .root
                .join("downloading/orphan/options.json")
                .exists()
    );
    let result = execute(
        &service,
        &plan.token,
        plan.items.iter().map(|item| item.id.clone()).collect(),
    )
    .unwrap();
    assert_eq!(result.removed_files, 3);
    assert!(!stage.exists());
    assert!(
        service
            .root
            .join("downloading/orphan/verification.json")
            .exists()
    );
    drop(service);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_also_finds_rejected_partial_transfers_by_their_saved_identity() {
    let (root, service, output) = fixture();
    let stage = orphan(&service, &output, "orphan");
    let checkpoint = service.root.join("downloading/orphan/state.json");
    super::super::integrity::isolate(&stage, &checkpoint).unwrap();
    let plan = preview(&service, &output).unwrap();
    assert!(plan.items.iter().any(|item| item.category == "失效下载"));
    let result = execute(
        &service,
        &plan.token,
        plan.items.iter().map(|item| item.id.clone()).collect(),
    )
    .unwrap();
    assert_eq!(result.removed_files, 4);
    assert!(
        service
            .root
            .join("downloading/orphan/verification.json")
            .exists()
    );
    drop(service);
    fs::remove_dir_all(root).unwrap();
}
