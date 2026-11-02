use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Bounds {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct AdminRegion {
    pub name: String,
    pub code: String,
    pub level: String,
    pub bounds: Bounds,
}
#[derive(Deserialize)]
struct FlatRegion {
    name: String,
    code: String,
    level: String,
    #[serde(flatten)]
    bounds: Bounds,
}
static REGIONS: OnceLock<Result<Vec<AdminRegion>, String>> = OnceLock::new();
fn catalog() -> Result<&'static Vec<AdminRegion>, String> {
    REGIONS
        .get_or_init(|| {
            let rows: Vec<FlatRegion> =
                serde_json::from_str(include_str!("../../resources/admin-regions.json"))
                    .map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .map(|r| AdminRegion {
                    name: r.name,
                    code: r.code,
                    level: r.level,
                    bounds: r.bounds,
                })
                .collect())
        })
        .as_ref()
        .map_err(Clone::clone)
}
#[tauri::command]
pub fn list_admin_regions(
    parent_code: Option<String>,
    level: String,
) -> Result<Vec<AdminRegion>, String> {
    let regions = catalog()?;
    if !["province", "city", "county"].contains(&level.as_str()) {
        return Err("无效行政区级别".into());
    }
    let Some(code) = parent_code else {
        if level != "province" {
            return Err("根级别必须是省级".into());
        }
        return Ok(regions
            .iter()
            .filter(|r| r.level == "province")
            .cloned()
            .collect());
    };
    let parent = regions
        .iter()
        .find(|r| r.code == code)
        .ok_or("行政区不存在")?;
    if parent.level == "county" {
        return Ok(vec![]);
    }
    if (level == "city" && parent.level != "province")
        || (level == "county" && !["province", "city"].contains(&parent.level.as_str()))
    {
        return Err("行政区层级不匹配".into());
    }
    let prefix_len = if parent.level == "province" { 5 } else { 7 };
    let prefix = &code[..prefix_len];
    let mut children: Vec<_> = regions
        .iter()
        .filter(|r| r.level == level && r.code.starts_with(prefix))
        .cloned()
        .collect();
    if level == "city" && children.is_empty() {
        children = regions
            .iter()
            .filter(|r| r.level == "county" && r.code.starts_with(prefix))
            .cloned()
            .collect();
    }
    Ok(children)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn municipalities_and_regular_provinces_keep_their_hierarchy() {
        let provinces = list_admin_regions(None, "province".into()).unwrap();
        assert!(provinces.len() > 30);
        let beijing = provinces.iter().find(|r| r.name == "北京市").unwrap();
        let counties = list_admin_regions(Some(beijing.code.clone()), "city".into()).unwrap();
        assert!(counties.iter().all(|r| r.level == "county"));
        assert!(!counties.is_empty());
        let jiangsu = provinces.iter().find(|r| r.name == "江苏省").unwrap();
        let cities = list_admin_regions(Some(jiangsu.code.clone()), "city".into()).unwrap();
        let nanjing = cities.iter().find(|r| r.name == "南京市").unwrap();
        assert!(
            !list_admin_regions(Some(nanjing.code.clone()), "county".into())
                .unwrap()
                .is_empty()
        );
        assert!(list_admin_regions(Some("../bad".into()), "city".into()).is_err());
    }
}
