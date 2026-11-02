use super::*;
use std::io::Write;
fn root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "sar-structure-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
#[test]
fn tiff_checks_dimensions_truncation_data_ranges_and_cancellation() {
    let root = root();
    let path = root.join("burst.tiff");
    let flag = AtomicBool::new(false);
    let bytes = super::test_tiff(3, 2);
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(tiff(&path, &flag).unwrap().width, 3);
    std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
    assert!(tiff(&path, &flag).is_err());
    std::fs::write(&path, b"<html>not a TIFF</html>").unwrap();
    assert!(tiff(&path, &flag).is_err());
    std::fs::write(&path, bytes).unwrap();
    assert!(
        tiff(&path, &AtomicBool::new(true))
            .unwrap_err()
            .contains("CANCEL")
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn safe_zip_checks_manifest_members_sizes_and_central_directory() {
    let root = root();
    let path = root.join("scene.zip");
    let flag = AtomicBool::new(false);
    std::fs::write(&path, test_zip()).unwrap();
    zip_structure(&path, &flag).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 25);
    std::fs::write(&path, bytes).unwrap();
    assert!(zip_structure(&path, &flag).is_err());
    let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
    zip.start_file(
        "scene.SAFE/manifest.safe",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"<x><byteStream size=\"1\"><fileLocation href=\"./measurement/missing.tiff\"/></byteStream></x>").unwrap();
    zip.start_file(
        "scene.SAFE/measurement/data.tiff",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"a").unwrap();
    zip.finish().unwrap();
    assert!(zip_structure(&path, &flag).unwrap_err().contains("缺失"));
    std::fs::remove_dir_all(root).unwrap();
}
pub(super) fn test_zip() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    let image = super::test_tiff(1, 1);
    zip.start_file(
        "scene.SAFE/manifest.safe",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    write!(zip,"<x><byteStream size=\"{}\"><fileLocation href=\"././measurement/data.tiff\"/></byteStream></x>",image.len()).unwrap();
    zip.start_file(
        "scene.SAFE/measurement/data.tiff",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&image).unwrap();
    zip.finish().unwrap().into_inner()
}
#[test]
fn burst_pair_requires_exact_source_polarization_dimensions_and_index() {
    let root = root();
    let source = "S1D_IW_SLC__1SDV_20261005T102814_20261005T102841_004879_0092D3_5831";
    let file = DownloadFile {
        task_id: "test".into(),
        name: "burst.tiff".into(),
        url: format!("https://sentinel1-burst.asf.alaska.edu/{source}/IW3/VV/1.tiff"),
        size: 0,
        sha256: String::new(),
        hash_type: None,
        hash_value: None,
        mirrors: vec![],
    };
    let metadata = DownloadFile {
        name: format!("{source}_VV.xml"),
        ..file.clone()
    };
    std::fs::write(root.join(&file.name), super::test_tiff(3, 2)).unwrap();
    let xml = "<burst><metadata><product><swath>IW3</swath><polarisation>VV</polarisation><content><adsHeader><missionId>S1D</missionId><absoluteOrbitNumber>4879</absoluteOrbitNumber><missionDataTakeId>37587</missionDataTakeId></adsHeader><swathTiming><linesPerBurst>2</linesPerBurst><samplesPerBurst>3</samplesPerBurst><burstList><burst><azimuthTime>x</azimuthTime></burst><burst><azimuthTime>x</azimuthTime></burst></burstList></swathTiming></content></product></metadata></burst>";
    std::fs::write(root.join(&metadata.name), xml).unwrap();
    assert_eq!(
        burst_pair(&root, &file, &metadata, &AtomicBool::new(false))
            .unwrap()
            .burst_index,
        1
    );
    for bad in [
        xml.replace("<samplesPerBurst>3", "<samplesPerBurst>4"),
        xml.replace("<absoluteOrbitNumber>4879", "<absoluteOrbitNumber>4878"),
        xml.replace("<polarisation>VV", "<polarisation>VH"),
    ] {
        std::fs::write(root.join(&metadata.name), bad).unwrap();
        assert!(burst_pair(&root, &file, &metadata, &AtomicBool::new(false)).is_err());
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "read-only validation of user-selected real downloads; requires SAR_STRUCTURE_ROOT"]
fn real_download_structures() {
    let root = std::path::PathBuf::from(std::env::var("SAR_STRUCTURE_ROOT").unwrap());
    let tasks: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("SAR_TASKS_JSON").unwrap()).unwrap())
            .unwrap();
    let mut checked = 0;
    for task in tasks.as_object().unwrap().values() {
        if task["status"] != "Completed" {
            continue;
        }
        let directory = std::path::PathBuf::from(task["save_dir"].as_str().unwrap());
        if !directory.starts_with(&root)
            && !directory
                .to_string_lossy()
                .contains(root.to_string_lossy().as_ref())
        {
            continue;
        }
        let file: DownloadFile = serde_json::from_value(task["file"].clone()).unwrap();
        check(&directory.join(&file.name), &file, &AtomicBool::new(false)).unwrap();
        if file.name.ends_with(".tiff") {
            let metadata: DownloadFile = serde_json::from_value(task["companion"].clone()).unwrap();
            burst_pair(&directory, &file, &metadata, &AtomicBool::new(false)).unwrap();
        }
        checked += 1;
    }
    assert!(checked > 0);
    println!("validated {checked} real download tasks");
}
