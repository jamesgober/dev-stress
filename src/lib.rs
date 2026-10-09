//! # dev-stress
//!
//! High-load stress testing for Rust. Concurrency, volume, saturation
//! under pressure. Part of the `dev-*` verification suite.
//!
//! `dev-stress` is the answer to "does this code survive real load?"
//! Not "is it fast" (that's `dev-bench`). Not "does it deadlock"
//! (that's `dev-async`). Not "does it recover from failure" (that's
//! `dev-chaos`).
//!
//! ## Quick example
//!
//! ```no_run
//! use dev_stress::{Workload, StressRun};
//!
//! #[derive(Clone)]
//! struct MyWorkload;
//! impl Workload for MyWorkload {
//!     fn run_once(&self) {
//!         std::hint::black_box(40 + 2);
//!     }
//! }
//!
//! let run = StressRun::new("hot_path")
//!     .iterations(100_000)
//!     .threads(8);
//!
//! let result = run.execute(&MyWorkload);
//! let _check = result.into_check_result(None);
//! ```
//!
//! ## What's measured
//!
//! - **`ops_per_sec`** — total iterations divided by total wall time.
//! - **`thread_time_cv`** — coefficient of variation across per-thread
//!   elapsed times. High CV indicates load imbalance or contention.
//! - **`latency_p50/p95/p99`** — per-operation latency percentiles
//!   (when [`LatencyTracker`] is enabled).
//!
//! ## Features
//!
//! - `system-stats` (opt-in): measure RSS and CPU time via
//!   `sysinfo`. See the `system` module
//!   (visible in rustdoc when the feature is enabled).

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

/// Version of this crate as compiled, taken from its `Cargo.toml`.
///
/// Lets tools that bundle this crate, such as the `dev` CLI in
/// `dev-tools`, report the version that is actually linked.
///
/// # Example
///
/// ```
/// assert!(!dev_stress::VERSION.is_empty());
/// ```
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::sync::Arc;
use std::time::{Duration, Instant};

use dev_report::{CheckResult, Evidence, Producer, Report, Severity};

pub mod latency;
pub mod soak;

#[cfg(feature = "system-stats")]
#[cfg_attr(docsrs, doc(cfg(feature = "system-stats")))]
pub mod system;

pub use latency::{LatencyStats, LatencyTracker};
pub use soak::{SoakCheckpoint, SoakResult, SoakRun};

/// A workload that can be executed many times under stress.
///
/// Implementations MUST be safe to call concurrently from multiple
/// threads (`Send + Sync`) and MUST be cheap to clone, since each
/// thread receives an `Arc<Self>`.
///
/// # Example
///
/// ```
/// use dev_stress::Workload;
///
/// #[derive(Clone)]
/// struct Noop;
/// impl Workload for Noop {
///     fn run_once(&self) {
///         std::hint::black_box(1 + 1);
///     }
/// }
/// ```
pub trait Workload: Send + Sync {
    /// Execute one unit of work. MUST be safe to call concurrently
    /// from multiple threads.
    fn run_once(&self);
}

/// Configuration for a stress run.
///
/// # Example
///
/// ```
/// use dev_stress::StressRun;
///
/// let run = StressRun::new("hot_path").iterations(1_000).threads(4);
/// assert_eq!(run.iterations_planned(), 1_000);
/// assert_eq!(run.threads_planned(), 4);
/// ```
pub struct StressRun {
    name: String,
    iterations: usize,
    threads: usize,
    track_latency: Option<usize>, // None = off; Some(n) = sample 1/n iterations
    /// Total target ops/sec rate across all threads. None = unbounded
    /// (run as fast as possible). Split across threads at execute time,
    /// so the order of `threads` and `target_ops_per_sec` calls does not
    /// matter.
    target_ops_per_sec_total: Option<f64>,
}

