//! Per-operation latency tracking for stress runs.
//!
//! [`LatencyTracker`] is a thread-local sampler. Each thread allocates
//! its own tracker; samples are merged at run finish to compute
//! [`LatencyStats`] (p50, p95, p99). No locking on the hot path.

use std::time::{Duration, Instant};

/// Captures per-operation latency samples for a single thread.
///
/// The tracker is intentionally thread-local: each thread keeps a
/// `Vec<Duration>` and the runner concatenates them at finish. There
/// is no shared state, so sample collection introduces no
/// synchronization that would distort the measurement.
pub struct LatencyTracker {
    samples: Vec<Duration>,
    sample_rate: usize,
}

impl LatencyTracker {
    /// Create a tracker that samples `1` of every `rate` iterations.
    ///
    /// `rate = 1` records every iteration. `rate = 100` records 1%.
    /// Pass at least `1`; values below are clamped.
    pub fn new(rate: usize) -> Self {
        Self {
            samples: Vec::new(),
            sample_rate: rate.max(1),
        }
    }

    /// Run the closure and, if `iter_index` is on the sampling
    /// schedule, record its duration.
    ///
    /// `iter_index` is the 0-based iteration counter on the calling
    /// thread. The tracker records the sample when
    /// `iter_index % sample_rate == 0`.
    ///
    /// # Example
    ///
    /// ```
    /// use dev_stress::LatencyTracker;
    ///
    /// let mut t = LatencyTracker::new(1);
    /// t.record(0, || std::hint::black_box(1 + 1));
    /// t.record(1, || std::hint::black_box(1 + 1));
    /// assert_eq!(t.samples_count(), 2);
    /// ```
    pub fn record<F, R>(&mut self, iter_index: usize, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        if iter_index % self.sample_rate == 0 {
            let start = Instant::now();
            let r = f();
            self.samples.push(start.elapsed());
            r
        } else {
            f()
        }
    }

    /// Number of samples currently held by this tracker.
    pub fn samples_count(&self) -> usize {
        self.samples.len()
    }

    /// Move all samples out of this tracker.
    pub fn into_samples(self) -> Vec<Duration> {
        self.samples
    }
}

/// Aggregated latency statistics across a stress run.
///
/// Computed from the concatenation of every per-thread tracker's samples.
///
/// Percentiles use the nearest-rank method: the p-th percentile is the
/// smallest sample such that at least p% of all samples are less than or
/// equal to it (sorted index `ceil(p/100 * n) - 1`). Every reported value
/// is a real sample. For 100 samples of 1..=100 ns, p50 is 50 ns, p95 is
/// 95 ns and p99 is 99 ns.
///
/// # Example
///
/// ```
/// use dev_stress::LatencyStats;
/// use std::time::Duration;
///
/// let stats = LatencyStats::from_samples(vec![
///     Duration::from_nanos(10),
///     Duration::from_nanos(20),
///     Duration::from_nanos(30),
///     Duration::from_nanos(40),
///     Duration::from_nanos(50),
/// ]);
/// assert_eq!(stats.samples_count, 5);
/// assert_eq!(stats.p50, Duration::from_nanos(30));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatencyStats {
    /// 50th percentile sample.
    pub p50: Duration,
    /// 95th percentile sample.
    pub p95: Duration,
    /// 99th percentile sample.
    pub p99: Duration,
    /// Total number of samples used to compute the percentiles.
    pub samples_count: usize,
}

impl LatencyStats {
    /// Compute percentile statistics from a sample set.
    ///
    /// Returns zero-valued percentiles when `samples` is empty.
    pub fn from_samples(mut samples: Vec<Duration>) -> Self {
        let n = samples.len();
        if n == 0 {
            return Self {
                p50: Duration::ZERO,
                p95: Duration::ZERO,
                p99: Duration::ZERO,
                samples_count: 0,
            };
        }
        samples.sort_unstable();
        Self {
            p50: samples[nearest_rank_index(n, 50)],
            p95: samples[nearest_rank_index(n, 95)],
            p99: samples[nearest_rank_index(n, 99)],
            samples_count: n,
        }
    }
}

