use futures_util::StreamExt;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::fs::OpenOptions;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::auth::authenticate_client;
use super::manager::DownloadManager;
use super::types::{DownloadFile, DownloadState, SerializableCookie, TaskStatus};

#[derive(Default)]
pub struct TransferMetrics {
    pub verified: std::sync::atomic::AtomicU64,
    pub received: std::sync::atomic::AtomicU64,
    pub durable: std::sync::atomic::AtomicU64,
}

pub async fn run_download_core<F>(
    manager: Arc<DownloadManager>,
    file: DownloadFile,
    save_dir: String,
    task_dir: std::path::PathBuf,
    username: String,
    password: String,
    concurrency: u32,
    cookies: Option<Vec<SerializableCookie>>,
    external_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    metrics: Option<Arc<TransferMetrics>>,
    progress_reporter: F,
) where
    F: Fn(u64, u64, f64, &str, Option<String>) + Send + Sync + Clone + 'static,
{
    let metrics = metrics.unwrap_or_default();
    let concurrency = concurrency.clamp(1, crate::modules::settings::MAX_FILE_CONNECTIONS);
    let file_name = file.name.clone();
    let task_id = file.task_id.clone();
    let cancel_flag = if let Some(flag) = external_cancel {
        manager
            .active_cancels
            .lock()
            .unwrap()
            .insert(task_id.clone(), flag.clone());
        flag
    } else {
        manager.register(&task_id)
    };

    let _file_permit = match manager.budget.file(&cancel_flag).await {
        Ok(permit) => permit,
        Err(_) => {
            progress_reporter(0, file.size, 0.0, TaskStatus::Paused.as_str(), None);
            manager.unregister(&task_id);
            return;
        }
    };
    progress_reporter(0, file.size, 0.0, TaskStatus::Connecting.as_str(), None);

    if let Err(error) = super::integrity::checksum_metadata(&file) {
        progress_reporter(0, file.size, 0.0, TaskStatus::Failed.as_str(), Some(error));
        manager.unregister(&task_id);
        return;
    }

    let final_dest_path = std::path::Path::new(&save_dir).join(&file.name);

    if !task_dir.exists() {
        let _ = tokio::fs::create_dir_all(&task_dir).await;
    }

    let task_options = match super::options::read_options(&task_dir) {
        Ok(options) => options,
        Err(error) => {
            progress_reporter(0, file.size, 0.0, TaskStatus::Failed.as_str(), Some(error));
            manager.unregister(&task_id);
            return;
        }
    };
    let temp_dest_path = if task_options
        .as_ref()
        .is_some_and(|options| options.schema_version == 1)
    {
        std::path::Path::new(&save_dir)
            .join(".sar-download")
            .join("staging")
            .join(&task_id)
            .join("payload.part")
    } else {
        std::path::Path::new(&save_dir).join(format!("{}.downloading", file.name))
    };
    if let Some(parent) = temp_dest_path.parent() {
        if let Err(error) = tokio::fs::create_dir_all(parent).await {
            progress_reporter(
                0,
                file.size,
                0.0,
                TaskStatus::Failed.as_str(),
                Some(error.to_string()),
            );
            manager.unregister(&task_id);
            return;
        }
    }
    let state_path = task_dir.join("state.json");

    tracing::info!(
        "初始化下载. ID: {}, 文件: [{}], 临时目录: {}",
        task_id,
        file_name,
        task_dir.display()
    );

    if let Some(ref cookies_list) = cookies {
        if !cookies_list.is_empty() {
            for c in cookies_list {
                let cookie_str = format!(
                    "{}={}; Domain={}; Path={}; Secure; HttpOnly",
                    c.name, c.value, c.domain, c.path
                );
                let host = c.domain.trim_start_matches('.');
                if let Ok(url) = reqwest::Url::parse(&format!("https://{}", host)) {
                    manager.cookie_jar.add_cookie_str(&cookie_str, &url);
                }
            }
        }
    }

    let auth_client = manager.shared_auth_client.clone();
    let download_client = manager.shared_client.clone();

    let metadata_permit = match manager.budget.request(&cancel_flag).await {
        Ok(permit) => permit,
        Err(_) => {
            progress_reporter(0, file.size, 0.0, TaskStatus::Paused.as_str(), None);
            manager.unregister(&task_id);
            return;
        }
    };

    let probe = async {
        let mut authenticated = false;
        for poll in 0..=60 {
            let response = super::integrity::send(download_client.get(&file.url)
                .header(reqwest::header::RANGE, "bytes=0-0")
                .header(reqwest::header::ACCEPT_ENCODING, "identity"), &cancel_flag).await?;
            let status=response.status();
            let login=response.url().host_str()==Some("urs.earthdata.nasa.gov");
            if login || matches!(status.as_u16(),401|403) {
                if authenticated || username.is_empty() || password.is_empty() {
                    return Err("AUTH_FAILED: Earthdata authentication required".to_string());
                }
                let _guard=tokio::select! {guard=manager.auth_mutex.lock()=>guard,_=super::integrity::cancelled(&cancel_flag)=>return Err("TASK_CANCELLED".into())};
                tokio::select! {
                    result=authenticate_client(&auth_client,&file.url,&username,&password)=>{result.map_err(|_|"AUTH_FAILED".to_string())?;},
                    _=super::integrity::cancelled(&cancel_flag)=>return Err("TASK_CANCELLED".into()),
                }
                authenticated=true;
                continue;
            }
            if status==reqwest::StatusCode::ACCEPTED {
                if poll==60 {return Err("REMOTE_PENDING_TIMEOUT".into());}
                super::integrity::wait_retry(Duration::from_secs(3),&cancel_flag).await?;
                continue;
            }
            super::integrity::reject_html(response.headers())?;
            if crate::common::http_range::unknown_length(status,response.headers()) {
                let identity=super::integrity::RemoteIdentity::from_response(&file.url,&response,0);
                return Ok((response.url().to_string(),0,false,identity,Some(response)));
            }
            let total=crate::common::http_range::probe_size(status,response.headers()).ok_or("REMOTE_SIZE_UNAVAILABLE_OR_INVALID_RESPONSE")?;
            let identity=super::integrity::RemoteIdentity::from_response(&file.url,&response,total);
            let ranges=status==reqwest::StatusCode::PARTIAL_CONTENT && identity.reliable();
            return Ok((response.url().to_string(),total,ranges,identity,None));
        }
        Err("REMOTE_PENDING_TIMEOUT".into())
    }.await;
    let (direct_url, total_size, supports_ranges, remote_identity, unknown_response) = match probe {
        Ok(probe) => probe,
        Err(error) => {
            let paused = cancel_flag.load(Ordering::SeqCst);
            progress_reporter(
                0,
                file.size,
                0.0,
                if paused {
                    TaskStatus::Paused.as_str()
                } else {
                    TaskStatus::Failed.as_str()
                },
                if paused { None } else { Some(error) },
            );
            manager.unregister(&task_id);
            return;
        }
    };

    if let Some(response) = unknown_response {
        let result = super::stream::run(
            &manager,
            response,
            &file,
            &temp_dest_path,
            &final_dest_path,
            &task_dir,
            cancel_flag.clone(),
            metrics.clone(),
            progress_reporter.clone(),
        )
        .await;
        if let Err(error) = result {
            let paused = cancel_flag.load(Ordering::SeqCst);
            progress_reporter(
                metrics.durable.load(Ordering::SeqCst),
                0,
                0.0,
                if paused { "Paused" } else { "Failed" },
                if paused { None } else { Some(error) },
            );
        }
        manager.unregister(&task_id);
        return;
    }

    let mut is_resumed = false;
    let mut download_state = if state_path.exists() && temp_dest_path.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&state_path).await {
            if let Ok(parsed) = serde_json::from_str::<DownloadState>(&content) {
                if supports_ranges
                    && parsed.remote_identity.as_ref() == Some(&remote_identity)
                    && parsed.task_id == task_id
                    && parsed.file_name == file_name
                    && parsed.sha256 == file.sha256
                    && parsed.hash_type == file.hash_type.as_deref().unwrap_or("none")
                    && parsed.hash_value == file.hash_value.as_deref().unwrap_or("")
                    && valid_resume(&parsed, total_size, &temp_dest_path)
                {
                    tracing::info!("开启断点续传: [{}]", task_id);
                    is_resumed = true;
                    parsed
                } else {
                    DownloadState::new_fresh(
                        &task_id,
                        &file_name,
                        &file.url,
                        total_size,
                        &file,
                        concurrency,
                    )
                }
            } else {
                DownloadState::new_fresh(
                    &task_id,
                    &file_name,
                    &file.url,
                    total_size,
                    &file,
                    concurrency,
                )
            }
        } else {
            DownloadState::new_fresh(
                &task_id,
                &file_name,
                &file.url,
                total_size,
                &file,
                concurrency,
            )
        }
    } else {
        DownloadState::new_fresh(
            &task_id,
            &file_name,
            &file.url,
            total_size,
            &file,
            concurrency,
        )
    };

    if !is_resumed && temp_dest_path.exists() {
        if let Err(error) = super::integrity::isolate(&temp_dest_path, &state_path) {
            progress_reporter(0, total_size, 0.0, TaskStatus::Failed.as_str(), Some(error));
            manager.unregister(&task_id);
            return;
        }
        tracing::warn!(
            task_id,
            "Untrusted checkpoint isolated; restarting whole object"
        );
        progress_reporter(0, total_size, 0.0, TaskStatus::Connecting.as_str(),
            Some("CHECKPOINT_RESTARTED: 原断点无法确认属于当前远端版本，已保留到 rejected 目录，并重新下载。".into()));
    }
    download_state.remote_identity = Some(remote_identity.clone());
    download_state.verification = None;
    if !supports_ranges {
        download_state.chunks = vec![super::types::ChunkRange {
            start: 0,
            current: 0,
            end: total_size - 1,
            completed: false,
            is_downloading: false,
        }];
    }
    download_state.status = TaskStatus::Downloading.as_str().to_string();
    progress_reporter(
        download_state.hashed_up_to,
        total_size,
        0.0,
        TaskStatus::Downloading.as_str(),
        None,
    );

    download_state.mirrors = vec![direct_url];

    let out_file = if is_resumed {
        match OpenOptions::new().write(true).open(&temp_dest_path).await {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("打开临时下载文件失败: {}, 错误: {}", task_id, e);
                progress_reporter(
                    0,
                    total_size,
                    0.0,
                    TaskStatus::Failed.as_str(),
                    Some(format!("File opening error: {}", e)),
                );
                manager.unregister(&task_id);
                return;
            }
        }
    } else {
        tracing::info!("创建目标位置增量下载文件: [{}]", temp_dest_path.display());
        let f = match OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_dest_path)
            .await
        {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("创建增量文件失败: {}, 错误: {}", task_id, e);
                progress_reporter(
                    0,
                    total_size,
                    0.0,
                    TaskStatus::Failed.as_str(),
                    Some(format!("File creation error: {}", e)),
                );
                manager.unregister(&task_id);
                return;
            }
        };

        if let Ok(content) = serde_json::to_string_pretty(&download_state) {
            if let Err(error) =
                crate::common::atomic_file::write_atomic(&state_path, content.as_bytes())
            {
                progress_reporter(
                    0,
                    total_size,
                    0.0,
                    TaskStatus::Failed.as_str(),
                    Some(format!("CHECKPOINT_WRITE_FAILED: {error}")),
                );
                manager.unregister(&task_id);
                return;
            }
        }

        f
    };

    drop(out_file);

    let mut initial_downloaded = 0;
    for chunk in &download_state.chunks {
        if chunk.completed {
            initial_downloaded += chunk.end.saturating_add(1).saturating_sub(chunk.start);
        } else {
            initial_downloaded += chunk.current.saturating_sub(chunk.start);
        }
    }

    let space_path = std::path::PathBuf::from(&save_dir);
    let mut space = match super::space::SpaceMonitor::new(
        &manager.disk_budget,
        Some(total_size.saturating_sub(initial_downloaded)),
        move || {
            crate::common::disk_space::probe(&space_path)
                .map(|space| (space.volume, space.available))
        },
    ) {
        Ok(space) => space,
        Err(error) => {
            progress_reporter(initial_downloaded, total_size, 0.0, "Failed", Some(error));
            manager.unregister(&task_id);
            return;
        }
    };

    metrics.durable.store(initial_downloaded, Ordering::SeqCst);
    let downloaded_bytes = Arc::new(std::sync::atomic::AtomicU64::new(initial_downloaded));
    let shared_state = Arc::new(Mutex::new(download_state.clone()));

    let (tx, mut rx) = mpsc::channel::<(usize, u64, Vec<u8>)>(32);

    let temp_dest_path_writer = temp_dest_path.clone();
    let shared_state_writer = shared_state.clone();
    let task_dir_writer = task_dir.clone();

    let written_bytes = downloaded_bytes.clone();
    let writer_metrics = metrics.clone();
    let writer_task = tokio::spawn(async move {
        let mut file = match OpenOptions::new()
            .write(true)
            .open(&temp_dest_path_writer)
            .await
        {
            Ok(f) => f,
            Err(e) => return Err(format!("Failed to open file in writer: {}", e)),
        };

        while let Some((chunk_idx, offset, data)) = rx.recv().await {
            {
                let st = shared_state_writer.lock().unwrap();
                let chunk = &st.chunks[chunk_idx];
                if offset != chunk.current
                    || offset
                        .checked_add(data.len() as u64)
                        .is_none_or(|next| next > chunk.end + 1)
                {
                    return Err("Invalid or duplicate chunk write".into());
                }
            }
            space.before_write(data.len() as u64)?;
            if let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await {
                return Err(format!("Seek error: {}", e));
            }
            if let Err(e) = file.write_all(&data).await {
                return Err(format!("Write error: {}", e));
            }

            space.after_write(data.len() as u64);
            written_bytes.fetch_add(data.len() as u64, Ordering::SeqCst);
            let completed = {
                let mut st = shared_state_writer.lock().unwrap();
                let current_offset = offset + data.len() as u64;
                st.chunks[chunk_idx].current = current_offset;

                if current_offset > st.chunks[chunk_idx].end {
                    st.chunks[chunk_idx].completed = true;
                    st.chunks[chunk_idx].is_downloading = false;
                    true
                } else {
                    false
                }
            };

            if completed {
                file.sync_all()
                    .await
                    .map_err(|e| format!("Sync failed: {e}"))?;
                let state_str = {
                    let st = shared_state_writer.lock().unwrap();
                    serde_json::to_string_pretty(&*st).unwrap()
                };
                let state_path = task_dir_writer.join("state.json");
                crate::common::atomic_file::write_atomic(&state_path, state_str.as_bytes())
                    .map_err(|e| format!("Checkpoint failed: {e}"))?;
                writer_metrics
                    .durable
                    .store(written_bytes.load(Ordering::SeqCst), Ordering::SeqCst);
            }
        }
        file.flush()
            .await
            .map_err(|e| format!("Flush failed: {e}"))?;
        file.sync_all()
            .await
            .map_err(|e| format!("Sync failed: {e}"))?;
        writer_metrics
            .durable
            .store(written_bytes.load(Ordering::SeqCst), Ordering::SeqCst);
        Ok(())
    });

    drop(metadata_permit);
    let mut join_handles = Vec::new();
    let client = Arc::new(download_client);
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency as usize));

    progress_reporter(
        initial_downloaded,
        total_size,
        0.0,
        TaskStatus::Downloading.as_str(),
        None,
    );

    let worker_abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
    for _worker_id in 0..concurrency {
        let sem = sem.clone();
        let tx = tx.clone();
        let client = client.clone();
        let cancel_flag = cancel_flag.clone();
        let shared_state_clone = shared_state.clone();
        let limiter = manager.shared_limiter.clone();
        let budget = manager.budget.clone();
        let identity = remote_identity.clone();
        let worker_abort = worker_abort.clone();
        let metrics = metrics.clone();

        let handle = tokio::spawn(async move {
            let work = async {
                loop {
                    if cancel_flag.load(Ordering::SeqCst) {
                        return Ok::<(), String>(());
                    }

                    let chunk_info = {
                        let mut st = shared_state_clone.lock().unwrap();
                        let mut found = None;
                        for (idx, chunk) in st.chunks.iter_mut().enumerate() {
                            if !chunk.completed && !chunk.is_downloading {
                                chunk.is_downloading = true;
                                found = Some((idx, chunk.clone()));
                                break;
                            }
                        }
                        found
                    };

                    let (chunk_idx, chunk) = match chunk_info {
                        Some(info) => info,
                        None => break,
                    };

                    let mut retries = 3u32;
                    let mut delay;
                    let mut current_offset = chunk.current;
                    let end_byte = chunk.end;
                    let mut chunk_success = false;

                    let mut local_buf = Vec::with_capacity(1024 * 1024);
                    let mut local_buf_offset = current_offset;

                    loop {
                        if current_offset > end_byte {
                            chunk_success = true;
                            break;
                        }

                        if cancel_flag.load(Ordering::SeqCst) {
                            break;
                        }

                        let _permit = match sem.acquire().await {
                            Ok(p) => p,
                            Err(_) => break,
                        };

                        let url = {
                            let st = shared_state_clone.lock().unwrap();
                            let mirror_idx = 0;
                            st.mirrors[mirror_idx].clone()
                        };

                        let req = client
                            .get(&url)
                            .header(reqwest::header::ACCEPT_ENCODING, "identity")
                            .header(
                                reqwest::header::RANGE,
                                format!("bytes={}-{}", current_offset, end_byte),
                            );

                        let _network_permit = match budget.request(&cancel_flag).await {
                            Ok(permit) => permit,
                            Err(_) => break,
                        };
                        let req = identity.condition(req);
                        let response = tokio::select! {result=req.send()=>result,_=super::integrity::cancelled(&cancel_flag)=>break};
                        delay = super::integrity::retry_delay(
                            &reqwest::header::HeaderMap::new(),
                            3 - retries,
                        )?;
                        match response {
                            Ok(resp) => {
                                let status = resp.status();
                                if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                                    || status.is_server_error()
                                {
                                    delay =
                                        super::integrity::retry_delay(resp.headers(), 3 - retries)?;
                                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                                        budget.defer(delay);
                                    }
                                } else if status == reqwest::StatusCode::PRECONDITION_FAILED {
                                    return Err("REMOTE_CHANGED: precondition failed".into());
                                } else if status != reqwest::StatusCode::OK
                                    && status != reqwest::StatusCode::PARTIAL_CONTENT
                                {
                                    return Err(format!("HTTP_NON_RETRYABLE: {status}"));
                                }
                                if status.is_success() {
                                    identity.validate(&resp)?;
                                }
                                let is_valid_status = status
                                    == reqwest::StatusCode::PARTIAL_CONTENT
                                    || status == reqwest::StatusCode::OK;
                                if is_valid_status {
                                    crate::common::http_range::validate_response(
                                        status,
                                        resp.headers(),
                                        current_offset,
                                        end_byte,
                                        total_size,
                                    )?;
                                    let mut stream = resp.bytes_stream();
                                    let stream_success;

                                    loop {
                                        enum StreamResult {
                                            Bytes(Vec<u8>),
                                            Eof,
                                            ErrorOrTimeout,
                                            Cancelled,
                                        }

                                        let stream_item = tokio::select! {
                                            res = tokio::time::timeout(Duration::from_secs(15), stream.next()) => {
                                                match res {
                                                    Ok(res_item) => {
                                                        match res_item {
                                                            Some(Ok(bytes)) => StreamResult::Bytes(bytes.to_vec()),
                                                            Some(Err(_)) => StreamResult::ErrorOrTimeout,
                                                            None => StreamResult::Eof,
                                                        }
                                                    }
                                                    Err(_) => StreamResult::ErrorOrTimeout,
                                                }
                                            }
                                            _ = async {
                                                loop {
                                                    tokio::time::sleep(Duration::from_millis(100)).await;
                                                    if cancel_flag.load(Ordering::SeqCst) {
                                                        break;
                                                    }
                                                }
                                            } => StreamResult::Cancelled,
                                        };

                                        match stream_item {
                                            StreamResult::Bytes(bytes) => {
                                                let len = bytes.len() as u64;
                                                metrics.received.fetch_add(len, Ordering::SeqCst);
                                                if len > end_byte + 1 - current_offset {
                                                    return Err(
                                                        "Response exceeds requested byte range"
                                                            .into(),
                                                    );
                                                }

                                                let cancelled = tokio::select! {
                                                    _ = limiter.consume(bytes.len()) => false,
                                                    _ = async { while !cancel_flag.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(100)).await; } } => true,
                                                };
                                                if cancelled {
                                                    stream_success = false;
                                                    break;
                                                }

                                                local_buf.extend_from_slice(&bytes);
                                                current_offset += len;

                                                if local_buf.len() >= 1024 * 1024 {
                                                    if tx
                                                        .send((
                                                            chunk_idx,
                                                            local_buf_offset,
                                                            local_buf.clone(),
                                                        ))
                                                        .await
                                                        .is_err()
                                                    {
                                                        stream_success = false;
                                                        break;
                                                    }
                                                    local_buf.clear();
                                                    local_buf_offset = current_offset;
                                                }
                                            }
                                            StreamResult::Eof => {
                                                stream_success = current_offset == end_byte + 1;
                                                break;
                                            }
                                            _ => {
                                                stream_success = false;
                                                break;
                                            }
                                        }
                                    }

                                    if stream_success {
                                        chunk_success = true;
                                        break;
                                    } else {
                                        if !local_buf.is_empty() {
                                            local_buf.clear();
                                        }
                                        current_offset = local_buf_offset;
                                    }
                                } else {
                                    tracing::warn!(
                                        "分块 {} 偏移 {} 收到非预期响应码: {}，准备重试",
                                        chunk_idx,
                                        current_offset,
                                        status
                                    );
                                    if !local_buf.is_empty() {
                                        local_buf.clear();
                                    }
                                    current_offset = local_buf_offset;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "请求分块 {} 失败 (剩余重试次数: {}), 错误: {}",
                                    chunk_idx,
                                    retries,
                                    e
                                );
                                if !local_buf.is_empty() {
                                    local_buf.clear();
                                }
                                current_offset = local_buf_offset;
                            }
                        }

                        if !identity.reliable() && current_offset > chunk.start {
                            return Err("REMOTE_IDENTITY_UNAVAILABLE: interrupted object requires a fresh transfer".into());
                        }
                        if retries == 0 {
                            break;
                        }

                        drop(_network_permit);
                        retries -= 1;
                        super::integrity::wait_retry(delay, &cancel_flag).await?;
                    }

                    if chunk_success && !local_buf.is_empty() {
                        if tx
                            .send((chunk_idx, local_buf_offset, local_buf))
                            .await
                            .is_err()
                        {
                            chunk_success = false;
                        }
                    }

                    {
                        // Ownership ends only when the writer commits the final bytes.
                        if !chunk_success {
                            return Err(format!("Worker failed to complete chunk: {}", chunk_idx));
                        }
                    }
                }
                Ok::<(), String>(())
            };
            let outcome = tokio::select! {result=work=>result, _=super::integrity::cancelled(&worker_abort)=>Err("TRANSFER_ABORTED".into())};
            if outcome.is_err() {
                worker_abort.store(true, Ordering::SeqCst);
            }
            outcome
        });
        join_handles.push(handle);
    }

    let progress_cancel_flag = cancel_flag.clone();
    let progress_shared_state = shared_state.clone();
    let progress_downloaded_bytes = downloaded_bytes.clone();
    let progress_network = metrics.clone();
    let reporter_clone = progress_reporter.clone();

    let progress_task = tokio::spawn(async move {
        let mut last_bytes = progress_network.received.load(Ordering::SeqCst);
        let mut last_time = std::time::Instant::now();

        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let now = std::time::Instant::now();
            let raw_bytes = progress_network.received.load(Ordering::SeqCst);
            let current_bytes = progress_downloaded_bytes
                .load(Ordering::SeqCst)
                .min(total_size);
            let duration_secs = now.duration_since(last_time).as_secs_f64();

            let speed = if duration_secs > 0.0 {
                (raw_bytes.saturating_sub(last_bytes)) as f64 / duration_secs
            } else {
                0.0
            };

            last_bytes = raw_bytes;
            last_time = now;

            reporter_clone(
                current_bytes,
                total_size,
                speed,
                TaskStatus::Downloading.as_str(),
                None,
            );

            let is_all_completed = {
                if let Ok(st) = progress_shared_state.lock() {
                    !st.chunks.is_empty() && st.chunks.iter().all(|c| c.completed)
                } else {
                    false
                }
            };

            if progress_cancel_flag.load(Ordering::SeqCst)
                || current_bytes >= total_size
                || is_all_completed
            {
                break;
            }
        }
    });

    let mut worker_error = None;
    for h in join_handles {
        match h.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if worker_error.as_deref() == Some("TRANSFER_ABORTED") {
                    worker_error = None;
                }
                worker_error.get_or_insert(error);
            }
            Err(error) => {
                worker_error.get_or_insert(error.to_string());
            }
        }
    }

    drop(tx);
    let writer_result = writer_task.await;
    progress_task.abort();
    let _ = progress_task.await;
    let writer_error = match writer_result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(error) => Some(error.to_string()),
    };
    let writer_failed = writer_error.is_some();
    let failure = writer_error.or_else(|| {
        (!cancel_flag.load(Ordering::SeqCst))
            .then_some(worker_error)
            .flatten()
    });
    if let Some(error) = failure {
        let state = {
            let mut state = shared_state.lock().unwrap();
            state.status = TaskStatus::Failed.as_str().to_string();
            if error.starts_with("REMOTE_CHANGED") {
                state.remote_identity = None;
            }
            serde_json::to_vec_pretty(&*state).ok()
        };
        if !writer_failed {
            if let Some(state) = state {
                let _ = crate::common::atomic_file::write_atomic(&state_path, &state);
            }
        }
        progress_reporter(
            downloaded_bytes.load(Ordering::SeqCst),
            total_size,
            0.0,
            TaskStatus::Failed.as_str(),
            Some(error),
        );
        manager.unregister(&task_id);
        return;
    }

    let final_downloaded = downloaded_bytes.load(Ordering::SeqCst);
    let all_chunks_completed = {
        if let Ok(st) = shared_state.lock() {
            !st.chunks.is_empty() && st.chunks.iter().all(|c| c.completed)
        } else {
            false
        }
    };

    if cancel_flag.load(Ordering::SeqCst) {
        tracing::info!("任务手动挂起暂停: ID: [{}]", task_id);
        progress_reporter(
            final_downloaded,
            total_size,
            0.0,
            TaskStatus::Paused.as_str(),
            None,
        );

        let updated = {
            let mut st = shared_state.lock().unwrap();
            st.status = TaskStatus::Paused.as_str().to_string();
            serde_json::to_string_pretty(&*st).ok()
        };
        if let Some(content) = updated {
            let _ = crate::common::atomic_file::write_atomic(&state_path, content.as_bytes());
        }

        manager.unregister(&task_id);
        return;
    }

    if !all_chunks_completed || final_downloaded != total_size {
        tracing::error!(
            "下载结束但字节缺失! 已存: {}, 预期: {}",
            final_downloaded,
            total_size
        );
        progress_reporter(
            final_downloaded,
            total_size,
            0.0,
            TaskStatus::Failed.as_str(),
            Some(format!(
                "Downloaded size mismatch: {}/{}",
                final_downloaded, total_size
            )),
        );

        let updated = {
            let mut st = shared_state.lock().unwrap();
            st.status = TaskStatus::Failed.as_str().to_string();
            serde_json::to_string_pretty(&*st).ok()
        };
        if let Some(content) = updated {
            let _ = crate::common::atomic_file::write_atomic(&state_path, content.as_bytes());
        }

        manager.unregister(&task_id);
        return;
    }

    progress_reporter(
        final_downloaded,
        total_size,
        0.0,
        TaskStatus::Downloading.as_str(),
        None,
    );
    let verify_path = temp_dest_path.clone();
    let verify_file = file.clone();
    let verify_cancel = cancel_flag.clone();
    progress_reporter(final_downloaded, total_size, 0.0, "Verifying", None);
    let verify_reporter = progress_reporter.clone();
    let verify_metrics = metrics.clone();
    let verification = tokio::task::spawn_blocking(move || {
        super::integrity::verify_with_progress(
            &verify_path,
            total_size,
            &verify_file,
            &verify_cancel,
            |bytes| {
                verify_metrics.verified.store(bytes, Ordering::SeqCst);
                verify_reporter(final_downloaded, total_size, 0.0, "Verifying", None);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    let verification = match verification {
        Ok(report) => report,
        Err(error) => {
            let mut state = shared_state.lock().unwrap();
            if !cancel_flag.load(Ordering::SeqCst) {
                state.remote_identity = None;
            }
            state.status = if cancel_flag.load(Ordering::SeqCst) {
                TaskStatus::Paused
            } else {
                TaskStatus::Failed
            }
            .as_str()
            .into();
            if let Ok(bytes) = serde_json::to_vec_pretty(&*state) {
                let _ = crate::common::atomic_file::write_atomic(&state_path, &bytes);
            }
            progress_reporter(
                final_downloaded,
                total_size,
                0.0,
                &state.status,
                if cancel_flag.load(Ordering::SeqCst) {
                    None
                } else {
                    Some(error)
                },
            );
            manager.unregister(&task_id);
            return;
        }
    };
    let report = serde_json::to_vec_pretty(&verification).expect("verification serializable");
    if let Err(error) =
        crate::common::atomic_file::write_atomic(&task_dir.join("verification.json"), &report)
    {
        progress_reporter(
            final_downloaded,
            total_size,
            0.0,
            TaskStatus::Failed.as_str(),
            Some(error.to_string()),
        );
        manager.unregister(&task_id);
        return;
    }
    shared_state.lock().unwrap().verification = Some(verification);

    if cancel_flag.load(Ordering::SeqCst) {
        progress_reporter(
            final_downloaded,
            total_size,
            0.0,
            TaskStatus::Paused.as_str(),
            None,
        );
        manager.unregister(&task_id);
        return;
    }

    {
        progress_reporter(final_downloaded, total_size, 0.0, "Publishing", None);
        if let Err(e) = super::storage::publish(&temp_dest_path, &final_dest_path) {
            let state = {
                let mut state = shared_state.lock().unwrap();
                state.status = TaskStatus::Failed.as_str().to_string();
                serde_json::to_vec_pretty(&*state).ok()
            };
            if let Some(state) = state {
                let _ = crate::common::atomic_file::write_atomic(&state_path, &state);
            }
            tracing::error!("重命名增量文件失败: {}", e);
            progress_reporter(
                final_downloaded,
                total_size,
                0.0,
                TaskStatus::Failed.as_str(),
                Some(format!("Rename temp file failed: {}", e)),
            );
            manager.unregister(&task_id);
            return;
        }
    }

    let updated = {
        let mut st = shared_state.lock().unwrap();
        st.status = TaskStatus::Completed.as_str().to_string();
        serde_json::to_string_pretty(&*st).ok()
    };
    if let Some(content) = updated {
        let _ = crate::common::atomic_file::write_atomic(&state_path, content.as_bytes());
    }

    if let Some(parent) = temp_dest_path.parent() {
        super::storage::cleanup_staging(parent, std::path::Path::new(&save_dir));
    }
    tracing::info!("下载成功并就绪: [{}] -> {:?}", task_id, final_dest_path);
    progress_reporter(
        total_size,
        total_size,
        0.0,
        TaskStatus::Completed.as_str(),
        None,
    );

    // Keep task metadata and options for history and restart reconciliation.

    manager.unregister(&task_id);
}

fn valid_resume(state: &DownloadState, total: u64, payload: &std::path::Path) -> bool {
    if state.total_size != total || state.chunks.is_empty() {
        return false;
    }
    let Ok(meta) = std::fs::metadata(payload) else {
        return false;
    };
    if meta.len() > total {
        return false;
    }
    let mut next = 0;
    for chunk in &state.chunks {
        if chunk.start != next
            || chunk.end < chunk.start
            || chunk.end >= total
            || chunk.current < chunk.start
            || chunk.current > chunk.end + 1
            || chunk.completed != (chunk.current == chunk.end + 1)
            || (chunk.current > chunk.start && meta.len() < chunk.current)
        {
            return false;
        }
        next = chunk.end + 1;
    }
    next == total
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod transfer_tests;
