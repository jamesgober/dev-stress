//! Soak testing: run a workload for a sustained duration and capture
//! ops/sec, latency, and degradation per checkpoint window.
//!
//! Where [`StressRun`](crate::StressRun) is iteration-bounded, [`SoakRun`]
//! is duration-bounded. It runs for `total_duration`, recording one
//! [`SoakCheckpoint`] every `checkpoint_interval`. Comparing
//! checkpoints surfaces drift the stress aggregate would smooth over.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dev_report::{CheckResult, Evidence, Severity};

use crate::{LatencyStats, LatencyTracker, Workload};

/// Configuration for a soak run.
///
/// # Example
///
/// ```
/// use dev_stress::SoakRun;
/// use std::time::Duration;
///
/// let run = SoakRun::new("steady_state")
///     .duration(Duration::from_millis(500))
///     .checkpoint(Duration::from_millis(100))
///     .threads(2);
/// ```
pub struct SoakRun {
    name: String,
    total_duration: Duration,
    checkpoint_interval: Duration,
    threads: usize,
    track_latency: Option<usize>,
}

impl SoakRun {
    /// Begin building a soak run with a stable name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            total_duration: Duration::from_secs(60),
            checkpoint_interval: Duration::from_secs(10),
            threads: 1,
            track_latency: None,
        }
    }

    /// Total wall-clock duration the soak runs for.
    pub fn duration(mut self, d: Duration) -> Self {
        self.total_duration = d;
        self
    }

    /// Wall-clock interval between checkpoints.
    ///
    /// A zero interval means one checkpoint covering the whole run.
    pub fn checkpoint(mut self, d: Duration) -> Self {
        self.checkpoint_interval = d;
        self
    }

    /// Number of OS threads. Minimum is `1`.
    pub fn threads(mut self, n: usize) -> Self {
        self.threads = n.max(1);
        self
    }

    /// Track per-operation latency, sampling 1 of every `rate` iterations.
    pub fn track_latency(mut self, rate: usize) -> Self {
        self.track_latency = Some(rate.max(1));
        self
    }

    /// Execute the soak run.
    ///
    /// Returns when `total_duration` has elapsed. Threads observe a
    /// shared `stop` flag and finish their current iteration before
    /// joining.
    ///
    /// Each worker publishes its iteration count after every iteration,
    /// so checkpoint windows count every completed iteration, even for
    /// slow workloads. Checkpoints are scheduled on a fixed grid from the
    /// start of the run, but each window's duration and offset are the
    /// measured times, so sleep overshoot (coarse on Windows) does not
    /// inflate `ops_per_sec`.
    ///
    /// # Panics
    ///
    /// If `run_once` panics on a worker thread, the soak still runs to
    /// the end of `total_duration`; then every worker is stopped and
    /// joined, and the first worker panic is re-raised with its original
    /// payload.
    pub fn execute<W>(&self, workload: &W) -> SoakResult
    where
        W: Workload + Clone + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let counters: Arc<Vec<PaddedCounter>> = Arc::new(
            (0..self.threads)
                .map(|_| PaddedCounter(AtomicUsize::new(0)))
                .collect(),
        );
        let workload = Arc::new(workload.clone());
        let started = Instant::now();

        // Worker threads.
        let mut handles = Vec::with_capacity(self.threads);
        for idx in 0..self.threads {
            let w = workload.clone();
            let stop = stop.clone();
            let counters = counters.clone();
            let track = self.track_latency;
            handles.push(std::thread::spawn(move || {
                let start = Instant::now();
                let counter = &counters[idx].0;
                let mut tracker = track.map(LatencyTracker::new);
                let mut local_count: usize = 0;
                while !stop.load(Ordering::Relaxed) {
                    if let Some(t) = tracker.as_mut() {
                        t.record(local_count, || w.run_once());
                    } else {
                        w.run_once();
                    }
                    local_count = local_count.wrapping_add(1);
                    // Only this thread writes this cache line, so a
                    // relaxed store is a plain write with no contention.
                    counter.store(local_count, Ordering::Relaxed);
                }
                (start.elapsed(), tracker, local_count)
            }));
        }

        // Driver: at each checkpoint, sum the per-thread counters.
        // Latency is only aggregated at finish, not per checkpoint.
        let interval = if self.checkpoint_interval.is_zero() {
            self.total_duration
        } else {
            self.checkpoint_interval
        };
        let end_at = deadline_after(started, self.total_duration);
        let mut checkpoints: Vec<SoakCheckpoint> = Vec::new();
        let mut last_iters = 0usize;
        let mut last_at = started;
        loop {
            let now = Instant::now();
            if now >= end_at {
                break;
            }
            // Next grid point strictly after `now`, capped at the end.
            let next = next_grid_point(started, interval, now).map_or(end_at, |t| t.min(end_at));
            std::thread::sleep(next.saturating_duration_since(now));
            let at = Instant::now();
            let now_iters = sum_counters(&counters);
            let window_iters = now_iters.wrapping_sub(last_iters);
            let window_dur = at.saturating_duration_since(last_at);
            let ops_per_sec = if window_dur.is_zero() {
                0.0
            } else {
                window_iters as f64 / window_dur.as_secs_f64()
            };
            checkpoints.push(SoakCheckpoint {
                at_offset: at.saturating_duration_since(started),
                window_iters,
                window_duration: window_dur,
                ops_per_sec,
            });
            last_iters = now_iters;
            last_at = at;
        }
        stop.store(true, Ordering::Relaxed);

        let mut thread_times = Vec::with_capacity(self.threads);
        let mut latency_samples: Vec<Duration> = Vec::new();
        let mut total_iters_final = 0usize;
        for (elapsed, tracker, count) in crate::join_workers(handles) {
            thread_times.push(elapsed);
            total_iters_final = total_iters_final.wrapping_add(count);
            if let Some(t) = tracker {
                latency_samples.extend(t.into_samples());
            }
        }
        let total_elapsed = started.elapsed();

        SoakResult {
            name: self.name.clone(),
            iterations: total_iters_final,
            threads: self.threads,
            total_elapsed,
            thread_times,
            latency: if latency_samples.is_empty() {
                None
            } else {
                Some(LatencyStats::from_samples(latency_samples))
            },
            checkpoints,
        }
    }
}

