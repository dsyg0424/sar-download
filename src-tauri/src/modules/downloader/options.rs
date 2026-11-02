use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub(super) struct TaskOptions {
    #[serde(default)]
    pub schema_version: u32,
    pub save_dir: String,
    pub concurrency: u32,
    pub auto_orbit: bool,
}

pub(super) fn read_options(dir: &Path) -> Result<Option<TaskOptions>, String> {
    let path = dir.join("options.json");
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let options: TaskOptions =
        serde_json::from_str(&content).map_err(|e| format!("任务参数文件损坏: {e}"))?;
    if options.schema_version > 1 {
        return Err("任务参数版本高于当前支持版本".into());
    }
    Ok(Some(options))
}

pub(crate) fn safe_component(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.ends_with([' ', '.'])
        && !value
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_versions_are_compatible() {
        let legacy: TaskOptions =
            serde_json::from_str(r#"{"save_dir":"D:/SAR","concurrency":4,"auto_orbit":false}"#)
                .unwrap();
        assert_eq!(legacy.schema_version, 0);
        assert!(safe_component("S1A_scene.zip"));
        for name in [
            "..", "../scene", "CON.zip", "LPT1", "name.", "name ", "x:y", "a?b",
        ] {
            assert!(!safe_component(name));
        }
    }
}
