use reqwest::{
    StatusCode,
    header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, HeaderMap},
};

pub fn content_range(headers: &HeaderMap) -> Option<(u64, u64, u64)> {
    let value = headers
        .get(CONTENT_RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (start.parse().ok()?, end.parse().ok()?, total.parse().ok()?);
    (start <= end && end < total).then_some((start, end, total))
}
pub fn probe_size(status: StatusCode, headers: &HeaderMap) -> Option<u64> {
    if headers
        .get(CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return None;
    }
    if status == StatusCode::PARTIAL_CONTENT {
        let (start, end, total) = content_range(headers)?;
        (start == 0 && end == 0).then_some(total)
    } else if status == StatusCode::OK {
        headers
            .get(CONTENT_LENGTH)?
            .to_str()
            .ok()?
            .parse::<u64>()
            .ok()
            .filter(|size| *size > 0)
    } else {
        None
    }
}
/// Only a full, unencoded response without a declared length is a streaming fallback.
/// Malformed ranges, explicit zero lengths and errors never become unknown-length downloads.
pub fn unknown_length(status: StatusCode, headers: &HeaderMap) -> bool {
    status == StatusCode::OK
        && !headers.contains_key(CONTENT_LENGTH)
        && !headers.contains_key(CONTENT_RANGE)
        && headers
            .get(CONTENT_ENCODING)
            .is_none_or(|v| v == "identity")
}
pub fn validate_response(
    status: StatusCode,
    headers: &HeaderMap,
    start: u64,
    end: u64,
    total: u64,
) -> Result<(), String> {
    if headers
        .get(CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return Err("下载响应使用了非预期内容编码".into());
    }
    let valid = if status == StatusCode::PARTIAL_CONTENT {
        content_range(headers) == Some((start, end, total))
    } else {
        status == StatusCode::OK && start == 0 && end.checked_add(1) == Some(total)
    };
    if !valid {
        return Err(format!(
            "分块响应范围不匹配：请求 bytes={start}-{end}/{total}，响应 {status} {:?}",
            headers.get(CONTENT_RANGE)
        ));
    }
    if let Some(length) = headers.get(CONTENT_LENGTH) {
        let length = length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        if length != Some(end - start + 1) {
            return Err("分块 Content-Length 与请求范围不匹配".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_probe_never_confuses_partial_length_with_object_size() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_LENGTH, "1".parse().unwrap());
        assert_eq!(probe_size(StatusCode::PARTIAL_CONTENT, &headers), None);
        headers.insert(CONTENT_RANGE, "bytes 0-0/4372613273".parse().unwrap());
        assert_eq!(
            probe_size(StatusCode::PARTIAL_CONTENT, &headers),
            Some(4372613273)
        );
        headers.insert(CONTENT_RANGE, "bytes 10-10/4372613273".parse().unwrap());
        assert_eq!(probe_size(StatusCode::PARTIAL_CONTENT, &headers), None);
        assert_eq!(probe_size(StatusCode::ACCEPTED, &headers), None);
    }
    #[test]
    fn only_exact_ranges_or_full_object_responses_are_accepted() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_RANGE, "bytes 10-19/100".parse().unwrap());
        headers.insert(CONTENT_LENGTH, "10".parse().unwrap());
        assert!(validate_response(StatusCode::PARTIAL_CONTENT, &headers, 10, 19, 100).is_ok());
        assert!(validate_response(StatusCode::PARTIAL_CONTENT, &headers, 0, 9, 100).is_err());
        assert!(validate_response(StatusCode::PARTIAL_CONTENT, &headers, 10, 19, 101).is_err());
        assert!(validate_response(StatusCode::OK, &headers, 0, 9, 100).is_err());
        headers.remove(CONTENT_RANGE);
        headers.insert(CONTENT_LENGTH, "100".parse().unwrap());
        assert!(validate_response(StatusCode::OK, &headers, 0, 99, 100).is_ok());
    }
}
