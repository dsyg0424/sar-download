//! Reserve remaining writes, periodically recheck external disk consumption.
use crate::common::disk_budget::{DiskBudget, Reservation};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
const WINDOW: u64 = 16 * 1024 * 1024;
const MARGIN: u64 = 64 * 1024 * 1024;
pub(super) struct SpaceMonitor {
    lease: Reservation,
    unknown: bool,
    probe: Box<dyn Fn() -> Result<(String, u64), String> + Send>,
    checked: Instant,
    written: u64,
}
impl SpaceMonitor {
    pub fn new(
        pool: &Arc<DiskBudget>,
        remaining: Option<u64>,
        probe: impl Fn() -> Result<(String, u64), String> + Send + 'static,
    ) -> Result<Self, String> {
        let bytes = remaining
            .unwrap_or(WINDOW)
            .checked_add(MARGIN)
            .ok_or("DISK_BUDGET_OVERFLOW")?;
        let lease = pool.reserve(bytes, &probe)?;
        Ok(Self {
            lease,
            unknown: remaining.is_none(),
            probe: Box::new(probe),
            checked: Instant::now(),
            written: 0,
        })
    }
    pub fn before_write(&mut self, bytes: u64) -> Result<(), String> {
        let replenish = self.unknown && self.lease.remaining() < bytes.saturating_add(MARGIN);
        if replenish || self.written >= WINDOW || self.checked.elapsed() >= Duration::from_secs(1) {
            let target = if replenish {
                bytes
                    .max(WINDOW)
                    .checked_add(MARGIN)
                    .ok_or("DISK_BUDGET_OVERFLOW")?
            } else {
                self.lease.remaining()
            };
            self.lease.resize(target, &self.probe)?;
            self.written = 0;
            self.checked = Instant::now();
        }
        Ok(())
    }
    pub fn after_write(&mut self, bytes: u64) {
        self.lease.consume(bytes);
        self.written = self.written.saturating_add(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[test]
    fn detects_external_consumption_and_releases_written_reservations() {
        let pool = Arc::new(DiskBudget::default());
        let free = Arc::new(AtomicU64::new(200 * 1024 * 1024));
        let f = free.clone();
        let mut monitor = SpaceMonitor::new(&pool, Some(32 * 1024 * 1024), move || {
            Ok(("disk".into(), f.load(Ordering::SeqCst)))
        })
        .unwrap();
        monitor.before_write(WINDOW).unwrap();
        monitor.after_write(WINDOW);
        free.fetch_sub(WINDOW, Ordering::SeqCst);
        let other = pool
            .reserve(100 * 1024 * 1024, || {
                Ok(("disk".into(), free.load(Ordering::SeqCst)))
            })
            .unwrap();
        assert!(monitor.before_write(1).is_ok());
        free.store(10 * 1024 * 1024, Ordering::SeqCst);
        monitor.checked = Instant::now() - Duration::from_secs(2);
        assert!(
            monitor
                .before_write(1)
                .unwrap_err()
                .contains("DISK_SPACE_LOW")
        );
        drop(other);
        drop(monitor);
        assert!(pool.reserve(10, || Ok(("disk".into(), 10))).is_ok());
    }
    #[test]
    fn unknown_stream_grows_in_bounded_windows_and_volume_change_is_rejected() {
        let pool = Arc::new(DiskBudget::default());
        let mut monitor =
            SpaceMonitor::new(&pool, None, || Ok(("disk".into(), 200 * 1024 * 1024))).unwrap();
        for _ in 0..10 {
            monitor.before_write(WINDOW).unwrap();
            monitor.after_write(WINDOW);
        }
        assert_eq!(monitor.lease.remaining(), MARGIN);
        assert!(
            monitor
                .lease
                .resize(1, || Ok(("other".into(), u64::MAX)))
                .is_err()
        );
    }
}