impl StressRun {
    /// Begin building a stress run with a stable name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            iterations: 1_000,
            threads: 1,
            track_latency: None,
            target_ops_per_sec_total: None,
        }
    }

    /// Total iterations across all threads.
    pub fn iterations(mut self, n: usize) -> Self {
        self.iterations = n;
        self
    }

    /// Number of OS threads to run concurrently. Minimum is `1`.
    pub fn threads(mut self, n: usize) -> Self {
        self.threads = n.max(1);
        self
    }

    /// Track per-operation latency, sampling 1 of every `rate` iterations.
    ///
    /// `rate = 1` records every iteration; `rate = 100` records 1% of
    /// iterations. Lower rates lower memory and overhead at the cost
    /// of percentile precision.
    ///
    /// # Example
    ///
    /// ```
    /// use dev_stress::StressRun;
    ///
    /// let run = StressRun::new("hot").iterations(10_000).threads(2)
    ///     .track_latency(10); // 10% sample rate
    /// assert_eq!(run.iterations_planned(), 10_000);
    /// ```
    pub fn track_latency(mut self, rate: usize) -> Self {
        self.track_latency = Some(rate.max(1));
        self
    }

    /// Cap the workload at approximately `total_rate` ops per second
    /// (across all threads).
    ///
    /// Implemented as a per-thread sleep based on observed elapsed time
    /// vs. ideal scheduling. Sleep precision varies by OS; for very
    /// high target rates (e.g. > 10k ops/sec/thread) the actual rate
    /// may be lower than the target due to sleep granularity.
    ///
    /// The total is split evenly across threads when the run executes,
    /// so it does not matter whether [`threads`](Self::threads) is
    /// called before or after this method. A rate that is zero,
    /// negative, NaN or infinite removes the cap.
    ///
    /// # Example
    ///
    /// ```
    /// use dev_stress::StressRun;
    ///
    /// // Cap to ~1000 ops/sec total across 4 threads = 250 per thread.
    /// let run = StressRun::new("rate-limited")
    ///     .iterations(500)
    ///     .threads(4)
    ///     .target_ops_per_sec(1000.0);
    /// assert_eq!(run.threads_planned(), 4);
    /// ```
    pub fn target_ops_per_sec(mut self, total_rate: f64) -> Self {
        self.target_ops_per_sec_total = if total_rate.is_finite() && total_rate > 0.0 {
            Some(total_rate)
        } else {
            None
        };
        self
    }

    /// The configured iteration count.
    pub fn iterations_planned(&self) -> usize {
        self.iterations
    }

    /// The configured thread count.
    pub fn threads_planned(&self) -> usize {
        self.threads
    }

    /// The configured per-thread target rate, if any: the total from
    /// [`target_ops_per_sec`](Self::target_ops_per_sec) divided by the
    /// configured thread count.
    pub fn target_ops_per_sec_per_thread(&self) -> Option<f64> {
        self.target_ops_per_sec_total
            .map(|total| total / self.threads.max(1) as f64)
    }

    /// Execute the run. Returns a result with timing statistics.
    ///
    /// # Panics
    ///
    /// If `run_once` panics on any worker thread, `execute` first waits
    /// for every other worker to finish its share, then re-raises the
    /// first worker panic with its original payload. No worker thread is
    /// left running.
    pub fn execute<W>(&self, workload: &W) -> StressResult
    where
        W: Workload + Clone + 'static,
    {
        let per_thread = self.iterations / self.threads;
        let leftover = self.iterations % self.threads;
        let started = Instant::now();
        let mut handles = Vec::with_capacity(self.threads);
        let workload = Arc::new(workload.clone());
        let target_rate = self.target_ops_per_sec_per_thread();

        for t in 0..self.threads {
            let count = per_thread + if t < leftover { 1 } else { 0 };
            let w = workload.clone();
            let track = self.track_latency;
            handles.push(std::thread::spawn(move || {
                let start = Instant::now();
                let mut tracker = track.map(LatencyTracker::new);
                // Inverse of the per-thread rate, in seconds per op.
                let interval_s = target_rate.map(|r| 1.0 / r);
                for i in 0..count {
                    if let Some(interval) = interval_s {
                        // Checked math: a tiny rate gives a target too far
                        // out to represent; skip the wait rather than panic.
                        let target = Duration::try_from_secs_f64(interval * i as f64)
                            .ok()
                            .and_then(|offset| start.checked_add(offset));
                        if let Some(target) = target {
                            let now = Instant::now();
                            if target > now {
                                std::thread::sleep(target - now);
                            }
                        }
                    }
                    if let Some(t) = tracker.as_mut() {
                        t.record(i, || w.run_once());
                    } else {
                        w.run_once();
                    }
                }
                (start.elapsed(), tracker)
            }));
        }

        let mut thread_times = Vec::with_capacity(self.threads);
        let mut latency_samples: Vec<Duration> = Vec::new();
        for (elapsed, tracker) in join_workers(handles) {
            thread_times.push(elapsed);
            if let Some(t) = tracker {
                latency_samples.extend(t.into_samples());
            }
        }
        let total_elapsed = started.elapsed();

        StressResult {
            name: self.name.clone(),
            iterations: self.iterations,
            threads: self.threads,
            total_elapsed,
            thread_times,
            latency: if latency_samples.is_empty() {
                None
            } else {
                Some(LatencyStats::from_samples(latency_samples))
            },
        }
    }
}

