//! System-level memory and CPU stats. Available with the
//! `system-stats` feature.
//!
//! Wraps `sysinfo` to capture the resident set size (RSS) and the
//! CPU time consumed by the current process during a stress run.
//!
//! The captured stats are an *approximation*: `sysinfo` polls the OS,
//! so values reflect what was visible at sample time, not a
//! continuous trace. RSS is the value at sample time, not a peak
//! between samples. For tight per-thread CPU accounting, prefer
//! the platform-specific clocks in your benchmark harness.

use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, System};

use dev_report::{CheckResult, Evidence, Severity};

/// Snapshot of process-level memory and CPU usage.
///
/// Build via [`SystemSampler::sample`]. Pair before/after samples from
/// the same sampler to derive deltas during a stress run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemStats {
    /// Resident set size at sample time, in bytes.
    pub rss_bytes: u64,
    /// CPU time (user + system, all threads) used by the process since
    /// the [`SystemSampler`] that produced this snapshot was created.
    ///
    /// It is not counted from process start, so only compare values
    /// taken from the same sampler. The difference between two samples
    /// is the CPU time spent between them; with several busy threads it
    /// can exceed the wall-clock time between them.
    pub cpu_time: Duration,
}

/// Stateful sampler that refreshes process info on demand.
///
/// Holds one `sysinfo::System` instance for reuse across samples and
/// only refreshes the current process.
///
/// CPU time is accumulated from sysinfo's per-refresh CPU usage: each
/// [`sample`](Self::sample) adds `cpu_usage / 100 * time since the
/// previous refresh`. sysinfo computes that usage from the process's
/// CPU time over the same interval, so the sum tracks the CPU time the
/// process consumed while the sampler existed.
///
/// # Example
///
/// ```
/// use dev_stress::system::{SystemSampler, SystemStats};
///
/// let mut sampler = SystemSampler::new();
/// let before = sampler.sample().unwrap();
/// // ... run workload ...
/// let after = sampler.sample().unwrap();
/// assert!(after.cpu_time >= before.cpu_time);
/// let _check = SystemStats::compare("hot", before, after, None);
/// ```
pub struct SystemSampler {
    sys: System,
    pid: Pid,
    /// When the process was last refreshed; the start of the interval
    /// the next refresh's CPU usage covers.
    last_refresh: Instant,
    /// CPU seconds accumulated since the sampler was created.
    cpu_secs: f64,
}

impl SystemSampler {
    /// Build a new sampler bound to the current process.
    pub fn new() -> Self {
        let pid = Pid::from(std::process::id() as usize);
        let mut sys = System::new();
        // Prime the CPU counters: sysinfo's usage needs a previous refresh.
        sys.refresh_process_specifics(pid, Self::refresh_kind());
        Self {
            sys,
            pid,
            last_refresh: Instant::now(),
            cpu_secs: 0.0,
        }
    }

    fn refresh_kind() -> ProcessRefreshKind {
        ProcessRefreshKind::new().with_cpu().with_memory()
    }

    /// Capture a [`SystemStats`] snapshot.
    ///
    /// Returns `None` if the OS has no record of the process (extremely
    /// rare; would imply the current PID is unknown).
    pub fn sample(&mut self) -> Option<SystemStats> {
        self.sys
            .refresh_process_specifics(self.pid, Self::refresh_kind());
        let now = Instant::now();
        let interval = now.saturating_duration_since(self.last_refresh);
        self.last_refresh = now;
        let proc = self.sys.process(self.pid)?;
        let rss_bytes = proc.memory();
        // cpu_usage is the process's CPU time over the last refresh
        // interval as a percent of that interval (above 100 with several
        // busy threads), so usage / 100 * interval is the CPU time spent.
        let usage = f64::from(proc.cpu_usage());
        if usage.is_finite() && usage > 0.0 {
            self.cpu_secs += usage / 100.0 * interval.as_secs_f64();
        }
        Some(SystemStats {
            rss_bytes,
            cpu_time: Duration::try_from_secs_f64(self.cpu_secs).unwrap_or(Duration::MAX),
        })
    }
}

