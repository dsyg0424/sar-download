use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager};

pub const MAX_PARALLEL_FILES: u32 = 32;
pub const MAX_FILE_CONNECTIONS: u32 = 64;

static SETTINGS_LOCK: Mutex<()> = Mutex::new(());
const CREDENTIAL_PREFIX: &str = "sar-download/earthdata/v1:";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub output_dir: String,
    pub username: String,
    pub remember_password: bool,
    pub concurrency: u32,
    pub parallel_files: u32,
    pub speed_limit_mib: u32,
    pub organize: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            output_dir: String::new(),
            username: String::new(),
            remember_password: false,
            concurrency: 8,
            parallel_files: 3,
            speed_limit_mib: 0,
            organize: true,
        }
    }
}
pub fn root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_local_data_dir().map_err(|e| e.to_string())
}
fn read_path(path: &Path) -> Result<Settings, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("设置文件损坏：{e}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(format!("无法读取设置文件：{error}")),
    }
}
pub fn read(app: &AppHandle) -> Result<Settings, String> {
    read_path(&root(app)?.join("settings.json"))
}
fn vault() -> Result<keyring::Entry, String> {
    keyring::Entry::new("com.sar.download", "earthdata").map_err(|e| e.to_string())
}

trait CredentialStorage {
    fn get(&self) -> Result<Option<String>, String>;
    fn replace(&self, value: Option<&str>) -> Result<(), String>;
}
impl CredentialStorage for keyring::Entry {
    fn get(&self) -> Result<Option<String>, String> {
        match self.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("无法读取系统凭据：{error}")),
        }
    }
    fn replace(&self, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(value) => self
                .set_password(value)
                .map_err(|e| format!("无法保存系统凭据：{e}")),
            None => match self.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(format!("无法清除系统凭据：{error}")),
            },
        }
    }
}
#[derive(Serialize, Deserialize)]
struct BoundCredential {
    username: String,
    password: String,
}
fn encode_credential(username: &str, password: &str) -> Result<String, String> {
    Ok(format!(
        "{CREDENTIAL_PREFIX}{}",
        serde_json::to_string(&BoundCredential {
            username: username.into(),
            password: password.into()
        })
        .map_err(|_| "无法编码账号凭据")?
    ))
}
fn decode_credential(value: &str, username: &str) -> Result<String, String> {
    if let Some(json) = value.strip_prefix(CREDENTIAL_PREFIX) {
        let credential: BoundCredential =
            serde_json::from_str(json).map_err(|_| "已保存的凭据格式异常，请重新填写账号密码")?;
        if credential.username != username {
            return Err("已保存凭据与当前用户名不一致，请重新填写账号密码".into());
        }
        Ok(credential.password)
    } else {
        // Read credentials saved by the previous version; bind them on the next save.
        Ok(value.into())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    #[serde(flatten)]
    pub settings: Settings,
    pub password: String,
    pub credential_warning: Option<String>,
}
fn restore(settings: Settings, credential: Result<Option<String>, String>) -> SettingsView {
    let result = credential
        .and_then(|value| value.ok_or_else(|| "未找到已保存的密码，请在设置中重新填写".into()))
        .and_then(|value| decode_credential(&value, &settings.username));
    match result {
        Ok(password) => SettingsView {
            settings,
            password,
            credential_warning: None,
        },
        Err(warning) => SettingsView {
            settings,
            password: String::new(),
            credential_warning: Some(warning),
        },
    }
}
#[tauri::command]
pub fn load_settings(app: AppHandle) -> Result<SettingsView, String> {
    let _guard = SETTINGS_LOCK.lock().map_err(|_| "设置读取锁不可用")?;
    let mut settings = read(&app)?;
    if settings.output_dir.is_empty() {
        settings.output_dir = app
            .path()
            .download_dir()
            .map_err(|e| e.to_string())?
            .join("SAR download")
            .to_string_lossy()
            .into_owned();
    }
    if !settings.remember_password {
        return Ok(SettingsView {
            settings,
            password: String::new(),
            credential_warning: None,
        });
    }
    Ok(restore(settings, vault().and_then(|entry| entry.get())))
}
pub fn validate(settings: &Settings) -> Result<(), String> {
    if !Path::new(&settings.output_dir).is_absolute() {
        return Err("请选择绝对路径作为下载目录".into());
    }
    if !(1..=MAX_PARALLEL_FILES).contains(&settings.parallel_files) {
        return Err(format!("同时下载数应为 1–{MAX_PARALLEL_FILES}"));
    }
    if !(1..=MAX_FILE_CONNECTIONS).contains(&settings.concurrency) {
        return Err(format!("单文件连接数应为 1–{MAX_FILE_CONNECTIONS}"));
    }
    if settings.speed_limit_mib > 10000 {
        return Err("速度上限不能超过 10000 MiB/s".into());
    }
    Ok(())
}
fn persist(
    path: &Path,
    mut settings: Settings,
    password: String,
    storage: &impl CredentialStorage,
) -> Result<SettingsView, String> {
    settings.username = settings.username.trim().to_owned();
    validate(&settings)?;
    if settings.username.is_empty() != password.is_empty() {
        return Err("请同时填写用户名和密码，或清空两项后仅保存其他设置".into());
    }
    settings.remember_password = !settings.username.is_empty();
    let encoded = if settings.remember_password {
        Some(encode_credential(&settings.username, &password)?)
    } else {
        None
    };
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    let previous = storage.get()?;
    storage.replace(encoded.as_deref())?;
    if let Err(error) = crate::common::atomic_file::write_atomic(path, &bytes) {
        if storage.replace(previous.as_deref()).is_err() {
            return Err("设置文件保存失败，且无法恢复系统凭据，请重新填写账号密码后保存".into());
        }
        return Err(format!("设置文件保存失败，已恢复原有凭据：{error}"));
    }
    Ok(SettingsView {
        settings,
        password,
        credential_warning: None,
    })
}
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    settings: Settings,
    password: String,
) -> Result<SettingsView, String> {
    let _guard = SETTINGS_LOCK.lock().map_err(|_| "设置保存锁不可用")?;
    persist(
        &root(&app)?.join("settings.json"),
        settings,
        password,
        &vault()?,
    )
}

