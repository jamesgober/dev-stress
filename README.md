<h1 align="center">
    <strong>dev-stress</strong>
    <br>
    <sup><sub>HIGH-LOAD STRESS TESTING FOR RUST</sub></sup>
</h1>

<p align="center">
    <a href="https://crates.io/crates/dev-stress"><img alt="crates.io" src="https://img.shields.io/crates/v/dev-stress.svg"></a>
    <a href="https://docs.rs/dev-stress"><img alt="docs.rs" src="https://docs.rs/dev-stress/badge.svg"></a>
    <a href="https://github.com/jamesgober/dev-stress/blob/main/LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"></a>
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
dev-stress = "0.1"
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

let check = result.into_check_result(None);
```

## Thread-time CV

The coefficient of variation across thread elapsed times is the
clearest signal that a workload is contention-bound:

- CV near 0: threads finished at nearly the same time. Healthy.
- CV > 0.2: significant variance. Some threads were waiting on locks
  or contended resources.
- CV > 0.5: severe contention. Investigate.

## What's planned

- Latency percentile tracking (p50/p95/p99) per operation, not just
  per thread.
- Long-running soak tests with periodic checkpoint reports.
- Memory pressure tracking.
- CPU saturation detection.

## License

Apache-2.0. See [LICENSE](LICENSE).