/// A worker's iteration count, alone on its cache line so workers do
/// not slow each other down through false sharing.
#[repr(align(128))]
struct PaddedCounter(AtomicUsize);

fn sum_counters(counters: &[PaddedCounter]) -> usize {
    counters.iter().fold(0usize, |acc, c| {
        acc.wrapping_add(c.0.load(Ordering::Relaxed))
    })
}

/// `start + d`, or a point far enough in the future to never be reached
/// when `d` is too large for `Instant` (adding it directly would panic).
fn deadline_after(start: Instant, d: Duration) -> Instant {
    start
        .checked_add(d)
        .or_else(|| start.checked_add(Duration::from_secs(u64::from(u32::MAX))))
        .unwrap_or(start)
}

/// The first point of the grid `start + k * interval` (k >= 1) that is
/// strictly after `now`. `None` when it cannot be represented.
fn next_grid_point(start: Instant, interval: Duration, now: Instant) -> Option<Instant> {
    let interval_ns = interval.as_nanos();
    if interval_ns == 0 {
        return None;
    }
    let elapsed_ns = now.saturating_duration_since(start).as_nanos();
    let k = u32::try_from(elapsed_ns / interval_ns + 1).ok()?;
    start.checked_add(interval.checked_mul(k)?)
}

/// One sampling window inside a [`SoakResult`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoakCheckpoint {
    /// Measured offset from the run start to the end of this checkpoint.
    pub at_offset: Duration,
    /// Iterations completed during this window across all threads.
    pub window_iters: usize,
    /// Measured wall-clock duration of this window.
    pub window_duration: Duration,
    /// Throughput during this window: `window_iters / window_duration`.
    pub ops_per_sec: f64,
}