#[tauri::command]
pub async fn choose_output_dir() -> Result<Option<String>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("选择 SAR 下载目录")
        .pick_folder()
        .await
        .map(|file| file.path().to_string_lossy().into_owned()))
}
#[tauri::command]
pub fn open_output_dir(path: String) -> Result<(), String> {
    if !Path::new(&path).is_absolute() || !Path::new(&path).is_dir() {
        return Err("下载目录尚不存在".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe")
            .arg(&path)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn export_results(
    products: Vec<super::sar_search::product::SarProduct>,
) -> Result<Option<String>, String> {
    if products.is_empty() {
        return Err("请先选择要导出的影像".into());
    }
    let file = rfd::AsyncFileDialog::new()
        .set_title("导出影像检索结果")
        .add_filter("GeoJSON", &["geojson"])
        .set_file_name("sar-results.geojson")
        .save_file()
        .await;
    let Some(file) = file else {
        return Ok(None);
    };
    let features: Vec<_> = products.into_iter().map(|p| serde_json::json!({
        "type":"Feature","geometry":p.geometry,"properties":{
            "id":p.id,"granule":p.granule,"name":p.name,"url":p.url,"size":p.size,
            "date":p.date,"platform":p.platform,"product":p.product,"polarization":p.polarization,
            "direction":p.direction,"relativeOrbit":p.relative_orbit,"downloadHash":p.download_hash
        }
    })).collect();
    let bytes = serde_json::to_vec_pretty(
        &serde_json::json!({"type":"FeatureCollection","features":features}),
    )
    .map_err(|e| e.to_string())?;
    // The save dialog handles the user's overwrite choice.
    tokio::fs::write(file.path(), bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(file.path().to_string_lossy().into_owned()))
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Default)]
    struct MemoryVault {
        value: RefCell<Option<String>>,
        fail_write: Cell<bool>,
    }
    impl CredentialStorage for MemoryVault {
        fn get(&self) -> Result<Option<String>, String> {
            Ok(self.value.borrow().clone())
        }
        fn replace(&self, value: Option<&str>) -> Result<(), String> {
            if self.fail_write.get() {
                return Err("synthetic credential failure".into());
            }
            *self.value.borrow_mut() = value.map(str::to_owned);
            Ok(())
        }
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "sar-settings-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn settings(&self) -> Settings {
            Settings {
                output_dir: self.0.join("downloads").to_string_lossy().into_owned(),
                username: "fixture-user".into(),
                parallel_files: 5,
                concurrency: 12,
                speed_limit_mib: 4,
                organize: false,
                ..Settings::default()
            }
        }
        fn path(&self) -> PathBuf {
            self.0.join("settings.json")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            for entry in fs::read_dir(&self.0).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    fs::remove_dir(path).unwrap();
                } else {
                    fs::remove_file(path).unwrap();
                }
            }
            fs::remove_dir(&self.0).unwrap();
        }
    }

    #[test]
    fn saved_account_and_all_settings_restore_without_password_in_json() {
        let f = Fixture::new();
        let vault = MemoryVault::default();
        let saved = persist(&f.path(), f.settings(), "synthetic-secret".into(), &vault).unwrap();
        assert!(saved.settings.remember_password);
        let bytes = fs::read(f.path()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json.get("password").is_none());
        assert!(
            !String::from_utf8(bytes)
                .unwrap()
                .contains("synthetic-secret")
        );
        let restored = restore(read_path(&f.path()).unwrap(), vault.get());
        assert_eq!(restored.password, "synthetic-secret");
        assert_eq!(restored.settings.username, "fixture-user");
        assert_eq!(restored.settings.parallel_files, 5);
        assert_eq!(restored.settings.concurrency, 12);
        assert_eq!(restored.settings.speed_limit_mib, 4);
        assert!(!restored.settings.organize);
        assert_eq!(restored.settings.output_dir, f.settings().output_dir);
        assert!(restored.credential_warning.is_none());
    }
    #[test]
    fn legacy_credentials_migrate_and_bound_credentials_reject_other_accounts() {
        let f = Fixture::new();
        let vault = MemoryVault::default();
        vault.replace(Some("old-synthetic-secret")).unwrap();
        assert_eq!(
            decode_credential(&vault.get().unwrap().unwrap(), "fixture-user").unwrap(),
            "old-synthetic-secret"
        );
        persist(
            &f.path(),
            f.settings(),
            "old-synthetic-secret".into(),
            &vault,
        )
        .unwrap();
        let value = vault.get().unwrap().unwrap();
        assert!(value.starts_with(CREDENTIAL_PREFIX));
        assert!(decode_credential(&value, "another-user").is_err());
    }
    #[test]
    fn credential_write_failure_keeps_previous_config_and_secret() {
        let f = Fixture::new();
        let vault = MemoryVault::default();
        persist(&f.path(), f.settings(), "previous".into(), &vault).unwrap();
        let old = fs::read(f.path()).unwrap();
        let secret = vault.get().unwrap();
        vault.fail_write.set(true);
        let mut next = f.settings();
        next.parallel_files = 1;
        assert!(persist(&f.path(), next, "next".into(), &vault).is_err());
        assert_eq!(fs::read(f.path()).unwrap(), old);
        assert_eq!(vault.get().unwrap(), secret);
    }
    #[test]
    fn config_write_failure_rolls_back_the_credential() {
        let f = Fixture::new();
        let vault = MemoryVault::default();
        vault.replace(Some("previous")).unwrap();
        fs::create_dir(f.path()).unwrap();
        assert!(
            persist(&f.path(), f.settings(), "next".into(), &vault)
                .err()
                .unwrap()
                .contains("已恢复原有凭据")
        );
        assert_eq!(vault.get().unwrap().as_deref(), Some("previous"));
    }
    #[test]
    fn missing_credentials_preserve_non_secret_settings_and_report_a_warning() {
        let f = Fixture::new();
        let view = restore(f.settings(), Err("synthetic vault unavailable".into()));
        assert_eq!(view.settings.concurrency, 12);
        assert_eq!(view.settings.username, "fixture-user");
        assert!(view.password.is_empty());
        assert!(view.credential_warning.is_some());
        let view = restore(f.settings(), Ok(None));
        assert!(view.password.is_empty());
        assert!(view.credential_warning.is_some());
    }
    #[test]
    fn clearing_the_account_removes_its_credential_and_partial_accounts_are_rejected() {
        let f = Fixture::new();
        let vault = MemoryVault::default();
        persist(&f.path(), f.settings(), "synthetic".into(), &vault).unwrap();
        assert!(persist(&f.path(), f.settings(), String::new(), &vault).is_err());
        let mut blank = f.settings();
        blank.username.clear();
        persist(&f.path(), blank, String::new(), &vault).unwrap();
        assert!(vault.get().unwrap().is_none());
        let settings = read_path(&f.path()).unwrap();
        assert!(settings.username.is_empty());
        assert!(!settings.remember_password);
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "Writes only a temporary, uniquely named synthetic Windows credential"]
    fn windows_credential_roundtrip_restores_and_deletes_a_synthetic_account() {
        let f = Fixture::new();
        let service = f.0.file_name().unwrap().to_string_lossy();
        let entry = keyring::Entry::new(
            &format!("com.sar.download.tests.{service}"),
            "synthetic-earthdata",
        )
        .unwrap();
        struct Cleanup(keyring::Entry);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.delete_credential();
            }
        }
        let cleanup = Cleanup(entry);
        persist(
            &f.path(),
            f.settings(),
            "temporary-synthetic-password".into(),
            &cleanup.0,
        )
        .unwrap();
        let reopened = keyring::Entry::new(
            &format!("com.sar.download.tests.{service}"),
            "synthetic-earthdata",
        )
        .unwrap();
        let restored = restore(read_path(&f.path()).unwrap(), reopened.get());
        assert_eq!(restored.password, "temporary-synthetic-password");
        let mut blank = f.settings();
        blank.username.clear();
        persist(&f.path(), blank, String::new(), &reopened).unwrap();
        assert!(reopened.get().unwrap().is_none());
    }
    #[test]
    fn transfer_limits_accept_boundaries_and_reject_out_of_range_values() {
        let fixture = Fixture::new();
        for (files, connections) in [(1, 1), (32, 64), (32, 1), (1, 64)] {
            let mut settings = fixture.settings();
            settings.parallel_files = files;
            settings.concurrency = connections;
            assert!(validate(&settings).is_ok());
        }
        for (files, connections) in [(0, 8), (33, 8), (1, 0), (1, 65)] {
            let mut settings = fixture.settings();
            settings.parallel_files = files;
            settings.concurrency = connections;
            assert!(validate(&settings).is_err());
        }
    }

    #[test]
    fn transfer_limits_maximum_settings_persist_without_clamping() {
        let fixture = Fixture::new();
        let storage = MemoryVault::default();
        let mut settings = fixture.settings();
        settings.parallel_files = 32;
        settings.concurrency = 64;
        persist(
            &fixture.path(),
            settings,
            "synthetic-maximum-test".into(),
            &storage,
        )
        .unwrap();
        let restored = read_path(&fixture.path()).unwrap();
        assert_eq!(restored.parallel_files, 32);
        assert_eq!(restored.concurrency, 64);
    }
}
