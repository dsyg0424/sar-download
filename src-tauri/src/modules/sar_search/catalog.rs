use super::{
    product::{SarProduct, normalize},
    request::SearchRequest,
};
use futures_util::{StreamExt, TryStreamExt, stream};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

pub struct Catalog {
    client: reqwest::Client,
    slots: Arc<tokio::sync::Semaphore>,
    base: String,
}
impl Catalog {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            slots: Arc::new(tokio::sync::Semaphore::new(16)),
            base: "https://cmr.earthdata.nasa.gov/search".into(),
        }
    }
    async fn get(
        &self,
        path: &str,
        parameters: &[(String, String)],
        token: Option<&str>,
    ) -> Result<(reqwest::header::HeaderMap, Value), String> {
        let _permit = self.slots.acquire().await.map_err(|e| e.to_string())?;
        for attempt in 0..3 {
            let mut request = self
                .client
                .get(format!("{}{path}", self.base))
                .query(parameters);
            if let Some(token) = token {
                request = request.header("CMR-Search-After", token);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    if attempt < 2 && (status.as_u16() == 429 || status.is_server_error()) {
                        let delay = response
                            .headers()
                            .get("retry-after")
                            .and_then(|s| s.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(1 << attempt)
                            .min(60);
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        continue;
                    }
                    if !status.is_success() {
                        return Err(format!("CMR 请求失败：HTTP {status}"));
                    }
                    if response
                        .headers()
                        .get("CMR-Timed-Out")
                        .is_some_and(|v| v == "true")
                    {
                        return Err("CMR 返回超时结果，请缩小范围重试".into());
                    }
                    let headers = response.headers().clone();
                    let value = response
                        .json()
                        .await
                        .map_err(|_| "CMR 返回无效 JSON".to_string())?;
                    return Ok((headers, value));
                }
                Err(_) if attempt < 2 => {
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await
                }
                Err(_) => return Err("无法连接 NASA CMR，请检查网络后重试".into()),
            }
        }
        unreachable!()
    }
    async fn pages(
        &self,
        parameters: &[(String, String)],
        format: &str,
    ) -> Result<BTreeMap<String, Value>, String> {
        self.pages_progress(parameters, format, |_| Ok(())).await
    }
    async fn pages_progress(
        &self,
        parameters: &[(String, String)],
        format: &str,
        page: impl Fn(&[Value]) -> Result<(), String>,
    ) -> Result<BTreeMap<String, Value>, String> {
        let mut state = PageState::default();
        loop {
            let (headers, body) = self
                .get(
                    &format!("/granules.{format}"),
                    parameters,
                    state.token.as_deref(),
                )
                .await?;
            let complete = state.accept(&headers, body, format)?;
            page(&state.added)?;
            if complete {
                return Ok(state.records);
            }
        }
    }
    pub async fn search(
        &self,
        request: &SearchRequest,
        kind: &str,
        emit: impl Fn(Value) + Clone,
    ) -> Result<Vec<SarProduct>, String> {
        let parameters = request.parameters(kind)?;
        emit(json!({"event":"stage","product":kind,"stage":"batch_umm","state":"started"}));
        emit(json!({"event":"stage","product":kind,"stage":"json_index","state":"started"}));
        let details = self.pages_progress(&parameters, "umm_json", |records| {
            let mut products = vec![];
            for record in records {
                if let Ok(product) = normalize(record, kind, None) {
                    if request.matches_geometry(&product.geometry)? { products.push(product); }
                }
            }
            if !products.is_empty() {
                emit(json!({"event":"product_page","product":kind,"data":{"results":products,"limitReached":false}}));
            }
            Ok(())
        });
        let index = self.pages(&parameters, "json");
        let (details, index) = tokio::try_join!(details, index)?;
        emit(json!({"event":"stage","product":kind,"stage":"batch_parse","state":"started"}));
        let mut results = Vec::with_capacity(index.len());
        let mut missing = vec![];
        for (cid, entry) in &index {
            let granule = entry["title"].as_str().ok_or("CMR 索引缺少产品标识")?;
            match details
                .get(cid)
                .and_then(|v| normalize(v, kind, Some(granule)).ok())
            {
                Some(product) => results.push(product),
                None => missing.push((cid.clone(), granule.to_string())),
            }
        }
        if !missing.is_empty() {
            emit(
                json!({"event":"stage","product":kind,"stage":"concept_recovery","state":"started"}),
            );
            let recovered: Vec<SarProduct> =
                stream::iter(missing.into_iter().map(|(cid, granule)| async move {
                    // concept IDs are CMR-generated, but still validate before building a path.
                    if !cid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                        return Err("无效 CMR concept-id".into());
                    }
                    let (_, umm) = self
                        .get(&format!("/concepts/{cid}.umm_json"), &[], None)
                        .await?;
                    normalize(
                        &json!({"umm":umm,"meta":{"concept-id":cid}}),
                        kind,
                        Some(&granule),
                    )
                    .map_err(|e| format!("CMR 元数据补取失败（{cid}）：{e}"))
                }))
                .buffer_unordered(16)
                .try_collect()
                .await?;
            results.extend(recovered);
        }
        if results.len() != index.len() {
            return Err("CMR 产品目录不完整，请重新检索".into());
        }
        // Bounding-box requests avoid CMR polygon winding and holes limitations.
        // Exact user geometry, including holes and multipolygons, is applied locally.
        let mut filtered = vec![];
        for product in results {
            if request.matches_geometry(&product.geometry)? {
                filtered.push(product);
            }
        }
        filtered.sort_by(|a, b| b.date.cmp(&a.date).then(a.id.cmp(&b.id)));
        Ok(filtered)
    }
}

