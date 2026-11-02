use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub struct DiskBudget {
    held: Mutex<HashMap<String, u64>>,
}
pub struct Reservation {
    pool: Arc<DiskBudget>,
    volume: String,
    bytes: u64,
}
impl Reservation {
    pub fn remaining(&self) -> u64 {
        self.bytes
    }
    /// Written bytes are already reflected in filesystem free space; do not reserve them twice.
    pub fn consume(&mut self, bytes: u64) {
        let released = bytes.min(self.bytes);
        let mut held = self.pool.held.lock().unwrap();
        *held.entry(self.volume.clone()).or_default() -= released;
        self.bytes -= released;
    }
    pub fn resize(
        &mut self,
        bytes: u64,
        probe: impl FnOnce() -> Result<(String, u64), String>,
    ) -> Result<(), String> {
        let mut held = self.pool.held.lock().unwrap();
        let (volume, free) = probe()?;
        if volume != self.volume {
            return Err("DISK_VOLUME_CHANGED: 保存路径的磁盘已改变".into());
        }
        let other = held
            .get(&volume)
            .copied()
            .unwrap_or(0)
            .saturating_sub(self.bytes);
        if bytes > free.saturating_sub(other) {
            return Err("DISK_SPACE_LOW: 磁盘剩余空间不足，下载已停止，请释放空间后重试".into());
        }
        held.insert(volume, other + bytes);
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut held = self.pool.held.lock().unwrap();
        if let Some(value) = held.get_mut(&self.volume) {
            *value = value.saturating_sub(self.bytes);
            if *value == 0 {
                held.remove(&self.volume);
            }
        }
    }
}
impl DiskBudget {
    // Probe while holding the reservation lock; admission and release are atomic.
    pub fn reserve(
        self: &Arc<Self>,
        bytes: u64,
        probe: impl FnOnce() -> Result<(String, u64), String>,
    ) -> Result<Reservation, String> {
        let mut held = self.held.lock().unwrap();
        let (volume, free) = probe()?;
        let reserved = held.get(&volume).copied().unwrap_or(0);
        if bytes > free.saturating_sub(reserved) {
            return Err(format!(
                "磁盘空间不足：需要 {:.2} GiB，可用 {:.2} GiB（已扣除其他任务预留）",
                bytes as f64 / 1073741824.0,
                free.saturating_sub(reserved) as f64 / 1073741824.0
            ));
        }
        held.insert(volume.clone(), reserved + bytes);
        Ok(Reservation {
            pool: self.clone(),
            volume,
            bytes,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_tasks_cannot_reserve_the_same_free_space() {
        let pool = Arc::new(DiskBudget::default());
        let probe = || Ok(("volume".into(), 100));
        let first = pool.reserve(60, probe).unwrap();
        assert!(pool.reserve(60, probe).is_err());
        assert!(pool.reserve(60, || Ok(("other".into(), 100))).is_ok());
        drop(first);
        assert!(pool.reserve(60, probe).is_ok());
    }
    #[test]
    fn processing_growth_does_not_charge_written_bytes_twice() {
        let pool = Arc::new(DiskBudget::default());
        let mut processing = pool.reserve(60, || Ok(("v".into(), 100))).unwrap();
        let other = pool.reserve(30, || Ok(("v".into(), 100))).unwrap();
        // 20 bytes have been written, reducing both free space and future growth.
        processing.resize(40, || Ok(("v".into(), 80))).unwrap();
        assert!(pool.reserve(10, || Ok(("v".into(), 80))).is_ok());
        assert!(processing.resize(51, || Ok(("v".into(), 80))).is_err());
        assert_eq!(processing.remaining(), 40);
        assert!(
            processing
                .resize(1, || Ok(("different".into(), 100)))
                .is_err()
        );
        drop(other);
        processing.resize(60, || Ok(("v".into(), 80))).unwrap();
    }
}
