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
//! `dev-stress` measures how a workload behaves when the inputs scale
//! up: thousands of concurrent operations, millions of iterations,
//! sustained pressure. It detects throughput collapse, latency
//! cliff-falls, and lock contention.
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
//! let check = result.into_check_result(None);
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use dev_report::{CheckResult, Severity};

/// A workload that can be executed many times under stress.
pub trait Workload: Send + Sync {
    /// Execute one unit of work. MUST be safe to call concurrently
    /// from multiple threads.
    fn run_once(&self);
}

/// Configuration for a stress run.
pub struct StressRun {
    name: String,
    iterations: usize,
    threads: usize,
}

impl StressRun {
    /// Begin building a stress run with a stable name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            iterations: 1_000,
            threads: 1,
        }
    }

    /// Total iterations across all threads.
    pub fn iterations(mut self, n: usize) -> Self {
        self.iterations = n;
        self
    }

    /// Number of OS threads to run concurrently.
    pub fn threads(mut self, n: usize) -> Self {
        self.threads = n.max(1);
        self
    }

    /// Execute the run. Returns a result with timing statistics.
    pub fn execute<W: Workload + 'static>(&self, workload: &W) -> StressResult
    where
        W: Clone,
    {
        let per_thread = self.iterations / self.threads;
        let leftover = self.iterations % self.threads;
        let started = Instant::now();
        let mut handles = Vec::with_capacity(self.threads);
        let workload = Arc::new(workload.clone());

        for t in 0..self.threads {
            let count = per_thread + if t < leftover { 1 } else { 0 };
            let w = workload.clone();
            handles.push(std::thread::spawn(move || {
                let start = Instant::now();
                for _ in 0..count {
                    w.run_once();
                }
                start.elapsed()
            }));
        }

        let mut thread_times = Vec::with_capacity(self.threads);
        for h in handles {
            thread_times.push(h.join().unwrap());
        }
        let total_elapsed = started.elapsed();

        StressResult {
            name: self.name.clone(),
            iterations: self.iterations,
            threads: self.threads,
            total_elapsed,
            thread_times,
        }
    }
}

/// Result of a stress run.
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

    /// Convert this result into a `CheckResult`. If a baseline
    /// throughput is provided, regression-style verdict is computed.
    pub fn into_check_result(self, baseline_ops_per_sec: Option<f64>) -> CheckResult {
        let ops = self.ops_per_sec();
        let cv = self.thread_time_cv();
        let detail = format!(
            "iterations={}, threads={}, total={:.3}s, ops/sec={:.0}, thread_cv={:.3}",
            self.iterations,
            self.threads,
            self.total_elapsed.as_secs_f64(),
            ops,
            cv
        );
        let name = format!("stress::{}", self.name);
        match baseline_ops_per_sec {
            None => CheckResult::pass(name).with_detail(detail),
            Some(baseline) if ops < baseline * 0.9 => {
                CheckResult::fail(name, Severity::Warning).with_detail(detail)
            }
            Some(_) => CheckResult::pass(name).with_detail(detail),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(matches!(c.verdict, dev_report::Verdict::Pass));
    }
}
