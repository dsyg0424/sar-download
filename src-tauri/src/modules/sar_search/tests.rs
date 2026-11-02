use super::*;
use request::SearchRequest;
const FIXTURES: [(&str, &str); 4] = [
    (
        "SLC",
        include_str!("../../../../tests/fixtures/cmr/SLC.json"),
    ),
    (
        "BURST",
        include_str!("../../../../tests/fixtures/cmr/BURST.json"),
    ),
    (
        "GSLC",
        include_str!("../../../../tests/fixtures/cmr/GSLC.json"),
    ),
    (
        "RSLC",
        include_str!("../../../../tests/fixtures/cmr/RSLC.json"),
    ),
];
fn request() -> SearchRequest {
    serde_json::from_value(json!({"bounds":[-99.25,19.25,-98.9,19.6],"start":"2026-06-01","end":"2026-06-02",
        "productTypes":[],"satellites":[],"direction":"","polarization":"","relativeOrbit":null,"geometry":null})).unwrap()
}
#[test]
fn real_cmr_metadata_maps_all_four_products() {
    for (kind, fixture) in FIXTURES {
        let value: Value = serde_json::from_str(fixture).unwrap();
        let p = product::normalize(&value["items"][0], kind, None).unwrap();
        assert_eq!(p.product, kind);
        assert!(!p.id.is_empty());
        assert!(p.url.starts_with("https://"));
        assert!(p.geometry["coordinates"].is_array());
        if kind == "BURST" {
            assert!(p.name.starts_with("S1_"));
            assert!(p.url.ends_with("/0.tiff"));
            assert!(p.name.ends_with("-BURST.tiff"));
            assert_eq!(p.size, 0);
        }
        if ["GSLC", "RSLC"].contains(&kind) {
            assert!(p.name.ends_with(".h5"));
            assert!(p.size > 1_000_000);
            assert_eq!(p.download_hash.as_ref().unwrap().r#type, "md5");
        }
        if kind == "SLC" {
            assert_eq!(p.size, 0, "rounded MB must not be treated as exact bytes");
            assert_eq!(p.platform, "Sentinel-1A");
            let hash = p.download_hash.unwrap();
            assert_eq!(hash.r#type, "md5");
            assert_eq!(hash.value, "71ff38dc33153176fb9700b1cf39fb08");
        }
    }
}

#[test]
fn slc_md5sum_normalizes_case_rejects_invalid_values_and_allows_missing_metadata() {
    let mut fixture: Value = serde_json::from_str(FIXTURES[0].1).unwrap();
    let original = "71ff38dc33153176fb9700b1cf39fb08";
    for (input, valid) in [
        (format!("  {} \n", original.to_uppercase()), true),
        ("too-short".into(), false),
        ("g".repeat(32), false),
        (format!("{original}+{original}"), false),
        ("".into(), true),
    ] {
        let attribute = fixture["items"][0]["umm"]["AdditionalAttributes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|attribute| attribute["Name"] == "MD5SUM")
            .unwrap();
        attribute["Values"] = json!([input]);
        let result = product::normalize(&fixture["items"][0], "SLC", None);
        if !valid {
            assert!(result.unwrap_err().contains("MD5SUM"));
        } else if input.is_empty() {
            assert!(result.unwrap().download_hash.is_none());
        } else {
            assert_eq!(result.unwrap().download_hash.unwrap().value, original);
        }
    }
    fixture["items"][0]["umm"]["AdditionalAttributes"]
        .as_array_mut()
        .unwrap()
        .retain(|attribute| attribute["Name"] != "MD5SUM");
    assert!(
        product::normalize(&fixture["items"][0], "SLC", None)
            .unwrap()
            .download_hash
            .is_none()
    );
}

#[test]
fn slc_file_specific_checksum_precedes_md5sum_and_other_files_do_not_inherit_it() {
    let mut fixture: Value = serde_json::from_str(FIXTURES[0].1).unwrap();
    let name = product::normalize(&fixture["items"][0], "SLC", None)
        .unwrap()
        .name;
    fixture["items"][0]["umm"]["DataGranule"]["ArchiveAndDistributionInformation"] = json!([
        {"Name": "other.zip", "Checksum": {"Algorithm": "SHA-256", "Value": "a".repeat(64)}}
    ]);
    assert_eq!(
        product::normalize(&fixture["items"][0], "SLC", None)
            .unwrap()
            .download_hash
            .unwrap()
            .r#type,
        "md5"
    );
    fixture["items"][0]["umm"]["DataGranule"]["ArchiveAndDistributionInformation"][0]["Name"] =
        json!(name);
    let hash = product::normalize(&fixture["items"][0], "SLC", None)
        .unwrap()
        .download_hash
        .unwrap();
    assert_eq!(hash.r#type, "sha256");
    assert_eq!(hash.value, "a".repeat(64));
    for (kind, contents) in [FIXTURES[1], FIXTURES[2], FIXTURES[3]] {
        let mut fixture: Value = serde_json::from_str(contents).unwrap();
        fixture["items"][0]["umm"]["AdditionalAttributes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"Name": "MD5SUM", "Values": ["b".repeat(32)]}));
        fixture["items"][0]["umm"]["DataGranule"]["ArchiveAndDistributionInformation"] = json!([]);
        assert!(
            product::normalize(&fixture["items"][0], kind, None)
                .unwrap()
                .download_hash
                .is_none()
        );
    }
}
#[test]
fn product_identity_and_url_validation() {
    let mut value: Value = serde_json::from_str(FIXTURES[2].1).unwrap();
    assert!(product::normalize(&value["items"][0], "GSLC", Some("wrong")).is_err());
    value["items"][0]["umm"]["RelatedUrls"] =
        json!([{"Type":"GET DATA","URL":"https://evil.test/product.h5"}]);
    assert!(product::normalize(&value["items"][0], "GSLC", None).is_err());
    for s in [
        "http://datapool.asf.alaska.edu/a.zip",
        "https://datapool.asf.alaska.edu.evil.test/a.zip",
        "https://user:pass@datapool.asf.alaska.edu/a.zip",
    ] {
        assert!(!product::allowed_source(&s.parse().unwrap()));
    }
}
#[test]
fn queries_validate_and_do_not_apply_s1_filters_to_nisar() {
    let mut r = request();
    r.validate().unwrap();
    r.polarization = "VV".into();
    r.relative_orbit = Some(42);
    r.satellites = vec!["Sentinel-1C".into()];
    let s1 = r.parameters("SLC").unwrap();
    assert!(s1.contains(&("short_name[]".into(), "SENTINEL-1C_SLC".into())));
    assert!(s1.contains(&("attribute[]".into(), "int,PATH_NUMBER,42".into())));
    let nisar = r.parameters("GSLC").unwrap();
    assert!(
        !nisar
            .iter()
            .any(|(_, v)| v.contains("PATH_NUMBER") || v.contains("POLARIZATION"))
    );
    r.bounds = vec![1., 2., 0., 3.];
    assert!(r.validate().is_err());
    r = request();
    r.start = "2026-02-30".into();
    assert!(r.validate().is_err());
    r = request();
    r.product_types = vec!["BAD".into()];
    assert!(r.validate().is_err());
}
#[test]
fn polygon_holes_filter_bbox_false_positives() {
    let mut r = request();
    r.geometry = Some(json!({"type":"Polygon","coordinates":[
        [[0,0],[10,0],[10,10],[0,10],[0,0]],[[2,2],[2,8],[8,8],[8,2],[2,2]]]}));
    r.validate().unwrap();
    let hole = json!({"type":"Polygon","coordinates":[[[3,3],[4,3],[4,4],[3,4],[3,3]]]});
    assert!(!r.matches_geometry(&hole).unwrap());
    let edge = json!({"type":"Polygon","coordinates":[[[1,1],[3,1],[3,3],[1,3],[1,1]]]});
    assert!(r.matches_geometry(&edge).unwrap());
    r.geometry = Some(json!({"type":"Polygon","coordinates":[[[0,0],[2,2],[0,2],[2,0],[0,0]]]}));
    assert!(r.validate().is_err());
}

#[tokio::test]
#[ignore = "public NASA catalog integration test; requires network"]
async fn live_catalog_finds_real_fixture_products() {
    use geo::BoundingRect;
    for (kind, fixture) in FIXTURES {
        let v: Value = serde_json::from_str(fixture).unwrap();
        let p = product::normalize(&v["items"][0], kind, None).unwrap();
        let geo: geojson::Geometry = serde_json::from_value(p.geometry.clone()).unwrap();
        let geo: geo::Geometry<f64> = geo.try_into().unwrap();
        let bounds = geo.bounding_rect().unwrap();
        let mut r = request();
        r.start = p.date[..10].into();
        r.end = r.start.clone();
        r.product_types = vec![kind.into()];
        r.bounds = vec![
            bounds.min().x,
            bounds.min().y,
            bounds.max().x,
            bounds.max().y,
        ];
        let results = tokio::time::timeout(std::time::Duration::from_secs(180), search(r, |_| {}))
            .await
            .unwrap()
            .unwrap();
        assert!(
            results.iter().any(|v| v.id == p.id),
            "{kind}: real product not returned"
        );
        println!(
            "{kind}: verified {} products including {}",
            results.len(),
            p.id
        );
    }
}
