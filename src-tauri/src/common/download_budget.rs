//! Shared admission and byte budget for full-scene and regional HTTP downloads.
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub async fn cancelled(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
pub async fn cancellable<T>(
    flag: &AtomicBool,
    future: impl std::future::Future<Output = T>,
) -> Result<T, String> {
    if flag.load(Ordering::SeqCst) {
        return Err("已取消下载".into());
    }
    tokio::select! { biased; _ = cancelled(flag) => Err("已取消下载".into()), result = future => Ok(result) }
}

struct FileState {
    limit: usize,
    active: usize,
    sequence: u64,
    waiting: VecDeque<u64>,
}
pub struct DownloadBudget {
    files: Mutex<FileState>,
    requests: Arc<Semaphore>,
    backoff: Mutex<Instant>,
}
pub struct FilePermit {
    budget: Arc<DownloadBudget>,
}
impl Drop for FilePermit {
    fn drop(&mut self) {
        self.budget.files.lock().unwrap().active -= 1;
    }
}
struct Waiting {
    budget: Arc<DownloadBudget>,
    ticket: u64,
}
impl Drop for Waiting {
    fn drop(&mut self) {
        self.budget
            .files
            .lock()
            .unwrap()
            .waiting
            .retain(|id| *id != self.ticket);
    }
}
impl DownloadBudget {
    pub fn new(files: u32, requests: usize) -> Arc<Self> {
        Arc::new(Self {
            files: Mutex::new(FileState {
                limit: if files == 0 {
                    usize::MAX
                } else {
                    files as usize
                },
                active: 0,
                sequence: 0,
                waiting: VecDeque::new(),
            }),
            requests: Arc::new(Semaphore::new(requests.max(1))),
            backoff: Mutex::new(Instant::now()),
        })
    }
    pub fn set_files(&self, limit: u32) {
        self.files.lock().unwrap().limit = if limit == 0 {
            usize::MAX
        } else {
            limit as usize
        };
    }
    pub async fn file(self: &Arc<Self>, flag: &AtomicBool) -> Result<FilePermit, String> {
        let ticket = {
            let mut state = self.files.lock().unwrap();
            state.sequence += 1;
            let id = state.sequence;
            state.waiting.push_back(id);
            id
        };
        let _waiting = Waiting {
            budget: self.clone(),
            ticket,
        };
        loop {
            if flag.load(Ordering::SeqCst) {
                return Err("已取消下载".into());
            }
            {
                let mut state = self.files.lock().unwrap();
                if state.waiting.front() == Some(&ticket) && state.active < state.limit {
                    state.waiting.pop_front();
                    state.active += 1;
                    return Ok(FilePermit {
                        budget: self.clone(),
                    });
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    pub async fn request(&self, flag: &AtomicBool) -> Result<OwnedSemaphorePermit, String> {
        let permit = cancellable(flag, self.requests.clone().acquire_owned())
            .await?
            .map_err(|_| "下载调度已关闭".to_string())?;
        loop {
            let delay = self
                .backoff
                .lock()
                .unwrap()
                .saturating_duration_since(Instant::now());
            if delay.is_zero() {
                return Ok(permit);
            }
            cancellable(flag, tokio::time::sleep(delay)).await?;
        }
    }
    pub fn defer(&self, delay: Duration) {
        let mut until = self.backoff.lock().unwrap();
        *until = (*until).max(Instant::now() + delay);
    }
}

pub struct SharedLimiter {
    state: Mutex<(Option<usize>, f64, Instant)>,
}
impl SharedLimiter {
    pub fn new(rate: Option<usize>) -> Self {
        Self {
            state: Mutex::new((rate, rate.unwrap_or(0) as f64, Instant::now())),
        }
    }
    pub fn set_rate(&self, rate: Option<usize>) {
        *self.state.lock().unwrap() = (rate, rate.unwrap_or(0) as f64, Instant::now());
    }
    pub async fn consume(&self, amount: usize) {
        let mut remaining = amount as f64;
        while remaining > 0.0 {
            let wait = {
                let mut state = self.state.lock().unwrap();
                let Some(rate) = state.0.filter(|rate| *rate > 0) else {
                    return;
                };
                let now = Instant::now();
                state.1 = (state.1 + now.duration_since(state.2).as_secs_f64() * rate as f64)
                    .min(rate as f64);
                state.2 = now;
                let used = remaining.min(state.1);
                state.1 -= used;
                remaining -= used;
                Duration::from_secs_f64((remaining / (rate as f64)).clamp(0.001, 0.05))
            };
            if remaining > 0.0 {
                tokio::time::sleep(wait).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn file_waiters_cancel_and_limit_changes_take_effect() {
        let b = DownloadBudget::new(1, 1);
        let flag = Arc::new(AtomicBool::new(false));
        let first = b.file(&flag).await.unwrap();
        let c = flag.clone();
        let next = b.clone();
        let waiter = tokio::spawn(async move { next.file(&c).await });
        tokio::time::sleep(Duration::from_millis(40)).await;
        flag.store(true, Ordering::SeqCst);
        assert!(waiter.await.unwrap().is_err());
        assert!(b.files.lock().unwrap().waiting.is_empty());
        let flag = AtomicBool::new(false);
        b.set_files(2);
        let second = b.file(&flag).await.unwrap();
        drop((first, second));
        assert_eq!(b.files.lock().unwrap().active, 0);
    }
    #[tokio::test]
    async fn oversized_consume_finishes_and_disabling_wakes_waiters() {
        let limiter = Arc::new(SharedLimiter::new(Some(1000)));
        tokio::time::timeout(Duration::from_secs(2), limiter.consume(1500))
            .await
            .unwrap();
        limiter.set_rate(Some(1));
        let other = limiter.clone();
        let waiter = tokio::spawn(async move { other.consume(1_000_000).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        limiter.set_rate(None);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
    }
}