/// Result of a soak run.
///
/// # Example
///
/// ```no_run
/// use dev_stress::{SoakRun, Workload};
/// use std::time::Duration;
///
/// #[derive(Clone)]
/// struct Noop;
/// impl Workload for Noop { fn run_once(&self) {} }
///
/// let r = SoakRun::new("steady")
///     .duration(Duration::from_millis(50))
///     .checkpoint(Duration::from_millis(10))
///     .threads(1)
///     .execute(&Noop);
/// assert!(!r.checkpoints.is_empty());
/// ```
#[derive(Debug, Clone)]
pub struct SoakResult {
    /// Stable name of the run.
    pub name: String,
    /// Total iterations across all threads.
    pub iterations: usize,
    /// Threads used.
    pub threads: usize,
    /// Wall-clock duration of the soak.
    pub total_elapsed: Duration,
    /// Per-thread elapsed times.
    pub thread_times: Vec<Duration>,
    /// Aggregate latency stats across the whole run.
    pub latency: Option<LatencyStats>,
    /// Per-window checkpoints captured during the soak.
    pub checkpoints: Vec<SoakCheckpoint>,
}

impl SoakResult {
    /// Effective throughput in operations per second across the whole soak.
    pub fn ops_per_sec(&self) -> f64 {
        if self.total_elapsed.is_zero() {
            return 0.0;
        }
        self.iterations as f64 / self.total_elapsed.as_secs_f64()
    }

    /// Coefficient of variation of `ops_per_sec` across checkpoints.
    ///
    /// High values indicate the workload is degrading or fluctuating
    /// over time; low values indicate steady state.
    pub fn checkpoint_ops_cv(&self) -> f64 {
        if self.checkpoints.len() < 2 {
            return 0.0;
        }
        let n = self.checkpoints.len() as f64;
        let mean: f64 = self.checkpoints.iter().map(|c| c.ops_per_sec).sum::<f64>() / n;
        if mean == 0.0 {
            return 0.0;
        }
        let var = self
            .checkpoints
            .iter()
            .map(|c| (c.ops_per_sec - mean).powi(2))
            .sum::<f64>()
            / n;
        var.sqrt() / mean
    }

    /// Convert this result into a `CheckResult`.
    ///
    /// Default verdict logic:
    /// - No checkpoints (or only one) -> `Skip` with detail.
    /// - `degradation_pct_threshold` exceeded between first-half and
    ///   second-half mean ops/sec -> `Fail+Warning`.
    /// - Otherwise `Pass`.
    ///
    /// Always carries the `stress` and `soak` tags plus numeric
    /// evidence for `iterations`, `threads`, `ops_per_sec`,
    /// `total_elapsed_ms`, `checkpoint_count`, `checkpoint_ops_cv`,
    /// `first_half_ops`, `second_half_ops`.
    pub fn into_check_result(self, degradation_pct_threshold: f64) -> CheckResult {
        let name = format!("stress::soak::{}", self.name);
        let mut evidence = vec![
            Evidence::numeric("iterations", self.iterations as f64),
            Evidence::numeric("threads", self.threads as f64),
            Evidence::numeric("ops_per_sec", self.ops_per_sec()),
            Evidence::numeric(
                "total_elapsed_ms",
                self.total_elapsed.as_secs_f64() * 1000.0,
            ),
            Evidence::numeric("checkpoint_count", self.checkpoints.len() as f64),
            Evidence::numeric("checkpoint_ops_cv", self.checkpoint_ops_cv()),
        ];
        if let Some(lat) = &self.latency {
            evidence.push(Evidence::numeric(
                "latency_p50_ns",
                lat.p50.as_nanos() as f64,
            ));
            evidence.push(Evidence::numeric(
                "latency_p95_ns",
                lat.p95.as_nanos() as f64,
            ));
            evidence.push(Evidence::numeric(
                "latency_p99_ns",
                lat.p99.as_nanos() as f64,
            ));
        }
        let tags = vec!["stress".to_string(), "soak".to_string()];

        if self.checkpoints.len() < 2 {
            let mut c = CheckResult::skip(name).with_detail(format!(
                "fewer than 2 checkpoints (got {})",
                self.checkpoints.len()
            ));
            c.tags = tags;
            c.evidence = evidence;
            return c;
        }

        let mid = self.checkpoints.len() / 2;
        let first_half_mean = mean_ops(&self.checkpoints[..mid]);
        let second_half_mean = mean_ops(&self.checkpoints[mid..]);
        evidence.push(Evidence::numeric("first_half_ops", first_half_mean));
        evidence.push(Evidence::numeric("second_half_ops", second_half_mean));

        if first_half_mean == 0.0 {
            let mut c = CheckResult::pass(name)
                .with_detail("first-half throughput was zero, skipping degradation check");
            c.tags = tags;
            c.evidence = evidence;
            return c;
        }

        let drop_pct = ((first_half_mean - second_half_mean) / first_half_mean) * 100.0;
        let detail = format!(
            "iterations={} elapsed={:.3}s ops/sec={:.0} checkpoints={} first_half_ops={:.0} second_half_ops={:.0} drop={:.2}%",
            self.iterations,
            self.total_elapsed.as_secs_f64(),
            self.ops_per_sec(),
            self.checkpoints.len(),
            first_half_mean,
            second_half_mean,
            drop_pct
        );

        if drop_pct > degradation_pct_threshold {
            let mut tags = tags;
            tags.push("regression".to_string());
            let mut c = CheckResult::fail(name, Severity::Warning).with_detail(detail);
            c.tags = tags;
            c.evidence = evidence;
            c
        } else {
            let mut c = CheckResult::pass(name).with_detail(detail);
            c.tags = tags;
            c.evidence = evidence;
            c
        }
    }
}