/// Join every worker thread, then re-raise the first worker panic (with
/// its original payload) if any worker panicked. Joining all of them
/// first means no worker is left running when the panic propagates.
pub(crate) fn join_workers<T>(handles: Vec<std::thread::JoinHandle<T>>) -> Vec<T> {
    let mut results = Vec::with_capacity(handles.len());
    let mut first_panic = None;
    for h in handles {
        match h.join() {
            Ok(v) => results.push(v),
            Err(payload) => {
                if first_panic.is_none() {
                    first_panic = Some(payload);
                }
            }
        }
    }
    if let Some(payload) = first_panic {
        std::panic::resume_unwind(payload);
    }
    results
}

/// Result of a stress run.
///
/// # Example
///
/// ```no_run
/// use dev_stress::{StressRun, Workload};
///
/// #[derive(Clone)]
/// struct Noop;
/// impl Workload for Noop {
///     fn run_once(&self) { std::hint::black_box(1 + 1); }
/// }
///
/// let r = StressRun::new("noop").iterations(100).threads(2).execute(&Noop);
/// assert!(r.ops_per_sec() > 0.0);
/// ```
#[derive(Debug, Clone)]
pub struct StressResult {
    /// Stable name of the run.
    pub name: String,
    /// Iterations actually executed.
    pub iterations: usize,
    /// Threads used.
    pub threads: usize,
    /// Wall-clock time from run start to all threads finishing.
    pub total_elapsed: Duration,
    /// Per-thread elapsed times. Variance here indicates contention.
    pub thread_times: Vec<Duration>,
    /// Per-operation latency percentiles, when [`StressRun::track_latency`]
    /// was enabled. `None` otherwise.
    pub latency: Option<LatencyStats>,
}

impl StressResult {
    /// Effective throughput in operations per second.
    pub fn ops_per_sec(&self) -> f64 {
        if self.total_elapsed.is_zero() {
            return 0.0;
        }
        self.iterations as f64 / self.total_elapsed.as_secs_f64()
    }

    /// Coefficient of variation across thread times. Higher numbers
    /// indicate worse contention or load imbalance.
    pub fn thread_time_cv(&self) -> f64 {
        if self.thread_times.is_empty() {
            return 0.0;
        }
        let n = self.thread_times.len() as f64;
        let mean: f64 = self
            .thread_times
            .iter()
            .map(|d| d.as_secs_f64())
            .sum::<f64>()
            / n;
        if mean == 0.0 {
            return 0.0;
        }
        let var = self
            .thread_times
            .iter()
            .map(|d| (d.as_secs_f64() - mean).powi(2))
            .sum::<f64>()
            / n;
        var.sqrt() / mean
    }

    /// Convert this result into a `CheckResult` using the legacy
    /// behavior (90%-baseline ops/sec floor, no latency thresholds).
    ///
    /// `baseline_ops_per_sec` is the previously-recorded throughput.
    /// `None` -> `Pass` with detail. Below 90% baseline -> `Fail+Warning`.
    /// Otherwise `Pass`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dev_stress::{StressRun, Workload};
    ///
    /// #[derive(Clone)]
    /// struct Noop;
    /// impl Workload for Noop { fn run_once(&self) {} }
    ///
    /// let r = StressRun::new("noop").iterations(100).threads(1).execute(&Noop);
    /// let _check = r.into_check_result(None);
    /// ```
    pub fn into_check_result(self, baseline_ops_per_sec: Option<f64>) -> CheckResult {
        self.compare_with_options(&CompareOptions {
            baseline_ops_per_sec,
            ..CompareOptions::default()
        })
    }