impl Default for SystemSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemStats {
    /// Compare a `before`/`after` pair and emit a `CheckResult`.
    ///
    /// `peak_rss_bytes_threshold` flags `Fail+Warning` when the
    /// `after` RSS exceeds the threshold. `None` disables the check.
    /// Only the `after` snapshot is compared; a higher RSS between the
    /// two samples is not seen.
    ///
    /// Always carries the `stress`, `system` tags and numeric
    /// evidence for `rss_bytes_before`, `rss_bytes_after`,
    /// `rss_delta_bytes`, `cpu_time_before_s`, `cpu_time_after_s`,
    /// `cpu_time_delta_s`.
    pub fn compare(
        name: &str,
        before: SystemStats,
        after: SystemStats,
        peak_rss_bytes_threshold: Option<u64>,
    ) -> CheckResult {
        let check_name = format!("stress::system::{}", name);
        // i128 so the signed difference of two u64 values cannot overflow.
        let rss_delta = i128::from(after.rss_bytes) - i128::from(before.rss_bytes);
        let cpu_delta = after.cpu_time.saturating_sub(before.cpu_time);
        let evidence = vec![
            Evidence::numeric("rss_bytes_before", before.rss_bytes as f64),
            Evidence::numeric("rss_bytes_after", after.rss_bytes as f64),
            Evidence::numeric("rss_delta_bytes", rss_delta as f64),
            Evidence::numeric("cpu_time_before_s", before.cpu_time.as_secs_f64()),
            Evidence::numeric("cpu_time_after_s", after.cpu_time.as_secs_f64()),
            Evidence::numeric("cpu_time_delta_s", cpu_delta.as_secs_f64()),
        ];
        let detail = format!(
            "rss_before={} rss_after={} rss_delta={} cpu_delta={}s",
            before.rss_bytes,
            after.rss_bytes,
            rss_delta,
            cpu_delta.as_secs_f64()
        );

        let regressed = peak_rss_bytes_threshold
            .map(|threshold| after.rss_bytes > threshold)
            .unwrap_or(false);

        let tags = vec!["stress".to_string(), "system".to_string()];
        if regressed {
            let mut tags = tags;
            tags.push("regression".to_string());
            let mut c = CheckResult::fail(check_name, Severity::Warning).with_detail(detail);
            c.tags = tags;
            c.evidence = evidence;
            c
        } else {
            let mut c = CheckResult::pass(check_name).with_detail(detail);
            c.tags = tags;
            c.evidence = evidence;
            c
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dev_report::Verdict;

    #[test]
    fn sampler_returns_some_for_current_process() {
        let mut s = SystemSampler::new();
        let snap = s.sample();
        assert!(snap.is_some());
    }

    #[test]
    fn cpu_time_measures_cpu_not_wall_clock() {
        // cpu_time used to be sysinfo's run_time(): whole seconds of wall
        // clock since process start, so a short burst of CPU work showed
        // a delta of 0s (or 1s if a second boundary was crossed).
        let mut s = SystemSampler::new();
        let before = s.sample().unwrap();
        let burn = Duration::from_millis(400);
        let workers: Vec<_> = (0..2)
            .map(|_| {
                std::thread::spawn(move || {
                    let start = std::time::Instant::now();
                    let mut x = 0u64;
                    while start.elapsed() < burn {
                        x = std::hint::black_box(x.wrapping_add(1));
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().unwrap();
        }
        let after = s.sample().unwrap();
        let delta = after.cpu_time.saturating_sub(before.cpu_time);
        // Two threads busy for 400ms is about 0.8s of CPU. Wide bounds
        // keep this stable on loaded CI machines.
        assert!(
            delta >= Duration::from_millis(250),
            "cpu delta {delta:?} too small"
        );
        assert!(
            delta <= Duration::from_secs(5),
            "cpu delta {delta:?} too large"
        );
    }

    #[test]
    fn cpu_time_never_goes_backwards() {
        let mut s = SystemSampler::new();
        let mut last = Duration::ZERO;
        for _ in 0..5 {
            let snap = s.sample().unwrap();
            assert!(snap.cpu_time >= last);
            last = snap.cpu_time;
        }
    }

    #[test]
    fn compare_handles_rss_shrinking_and_extreme_values() {
        let before = SystemStats {
            rss_bytes: u64::MAX,
            cpu_time: Duration::ZERO,
        };
        let after = SystemStats {
            rss_bytes: 0,
            cpu_time: Duration::ZERO,
        };
        let c = SystemStats::compare("x", before, after, None);
        let delta = c
            .evidence
            .iter()
            .find(|e| e.label == "rss_delta_bytes")
            .unwrap();
        match delta.data {
            dev_report::EvidenceData::Numeric(n) => assert_eq!(n, -(u64::MAX as f64)),
            _ => panic!("expected numeric"),
        }
    }

    #[test]
    fn compare_below_threshold_passes() {
        let before = SystemStats {
            rss_bytes: 100,
            cpu_time: Duration::from_secs(0),
        };
        let after = SystemStats {
            rss_bytes: 200,
            cpu_time: Duration::from_secs(1),
        };
        let c = SystemStats::compare("x", before, after, Some(1_000_000));
        assert_eq!(c.verdict, Verdict::Pass);
        assert!(c.has_tag("stress"));
        assert!(c.has_tag("system"));
    }

    #[test]
    fn compare_over_threshold_fails() {
        let before = SystemStats {
            rss_bytes: 100,
            cpu_time: Duration::from_secs(0),
        };
        let after = SystemStats {
            rss_bytes: 2_000,
            cpu_time: Duration::from_secs(1),
        };
        let c = SystemStats::compare("x", before, after, Some(1_000));
        assert_eq!(c.verdict, Verdict::Fail);
        assert!(c.has_tag("regression"));
    }

    #[test]
    fn compare_no_threshold_passes() {
        let before = SystemStats {
            rss_bytes: 100,
            cpu_time: Duration::from_secs(0),
        };
        let after = SystemStats {
            rss_bytes: 1_000_000,
            cpu_time: Duration::from_secs(10),
        };
        let c = SystemStats::compare("x", before, after, None);
        assert_eq!(c.verdict, Verdict::Pass);
    }

    #[test]
    fn compare_carries_all_evidence_labels() {
        let before = SystemStats {
            rss_bytes: 100,
            cpu_time: Duration::from_secs(0),
        };
        let after = SystemStats {
            rss_bytes: 200,
            cpu_time: Duration::from_secs(1),
        };
        let c = SystemStats::compare("x", before, after, None);
        let labels: Vec<&str> = c.evidence.iter().map(|e| e.label.as_str()).collect();
        for lbl in &[
            "rss_bytes_before",
            "rss_bytes_after",
            "rss_delta_bytes",
            "cpu_time_before_s",
            "cpu_time_after_s",
            "cpu_time_delta_s",
        ] {
            assert!(labels.contains(lbl), "missing evidence label: {}", lbl);
        }
    }
}
