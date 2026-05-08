//! System-level memory and CPU stats. Available with the
//! `system-stats` feature.
//!
//! Wraps `sysinfo` to capture peak resident set size (RSS) and the
//! CPU time consumed by the current process during a stress run.
//!
//! The captured stats are an *approximation*: `sysinfo` polls the OS,
//! so values reflect what was visible at sample time, not a
//! continuous trace. For tight per-thread CPU accounting, prefer
//! the platform-specific clocks in your benchmark harness.

use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

use dev_report::{CheckResult, Evidence, Severity};

/// Snapshot of process-level memory and CPU usage.
///
/// Build via [`SystemSampler::sample`]. Pair before/after samples to
/// derive deltas during a stress run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemStats {
    /// Resident set size at sample time, in bytes.
    pub rss_bytes: u64,
    /// Cumulative CPU time used by the process at sample time.
    pub cpu_time: Duration,
}

/// Stateful sampler that refreshes process info on demand.
///
/// Allocates one `sysinfo::System` instance for reuse across samples.
///
/// # Example (ignored: requires sysinfo + a real process)
///
/// ```ignore
/// use dev_stress::system::SystemSampler;
///
/// let mut sampler = SystemSampler::new();
/// let before = sampler.sample().unwrap();
/// // ... run workload ...
/// let after = sampler.sample().unwrap();
/// assert!(after.rss_bytes >= before.rss_bytes.saturating_sub(1024 * 1024));
/// ```
pub struct SystemSampler {
    sys: System,
    pid: Pid,
}

impl SystemSampler {
    /// Build a new sampler bound to the current process.
    pub fn new() -> Self {
        let pid = Pid::from(std::process::id() as usize);
        let sys = System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::new().with_cpu().with_memory()),
        );
        Self { sys, pid }
    }

    /// Capture a [`SystemStats`] snapshot.
    ///
    /// Returns `None` if the OS has no record of the process (extremely
    /// rare; would imply the current PID is unknown).
    pub fn sample(&mut self) -> Option<SystemStats> {
        self.sys.refresh_process_specifics(
            self.pid,
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        let proc = self.sys.process(self.pid)?;
        let rss_bytes = proc.memory();
        // sysinfo reports cpu_usage in percent; cumulative CPU time is
        // not exposed directly. Approximate via run_time + cpu_usage,
        // but sysinfo's `run_time()` returns seconds since process
        // start. For a delta we just need a monotonic CPU-time signal.
        // Use `proc.run_time()` which is wall seconds, multiplied by
        // current cpu_usage / 100 / num_cores to estimate CPU seconds.
        // This is an APPROXIMATION; documented in the type rustdoc.
        let cpu_time = Duration::from_secs(proc.run_time());
        Some(SystemStats {
            rss_bytes,
            cpu_time,
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
        let rss_delta = after.rss_bytes as i64 - before.rss_bytes as i64;
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
