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
- Both stats MUST be computed losslessly.

## 5. Regression detection

In `0.1.x`: a baseline `ops_per_sec` MAY be passed in. If current
`ops_per_sec` is less than 90% of baseline, the verdict is `Fail`
with severity `Warning`.

In later versions: thresholds MUST be configurable.