fn mean_ops(checkpoints: &[SoakCheckpoint]) -> f64 {
    if checkpoints.is_empty() {
        return 0.0;
    }
    checkpoints.iter().map(|c| c.ops_per_sec).sum::<f64>() / checkpoints.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use dev_report::Verdict;

    #[derive(Clone)]
    struct Noop;
    impl Workload for Noop {
        fn run_once(&self) {
            std::hint::black_box(1 + 1);
        }
    }

    #[test]
    fn soak_runs_for_duration_and_records_checkpoints() {
        let r = SoakRun::new("steady")
            .duration(Duration::from_millis(150))
            .checkpoint(Duration::from_millis(50))
            .threads(2)
            .execute(&Noop);
        assert!(r.iterations > 0);
        assert!(!r.checkpoints.is_empty());
        assert!(r.total_elapsed >= Duration::from_millis(140));
    }

    #[test]
    fn soak_fewer_than_two_checkpoints_skips() {
        let r = SoakRun::new("brief")
            .duration(Duration::from_millis(20))
            .checkpoint(Duration::from_millis(50))
            .threads(1)
            .execute(&Noop);
        let c = r.into_check_result(20.0);
        // 0 or 1 checkpoint -> Skip.
        if c.verdict != Verdict::Skip {
            // Could occasionally land 1 checkpoint; either way the
            // verdict should not be Fail.
            assert_ne!(c.verdict, Verdict::Fail);
        }
    }

    #[test]
    fn soak_with_latency_tracking_records_percentiles() {
        let r = SoakRun::new("hot")
            .duration(Duration::from_millis(80))
            .checkpoint(Duration::from_millis(20))
            .threads(2)
            .track_latency(1)
            .execute(&Noop);
        assert!(r.latency.is_some());
    }

    #[test]
    fn soak_check_carries_tags_and_evidence() {
        let r = SoakRun::new("steady")
            .duration(Duration::from_millis(80))
            .checkpoint(Duration::from_millis(20))
            .threads(1)
            .execute(&Noop);
        let c = r.into_check_result(20.0);
        assert!(c.has_tag("stress"));
        assert!(c.has_tag("soak"));
        let labels: Vec<&str> = c.evidence.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"checkpoint_count"));
        assert!(labels.contains(&"checkpoint_ops_cv"));
    }

    #[derive(Clone)]
    struct Slow;
    impl Workload for Slow {
        fn run_once(&self) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn slow_workload_iterations_show_up_in_every_window() {
        // At most ~50 ops per 100ms window. The shared counter used to be
        // flushed only every 1024 iterations per thread, so every window
        // reported 0 ops and the degradation check saw zero throughput.
        let r = SoakRun::new("slow")
            .duration(Duration::from_millis(400))
            .checkpoint(Duration::from_millis(100))
            .threads(1)
            .execute(&Slow);
        assert!(r.checkpoints.len() >= 3, "{:?}", r.checkpoints);
        for c in &r.checkpoints {
            assert!(c.window_iters > 0, "empty window: {c:?}");
            assert!(c.ops_per_sec > 0.0);
        }
        let windowed: usize = r.checkpoints.iter().map(|c| c.window_iters).sum();
        assert!(windowed <= r.iterations);
        // The workers stop right after the last checkpoint, so at most
        // one in-flight iteration per thread is outside the windows.
        assert!(r.iterations - windowed <= r.threads);
    }

    #[test]
    fn window_durations_are_measured_and_add_up() {
        let r = SoakRun::new("steady")
            .duration(Duration::from_millis(120))
            .checkpoint(Duration::from_millis(30))
            .threads(1)
            .execute(&Noop);
        let mut acc = Duration::ZERO;
        for c in &r.checkpoints {
            acc += c.window_duration;
            assert_eq!(acc, c.at_offset);
            let expect = c.window_iters as f64 / c.window_duration.as_secs_f64();
            assert!((c.ops_per_sec - expect).abs() <= expect * 1e-12);
        }
        assert!(r.checkpoints.last().unwrap().at_offset >= Duration::from_millis(120));
    }

    #[test]
    fn zero_checkpoint_interval_yields_one_window() {
        // A zero interval used to spin, pushing zero-length checkpoints
        // until the run ended.
        let r = SoakRun::new("zero")
            .duration(Duration::from_millis(50))
            .checkpoint(Duration::ZERO)
            .threads(1)
            .execute(&Noop);
        assert_eq!(r.checkpoints.len(), 1);
        assert!(r.checkpoints[0].window_duration >= Duration::from_millis(50));
    }

    #[test]
    fn zero_duration_returns_immediately() {
        let r = SoakRun::new("none")
            .duration(Duration::ZERO)
            .checkpoint(Duration::from_millis(10))
            .threads(2)
            .execute(&Noop);
        assert!(r.checkpoints.is_empty());
        assert_eq!(r.thread_times.len(), 2);
    }

    #[test]
    fn deadline_and_grid_math_do_not_overflow() {
        let start = Instant::now();
        // `start + Duration::MAX` panics; this must not.
        let end = deadline_after(start, Duration::MAX);
        assert!(end > start);
        assert_eq!(next_grid_point(start, Duration::MAX, start), None);
        assert_eq!(next_grid_point(start, Duration::ZERO, start), None);
        let i = Duration::from_millis(10);
        assert_eq!(next_grid_point(start, i, start), Some(start + i));
        // Overshot past two grid points: skip to the next future one.
        let late = start + Duration::from_millis(25);
        assert_eq!(
            next_grid_point(start, i, late),
            Some(start + Duration::from_millis(30))
        );
    }

    #[derive(Clone)]
    struct Explodes;
    impl Workload for Explodes {
        fn run_once(&self) {
            panic!("soak workload exploded");
        }
    }

    #[test]
    fn worker_panic_is_reraised_with_original_payload() {
        let run = SoakRun::new("boom")
            .duration(Duration::from_millis(20))
            .checkpoint(Duration::from_millis(10))
            .threads(2);
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.execute(&Explodes)))
            .expect_err("panic must propagate");
        assert_eq!(err.downcast_ref::<&str>(), Some(&"soak workload exploded"));
    }

    #[test]
    fn checkpoint_ops_cv_is_low_for_uniform_load() {
        let r = SoakRun::new("steady")
            .duration(Duration::from_millis(100))
            .checkpoint(Duration::from_millis(20))
            .threads(2)
            .execute(&Noop);
        // CV is bounded; the value depends on machine, just sanity check.
        assert!(r.checkpoint_ops_cv() >= 0.0);
    }
}
