//! Bounded native format checks. These validate structure, not scientific quality.
use super::types::DownloadFile;
use quick_xml::events::Event;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn invalid(message: impl std::fmt::Display) -> String {
    format!("STRUCTURE_INVALID: {message}")
}
fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err("TASK_CANCELLED".into())
    } else {
        Ok(())
    }
}
fn read_at(file: &mut File, offset: u64, count: usize, size: u64) -> Result<Vec<u8>, String> {
    if offset
        .checked_add(count as u64)
        .is_none_or(|end| end > size)
    {
        return Err(invalid("文件字段越界"));
    }
    file.seek(SeekFrom::Start(offset)).map_err(invalid)?;
    let mut bytes = vec![0; count];
    file.read_exact(&mut bytes).map_err(invalid)?;
    Ok(bytes)
}
fn number(bytes: &[u8], little: bool) -> u64 {
    if little {
        bytes.iter().rev().fold(0, |v, b| (v << 8) | u64::from(*b))
    } else {
        bytes.iter().fold(0, |v, b| (v << 8) | u64::from(*b))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TiffShape {
    pub width: u64,
    pub height: u64,
    pub strips_or_tiles: usize,
}
pub fn tiff(path: &Path, cancel: &AtomicBool) -> Result<TiffShape, String> {
    let mut file = File::open(path).map_err(invalid)?;
    let size = file.metadata().map_err(invalid)?.len();
    let head = read_at(&mut file, 0, 8, size)?;
    let little = match &head[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(invalid("TIFF 字节序无效")),
    };
    let version = number(&head[2..4], little);
    let (offset, count_size, entry_size, inline) = match version {
        42 => (number(&head[4..8], little), 2, 12, 4),
        43 => {
            let head = read_at(&mut file, 0, 16, size)?;
            if number(&head[4..6], little) != 8 || number(&head[6..8], little) != 0 {
                return Err(invalid("BigTIFF 头无效"));
            };
            (number(&head[8..16], little), 8, 20, 8)
        }
        _ => return Err(invalid("不是 TIFF/BigTIFF 文件")),
    };
    if offset < 8 {
        return Err(invalid("TIFF 缺少 IFD"));
    }
    let count = number(&read_at(&mut file, offset, count_size, size)?, little);
    if count == 0 || count > 4096 {
        return Err(invalid("TIFF IFD 项数无效"));
    }
    let mut tags = BTreeMap::new();
    for index in 0..count {
        cancelled(cancel)?;
        let entry = read_at(
            &mut file,
            offset + count_size as u64 + index * entry_size as u64,
            entry_size,
            size,
        )?;
        let tag = number(&entry[..2], little);
        if ![256, 257, 258, 259, 273, 277, 279, 324, 325, 339].contains(&tag) {
            continue;
        }
        let kind = number(&entry[2..4], little);
        let width = match kind {
            3 => 2,
            4 | 13 => 4,
            16 | 18 => 8,
            _ => return Err(invalid("TIFF 核心字段类型无效")),
        };
        let (n, pos) = if version == 42 {
            (number(&entry[4..8], little), 8)
        } else {
            (number(&entry[4..12], little), 12)
        };
        if n == 0 || n > 1_000_000 {
            return Err(invalid("TIFF 核心数组长度无效"));
        }
        let length = n as usize * width;
        let bytes = if length <= inline {
            entry[pos..pos + length].to_vec()
        } else {
            read_at(
                &mut file,
                number(&entry[pos..pos + inline], little),
                length,
                size,
            )?
        };
        tags.insert(
            tag,
            bytes
                .chunks_exact(width)
                .map(|value| number(value, little))
                .collect::<Vec<_>>(),
        );
    }
    let scalar = |tag| {
        tags.get(&tag)
            .filter(|v| v.len() == 1)
            .map(|v| v[0])
            .unwrap_or(0)
    };
    let width = scalar(256);
    let height = scalar(257);
    if scalar(258) != 32 || scalar(277) != 1 || scalar(339) != 5 {
        return Err(invalid("Burst TIFF 应为单波段 CInt16 复数影像"));
    }
    let (offsets, counts) = match (
        tags.get(&273).or(tags.get(&324)),
        tags.get(&279).or(tags.get(&325)),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err(invalid("TIFF 缺少数据区索引")),
    };
    if width == 0 || height == 0 || offsets.len() != counts.len() {
        return Err(invalid("TIFF 尺寸或数据区索引无效"));
    }
    for (&start, &length) in offsets.iter().zip(counts) {
        if start < 8 || length == 0 || start.checked_add(length).is_none_or(|end| end > size) {
            return Err(invalid("TIFF 数据区超出文件范围"));
        }
    }
    if scalar(259) == 1 {
        let required = width
            .checked_mul(height)
            .and_then(|size| size.checked_mul(4))
            .ok_or_else(|| invalid("TIFF 尺寸溢出"))?;
        let actual = counts
            .iter()
            .try_fold(0u64, |total, count| total.checked_add(*count))
            .ok_or_else(|| invalid("TIFF 数据长度溢出"))?;
        if actual < required {
            return Err(invalid("TIFF 未压缩像素数据不足"));
        }
    }
    Ok(TiffShape {
        width,
        height,
        strips_or_tiles: offsets.len(),
    })
}

fn zip_structure(path: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let file = File::open(path).map_err(invalid)?;
    let size = file.metadata().map_err(invalid)?.len();
    let mut archive = zip::ZipArchive::new(file).map_err(invalid)?;
    if archive.is_empty() || archive.len() > 100_000 {
        return Err(invalid("ZIP 成员数量无效"));
    }
    let mut names = BTreeMap::new();
    let mut manifests = vec![];
    for index in 0..archive.len() {
        cancelled(cancel)?;
        let member = archive.by_index(index).map_err(invalid)?;
        if member.enclosed_name().is_none() || member.name().contains('\\') {
            return Err(invalid("ZIP 成员路径无效"));
        }
        if member
            .data_start()
            .checked_add(member.compressed_size())
            .is_none_or(|end| end > size)
        {
            return Err(invalid("ZIP 数据区越界"));
        }
        if names
            .insert(member.name().to_owned(), member.size())
            .is_some()
        {
            return Err(invalid("ZIP 成员重名"));
        }
        if member.name().ends_with("/manifest.safe") {
            manifests.push(member.name().to_owned());
        }
    }
    if manifests.len() != 1
        || !names
            .keys()
            .any(|name| name.contains("/measurement/") && name.ends_with(".tiff"))
    {
        return Err(invalid("SLC ZIP 缺少 SAFE 清单或影像"));
    }
    let name = &manifests[0];
    let prefix = name.rsplit_once('/').unwrap().0;
    let member = archive.by_name(name).map_err(invalid)?;
    if member.size() > 16 * 1024 * 1024 {
        return Err(invalid("SAFE 清单过大"));
    }
    let mut xml = quick_xml::Reader::from_reader(BufReader::new(member));
    let mut buffer = vec![];
    let mut expected = None;
    loop {
        cancelled(cancel)?;
        match xml.read_event_into(&mut buffer).map_err(invalid)? {
            Event::Start(ref node) | Event::Empty(ref node) => {
                let tag = local(node.name().as_ref());
                if tag == "byteStream" {
                    expected = node
                        .attributes()
                        .filter_map(Result::ok)
                        .find(|a| a.key.as_ref() == "size")
                        .and_then(|a| a.value.parse::<u64>().ok());
                }
                if tag == "fileLocation" {
                    let href = node
                        .attributes()
                        .filter_map(Result::ok)
                        .find(|a| a.key.as_ref() == "href")
                        .ok_or_else(|| invalid("SAFE 清单引用缺少路径"))?;
                    let href = href.value.as_ref();
                    if href.starts_with('/') || href.split('/').any(|part| part == "..") {
                        return Err(invalid("SAFE 清单路径越界"));
                    }
                    let relative = href
                        .split('/')
                        .filter(|part| !part.is_empty() && *part != ".")
                        .collect::<Vec<_>>()
                        .join("/");
                    let actual = names
                        .get(&format!("{prefix}/{relative}"))
                        .ok_or_else(|| invalid("SAFE 清单引用文件缺失"))?;
                    if expected.is_some_and(|size| size != *actual) {
                        return Err(invalid("SAFE 清单文件长度不匹配"));
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(())
}
pub fn check(path: &Path, file: &DownloadFile, cancel: &AtomicBool) -> Result<Vec<String>, String> {
    cancelled(cancel)?;
    let lower = file.name.to_ascii_lowercase();
    if lower.ends_with(".tiff") || lower.ends_with(".tif") {
        tiff(path, cancel)?;
        return Ok(vec!["tiff_structure".into()]);
    }
    if lower.ends_with(".zip") {
        zip_structure(path, cancel)?;
        return Ok(vec![
            "zip_directory".into(),
            "safe_manifest_structure".into(),
        ]);
    }
    if lower.ends_with(".h5") || lower.ends_with(".hdf5") {
        let mut file = File::open(path).map_err(invalid)?;
        let size = file.metadata().map_err(invalid)?.len();
        let mut offset = 0;
        while offset + 9 <= size {
            let head = read_at(&mut file, offset, 9, size)?;
            if &head[..8] == b"\x89HDF\r\n\x1a\n" && head[8] <= 3 {
                return Ok(vec!["hdf5_signature".into()]);
            }
            offset = if offset == 0 {
                512
            } else {
                offset.saturating_mul(2)
            };
            if offset == u64::MAX {
                break;
            }
        }
        return Err(invalid("HDF5 签名或超级块版本无效"));
    }
    Ok(vec![])
}
fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or("").to_owned()
}

#[derive(Default)]
struct Annotation {
    swath: String,
    pol: String,
    width: u64,
    height: u64,
    bursts: u64,
    orbit: String,
    take: String,
    mission: String,
}
#[derive(Debug, Serialize)]
pub struct BurstMapping {
    pub source_slc: String,
    pub swath: String,
    pub polarization: String,
    pub burst_index: u64,
    pub shape: TiffShape,
}
pub fn burst_pair(
    directory: &Path,
    file: &DownloadFile,
    metadata: &DownloadFile,
    cancel: &AtomicBool,
) -> Result<BurstMapping, String> {
    let url = reqwest::Url::parse(&file.url).map_err(invalid)?;
    let segments = url
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    if segments.len() != 4 {
        return Err(invalid("Burst 地址缺少索引"));
    }
    let (source, swath, pol) = (segments[0], segments[1], segments[2]);
    let index = segments[3]
        .strip_suffix(".tiff")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| invalid("Burst 索引无效"))?;
    if metadata.name != format!("{source}_{pol}.xml") {
        return Err(invalid("Burst 元数据与源场景名称不匹配"));
    }
    let expected_orbit = source
        .rsplit('_')
        .nth(2)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| invalid("源场景轨道无效"))?;
    let expected_take = source
        .rsplit('_')
        .nth(1)
        .and_then(|value| u64::from_str_radix(value, 16).ok())
        .ok_or_else(|| invalid("源场景数据条带标识无效"))?;
    let mut reader = quick_xml::Reader::from_reader(BufReader::new(
        File::open(directory.join(&metadata.name)).map_err(invalid)?,
    ));
    reader.config_mut().trim_text(true);
    let mut buffer = vec![];
    let mut stack: Vec<String> = vec![];
    let mut annotation = None;
    let mut matched = None;
    loop {
        cancelled(cancel)?;
        match reader.read_event_into(&mut buffer).map_err(invalid)? {
            Event::Start(ref node) => {
                let name = local(node.name().as_ref());
                if stack.last().is_some_and(|parent| parent == "metadata") && name == "product" {
                    annotation = Some(Annotation::default());
                }
                stack.push(name);
                if stack.ends_with(&["swathTiming".into(), "burstList".into(), "burst".into()]) {
                    if let Some(ref mut current) = annotation {
                        current.bursts += 1;
                    }
                }
            }
            Event::Text(ref text) => {
                if let Some(ref mut current) = annotation {
                    let value = text.as_ref().to_owned();
                    let path = stack.join("/");
                    match path.as_str() {
                        "burst/metadata/product/swath" => current.swath = value,
                        "burst/metadata/product/polarisation" => current.pol = value,
                        "burst/metadata/product/content/swathTiming/samplesPerBurst" => {
                            current.width = value.parse().map_err(invalid)?
                        }
                        "burst/metadata/product/content/swathTiming/linesPerBurst" => {
                            current.height = value.parse().map_err(invalid)?
                        }
                        "burst/metadata/product/content/adsHeader/absoluteOrbitNumber" => {
                            current.orbit = value
                        }
                        "burst/metadata/product/content/adsHeader/missionDataTakeId" => {
                            current.take = value
                        }
                        "burst/metadata/product/content/adsHeader/missionId" => {
                            current.mission = value
                        }
                        _ => {}
                    }
                }
            }
            Event::End(ref node) => {
                if local(node.name().as_ref()) == "product" && stack.len() == 3 {
                    if let Some(current) = annotation.take() {
                        if current.swath == swath && current.pol == pol {
                            if matched.is_some() {
                                return Err(invalid("Burst 标注重复"));
                            }
                            matched = Some(current);
                        }
                    }
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    let annotation = matched.ok_or_else(|| invalid("XML 缺少所需子波束和极化的标注"))?;
    let shape = tiff(&directory.join(&file.name), cancel)?;
    if annotation.width != shape.width
        || annotation.height != shape.height
        || index >= annotation.bursts
    {
        return Err(invalid("TIFF 尺寸或 Burst 索引与 XML 不一致"));
    }
    if annotation.orbit.parse::<u64>().ok() != Some(expected_orbit)
        || annotation.take.parse::<u64>().ok() != Some(expected_take)
        || !source.starts_with(&format!("{}_", annotation.mission))
    {
        return Err(invalid("XML 的源场景轨道或数据条带不匹配"));
    }
    Ok(BurstMapping {
        source_slc: source.into(),
        swath: swath.into(),
        polarization: pol.into(),
        burst_index: index,
        shape,
    })
}

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn test_tiff(width: u32, height: u32) -> Vec<u8> {
    let mut data = b"II\x2a\0\x08\0\0\0".to_vec();
    data.extend(9u16.to_le_bytes());
    let offset = 8 + 2 + 9 * 12 + 4;
    for (tag, value) in [
        (256u16, width),
        (257, height),
        (258, 32),
        (259, 1),
        (273, offset),
        (277, 1),
        (278, height),
        (279, width * height * 4),
        (339, 5),
    ] {
        data.extend(tag.to_le_bytes());
        data.extend(4u16.to_le_bytes());
        data.extend(1u32.to_le_bytes());
        data.extend(value.to_le_bytes());
    }
    data.extend(0u32.to_le_bytes());
    data.resize(offset as usize + (width * height * 4) as usize, 1);
    data
}
#[cfg(test)]
pub(crate) fn test_zip() -> Vec<u8> {
    tests::test_zip()
}