/// Zero-based index of the nearest-rank `pct` percentile in a sorted
/// slice of `n > 0` samples: `ceil(pct * n / 100) - 1`, clamped to
/// `[0, n - 1]`. Integer math, so no floating-point rounding at exact
/// boundaries.
fn nearest_rank_index(n: usize, pct: u64) -> usize {
    let rank = (n as u128 * pct as u128).div_ceil(100);
    (rank.max(1) - 1).min(n as u128 - 1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_one_records_every_iter() {
        let mut t = LatencyTracker::new(1);
        for i in 0..10 {
            t.record(i, || std::hint::black_box(i));
        }
        assert_eq!(t.samples_count(), 10);
    }

    #[test]
    fn rate_n_records_one_in_n() {
        let mut t = LatencyTracker::new(5);
        for i in 0..50 {
            t.record(i, || std::hint::black_box(i));
        }
        // Iter indices 0, 5, 10, 15, 20, 25, 30, 35, 40, 45 -> 10 samples.
        assert_eq!(t.samples_count(), 10);
    }

    #[test]
    fn empty_samples_yield_zero_stats() {
        let s = LatencyStats::from_samples(vec![]);
        assert_eq!(s.p50, Duration::ZERO);
        assert_eq!(s.p95, Duration::ZERO);
        assert_eq!(s.p99, Duration::ZERO);
        assert_eq!(s.samples_count, 0);
    }

    #[test]
    fn percentiles_are_ordered() {
        let samples: Vec<Duration> = (1..=100).map(|i| Duration::from_nanos(i as u64)).collect();
        let s = LatencyStats::from_samples(samples);
        assert!(s.p50 <= s.p95);
        assert!(s.p95 <= s.p99);
    }

    fn ns(values: &[u64]) -> Vec<Duration> {
        values.iter().map(|&v| Duration::from_nanos(v)).collect()
    }

    #[test]
    fn nearest_rank_percentiles_for_one_to_hundred() {
        // The old floor(n * p) index reported p95 = 96 and p99 = 100
        // (the maximum) here, one rank too high.
        let samples: Vec<Duration> = (1..=100).map(Duration::from_nanos).collect();
        let s = LatencyStats::from_samples(samples);
        assert_eq!(s.p50, Duration::from_nanos(50));
        assert_eq!(s.p95, Duration::from_nanos(95));
        assert_eq!(s.p99, Duration::from_nanos(99));
    }

    #[test]
    fn p99_is_not_the_max_once_there_are_enough_samples() {
        // 200 samples: 198 fast, 2 slow outliers. 1% of 200 = 2, so the
        // p99 must still be a fast sample.
        let mut v = vec![10u64; 198];
        v.extend([5_000, 9_000]);
        let s = LatencyStats::from_samples(ns(&v));
        assert_eq!(s.p99, Duration::from_nanos(10));
        // With 3 slow samples out of 200, more than 1% are slow, so p99 is
        // the smallest slow one.
        let mut v = vec![10u64; 197];
        v.extend([5_000, 6_000, 9_000]);
        assert_eq!(
            LatencyStats::from_samples(ns(&v)).p99,
            Duration::from_nanos(5_000)
        );
    }

    #[test]
    fn two_samples_use_lower_median() {
        // The old n / 2 index made the median of [10, 20] equal to 20.
        let s = LatencyStats::from_samples(ns(&[20, 10]));
        assert_eq!(s.p50, Duration::from_nanos(10));
        assert_eq!(s.p95, Duration::from_nanos(20));
        assert_eq!(s.p99, Duration::from_nanos(20));
    }

    #[test]
    fn single_sample_is_every_percentile() {
        let s = LatencyStats::from_samples(ns(&[7]));
        assert_eq!(s.p50, Duration::from_nanos(7));
        assert_eq!(s.p95, Duration::from_nanos(7));
        assert_eq!(s.p99, Duration::from_nanos(7));
        assert_eq!(s.samples_count, 1);
    }

    #[test]
    fn unsorted_input_is_sorted_first() {
        let s = LatencyStats::from_samples(ns(&[50, 10, 40, 20, 30]));
        assert_eq!(s.p50, Duration::from_nanos(30));
        assert_eq!(s.p99, Duration::from_nanos(50));
    }

    #[test]
    fn nearest_rank_index_edges() {
        assert_eq!(nearest_rank_index(1, 50), 0);
        assert_eq!(nearest_rank_index(1, 99), 0);
        assert_eq!(nearest_rank_index(20, 95), 18);
        assert_eq!(nearest_rank_index(20, 99), 19);
        assert_eq!(nearest_rank_index(1000, 99), 989);
        assert_eq!(
            nearest_rank_index(usize::MAX, 99),
            (usize::MAX as u128 * 99).div_ceil(100) as usize - 1
        );
    }

    #[test]
    fn into_samples_moves_data() {
        let mut t = LatencyTracker::new(1);
        for i in 0..5 {
            t.record(i, || ());
        }
        let s = t.into_samples();
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn rate_zero_clamps_to_one() {
        let mut t = LatencyTracker::new(0);
        for i in 0..5 {
            t.record(i, || ());
        }
        assert_eq!(t.samples_count(), 5);
    }
}
