<h1 align="center">
    <strong>dev-stress</strong>
    <br>
    <sup><sub>HIGH-LOAD STRESS TESTING FOR RUST</sub></sup>
</h1>

<p align="center">
    <a href="https://crates.io/crates/dev-stress"><img alt="crates.io" src="https://img.shields.io/crates/v/dev-stress.svg"></a>
    <a href="https://crates.io/crates/dev-stress"><img alt="downloads" src="https://img.shields.io/crates/d/dev-stress.svg"></a>
    <a href="https://github.com/jamesgober/dev-stress/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/jamesgober/dev-stress/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://docs.rs/dev-stress"><img alt="docs.rs" src="https://docs.rs/dev-stress/badge.svg"></a>
</p>

<p align="center">
    Concurrency, volume, saturation under pressure.<br>
    Part of the <code>dev-*</code> verification suite.
</p>

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
dev-stress = "0.9"
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
dev-stress = { version = "0.9", features = ["system-stats"] }
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

## Status

`v0.9.x` is the pre-1.0 stabilization line. APIs are expected to be
near-final; minor adjustments may still happen ahead of `1.0`. The
statistic definitions (`ops_per_sec`, `thread_time_cv`) are pinned
and will not change.

## Minimum supported Rust version

`1.75` — pinned in `Cargo.toml` via `rust-version` and verified by
the MSRV job in CI.

## License

Apache-2.0. See [LICENSE](LICENSE).
