<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <strong>dev-stress</strong>
    <br>
    <sup><sub>HIGH-LOAD STRESS TESTING FOR RUST</sub></sup>
</h1>
<p align="center">
    <a href="https://crates.io/crates/dev-stress"><img alt="crates.io" src="https://img.shields.io/crates/v/dev-stress.svg"></a>
    <a href="https://crates.io/crates/dev-stress"><img alt="downloads" src="https://img.shields.io/crates/d/dev-stress.svg"></a>
    <a href="https://github.com/jamesgober/dev-stress/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/jamesgober/dev-stress/actions/workflows/ci.yml/badge.svg"></a>
    <img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue.svg?style=flat-square" title="Rust Version">
    <a href="https://docs.rs/dev-stress"><img alt="docs.rs" src="https://docs.rs/dev-stress/badge.svg"></a>
</p>

<p align="center">
    <strong>Stress and soak workloads with latency percentiles.</strong> Thousands of concurrent ops, millions of iterations, sustained pressure — and a verdict at the end.
</p>

<br>

<div align="center">
    <strong>Part of the <a href="https://crates.io/crates/dev-tools"><code>dev-*</code></a> verification collection.</strong><br>
    <sub>Also available as the <code>stress</code> feature of the <a href="https://crates.io/crates/dev-tools"><code>dev-tools</code></a> umbrella crate &mdash; one dependency, every verification layer.</sub>
</div>

<br>

---

## What it does

`dev-stress` measures how a workload behaves when scaled up: thousands
of concurrent operations, millions of iterations, sustained pressure.

It detects:

- Throughput collapse under concurrency
- Lock contention (via thread-time variance)
- Latency cliff-falls
- Sustained-load instability

It does NOT do:

- Single-threaded micro-benchmarking (use [`dev-bench`](https://github.com/jamesgober/dev-bench))
- Async-specific issue detection (use [`dev-async`](https://github.com/jamesgober/dev-async))
- Failure injection (use [`dev-chaos`](https://github.com/jamesgober/dev-chaos))

## Quick start

```toml
[dependencies]
dev-stress = "0.9.4"
```

```rust
use dev_stress::{Workload, StressRun};

#[derive(Clone)]
struct MyWorkload;
impl Workload for MyWorkload {
    fn run_once(&self) {
        std::hint::black_box(40 + 2);
    }
}

let run = StressRun::new("hot_path")
    .iterations(100_000)
    .threads(8);

let result = run.execute(&MyWorkload);
println!("ops/sec: {}", result.ops_per_sec());
println!("thread CV: {}", result.thread_time_cv());

let _check = result.into_check_result(None);
```

The returned `CheckResult` carries the `stress` tag and numeric
`Evidence` for `iterations`, `threads`, `ops_per_sec`,
`thread_time_cv`, and `total_elapsed_ms` — no detail-string parsing.

## Per-op latency percentiles

Track p50/p95/p99 per operation by enabling latency tracking:

```rust
use dev_stress::{StressRun, Workload};

#[derive(Clone)]
struct MyWorkload;
impl Workload for MyWorkload {
    fn run_once(&self) { std::hint::black_box(40 + 2); }
}

let run = StressRun::new("hot")
    .iterations(100_000)
    .threads(4)
    .track_latency(10);  // 10% sampling

let r = run.execute(&MyWorkload);
if let Some(lat) = &r.latency {
    println!("p99 = {}ns", lat.p99.as_nanos());
}
```

## Configurable thresholds

```rust
use dev_stress::{CompareOptions, StressRun, Workload};
use std::time::Duration;

#[derive(Clone)]
struct MyWorkload;
impl Workload for MyWorkload {
    fn run_once(&self) { std::hint::black_box(40 + 2); }
}

let r = StressRun::new("hot").iterations(10_000).threads(4)
    .track_latency(1)
    .execute(&MyWorkload);

let opts = CompareOptions {
    baseline_ops_per_sec: Some(900_000.0),
    ops_drop_pct_threshold: 10.0,
    baseline_p99: Some(Duration::from_micros(50)),
    p99_regression_pct_threshold: 25.0,
};
let _check = r.compare_with_options(&opts);
```

## Soak tests

Run a workload for sustained duration and detect degradation across
checkpoints:

```rust
use dev_stress::{SoakRun, Workload};
use std::time::Duration;

#[derive(Clone)]
struct MyWorkload;
impl Workload for MyWorkload {
    fn run_once(&self) { std::hint::black_box(40 + 2); }
}

let r = SoakRun::new("steady")
    .duration(Duration::from_secs(30))
    .checkpoint(Duration::from_secs(5))
    .threads(4)
    .track_latency(100)
    .execute(&MyWorkload);

// Fail if second-half mean ops/sec drops more than 15% below first half.
let _check = r.into_check_result(15.0);
```

## Producer trait

```rust
use dev_stress::{CompareOptions, StressProducer, StressRun, Workload};
use dev_report::Producer;

#[derive(Clone)]
struct MyWorkload;
impl Workload for MyWorkload {
    fn run_once(&self) { std::hint::black_box(40 + 2); }
}

let producer = StressProducer::new(
    || StressRun::new("hot").iterations(10_000).threads(4).execute(&MyWorkload),
    "0.1.0",
    CompareOptions::default(),
);
let report = producer.produce(); // dev_report::Report
```

## System stats (opt-in)

```toml
[dependencies]
dev-stress = { version = "0.9.4", features = ["system-stats"] }
```

```rust,ignore
use dev_stress::system::{SystemSampler, SystemStats};

let mut sampler = SystemSampler::new();
let before = sampler.sample().unwrap();
// ... run workload ...
let after = sampler.sample().unwrap();
let _check = SystemStats::compare("hot", before, after, Some(500_000_000));
```

## Thread-time CV

The coefficient of variation across thread elapsed times is the
clearest signal that a workload is contention-bound:

- CV near 0: threads finished at nearly the same time. Healthy.
- CV > 0.2: significant variance. Some threads were waiting on locks
  or contended resources.
- CV > 0.5: severe contention. Investigate.

## The `dev-*` collection

`dev-stress` ships independently and is also re-exported by the
[`dev-tools`](https://crates.io/crates/dev-tools) umbrella crate as
the `stress` feature. Sister crates cover the other verification
dimensions:

- [`dev-report`](https://crates.io/crates/dev-report) &mdash; report schema everything emits
- [`dev-fixtures`](https://crates.io/crates/dev-fixtures) &mdash; deterministic test fixtures
- [`dev-bench`](https://crates.io/crates/dev-bench) &mdash; performance and regression detection
- [`dev-async`](https://crates.io/crates/dev-async) &mdash; async runtime verification
- [`dev-chaos`](https://crates.io/crates/dev-chaos) &mdash; fault injection and recovery testing
- [`dev-coverage`](https://crates.io/crates/dev-coverage) &mdash; code coverage with regression gates
- [`dev-security`](https://crates.io/crates/dev-security) &mdash; CVE / license / banned-crate audit
- [`dev-deps`](https://crates.io/crates/dev-deps) &mdash; unused / outdated dep detection
- [`dev-ci`](https://crates.io/crates/dev-ci) &mdash; GitHub Actions workflow generator
- [`dev-fuzz`](https://crates.io/crates/dev-fuzz) &mdash; fuzz testing workflow
- [`dev-flaky`](https://crates.io/crates/dev-flaky) &mdash; flaky-test detection
- [`dev-mutate`](https://crates.io/crates/dev-mutate) &mdash; mutation testing

## Status

`v0.9.x` is the pre-1.0 stabilization line. APIs are expected to be
near-final; minor adjustments may still happen ahead of `1.0`. The
statistic definitions (`ops_per_sec`, `thread_time_cv`) are pinned
and will not change.

## Minimum supported Rust version

`1.85` — pinned in `Cargo.toml` via `rust-version` and verified by
the MSRV job in CI. (Bumped from 1.75 to align with the suite's
shared MSRV after sibling crates picked up dependencies that require
`edition2024`.)

## License

Apache-2.0. See [LICENSE](LICENSE).




<!-- COPYRIGHT
---------------------------------->
<div align="center">
    <br>
    <h2></h2>
    Copyright &copy; 2026 James Gober.
</div>
