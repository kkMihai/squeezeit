use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

const RATE_WINDOW: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub struct SqueezeReport {
    total_files: AtomicU64,
    optimized: AtomicU64,
    locked: AtomicU64,
    skipped: AtomicU64,
    failed: AtomicU64,
    bytes_before: AtomicU64,
    bytes_after: AtomicU64,
    start: Mutex<Option<Instant>>,
    finished_elapsed: Mutex<std::time::Duration>,
    rate_samples: Mutex<VecDeque<(Instant, u64, u64)>>,
}

impl Default for SqueezeReport {
    fn default() -> Self {
        Self::new()
    }
}

impl SqueezeReport {
    pub fn new() -> Self {
        Self {
            total_files: AtomicU64::new(0),
            optimized: AtomicU64::new(0),
            locked: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            bytes_before: AtomicU64::new(0),
            bytes_after: AtomicU64::new(0),
            start: Mutex::new(None),
            finished_elapsed: Mutex::new(std::time::Duration::ZERO),
            rate_samples: Mutex::new(VecDeque::new()),
        }
    }

    pub fn reset_timer(&self) {
        *self.start.lock().unwrap() = Some(Instant::now());
        *self.finished_elapsed.lock().unwrap() = std::time::Duration::ZERO;
    }

    pub fn begin(&self, total_files: u64) {
        if self.start.lock().unwrap().is_none() {
            *self.start.lock().unwrap() = Some(Instant::now());
        }
        self.total_files.store(total_files, Relaxed);
        self.rate_samples.lock().unwrap().clear();
        for counter in [
            &self.optimized,
            &self.locked,
            &self.skipped,
            &self.failed,
            &self.bytes_before,
            &self.bytes_after,
        ] {
            counter.store(0, Relaxed);
        }
    }

    pub fn mark_finished(&self) {
        let mut guard = self.start.lock().unwrap();
        if let Some(start) = *guard {
            *self.finished_elapsed.lock().unwrap() = start.elapsed();
        }
        *guard = None;
    }

    pub fn record_optimized(&self, bytes_before: u64, bytes_after: u64) {
        self.optimized.fetch_add(1, Relaxed);
        self.bytes_before.fetch_add(bytes_before, Relaxed);
        self.bytes_after.fetch_add(bytes_after, Relaxed);
    }

    pub fn record_unchanged(&self, bytes: u64, locked: bool) {
        if locked {
            self.locked.fetch_add(1, Relaxed);
        } else {
            self.skipped.fetch_add(1, Relaxed);
        }
        self.bytes_before.fetch_add(bytes, Relaxed);
        self.bytes_after.fetch_add(bytes, Relaxed);
    }

    pub fn record_failed(&self) {
        self.failed.fetch_add(1, Relaxed);
    }

    pub fn snapshot(&self) -> ReportSnapshot {
        let (elapsed, running) = {
            let guard = self.start.lock().unwrap();
            match *guard {
                Some(start) => (start.elapsed(), true),
                None => (*self.finished_elapsed.lock().unwrap(), false),
            }
        };
        let mut snapshot = ReportSnapshot {
            total_files: self.total_files.load(Relaxed),
            optimized: self.optimized.load(Relaxed),
            locked: self.locked.load(Relaxed),
            skipped: self.skipped.load(Relaxed),
            failed: self.failed.load(Relaxed),
            bytes_before: self.bytes_before.load(Relaxed),
            bytes_after: self.bytes_after.load(Relaxed),
            elapsed,
            recent_files_per_sec: 0.0,
            recent_work_per_sec: 0.0,
        };
        let recent = if running {
            self.sample_recent_rates(snapshot.processed(), snapshot.worked())
        } else {
            None
        };
        (snapshot.recent_files_per_sec, snapshot.recent_work_per_sec) =
            recent.unwrap_or((snapshot.files_per_sec(), snapshot.work_per_sec()));
        snapshot
    }

    fn sample_recent_rates(&self, processed: u64, worked: u64) -> Option<(f64, f64)> {
        let now = Instant::now();
        let mut samples = self.rate_samples.lock().unwrap();
        samples.push_back((now, processed, worked));
        while samples
            .front()
            .is_some_and(|&(t, ..)| now.duration_since(t) > RATE_WINDOW)
        {
            samples.pop_front();
        }

        let &(oldest_time, oldest_processed, oldest_worked) = samples.front()?;
        let span = now.duration_since(oldest_time).as_secs_f64();
        if span < 0.25 {
            return None;
        }
        Some((
            processed.saturating_sub(oldest_processed) as f64 / span,
            worked.saturating_sub(oldest_worked) as f64 / span,
        ))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReportSnapshot {
    pub total_files: u64,
    pub optimized: u64,
    pub locked: u64,
    pub skipped: u64,
    pub failed: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub elapsed: std::time::Duration,
    pub recent_files_per_sec: f64,
    pub recent_work_per_sec: f64,
}

impl ReportSnapshot {
    pub fn processed(&self) -> u64 {
        self.optimized + self.locked + self.skipped + self.failed
    }

    pub fn worked(&self) -> u64 {
        self.optimized + self.failed
    }

    pub fn progress(&self) -> f32 {
        if self.total_files == 0 {
            return 0.0;
        }
        self.processed() as f32 / self.total_files as f32
    }

    pub fn bytes_saved(&self) -> u64 {
        self.bytes_before.saturating_sub(self.bytes_after)
    }

    pub fn percent_saved(&self) -> f64 {
        if self.bytes_before == 0 {
            return 0.0;
        }
        self.bytes_saved() as f64 * 100.0 / self.bytes_before as f64
    }

    pub fn files_per_sec(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        self.processed() as f64 / secs
    }

    pub fn work_per_sec(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        self.worked() as f64 / secs
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.elapsed.as_secs_f64()
    }
}
