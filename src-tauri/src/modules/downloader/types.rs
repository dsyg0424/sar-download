use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Imported,
    Parsing,
    Ready,
    Connecting,
    Downloading,
    Paused,
    Completed,
    Failed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Imported => "Imported",
            TaskStatus::Parsing => "Parsing",
            TaskStatus::Ready => "Ready",
            TaskStatus::Connecting => "Connecting",
            TaskStatus::Downloading => "Downloading",
            TaskStatus::Paused => "Paused",
            TaskStatus::Completed => "Completed",
            TaskStatus::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadFile {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub hash_type: Option<String>,
    pub hash_value: Option<String>,
    pub task_id: String,
    pub mirrors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChunkRange {
    pub start: u64,
    pub current: u64,
    pub end: u64,
    pub completed: bool,
    #[serde(skip)]
    pub is_downloading: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadState {
    pub task_id: String,
    pub file_name: String,
    pub url: String,
    #[serde(skip)]
    pub mirrors: Vec<String>,
    pub remote_identity: Option<super::integrity::RemoteIdentity>,
    pub verification: Option<super::integrity::Verification>,
    pub total_size: u64,
    pub sha256: String,
    pub hash_type: String,
    pub hash_value: String,
    pub chunks: Vec<ChunkRange>,
    pub hashed_up_to: u64,
    pub status: String,
}

impl DownloadState {
    pub fn new_fresh(
        task_id: &str,
        file_name: &str,
        url: &str,
        total_size: u64,
        file: &DownloadFile,
        concurrency: u32,
    ) -> Self {
        let num_chunks = if total_size < 10 * 1024 * 1024 {
            1
        } else if total_size < 100 * 1024 * 1024 {
            4
        } else {
            (concurrency * 8) as u64
        };

        let chunk_size = (total_size + num_chunks - 1) / num_chunks;
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < total_size {
            let end = std::cmp::min(start + chunk_size - 1, total_size - 1);
            chunks.push(ChunkRange {
                start,
                current: start,
                end,
                completed: false,
                is_downloading: false,
            });
            start += chunk_size;
        }

        let mut mirrors = file.mirrors.clone();
        if mirrors.is_empty() {
            mirrors.push(url.to_string());
        }

        DownloadState {
            task_id: task_id.to_string(),
            file_name: file_name.to_string(),
            url: url.to_string(),
            mirrors,
            remote_identity: None,
            verification: None,
            total_size,
            sha256: file.sha256.clone(),
            hash_type: file.hash_type.clone().unwrap_or_else(|| "none".to_string()),
            hash_value: file.hash_value.clone().unwrap_or_else(|| "".to_string()),
            chunks,
            hashed_up_to: 0,
            status: TaskStatus::Ready.as_str().to_string(),
        }
    }
}
