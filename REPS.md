# dev-stress — Project Specification (REPS)

> Rust Engineering Project Specification.
> Normative language follows RFC 2119.

## 1. Purpose

`dev-stress` MUST measure how a workload behaves under scaled-up
concurrency, volume, and sustained load. Output MUST be a
`dev-report::CheckResult` or `Report`.

## 2. Scope

This crate MUST provide:

- A `Workload` trait with concurrency-safe `run_once`.
- A `StressRun` builder with iterations + thread count.
- A `StressResult` with at least `ops_per_sec` and `thread_time_cv`.

This crate SHOULD provide (later versions):

- Per-operation latency percentile tracking (p50, p95, p99).
- Soak test infrastructure with checkpoint reports.
- Memory pressure tracking integration.
- CPU saturation detection.

This crate MUST NOT:

- Run single-threaded micro-benchmarks (use `dev-bench`).
- Test async-specific behavior (use `dev-async`).
- Inject failures (use `dev-chaos`).

## 3. Concurrency model

`StressRun` MUST distribute iterations across OS threads. The
distribution MUST be approximately equal: when iterations don't
divide evenly, the leftover MUST be spread across threads, not
piled onto one thread.

`Workload` implementations MUST be `Send + Sync + Clone`.

## 4. Statistics

- `ops_per_sec`: total iterations divided by total wall-clock elapsed.
- `thread_time_cv`: coefficient of variation across per-thread
  elapsed times. MUST use sample standard deviation divided by mean.
- `latency_p50`, `latency_p95`, `latency_p99`: per-operation latency
  percentiles, computed losslessly from a sorted sample set when
  [`StressRun::track_latency`] is enabled.
- All stats MUST be computed losslessly (no precomputed bins).

`ops_per_sec` and `thread_time_cv` are the **immutable contract**;
their definitions MUST NOT change. Latency percentiles are additive
at v0.2.x.

## 5. Regression detection

`CompareOptions` configures thresholds:

- `ops_drop_pct_threshold`: max allowed drop in `ops_per_sec` vs
  `baseline_ops_per_sec` (e.g. `10.0` = fail if more than 10% slower).
- `p99_regression_pct_threshold`: max allowed growth in p99 latency
  vs `baseline_p99` (e.g. `20.0` = fail if more than 20% slower).

Verdict semantics:

- No baselines provided -> `Pass` with detail.
- Any threshold breached -> `Fail+Warning`, with `regression` tag.
- All thresholds satisfied -> `Pass`.

Severity escalation to `Error` is the consumer's choice via report
aggregation rules.

### 5.1 Required evidence

Every `CheckResult` emitted by `compare_*` MUST carry the `stress`
tag and numeric `Evidence` for:

- `iterations`
- `threads`
- `ops_per_sec`
- `thread_time_cv`
- `total_elapsed_ms`

When latency was tracked, MUST additionally carry:

- `latency_p50_ns`, `latency_p95_ns`, `latency_p99_ns`, `latency_samples`

When a baseline was provided, MUST additionally carry:

- `baseline_ops_per_sec` (when `baseline_ops_per_sec` is set)
- `baseline_p99_ns` (when `baseline_p99` is set)

Regression-flagged checks MUST additionally carry the `regression` tag.

## 6. Soak tests

`SoakRun` is duration-bounded. It runs for `total_duration` and
records one [`SoakCheckpoint`] every `checkpoint_interval`. Soak
verdicts derive from a degradation comparison between the first
half and second half of checkpoints.

A `SoakResult::into_check_result` MUST:

- Skip when fewer than 2 checkpoints exist.
- Fail when `(first_half_mean_ops - second_half_mean_ops) /
  first_half_mean_ops * 100` exceeds the configured degradation
  threshold.
- Carry both `stress` and `soak` tags.

## 7. System stats (opt-in)

Available with the `system-stats` feature flag.

- `dev_stress::system::SystemSampler` wraps `sysinfo` to capture
  process RSS and CPU time.
- `SystemStats::compare(name, before, after, threshold)` emits a
  `CheckResult` tagged `stress` and `system`.

The feature MUST NOT be required for basic use.

## 8. Producer integration

This crate MUST provide a way to satisfy `dev_report::Producer`. The
provided `StressProducer` wraps a closure that returns a
`StressResult` and emits a single-check `Report` with
`producer = "dev-stress"`.