#[derive(Default)]
struct PageState {
    records: BTreeMap<String, Value>,
    expected: Option<u64>,
    token: Option<String>,
    tokens: HashSet<String>,
    added: Vec<Value>,
}
impl PageState {
    fn accept(
        &mut self,
        headers: &reqwest::header::HeaderMap,
        body: Value,
        format: &str,
    ) -> Result<bool, String> {
        self.added.clear();
        let hits = body["hits"]
            .as_u64()
            .or_else(|| {
                headers
                    .get("CMR-Hits")
                    .and_then(|v| v.to_str().ok())?
                    .parse()
                    .ok()
            })
            .ok_or("CMR 缺少总记录数")?;
        if self.expected.is_some_and(|expected| expected != hits) {
            return Err("CMR 目录在分页期间发生变化，请重新检索".into());
        }
        self.expected = Some(hits);
        let records = if format == "json" {
            body["feed"]["entry"].as_array()
        } else {
            body["items"].as_array()
        }
        .ok_or("CMR 分页结构无效")?;
        for record in records {
            let id = if format == "json" {
                record["id"].as_str()
            } else {
                record["meta"]["concept-id"].as_str()
            }
            .ok_or("CMR 缺少记录 ID")?;
            if self.records.insert(id.into(), record.clone()).is_none() {
                self.added.push(record.clone());
            }
        }
        if self.records.len() as u64 > hits {
            return Err("CMR 返回记录数超过目录总数".into());
        }
        if self.records.len() as u64 == hits {
            return Ok(true);
        }
        let next = headers
            .get("CMR-Search-After")
            .and_then(|v| v.to_str().ok());
        let Some(token) = next else {
            // UMM can omit broken records; the JSON index and concept recovery
            // establish completeness. Missing JSON records are always an error.
            if format == "json" {
                return Err(format!("CMR 目录不完整：{}/{hits}", self.records.len()));
            }
            return Ok(true);
        };
        if !self.tokens.insert(token.into()) {
            return Err("CMR 返回重复分页游标".into());
        }
        self.token = Some(token.into());
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(hits: &str, token: Option<&str>) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("cmr-hits", hits.parse().unwrap());
        if let Some(token) = token {
            h.insert("cmr-search-after", token.parse().unwrap());
        }
        h
    }
    fn page(ids: &[&str]) -> Value {
        json!({"feed":{"entry":ids.iter().map(|id| json!({"id":id})).collect::<Vec<_>>()}})
    }
    #[test]
    fn short_pages_continue_and_deduplicate() {
        let mut s = PageState::default();
        assert!(
            !s.accept(&headers("3", Some("a")), page(&["1"]), "json")
                .unwrap()
        );
        assert!(
            !s.accept(&headers("3", Some("b")), page(&["1", "2"]), "json")
                .unwrap()
        );
        assert!(s.accept(&headers("3", None), page(&["3"]), "json").unwrap());
    }
    #[test]
    fn incomplete_changing_and_repeated_pages_fail() {
        assert!(
            PageState::default()
                .accept(&headers("2", None), page(&["1"]), "json")
                .is_err()
        );
        let mut s = PageState::default();
        s.accept(&headers("3", Some("a")), page(&["1"]), "json")
            .unwrap();
        assert!(
            s.accept(&headers("4", Some("b")), page(&["2"]), "json")
                .is_err()
        );
        assert!(
            s.accept(&headers("3", Some("a")), page(&["1"]), "json")
                .is_err()
        );
    }

    #[tokio::test]
    async fn parsed_page_is_emitted_while_parallel_index_is_still_waiting() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let fixture: Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/cmr/BURST.json"))
                .unwrap();
        let item = fixture["items"][0].clone();
        let concept = item["meta"]["concept-id"].as_str().unwrap().to_string();
        let title = item["umm"]["GranuleUR"].as_str().unwrap().to_string();
        let index_started = Arc::new(tokio::sync::Notify::new());
        let page_seen = Arc::new(tokio::sync::Notify::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started, seen) = (index_started.clone(), page_seen.clone());
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (started, seen, item, concept, title) = (
                    started.clone(),
                    seen.clone(),
                    item.clone(),
                    concept.clone(),
                    title.clone(),
                );
                tokio::spawn(async move {
                    let mut request = vec![0; 8192];
                    let count = socket.read(&mut request).await.unwrap();
                    let request = String::from_utf8_lossy(&request[..count]);
                    let body = if request.contains("granules.umm_json") {
                        started.notified().await;
                        json!({"hits":1,"items":[item]})
                    } else {
                        started.notify_one();
                        seen.notified().await;
                        json!({"feed":{"entry":[{"id":concept,"title":title}]},"hits":1})
                    };
                    let body = body.to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        let catalog = Catalog {
            client: reqwest::Client::new(),
            slots: Arc::new(tokio::sync::Semaphore::new(16)),
            base: format!("http://{address}"),
        };
        let request:SearchRequest=serde_json::from_value(json!({"bounds":[-180.0,-89.0,180.0,89.0],"start":"2014-10-03","end":"2014-10-03","productTypes":["BURST"],"satellites":[],"direction":"","polarization":"","geometry":null})).unwrap();
        let events = Arc::new(std::sync::Mutex::new(vec![]));
        let collected = events.clone();
        let results = tokio::time::timeout(
            Duration::from_secs(4),
            catalog.search(&request, "BURST", move |event| {
                if event["event"] == "product_page" {
                    page_seen.notify_one();
                }
                collected.lock().unwrap().push(event);
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event["event"] == "product_page")
        );
        server.abort();
    }
}
