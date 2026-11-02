use std::{
    collections::BTreeMap,
    io,
    path::{Component, Path},
};

pub use crate::common::disk_space::DiskSpace;
use crate::common::disk_space::resolve_existing;

#[tauri::command]
pub fn probe_download_space(path: String) -> Result<DiskSpace, String> {
    crate::common::disk_space::probe(Path::new(path.trim()))
}

#[tauri::command]
pub fn plan_download_storage(
    root: String,
    categories: Vec<String>,
) -> Result<BTreeMap<String, String>, String> {
    let root = Path::new(root.trim());
    if !root.is_absolute() || root.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("请选择不包含上级跳转的绝对目录".into());
    }
    let root = resolve_existing(root)?;
    let mut result = BTreeMap::new();
    for category in categories {
        let suffix = match category.as_str() {
            "SLC" => "SAR/Sentinel-1/SLC",
            "BURST" => "SAR/Sentinel-1/BURST",
            "SAFE" => "SAR/Sentinel-1/BURST",
            "GSLC" => "SAR/NISAR/GSLC",
            "RSLC" => "SAR/NISAR/RSLC",
            "Imports" => "Imports",
            "direct" => "",
            _ => return Err("不支持的下载分类".into()),
        };
        let path = resolve_existing(&root.join(suffix))?;
        if !path.starts_with(&root) {
            return Err("分类目录通过链接指向数据根目录之外".into());
        }
        let display = path.to_string_lossy();
        let display = if let Some(unc) = display.strip_prefix("\\\\?\\UNC\\") {
            format!("\\\\{unc}")
        } else {
            display
                .strip_prefix("\\\\?\\")
                .unwrap_or(&display)
                .to_string()
        };
        result.insert(category, display);
    }
    Ok(result)
}

/// Remove only empty staging directories, stopping at the shared cache root.
pub(crate) fn cleanup_staging(leaf: &Path, output: &Path) {
    let root = output.join(".sar-download");
    if !leaf.starts_with(&root) || leaf.components().any(|c| matches!(c, Component::ParentDir)) {
        return;
    }
    let mut current = leaf;
    loop {
        // Never recursively delete: another task may still own files here.
        match std::fs::remove_dir(current) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => break,
        }
        if current == root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
}

/// Publish without replacing an existing destination, including concurrent writers.
pub(crate) fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::hard_link(source, destination)?;
        std::fs::remove_file(source)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cleanup_preserves_other_tasks_and_removes_empty_cache_root() {
        let output = std::env::temp_dir().join(format!("sar-cache-cleanup-{}", std::process::id()));
        let first = output.join(".sar-download/staging/first");
        let second = output.join(".sar-download/staging/second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("payload.part"), b"in progress").unwrap();
        super::cleanup_staging(&first, &output);
        assert!(!first.exists());
        assert_eq!(
            std::fs::read(second.join("payload.part")).unwrap(),
            b"in progress"
        );
        std::fs::remove_file(second.join("payload.part")).unwrap();
        super::cleanup_staging(&second, &output);
        assert!(!output.join(".sar-download").exists());
        assert!(output.exists());
        std::fs::remove_dir(output).unwrap();
    }
    use super::*;
    #[cfg(windows)]
    #[test]
    fn safe_directory_publication_preserves_existing_product() {
        let root = std::env::temp_dir().join(format!("sar-safe-publish-{}", std::process::id()));
        let staged = root.join("staged.SAFE");
        let target = root.join("scene.SAFE");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(staged.join("manifest.safe"), b"new").unwrap();
        std::fs::write(target.join("manifest.safe"), b"existing").unwrap();
        assert!(publish(&staged, &target).is_err());
        assert_eq!(
            std::fs::read(target.join("manifest.safe")).unwrap(),
            b"existing"
        );
        std::fs::remove_file(target.join("manifest.safe")).unwrap();
        std::fs::remove_dir(&target).unwrap();
        publish(&staged, &target).unwrap();
        assert!(!staged.exists());
        assert_eq!(std::fs::read(target.join("manifest.safe")).unwrap(), b"new");
        std::fs::remove_file(target.join("manifest.safe")).unwrap();
        std::fs::remove_dir(target).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn probes_existing_volume_without_creating_output() {
        let root = std::env::temp_dir().join(format!("sar-space-{}", std::process::id()));
        let space = probe_download_space(root.join("SAR/SLC").to_string_lossy().into()).unwrap();
        assert!(!space.volume.is_empty());
        assert!(!root.exists());
        assert!(probe_download_space("relative".into()).is_err());
    }
    #[test]
    fn storage_plans_are_read_only_and_classified() {
        let root = std::env::temp_dir().join(format!("sar-plan-{}", std::process::id()));
        let paths = plan_download_storage(
            root.to_string_lossy().into(),
            vec!["SLC".into(), "SAFE".into(), "direct".into()],
        )
        .unwrap();
        assert!(Path::new(&paths["SLC"]).ends_with("SAR/Sentinel-1/SLC"));
        assert!(Path::new(&paths["SAFE"]).ends_with("SAR/Sentinel-1/BURST"));
        assert!(!root.exists());
        assert!(plan_download_storage("relative".into(), vec![]).is_err());
        assert!(
            plan_download_storage(root.to_string_lossy().into(), vec!["../../escape".into()])
                .is_err()
        );
    }
    #[test]
    fn publishing_never_overwrites_existing_output() {
        let dir = std::env::temp_dir().join(format!("sar-publish-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("payload.part");
        let target = dir.join("scene.zip");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"original").unwrap();
        assert!(publish(&source, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
        std::fs::remove_file(&target).unwrap();
        publish(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        std::fs::remove_file(target).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
