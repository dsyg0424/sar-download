use crate::common::{
    disk_budget::DiskBudget,
    download_budget::{DownloadBudget, SharedLimiter},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Duration,
};

// Transport dependencies retained from the workbench; application task persistence
// lives in service.rs so the engine has no project/workspace dependency.
pub struct DownloadManager {
    pub disk_budget: Arc<DiskBudget>,
    pub budget: Arc<DownloadBudget>,
    pub active_cancels: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    pub shared_client: reqwest::Client,
    pub shared_auth_client: reqwest::Client,
    pub cookie_jar: Arc<reqwest::cookie::Jar>,
    pub shared_limiter: Arc<SharedLimiter>,
    pub auth_mutex: Arc<tokio::sync::Mutex<()>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        let cookie_jar = Arc::new(reqwest::cookie::Jar::default());
        let build = |redirect| {
            reqwest::Client::builder()
                .cookie_provider(cookie_jar.clone())
                .redirect(redirect)
                .connect_timeout(Duration::from_secs(30))
                .read_timeout(Duration::from_secs(60))
                .tcp_keepalive(Duration::from_secs(60))
                .build()
                .expect("create HTTP client")
        };
        Self {
            disk_budget: Arc::new(DiskBudget::default()),
            budget: DownloadBudget::new(3, crate::modules::settings::MAX_FILE_CONNECTIONS as usize),
            active_cancels: Arc::new(Mutex::new(HashMap::new())),
            shared_client: build(reqwest::redirect::Policy::limited(15)),
            shared_auth_client: build(reqwest::redirect::Policy::none()),
            cookie_jar,
            shared_limiter: Arc::new(SharedLimiter::new(None)),
            auth_mutex: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    pub fn register(&self, id: &str) -> Arc<AtomicBool> {
        self.active_cancels
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }
    pub fn unregister(&self, id: &str) {
        self.active_cancels.lock().unwrap().remove(id);
    }
}

#[cfg(test)]
mod transfer_limit_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[tokio::test]
    async fn transfer_limits_scheduler_admits_32_files_and_64_requests() {
        let manager = DownloadManager::new();
        let flag = AtomicBool::new(false);
        manager
            .budget
            .set_files(crate::modules::settings::MAX_PARALLEL_FILES);
        let mut files = Vec::new();
        for _ in 0..32 {
            files.push(manager.budget.file(&flag).await.unwrap());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(25), manager.budget.file(&flag))
                .await
                .is_err()
        );
        let mut requests = Vec::new();
        for _ in 0..64 {
            requests.push(manager.budget.request(&flag).await.unwrap());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(25), manager.budget.request(&flag))
                .await
                .is_err()
        );
        requests.pop();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), manager.budget.request(&flag))
                .await
                .unwrap()
                .is_ok()
        );
        drop((files, requests));
    }
}
