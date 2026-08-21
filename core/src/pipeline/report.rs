use crate::squeezers::TextureBytes;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};
const RATE_WINDOW: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
struct Timing {
    started: Option<Instant>,
    finished: Duration,
    samples: VecDeque<(Instant, u64, u64)>,
}

#[derive(Debug, Default)]
pub struct SqueezeReport {
    total_files: AtomicU64,
    optimized: AtomicU64,
    locked: AtomicU64,
    skipped: AtomicU64,
    failed: AtomicU64,
    bytes_before: AtomicU64,
    bytes_after: AtomicU64,
    textures_before: AtomicU64,
    textures_after: AtomicU64,
    timing: Mutex<Timing>,
}

impl SqueezeReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_timer(&self) {
        let mut timing = self.lock();
        timing.started = Some(Instant::now());
        timing.finished = Duration::ZERO;
    }

    pub fn begin(&self, total_files: u64) {
        {
            let mut timing = self.lock();
            timing.started.get_or_insert_with(Instant::now);
            timing.samples.clear();
        }
        self.total_files.store(total_files, Relaxed);
        for counter in [
            &self.optimized,
            &self.locked,
            &self.skipped,
            &self.failed,
            &self.bytes_before,
            &self.bytes_after,
            &self.textures_before,
            &self.textures_after,
        ] {
            counter.store(0, Relaxed);
        }
    }

    pub fn mark_finished(&self) {
        let mut timing = self.lock();
        if let Some(started) = timing.started.take() {
            timing.finished = started.elapsed();
        }
    }

    pub fn record_optimized(&self, bytes_before: u64, bytes_after: u64, textures: TextureBytes) {
        self.optimized.fetch_add(1, Relaxed);
        self.bytes_before.fetch_add(bytes_before, Relaxed);
        self.bytes_after.fetch_add(bytes_after, Relaxed);
        self.record_textures(textures);
    }

    pub fn record_unchanged(&self, bytes: u64, locked: bool, textures: TextureBytes) {
        if locked {
            self.locked.fetch_add(1, Relaxed);
        } else {
            self.skipped.fetch_add(1, Relaxed);
        }
        self.bytes_before.fetch_add(bytes, Relaxed);
        self.bytes_after.fetch_add(bytes, Relaxed);
        self.record_textures(textures);
    }

    fn record_textures(&self, textures: TextureBytes) {
        self.textures_before.fetch_add(textures.before, Relaxed);
        self.textures_after.fetch_add(textures.after, Relaxed);
    }

    pub fn record_failed(&self) {
        self.failed.fetch_add(1, Relaxed);
    }

    pub fn snapshot(&self) -> ReportSnapshot {
        let mut snapshot = ReportSnapshot {
            total_files: self.total_files.load(Relaxed),
            optimized: self.optimized.load(Relaxed),
            locked: self.locked.load(Relaxed),
            skipped: self.skipped.load(Relaxed),
            failed: self.failed.load(Relaxed),
            bytes_before: self.bytes_before.load(Relaxed),
            bytes_after: self.bytes_after.load(Relaxed),
            textures_before: self.textures_before.load(Relaxed),
            textures_after: self.textures_after.load(Relaxed),
            elapsed: Duration::ZERO,
            recent_files_per_sec: 0.0,
            recent_work_per_sec: 0.0,
        };

        let (processed, worked) = (snapshot.processed(), snapshot.worked());
        let mut timing = self.lock();
        let recent = match timing.started {
            Some(started) => {
                snapshot.elapsed = started.elapsed();
                sample_rates(&mut timing.samples, processed, worked)
            }
            None => {
                snapshot.elapsed = timing.finished;
                None
            }
        };
        drop(timing);

        (snapshot.recent_files_per_sec, snapshot.recent_work_per_sec) =
            recent.unwrap_or((snapshot.files_per_sec(), snapshot.work_per_sec()));
        snapshot
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Timing> {
        self.timing.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn sample_rates(
    samples: &mut VecDeque<(Instant, u64, u64)>,
    processed: u64,
    worked: u64,
) -> Option<(f64, f64)> {
    let now = Instant::now();
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

#[derive(Debug, Clone, Copy, Default)]
pub struct ReportSnapshot {
    pub total_files: u64,
    pub optimized: u64,
    pub locked: u64,
    pub skipped: u64,
    pub failed: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub textures_before: u64,
    pub textures_after: u64,
    pub elapsed: Duration,
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

    pub fn textures_saved(&self) -> u64 {
        self.textures_before.saturating_sub(self.textures_after)
    }

    pub fn percent_textures_saved(&self) -> f64 {
        if self.textures_before == 0 {
            return 0.0;
        }
        self.textures_saved() as f64 * 100.0 / self.textures_before as f64
    }

    pub fn has_texture_memory(&self) -> bool {
        self.textures_before > 0
    }

    pub fn files_per_sec(&self) -> f64 {
        self.per_sec(self.processed())
    }

    pub fn work_per_sec(&self) -> f64 {
        self.per_sec(self.worked())
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.elapsed.as_secs_f64()
    }

    fn per_sec(&self, count: u64) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            0.0
        } else {
            count as f64 / secs
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shrank(before: u64, after: u64) -> TextureBytes {
        TextureBytes { before, after }
    }

    #[test]
    fn untouched_files_count_toward_the_total() {
        let report = SqueezeReport::new();
        report.begin(3);
        report.record_optimized(400, 100, shrank(1000, 250));
        report.record_unchanged(200, false, TextureBytes::unchanged(500));
        report.record_unchanged(50, true, TextureBytes::unchanged(120));

        let s = report.snapshot();
        assert_eq!(s.textures_before, 1620);
        assert_eq!(s.textures_after, 870);
        assert_eq!(s.textures_saved(), 750);
        assert!((s.percent_textures_saved() - 46.296).abs() < 0.01);
    }

    #[test]
    fn texture_memory_is_tracked_apart_from_file_size() {
        let report = SqueezeReport::new();
        report.begin(1);
        report.record_optimized(1000, 900, shrank(8000, 2000));

        let s = report.snapshot();
        assert_eq!(s.bytes_saved(), 100);
        assert_eq!(s.textures_saved(), 6000);
        assert!(s.percent_textures_saved() > s.percent_saved());
    }

    #[test]
    fn a_run_with_nothing_measurable_says_so() {
        let report = SqueezeReport::new();
        report.begin(1);
        report.record_optimized(400, 100, TextureBytes::ZERO);

        let s = report.snapshot();
        assert!(!s.has_texture_memory());
        assert_eq!(s.percent_textures_saved(), 0.0);
        assert!(s.bytes_saved() > 0, "file size is still reported");
    }

    #[test]
    fn a_second_run_does_not_inherit_the_first() {
        let report = SqueezeReport::new();
        report.begin(1);
        report.record_optimized(400, 100, shrank(1000, 250));
        report.begin(1);
        report.record_optimized(400, 100, shrank(80, 20));

        let s = report.snapshot();
        assert_eq!(s.textures_before, 80);
        assert_eq!(s.textures_after, 20);
    }
}