    /// Compare this result against a baseline using full options.
    ///
    /// Always returns a `CheckResult` tagged `stress`, with numeric
    /// `Evidence` for `iterations`, `threads`, `ops_per_sec`,
    /// `thread_time_cv`, `total_elapsed_ms`, plus latency percentiles
    /// (when tracked) and any baseline values provided.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use dev_stress::{CompareOptions, StressRun, Workload};
    /// use std::time::Duration;
    ///
    /// #[derive(Clone)]
    /// struct Noop;
    /// impl Workload for Noop { fn run_once(&self) {} }
    ///
    /// let r = StressRun::new("noop").iterations(100).threads(1).execute(&Noop);
    /// let opts = CompareOptions {
    ///     baseline_ops_per_sec: Some(1_000_000.0),
    ///     ops_drop_pct_threshold: 10.0,
    ///     baseline_p99: None,
    ///     p99_regression_pct_threshold: 20.0,
    /// };
    /// let _ = r.compare_with_options(&opts);
    /// ```
    pub fn compare_with_options(&self, opts: &CompareOptions) -> CheckResult {
        let name = format!("stress::{}", self.name);
        let mut evidence = self.numeric_evidence();
        let mut tags = vec!["stress".to_string()];

        let mut regressions: Vec<String> = Vec::new();

        // ops/sec drop check.
        if let Some(baseline_ops) = opts.baseline_ops_per_sec {
            evidence.push(Evidence::numeric("baseline_ops_per_sec", baseline_ops));
            let floor = baseline_ops * (1.0 - opts.ops_drop_pct_threshold / 100.0);
            if self.ops_per_sec() < floor {
                regressions.push(format!(
                    "ops_per_sec {:.0} < floor {:.0} ({}% drop allowed)",
                    self.ops_per_sec(),
                    floor,
                    opts.ops_drop_pct_threshold
                ));
            }
        }

        // p99 regression check.
        if let (Some(baseline_p99), Some(latency)) = (opts.baseline_p99, &self.latency) {
            evidence.push(Evidence::numeric(
                "baseline_p99_ns",
                baseline_p99.as_nanos() as f64,
            ));
            let allowed =
                baseline_p99.as_nanos() as f64 * (1.0 + opts.p99_regression_pct_threshold / 100.0);
            if (latency.p99.as_nanos() as f64) > allowed {
                regressions.push(format!(
                    "p99_ns {} > allowed {:.0} ({}% regression allowed)",
                    latency.p99.as_nanos(),
                    allowed,
                    opts.p99_regression_pct_threshold
                ));
            }
        }

        let detail = self.detail_string();

        if regressions.is_empty() {
            // No baseline OR within thresholds.
            let mut c = CheckResult::pass(name).with_detail(detail);
            c.tags = tags;
            c.evidence = evidence;
            return c;
        }

        tags.push("regression".to_string());
        let combined_detail = format!("{} :: {}", detail, regressions.join("; "));
        let mut c = CheckResult::fail(name, Severity::Warning).with_detail(combined_detail);
        c.tags = tags;
        c.evidence = evidence;
        c
    }

    /// Build a one-check `Report` containing the comparison result.
    ///
    /// Sets `subject = self.name`, `producer = "dev-stress"`.
    pub fn into_report(self, subject_version: impl Into<String>, opts: &CompareOptions) -> Report {
        let name = self.name.clone();
        let check = self.compare_with_options(opts);
        let mut r = Report::new(name, subject_version).with_producer("dev-stress");
        r.push(check);
        r.finish();
        r
    }

