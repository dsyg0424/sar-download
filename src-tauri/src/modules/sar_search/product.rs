use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadHash {
    pub r#type: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SarProduct {
    pub id: String,
    pub granule: String,
    pub name: String,
    pub url: String,
    pub size: u64,
    pub date: String,
    pub platform: String,
    pub product: String,
    pub polarization: String,
    pub direction: String,
    pub relative_orbit: Option<u32>,
    pub geometry: Value,
    pub download_hash: Option<DownloadHash>,
}
pub fn attribute(umm: &Value, name: &str) -> String {
    umm["AdditionalAttributes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["Name"] == name)
        .and_then(|v| v["Values"].as_array())
        .map(|vs| {
            vs.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("+")
        })
        .unwrap_or_default()
}
pub fn allowed_source(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or("");
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && (host == "nisar.asf.earthdatacloud.nasa.gov"
            || host == "datapool.asf.alaska.edu"
            || host == "sentinel1-burst.asf.alaska.edu")
}
pub fn normalize(
    record: &Value,
    kind: &str,
    expected_granule: Option<&str>,
) -> Result<SarProduct, String> {
    let umm = &record["umm"];
    let granule = umm["GranuleUR"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("CMR 产品缺少 GranuleUR")?;
    if expected_granule.is_some_and(|v| v != granule) {
        return Err("CMR 产品标识不匹配".into());
    }
    let actual_kind = attribute(
        umm,
        if ["GSLC", "RSLC"].contains(&kind) {
            "PRODUCT_TYPE"
        } else {
            "PROCESSING_TYPE"
        },
    );
    if actual_kind != kind {
        return Err(format!("CMR 产品类型不匹配：{actual_kind}"));
    }
    let urls = umm["RelatedUrls"].as_array().ok_or("CMR 缺少下载地址")?;
    let url = urls
        .iter()
        .filter_map(|v| {
            let url = reqwest::Url::parse(v["URL"].as_str()?).ok()?;
            let matches = if kind == "BURST" {
                url.path().ends_with(".tiff") && v["Type"] == "USE SERVICE API"
            } else {
                v["Type"] == "GET DATA"
                    && url
                        .path()
                        .ends_with(if kind == "SLC" { ".zip" } else { ".h5" })
            };
            (allowed_source(&url) && matches).then_some(url)
        })
        .next()
        .ok_or("CMR 未返回匹配的 HTTPS 数据文件")?;
    let name = if kind == "BURST" {
        format!("{granule}.tiff")
    } else {
        percent_encoding::percent_decode_str(
            url.path_segments()
                .and_then(|mut s| s.next_back())
                .ok_or("下载地址缺少文件名")?,
        )
        .decode_utf8()
        .map_err(|_| "文件名编码无效")?
        .into_owned()
    };
    let archive = umm["DataGranule"]["ArchiveAndDistributionInformation"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["Name"].as_str() == Some(&name));
    // Preserve exact byte counts only; old SLC records report rounded megabytes.
    let size = archive.and_then(|v| v["SizeInBytes"].as_u64()).unwrap_or(0);
    let mut download_hash = archive.and_then(|v| {
        let algorithm = v["Checksum"]["Algorithm"]
            .as_str()?
            .to_lowercase()
            .replace('-', "");
        ["md5", "sha256"]
            .contains(&algorithm.as_str())
            .then(|| DownloadHash {
                r#type: algorithm,
                value: v["Checksum"]["Value"].as_str().unwrap_or("").into(),
            })
    });
    // Older Sentinel-1 SLC records publish their whole-ZIP MD5 here instead of
    // ArchiveAndDistributionInformation.Checksum. Never use a source-SLC digest
    // for a derived Burst TIFF/XML (or an unrelated NISAR file).
    if download_hash.is_none() && kind == "SLC" {
        let md5 = attribute(umm, "MD5SUM").trim().to_ascii_lowercase();
        if !md5.is_empty() {
            if md5.len() != 32 || !md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("CMR MD5SUM 无效：应为 32 位十六进制整包摘要".into());
            }
            download_hash = Some(DownloadHash {
                r#type: "md5".into(),
                value: md5,
            });
        }
    }
    let domain = &umm["SpatialExtent"]["HorizontalSpatialDomain"]["Geometry"];
    let mut polygons = vec![];
    for polygon in domain["GPolygons"].as_array().into_iter().flatten() {
        let points = polygon["Boundary"]["Points"]
            .as_array()
            .ok_or("CMR 影像边界无效")?;
        let mut ring: Vec<Value> = points
            .iter()
            .map(|p| {
                let x = p["Longitude"].as_f64().ok_or("影像经度无效")?;
                let y = p["Latitude"].as_f64().ok_or("影像纬度无效")?;
                if !x.is_finite()
                    || !y.is_finite()
                    || !(-180.0..=180.0).contains(&x)
                    || !(-90.0..=90.0).contains(&y)
                {
                    return Err("影像坐标无效");
                }
                Ok(json!([x, y]))
            })
            .collect::<Result<_, _>>()?;
        if ring.len() < 3 {
            return Err("CMR 影像边界过短".into());
        }
        if ring.first() != ring.last() {
            ring.push(ring[0].clone());
        }
        polygons.push(json!([ring]));
    }
    if polygons.is_empty() {
        for rectangle in domain["BoundingRectangles"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let w = rectangle["WestBoundingCoordinate"]
                .as_f64()
                .ok_or("无效影像边界")?;
            let e = rectangle["EastBoundingCoordinate"]
                .as_f64()
                .ok_or("无效影像边界")?;
            let s = rectangle["SouthBoundingCoordinate"]
                .as_f64()
                .ok_or("无效影像边界")?;
            let n = rectangle["NorthBoundingCoordinate"]
                .as_f64()
                .ok_or("无效影像边界")?;
            polygons.push(json!([[[w, s], [e, s], [e, n], [w, n], [w, s]]]));
        }
    }
    let geometry = match polygons.len() {
        0 => return Err("CMR 未返回影像范围".into()),
        1 => json!({"type":"Polygon","coordinates":polygons[0]}),
        _ => json!({"type":"MultiPolygon","coordinates":polygons}),
    };
    let platform = umm["Platforms"][0]["ShortName"].as_str().unwrap_or("");
    let platform = if let Some(suffix) = platform.strip_prefix("SENTINEL-1") {
        format!("Sentinel-1{suffix}")
    } else {
        platform.into()
    };
    let date = umm["TemporalExtent"]["RangeDateTime"]["BeginningDateTime"]
        .as_str()
        .or_else(|| umm["TemporalExtent"]["SingleDateTime"].as_str())
        .filter(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
        .ok_or("CMR 缺少有效采集日期")?;
    Ok(SarProduct {
        id: record["meta"]["concept-id"]
            .as_str()
            .ok_or("CMR 缺少 concept-id")?
            .into(),
        granule: granule.into(),
        name,
        url: url.into(),
        size,
        date: date.into(),
        platform,
        product: kind.into(),
        polarization: attribute(
            umm,
            if ["GSLC", "RSLC"].contains(&kind) {
                "FREQUENCY_A_POLARIZATION_CONCAT"
            } else {
                "POLARIZATION"
            },
        ),
        direction: attribute(umm, "ASCENDING_DESCENDING"),
        relative_orbit: attribute(
            umm,
            if ["GSLC", "RSLC"].contains(&kind) {
                "TRACK_NUMBER"
            } else {
                "PATH_NUMBER"
            },
        )
        .parse()
        .ok(),
        geometry,
        download_hash,
    })
}
