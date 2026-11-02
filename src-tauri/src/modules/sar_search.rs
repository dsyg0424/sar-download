pub mod catalog;
mod cancellation;
pub mod product;
pub mod request;
use futures_util::{StreamExt, TryStreamExt, stream};
use product::SarProduct;
use request::SearchRequest;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tauri::ipc::Channel;

static SAR_QUERY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn search(
    request: SearchRequest,
    emit: impl Fn(Value) + Clone + Send + Sync + 'static,
) -> Result<Vec<SarProduct>, String> {
    request.validate()?;
    let client = reqwest::Client::builder()
        .user_agent("SAR-download/0.1 (native Rust CMR client)")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;
    let catalog = Arc::new(catalog::Catalog::new(client));
    let request = Arc::new(request);
    let products = request.products();
    let results: Vec<Vec<SarProduct>> = stream::iter(products.into_iter().map(|kind| {
        let catalog = catalog.clone(); let request = request.clone(); let emit = emit.clone();
        async move {
            emit(json!({"event":"product_started","product":kind}));
            let results = catalog.search(&request, &kind, emit.clone()).await?;
            emit(json!({"event":"product_complete","product":kind,"data":{"results":results,"limitReached":false}}));
            Ok::<_, String>(results)
        }
    })).buffer_unordered(4).try_collect().await?;
    let mut unique = BTreeMap::new();
    for product in results.into_iter().flatten() {
        unique.insert(product.id.clone(), product);
    }
    let mut results: Vec<_> = unique.into_values().collect();
    results.sort_by(|a, b| b.date.cmp(&a.date).then(a.id.cmp(&b.id)));
    Ok(results)
}

#[tauri::command]
pub async fn sar_search(request: SearchRequest, progress: Channel<Value>, request_id: String) -> Result<Value, String> {
    let _guard = SAR_QUERY_LOCK
        .try_lock()
        .map_err(|_| "已有 SAR 查询正在执行，请等待完成")?;
    let registration = cancellation::register(&request_id)?;
    let mut cancelled = registration.receiver.clone();
    if *cancelled.borrow() { return Err("SAR_SEARCH_CANCELLED".into()); }
    let results = tokio::select! {
      biased;
      _ = cancelled.changed() => return Err("SAR_SEARCH_CANCELLED".into()),
      result = tokio::time::timeout(
        Duration::from_secs(600),
        search(request, move |event| {
            let _ = progress.send(event);
        }),
    ) => result,
    }
    .map_err(|_| "检索超时，请缩小区域或日期范围后重试".to_string())??;
    Ok(json!({"results":results,"limitReached":false,"warnings":[]}))
}

#[cfg(test)]
mod tests;

#[tauri::command]
pub fn cancel_sar_search(request_id: String) -> Result<(), String> {
    cancellation::cancel(&request_id)
}
