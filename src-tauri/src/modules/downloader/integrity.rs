//! Object identity, retry policy and final checksums. No catalog-size assumptions.
#[cfg(test)]
#[path = "integrity_tests.rs"]
mod tests;
use reqwest::{RequestBuilder, Response, StatusCode, header::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteIdentity {
    pub resource_key: String,
    pub endpoint_key: String,
    pub total: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}
fn digest(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}
/// Keep rejected bytes and their checkpoint together. Never truncate an untrusted partial.
pub fn isolate(payload: &Path, checkpoint: &Path) -> Result<(), String> {
    let parent = payload.parent().ok_or("INVALID_STAGING_PATH")?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let destination = parent.join(format!("rejected-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&destination).map_err(|e| format!("CHECKPOINT_ISOLATION_FAILED: {e}"))?;
    if checkpoint.exists() {
        std::fs::copy(checkpoint, destination.join("state.json"))
            .map_err(|e| format!("CHECKPOINT_ISOLATION_FAILED: {e}"))?;
    }
    std::fs::rename(payload, destination.join("payload.part"))
        .map_err(|e| format!("CHECKPOINT_ISOLATION_FAILED: {e}"))?;
    Ok(())
}
fn endpoint(url: &reqwest::Url) -> String {
    // Authorization query changes do not change an object's endpoint; original resource is separately bound.
    let mut url = url.clone();
    url.set_query(None);
    url.set_fragment(None);
    digest(url.as_str().as_bytes())
}
impl RemoteIdentity {
    pub fn from_response(resource: &str, response: &Response, total: u64) -> Self {
        let headers = response.headers();
        let etag = headers
            .get(ETAG)
            .and_then(|h| h.to_str().ok())
            .filter(|s| {
                s.len() >= 2
                    && s.starts_with('"')
                    && s.ends_with('"')
                    && s.as_bytes()[1..s.len() - 1]
                        .iter()
                        .all(|b| *b == 0x21 || (0x23..=0x7e).contains(b))
            })
            .map(str::to_string);
        let modified = headers.get(LAST_MODIFIED).and_then(|h| h.to_str().ok());
        let strong_date = modified
            .zip(headers.get(DATE).and_then(|h| h.to_str().ok()))
            .is_some_and(|(modified, date)| {
                match (
                    chrono::DateTime::parse_from_rfc2822(modified),
                    chrono::DateTime::parse_from_rfc2822(date),
                ) {
                    (Ok(m), Ok(d)) => (d - m).num_seconds() >= 60,
                    _ => false,
                }
            });
        Self {
            resource_key: digest(resource.as_bytes()),
            endpoint_key: endpoint(response.url()),
            total,
            etag,
            last_modified: if strong_date {
                modified.map(str::to_string)
            } else {
                None
            },
        }
    }
    pub fn reliable(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
    pub fn condition(&self, request: RequestBuilder) -> RequestBuilder {
        if let Some(tag) = &self.etag {
            request.header(IF_MATCH, tag)
        } else if let Some(date) = &self.last_modified {
            request.header(IF_UNMODIFIED_SINCE, date)
        } else {
            request
        }
    }
    pub fn validate(&self, response: &Response) -> Result<(), String> {
        if response.status() == StatusCode::PRECONDITION_FAILED {
            return Err("REMOTE_CHANGED: 远端对象版本已变化".into());
        }
        if self.endpoint_key != endpoint(response.url()) {
            return Err("REMOTE_CHANGED: 下载响应切换了资源地址，禁止混用镜像断点".into());
        }
        let same = if let Some(tag) = &self.etag {
            response.headers().get(ETAG).and_then(|h| h.to_str().ok()) == Some(tag)
        } else if let Some(date) = &self.last_modified {
            response
                .headers()
                .get(LAST_MODIFIED)
                .and_then(|h| h.to_str().ok())
                == Some(date)
        } else {
            true
        };
        if !same {
            return Err("REMOTE_CHANGED: 响应版本与探测不一致".into());
        }
        reject_html(response.headers())
    }
}
pub fn reject_html(headers: &HeaderMap) -> Result<(), String> {
    if headers
        .get(CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| {
            s.to_ascii_lowercase().contains("text/html")
                || s.to_ascii_lowercase().contains("application/xhtml")
        })
    {
        return Err("AUTH_FAILED: 响应为 HTML 页面而非影像".into());
    }
    Ok(())
}
pub async fn cancelled(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
pub fn retry_delay(headers: &HeaderMap, attempt: u32) -> Result<Duration, String> {
    let seconds = if let Some(value) = headers.get(RETRY_AFTER) {
        let value = value.to_str().map_err(|_| "RETRY_AFTER_INVALID")?;
        if let Ok(n) = value.parse::<u64>() {
            n
        } else {
            let date =
                chrono::DateTime::parse_from_rfc2822(value).map_err(|_| "RETRY_AFTER_INVALID")?;
            let milliseconds = (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
                .num_milliseconds()
                .max(0) as u64;
            milliseconds.div_ceil(1000)
        }
    } else {
        1u64 << attempt.min(3)
    };
    if seconds > 60 {
        return Err(format!(
            "REMOTE_RATE_LIMIT: 服务端要求等待 {seconds} 秒，请稍后重试"
        ));
    }
    let jitter = (chrono::Utc::now().timestamp_subsec_millis() % 201) as u64;
    Ok(Duration::from_secs(seconds) + Duration::from_millis(jitter))
}
pub async fn wait_retry(delay: Duration, cancel: &AtomicBool) -> Result<(), String> {
    tokio::select! {_=tokio::time::sleep(delay)=>Ok(()),_=cancelled(cancel)=>Err("TASK_CANCELLED".into())}
}
pub async fn send(request: RequestBuilder, cancel: &AtomicBool) -> Result<Response, String> {
    for attempt in 0..=3 {
        let next = request.try_clone().ok_or("REQUEST_NOT_REPLAYABLE")?;
        let result = tokio::select! {r=next.send()=>r,_=cancelled(cancel)=>return Err("TASK_CANCELLED".into())};
        match result {
            Ok(response) => {
                if response.status() != StatusCode::TOO_MANY_REQUESTS
                    && !response.status().is_server_error()
                {
                    return Ok(response);
                }
                if attempt == 3 {
                    return Err(format!("HTTP_RETRY_EXHAUSTED: {}", response.status()));
                }
                let delay = retry_delay(response.headers(), attempt)?;
                drop(response);
                wait_retry(delay, cancel).await?;
            }
            Err(_) => {
                if attempt == 3 {
                    return Err("NETWORK_RETRY_EXHAUSTED".into());
                }
                wait_retry(retry_delay(&HeaderMap::new(), attempt)?, cancel).await?;
            }
        }
    }
    unreachable!()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub verified_at: String,
    pub checks: Vec<String>,
    pub level: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<Checksum>,
    pub supplier_algorithm: Option<String>,
    pub supplier_value: Option<String>,
    pub bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: String,
    pub value: String,
}
impl Verification {
    pub fn digest(&self) -> Checksum {
        self.checksum.clone().unwrap_or_else(|| Checksum {
            algorithm: "sha256".into(),
            value: self.sha256.clone(),
        })
    }
}
pub fn checksum_metadata(file: &super::types::DownloadFile) -> Result<(String, String), String> {
    let algorithm = file
        .hash_type
        .as_deref()
        .unwrap_or("none")
        .to_ascii_lowercase()
        .replace('-', "");
    let expected = file
        .hash_value
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !matches!(algorithm.as_str(), "none" | "" | "sha256" | "md5") {
        return Err(format!("HASH_ALGORITHM_UNSUPPORTED: {algorithm}"));
    }
    if !expected.is_empty() && !matches!(algorithm.as_str(), "sha256" | "md5") {
        return Err("HASH_METADATA_INVALID".into());
    }
    if matches!(algorithm.as_str(), "sha256" | "md5")
        && (expected.len() != if algorithm == "md5" { 32 } else { 64 }
            || !expected.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("HASH_METADATA_INVALID".into());
    }
    if !file.sha256.is_empty()
        && (file.sha256.len() != 64 || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("HASH_METADATA_INVALID".into());
    }
    Ok((algorithm, expected))
}
#[cfg(test)]
pub fn verify(
    path: &Path,
    total: u64,
    file: &super::types::DownloadFile,
    cancel: &AtomicBool,
) -> Result<Verification, String> {
    verify_with_progress(path, total, file, cancel, |_| {})
}
pub fn verify_with_progress(
    path: &Path,
    total: u64,
    file: &super::types::DownloadFile,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<Verification, String> {
    let (algorithm, expected) = checksum_metadata(file)?;
    // A supplied SHA-256 takes precedence over MD5. Otherwise use the supplier's
    // algorithm, falling back to SHA-256 only when no supplier digest exists.
    let use_md5 = algorithm == "md5" && file.sha256.is_empty();
    let started = std::time::Instant::now();
    let mut tick = started;
    progress(0);
    let mut reader = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut sha = Sha256::new();
    let mut md5 = md5::Context::new();
    let mut bytes = 0;
    let mut read_time = std::time::Duration::ZERO;
    let mut hash_time = std::time::Duration::ZERO;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("TASK_CANCELLED".into());
        }
        let read_started = std::time::Instant::now();
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        read_time += read_started.elapsed();
        if n == 0 {
            break;
        }
        let hash_started = std::time::Instant::now();
        if use_md5 {
            md5.consume(&buffer[..n]);
        } else {
            sha.update(&buffer[..n]);
        }
        hash_time += hash_started.elapsed();
        bytes += n as u64;
        if tick.elapsed() >= std::time::Duration::from_millis(500) {
            progress(bytes.min(total.saturating_sub(1)));
            tick = std::time::Instant::now();
        }
    }
    if bytes != total {
        return Err("SIZE_MISMATCH".into());
    }
    let actual = if use_md5 {
        format!("{:x}", md5.compute())
    } else {
        hex::encode(sha.finalize())
    };
    if (!file.sha256.is_empty() && !actual.eq_ignore_ascii_case(&file.sha256))
        || (algorithm == "sha256" && expected != actual)
        || (use_md5 && expected != actual)
    {
        return Err("HASH_MISMATCH: 文件内容与供应商校验值不一致".into());
    }
    let supplied = !expected.is_empty() || !file.sha256.is_empty();
    let selected = if use_md5 { "md5" } else { "sha256" };
    if cancel.load(Ordering::SeqCst) {
        return Err("TASK_CANCELLED".into());
    }
    let xml = file.name.to_ascii_lowercase().ends_with(".xml");
    if xml {
        verify_xml(path, cancel)?;
    }
    let structure_checks = super::structure::check(path, file, cancel)?;
    progress(bytes);
    tracing::info!(
        algorithm = selected,
        bytes,
        elapsed_ms = started.elapsed().as_millis() as u64,
        read_ms = read_time.as_millis() as u64,
        hash_ms = hash_time.as_millis() as u64,
        "download verification completed"
    );
    let mut checks = if supplied {
        vec![
            "remote_length".into(),
            format!("local_{selected}"),
            "supplier_checksum".into(),
        ]
    } else {
        vec!["remote_length".into(), "local_sha256".into()]
    };
    if xml {
        checks.push("xml_document".into());
    }
    checks.extend(structure_checks);
    Ok(Verification {
        verified_at: chrono::Utc::now().to_rfc3339(),
        checks,
        level: if supplied {
            "supplier_checksum"
        } else {
            "local_hash"
        }
        .into(),
        sha256: if use_md5 {
            String::new()
        } else {
            actual.clone()
        },
        checksum: Some(Checksum {
            algorithm: selected.into(),
            value: actual,
        }),
        supplier_algorithm: if !file.sha256.is_empty() {
            Some("sha256".into())
        } else if !expected.is_empty() {
            Some(algorithm)
        } else if supplied {
            Some("sha256".into())
        } else {
            None
        },
        supplier_value: if !file.sha256.is_empty() {
            Some(file.sha256.to_ascii_lowercase())
        } else if !expected.is_empty() {
            Some(expected)
        } else if supplied {
            Some(file.sha256.to_ascii_lowercase())
        } else {
            None
        },
        bytes,
    })
}

fn verify_xml(path: &Path, cancel: &AtomicBool) -> Result<(), String> {
    use quick_xml::events::Event;
    let input = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
    let mut reader = quick_xml::Reader::from_reader(input);
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut root = false;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("TASK_CANCELLED".into());
        }
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|e| format!("XML_INVALID: {e}"))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(event) | Event::Empty(event) => {
                if depth == 0 {
                    if root {
                        return Err("XML_INVALID: XML 必须只有一个根元素".into());
                    }
                    let name = event.local_name();
                    if ["html", "error"]
                        .iter()
                        .any(|v| name.as_ref().eq_ignore_ascii_case(v))
                    {
                        return Err("XML_INVALID: 响应是错误页面而非 Burst 元数据".into());
                    }
                    root = true;
                }
                for attr in event.attributes() {
                    attr.map_err(|e| format!("XML_INVALID: {e}"))?;
                }
                // Empty elements do not increase nesting depth.
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or("XML_INVALID: XML 结构无效")?;
            }
            Event::Text(text)
                if depth == 0 && text.as_ref().bytes().any(|c| !c.is_ascii_whitespace()) =>
            {
                return Err("XML_INVALID: 响应不是有效 XML 文档".into());
            }
            Event::CData(_) if depth == 0 => return Err("XML_INVALID: XML 结构无效".into()),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !root || depth != 0 {
        return Err("XML_INVALID: XML 文档为空或不完整".into());
    }
    Ok(())
}