    fn numeric_evidence(&self) -> Vec<Evidence> {
        let mut e = vec![
            Evidence::numeric("iterations", self.iterations as f64),
            Evidence::numeric("threads", self.threads as f64),
            Evidence::numeric("ops_per_sec", self.ops_per_sec()),
            Evidence::numeric("thread_time_cv", self.thread_time_cv()),
            Evidence::numeric(
                "total_elapsed_ms",
                self.total_elapsed.as_secs_f64() * 1000.0,
            ),
        ];
        if let Some(lat) = &self.latency {
            e.push(Evidence::numeric(
                "latency_p50_ns",
                lat.p50.as_nanos() as f64,
            ));
            e.push(Evidence::numeric(
                "latency_p95_ns",
                lat.p95.as_nanos() as f64,
            ));
            e.push(Evidence::numeric(
                "latency_p99_ns",
                lat.p99.as_nanos() as f64,
            ));
            e.push(Evidence::numeric(
                "latency_samples",
                lat.samples_count as f64,
            ));
        }
        e
    }

    fn detail_string(&self) -> String {
        let lat = match &self.latency {
            Some(l) => format!(
                ", p50={}ns, p95={}ns, p99={}ns",
                l.p50.as_nanos(),
                l.p95.as_nanos(),
                l.p99.as_nanos()
            ),
            None => String::new(),
        };
        format!(
            "iterations={}, threads={}, total={:.3}s, ops/sec={:.0}, thread_cv={:.3}{}",
            self.iterations,
            self.threads,
            self.total_elapsed.as_secs_f64(),
            self.ops_per_sec(),
            self.thread_time_cv(),
            lat
        )
    }
}

/// Options controlling how a [`StressResult`] is compared against a baseline.
///
/// Defaults: no baseline; ops/sec drop threshold 10%; p99 regression threshold 20%.
///
/// # Example
///
/// ```
/// use dev_stress::CompareOptions;
///
/// let opts = CompareOptions {
///     baseline_ops_per_sec: Some(900_000.0),
///     ops_drop_pct_threshold: 5.0,
///     baseline_p99: None,
///     p99_regression_pct_threshold: 25.0,
/// };
/// assert_eq!(opts.ops_drop_pct_threshold, 5.0);
/// ```
#[derive(Debug, Clone)]
pub struct CompareOptions {
    /// Baseline throughput (ops/sec). When `Some`, the run fails if
    /// `ops_per_sec < baseline * (1 - ops_drop_pct_threshold/100)`.
    pub baseline_ops_per_sec: Option<f64>,
    /// Maximum allowed drop, as a percent.
    pub ops_drop_pct_threshold: f64,
    /// Baseline p99 latency. When `Some` AND latency was tracked, the
    /// run fails if `p99 > baseline_p99 * (1 + p99_regression_pct_threshold/100)`.
    pub baseline_p99: Option<Duration>,
    /// Maximum allowed p99 regression, as a percent.
    pub p99_regression_pct_threshold: f64,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            baseline_ops_per_sec: None,
            ops_drop_pct_threshold: 10.0,
            baseline_p99: None,
            p99_regression_pct_threshold: 20.0,
        }
    }
}

/// Producer wrapper that runs a stress run and emits a single-check `Report`.
///
/// # Example
///
/// ```no_run
/// use dev_stress::{CompareOptions, StressProducer, StressRun, Workload};
/// use dev_report::Producer;
///
/// #[derive(Clone)]
/// struct Noop;
/// impl Workload for Noop { fn run_once(&self) {} }
///
/// let producer = StressProducer::new(
///     || StressRun::new("hot").iterations(1_000).threads(2).execute(&Noop),
///     "0.1.0",
///     CompareOptions::default(),
/// );
/// let report = producer.produce();
/// assert_eq!(report.checks.len(), 1);
/// ```
pub struct StressProducer<F>
where
    F: Fn() -> StressResult,
{
    run: F,
    subject_version: String,
    opts: CompareOptions,
}

impl<F> StressProducer<F>
where
    F: Fn() -> StressResult,
{
    /// Build a new producer.
    pub fn new(run: F, subject_version: impl Into<String>, opts: CompareOptions) -> Self {
        Self {
            run,
            subject_version: subject_version.into(),
            opts,
        }
    }
}

