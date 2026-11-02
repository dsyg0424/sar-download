use geo::{BoundingRect, CoordsIter, Geometry, Intersects, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub bounds: Vec<f64>,
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub product_types: Vec<String>,
    #[serde(default)]
    pub satellites: Vec<String>,
    #[serde(default)]
    pub direction: String,
    #[serde(default)]
    pub polarization: String,
    pub relative_orbit: Option<u32>,
    pub geometry: Option<Value>,
}
pub const PRODUCTS: [&str; 4] = ["GSLC", "RSLC", "SLC", "BURST"];
pub const SATELLITES: [&str; 4] = ["Sentinel-1A", "Sentinel-1B", "Sentinel-1C", "Sentinel-1D"];

pub fn polygon(value: &Value) -> Result<Geometry<f64>, String> {
    let json: geojson::Geometry =
        serde_json::from_value(value.clone()).map_err(|_| "无效的 GeoJSON 几何")?;
    let geometry: Geometry<f64> = json.try_into().map_err(|_| "无法解析 GeoJSON 坐标")?;
    if !matches!(geometry, Geometry::Polygon(_) | Geometry::MultiPolygon(_)) {
        return Err("范围须为 Polygon 或 MultiPolygon".into());
    }
    if !geometry.is_valid()
        || geometry.coords_iter().any(|c| {
            !c.x.is_finite()
                || !c.y.is_finite()
                || !(-180.0..=180.0).contains(&c.x)
                || !(-90.0..=90.0).contains(&c.y)
        })
    {
        return Err("范围存在无效多边形或非 WGS84 经纬度".into());
    }
    if geometry.bounding_rect().is_none() {
        return Err("筛选范围不能为空".into());
    }
    Ok(geometry)
}
impl SearchRequest {
    pub fn products(&self) -> Vec<String> {
        let mut products = if self.product_types.is_empty() {
            PRODUCTS.iter().map(|s| s.to_string()).collect()
        } else {
            self.product_types.clone()
        };
        products.sort();
        products.dedup();
        products
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.bounds.len() != 4 || self.bounds.iter().any(|n| !n.is_finite()) {
            return Err("请填写西、南、东、北四个有限坐标".into());
        }
        let b = &self.bounds;
        if !(-180.0 <= b[0]
            && b[0] < b[2]
            && b[2] <= 180.0
            && -90.0 <= b[1]
            && b[1] < b[3]
            && b[3] <= 90.0)
        {
            return Err("范围无效；跨日期变更线请分两次搜索".into());
        }
        let date = |s: &str| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .filter(|d| d.format("%Y-%m-%d").to_string() == s)
        };
        let start = date(&self.start).ok_or("开始日期无效")?;
        let end = date(&self.end).ok_or("结束日期无效")?;
        if start > end
            || self.start.as_str() < "2014-01-01"
            || end > chrono::Local::now().date_naive()
        {
            return Err("日期范围应在 2014 年至今天之间，且开始不晚于结束".into());
        }
        if self
            .product_types
            .iter()
            .any(|s| !PRODUCTS.contains(&s.as_str()))
            || self
                .satellites
                .iter()
                .any(|s| !SATELLITES.contains(&s.as_str()))
        {
            return Err("产品或卫星选项无效".into());
        }
        if !["", "ASCENDING", "DESCENDING"].contains(&self.direction.as_str()) {
            return Err("轨道方向无效".into());
        }
        let products = self.products();
        let nisar_only = products
            .iter()
            .all(|s| ["GSLC", "RSLC"].contains(&s.as_str()));
        let allowed: &[&str] = if nisar_only {
            &[""]
        } else if products.iter().any(|s| s == "BURST") {
            if products.iter().any(|s| s == "SLC") {
                &["", "VV", "HH"]
            } else {
                &["", "VV", "VH", "HH", "HV"]
            }
        } else {
            &["", "VV", "HH", "VV+VH", "HH+HV"]
        };
        if !allowed.contains(&self.polarization.as_str())
            || (nisar_only && (!self.satellites.is_empty() || self.relative_orbit.is_some()))
        {
            return Err("产品与卫星、极化或相对轨道条件不兼容".into());
        }
        if self.relative_orbit.is_some_and(|v| !(1..=175).contains(&v)) {
            return Err("S1 相对轨道号应为 1–175".into());
        }
        if let Some(value) = &self.geometry {
            polygon(value)?;
        }
        Ok(())
    }
    pub fn parameters(&self, product: &str) -> Result<Vec<(String, String)>, String> {
        let bbox = if let Some(value) = &self.geometry {
            let bounds = polygon(value)?.bounding_rect().ok_or("范围不能为空")?;
            vec![
                bounds.min().x,
                bounds.min().y,
                bounds.max().x,
                bounds.max().y,
            ]
        } else {
            self.bounds.clone()
        };
        let mut p: Vec<(String, String)> = vec![
            ("provider".into(), "ASF".into()),
            ("page_size".into(), "250".into()),
            ("sort_key[]".into(), "-end_date".into()),
            ("sort_key[]".into(), "granule_ur".into()),
            (
                "temporal".into(),
                format!("{}T00:00:00Z,{}T23:59:59Z", self.start, self.end),
            ),
            (
                "bounding_box".into(),
                bbox.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            ("options[platform][ignore_case]".into(), "true".into()),
        ];
        if ["GSLC", "RSLC"].contains(&product) {
            p.push(("platform[]".into(), "NISAR".into()));
            p.push((
                "attribute[]".into(),
                format!("string,PRODUCT_TYPE,{product}"),
            ));
        } else {
            if product == "BURST" {
                p.push(("short_name[]".into(), "SENTINEL-1_BURSTS".into()));
                for satellite in &self.satellites {
                    p.push(("platform[]".into(), satellite.to_uppercase()));
                }
            } else {
                let satellites: Vec<&str> = if self.satellites.is_empty() {
                    SATELLITES.to_vec()
                } else {
                    self.satellites.iter().map(String::as_str).collect()
                };
                for satellite in satellites {
                    p.push((
                        "short_name[]".into(),
                        format!("{}_SLC", satellite.to_uppercase()),
                    ));
                }
            }
            p.push((
                "attribute[]".into(),
                format!("string,PROCESSING_TYPE,{product}"),
            ));
            if !self.polarization.is_empty() {
                p.push((
                    "attribute[]".into(),
                    format!("string,POLARIZATION,{}", self.polarization),
                ));
            }
            if let Some(orbit) = self.relative_orbit {
                p.push(("attribute[]".into(), format!("int,PATH_NUMBER,{orbit}")));
            }
        }
        if !self.direction.is_empty() {
            p.push((
                "attribute[]".into(),
                format!("string,ASCENDING_DESCENDING,{}", self.direction),
            ));
        }
        Ok(p)
    }
    pub fn matches_geometry(&self, geometry: &Value) -> Result<bool, String> {
        if let Some(filter) = &self.geometry {
            let footprint: geojson::Geometry =
                serde_json::from_value(geometry.clone()).map_err(|e| e.to_string())?;
            let footprint: Geometry<f64> = footprint.try_into().map_err(|_| "无效影像范围")?;
            Ok(polygon(filter)?.intersects(&footprint))
        } else {
            Ok(true)
        }
    }
}