impl<F> Producer for StressProducer<F>
where
    F: Fn() -> StressResult,
{
    fn produce(&self) -> Report {
        let result = (self.run)();
        result.into_report(self.subject_version.clone(), &self.opts)
    }
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
    fn run_completes() {
        let run = StressRun::new("noop").iterations(1_000).threads(2);
        let r = run.execute(&Noop);
        assert_eq!(r.iterations, 1_000);
        assert_eq!(r.threads, 2);
        assert!(r.ops_per_sec() > 0.0);
    }

    #[test]
    fn no_baseline_passes() {
        let run = StressRun::new("noop").iterations(100).threads(1);
        let r = run.execute(&Noop);
        let c = r.into_check_result(None);
        assert_eq!(c.verdict, Verdict::Pass);
        assert!(c.has_tag("stress"));
    }

    #[test]
    fn check_result_has_numeric_evidence() {
        let run = StressRun::new("noop").iterations(100).threads(2);
        let r = run.execute(&Noop);
        let c = r.into_check_result(None);
        let labels: Vec<&str> = c.evidence.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"iterations"));
        assert!(labels.contains(&"threads"));
        assert!(labels.contains(&"ops_per_sec"));
        assert!(labels.contains(&"thread_time_cv"));
        assert!(labels.contains(&"total_elapsed_ms"));
    }

    #[test]
    fn ops_drop_below_threshold_fails() {
        let run = StressRun::new("x").iterations(50).threads(1);
        let r = run.execute(&Noop);
        let baseline = r.ops_per_sec() * 100.0; // way higher than current
        let opts = CompareOptions {
            baseline_ops_per_sec: Some(baseline),
            ops_drop_pct_threshold: 10.0,
            ..CompareOptions::default()
        };
        let c = r.compare_with_options(&opts);
        assert_eq!(c.verdict, Verdict::Fail);
        assert!(c.has_tag("regression"));
    }

    #[test]
    fn ops_within_threshold_passes() {
        let run = StressRun::new("x").iterations(50).threads(1);
        let r = run.execute(&Noop);
        let baseline = r.ops_per_sec(); // exactly current
        let opts = CompareOptions {
            baseline_ops_per_sec: Some(baseline),
            ops_drop_pct_threshold: 10.0,
            ..CompareOptions::default()
        };
        let c = r.compare_with_options(&opts);
        assert_eq!(c.verdict, Verdict::Pass);
    }

    #[test]
    fn latency_tracking_records_percentiles() {
        let run = StressRun::new("hot")
            .iterations(1_000)
            .threads(2)
            .track_latency(1);
        let r = run.execute(&Noop);
        let lat = r.latency.expect("latency tracked");
        assert!(lat.samples_count > 0);
        assert!(lat.p50.as_nanos() <= lat.p95.as_nanos());
        assert!(lat.p95.as_nanos() <= lat.p99.as_nanos());
    }

    #[test]
    fn p99_regression_detected() {
        let run = StressRun::new("hot")
            .iterations(200)
            .threads(2)
            .track_latency(1);
        let r = run.execute(&Noop);
        // Baseline p99 set so far below current that any non-zero p99 fails.
        let opts = CompareOptions {
            baseline_p99: Some(Duration::from_nanos(1)),
            p99_regression_pct_threshold: 0.0,
            ..CompareOptions::default()
        };
        let c = r.compare_with_options(&opts);
        // If p99 was 0 (very fast noop) treat as pass; otherwise fail.
        if r.latency.as_ref().unwrap().p99.as_nanos() > 1 {
            assert_eq!(c.verdict, Verdict::Fail);
            assert!(c.has_tag("regression"));
        }
    }

    #[test]
    fn into_report_emits_one_check() {
        let run = StressRun::new("noop").iterations(100).threads(2);
        let r = run.execute(&Noop);
        let report = r.into_report("0.1.0", &CompareOptions::default());
        assert_eq!(report.checks.len(), 1);
        assert_eq!(report.producer.as_deref(), Some("dev-stress"));
        assert_eq!(report.subject, "noop");
    }

    #[test]
    fn stress_producer_implements_producer_trait() {
        let producer = StressProducer::new(
            || {
                StressRun::new("hot")
                    .iterations(50)
                    .threads(1)
                    .execute(&Noop)
            },
            "0.1.0",
            CompareOptions::default(),
        );
        let report = producer.produce();
        assert_eq!(report.checks.len(), 1);
    }

    #[test]
    fn iterations_distribute_evenly_with_leftover() {
        // 7 iters across 3 threads -> 3, 2, 2 (front-loaded).
        let run = StressRun::new("x").iterations(7).threads(3);
        let r = run.execute(&Noop);
        assert_eq!(r.iterations, 7);
        assert_eq!(r.thread_times.len(), 3);
    }

    #[test]
    fn target_ops_per_sec_divides_across_threads() {
        let run = StressRun::new("x")
            .iterations(100)
            .threads(4)
            .target_ops_per_sec(1000.0);
        // 1000 ops/sec / 4 threads = 250 ops/sec/thread.
        assert_eq!(run.target_ops_per_sec_per_thread(), Some(250.0));
    }

    #[test]
    fn target_rate_does_not_depend_on_builder_order() {
        // Calling threads() after target_ops_per_sec() used to keep the
        // per-thread rate computed for 1 thread, so 4 threads ran at 4x
        // the requested total.
        let run = StressRun::new("x")
            .iterations(100)
            .target_ops_per_sec(1000.0)
            .threads(4);
        assert_eq!(run.target_ops_per_sec_per_thread(), Some(250.0));
    }

    #[test]
    fn non_finite_target_rate_disables_cap_without_panicking() {
        for rate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let run = StressRun::new("x")
                .iterations(10)
                .threads(2)
                .target_ops_per_sec(rate);
            assert_eq!(run.target_ops_per_sec_per_thread(), None);
            // NaN used to reach Duration::from_secs_f64 and panic.
            let r = run.execute(&Noop);
            assert_eq!(r.iterations, 10);
        }
    }

    #[test]
    fn tiny_target_rate_does_not_panic() {
        // 1e-300 ops/sec makes the second op's target time overflow
        // Duration / Instant; that used to panic.
        let r = StressRun::new("x")
            .iterations(3)
            .threads(1)
            .target_ops_per_sec(1e-300)
            .execute(&Noop);
        assert_eq!(r.iterations, 3);
    }

    #[derive(Clone)]
    struct PanicsOnce {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Workload for PanicsOnce {
        fn run_once(&self) {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                panic!("workload exploded");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn worker_panic_is_reraised_after_all_workers_finish() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = PanicsOnce {
            calls: calls.clone(),
        };
        let run = StressRun::new("x").iterations(80).threads(4);
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.execute(&w)))
            .expect_err("worker panic must propagate");
        // The original payload, not a generic unwrap() message.
        assert_eq!(err.downcast_ref::<&str>(), Some(&"workload exploded"));
        // Every other worker had already finished: no calls after return.
        let at_return = calls.load(std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), at_return);
    }

    #[test]
    fn target_ops_per_sec_zero_or_negative_disables() {
        let run = StressRun::new("x")
            .iterations(10)
            .threads(2)
            .target_ops_per_sec(0.0);
        assert_eq!(run.target_ops_per_sec_per_thread(), None);
        let run = StressRun::new("x")
            .iterations(10)
            .threads(2)
            .target_ops_per_sec(-5.0);
        assert_eq!(run.target_ops_per_sec_per_thread(), None);
    }

    #[test]
    fn rate_limited_run_takes_at_least_target_duration() {
        // 50 ops at 100 ops/sec total -> at least 0.5 seconds (with one thread).
        // Use a reasonable lower bound to avoid flakes; we just want to
        // confirm the limiter actually slows things down.
        let started = Instant::now();
        let run = StressRun::new("x")
            .iterations(50)
            .threads(1)
            .target_ops_per_sec(100.0);
        let _ = run.execute(&Noop);
        let elapsed = started.elapsed();
        // Lower bound: 50/100 = 500ms; allow 50% slack for sleep granularity
        // on slow CI runners. Concretely we just want > 100ms (prove it slept).
        assert!(elapsed >= Duration::from_millis(100));
    }
}
