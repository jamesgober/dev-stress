# Changelog

## [Unreleased]

## [0.9.5] - 2026-10-09

Percentile, soak and CPU-time fixes from a review pass, plus the MSRV
rollback to Rust 1.75. The rollback follows `dev-fixtures` 0.9.5
swapping `tempfile` for `mod-tempdir` 1.0, which removed the
`getrandom 0.4.2 -> edition2024` chain that held the dev-* collection
at 1.85.

### Added

- `VERSION` constant with the crate version as compiled, so tools that
  bundle this crate can report what is actually linked.

### Fixed

- Latency percentiles were one rank too high. For samples 1..=100,
  p95 was 96 and p99 was 100 (the maximum), and the median of
  `[10, 20]` was 20. They now use the nearest-rank method with integer
  math, so every reported value is a real sample. Expect slightly lower
  p50/p95/p99 values than 0.9.4 reported for the same samples.
- Soak checkpoints only saw a worker's iterations in batches of 1,024,
  so slow workloads showed 0 ops per window and the degradation check
  meant nothing. Each worker now publishes its count every iteration
  on its own cache line. Window throughput uses the measured window
  length instead of the planned one, a zero checkpoint interval no
  longer spins, and a very large duration no longer panics on
  `Instant` overflow.
- `target_ops_per_sec(...)` followed by `threads(n)` kept a per-thread
  rate computed for one thread, so the run went n times too fast. A
  NaN or tiny rate panicked in the pacing math; a non-finite rate now
  disables the cap.
- A panicking workload left the other workers running and replaced the
  panic message with a generic one. Every worker is now joined first
  and the original panic is re-raised.
- `SystemStats::cpu_time` reported sysinfo's `run_time()`, which is
  wall-clock seconds since process start, not CPU time. It is now
  accumulated from CPU usage between samples, counted from when the
  sampler was created. Building a `SystemSampler` no longer loads every
  process on the system, and the RSS delta can no longer overflow.

### Changed

- `rust-version` lowered from `1.85` to `1.75`. CI's MSRV job now
  builds on 1.75 against an MSRV-compatible lockfile; it was still
  pinned to 1.85.

### Documentation

- The sampler reports RSS at sample time, not a peak; docs said "peak".
- The percentile method is documented, and the `SystemSampler`
  doctest now compiles and runs.
- README: version snippet and MSRV section corrected.

[0.9.5]: https://github.com/jamesgober/dev-stress/releases/tag/v0.9.5

## [0.9.4] - 2026-05-12

Documentation and SEO pass. No code changes.

### Changed

- README header standardized to match the collection-wide template: Rust logo image, MSRV badge between CI and docs.rs, copyright block at bottom.
- Tagline rewritten to lead with the developer outcome (stress + soak workloads, latency percentiles, verdict at the end).
- `## The dev-* collection` section added with the full 14-crate map.
- `Cargo.toml` description rewritten: enumerates concurrent workloads, latency percentiles, throughput-collapse detection.
- `Cargo.toml` keywords retuned: dropped `verification` and `ai-tools`, added `soak` and `latency` for crates.io search.

### Added

- "Part of the `dev-*` verification collection" block on the README, under the intro, linking the umbrella `dev-tools` crate.

[0.9.4]: https://github.com/jamesgober/dev-stress/releases/tag/v0.9.4

## [0.9.3] - 2026-05-12

### Added

- `examples/workload.rs` — runnable demonstration of the `Workload` trait + `StressRun::execute` flow on multiple OS threads, printing `ops_per_sec`, `total_elapsed`, and `thread_time_cv` from the resulting `StressResult`.

### Changed

- CI: `actions/checkout` bumped from `v4` to `v5` (removes Node 20 deprecation warnings).

[0.9.3]: https://github.com/jamesgober/dev-stress/releases/tag/v0.9.3

## [0.9.2] - 2026-05-10

### Added

- `StressRun::target_ops_per_sec(rate)` — cap workload at approximately `rate` operations per second across all threads. Implemented as deadline-based per-iteration sleep; precision varies by OS but reliably slows below the unbounded ceiling.
- `StressRun::target_ops_per_sec_per_thread()` accessor returning the configured per-thread rate, if any.

### Notes

- Sleep granularity on different OSes means very high target rates (>10k ops/sec/thread) may not be achievable; the limiter never speeds the workload up, only slows it down.

[0.9.2]: https://github.com/jamesgober/dev-stress/releases/tag/v0.9.2

## [0.9.1] - 2026-05-09

### Fixed

- Broken intra-doc link `[`system`]` in the crate-level docstring would warn under `cargo doc` when the `system-stats` feature is disabled. The link is now a plain code span.

[0.9.1]: https://github.com/jamesgober/dev-stress/releases/tag/v0.9.1

## [0.9.0] - 2026-05-08

### Added

#### Adoption of dev-report 0.9

- Bumped `dev-report` dep to `0.9`.
- `into_check_result` and the new `compare_with_options` now emit `CheckResult`s tagged `stress` and carrying numeric `Evidence` for `iterations`, `threads`, `ops_per_sec`, `thread_time_cv`, `total_elapsed_ms`. Latency percentiles and baselines add their own labeled evidence.
- Regression checks additionally carry the `regression` tag.

#### Latency percentiles (v0.2 milestone)

- New `LatencyTracker` for thread-local sampling at a configurable rate.
- `LatencyStats { p50, p95, p99, samples_count }` computed losslessly.
- `StressRun::track_latency(rate)` opts into per-op tracking.
- `StressResult::latency: Option<LatencyStats>`.
- `CompareOptions::baseline_p99` / `p99_regression_pct_threshold` for tail-latency regression detection.

#### Soak tests (v0.3 milestone)

- New `SoakRun` builder bounded by `total_duration` + `checkpoint_interval`.
- `SoakCheckpoint` records `at_offset`, `window_iters`, `window_duration`, `ops_per_sec` per window.
- `SoakResult::checkpoint_ops_cv` for stability across windows.
- `SoakResult::into_check_result(degradation_pct_threshold)` flags degradation between first-half and second-half mean ops/sec. Tagged `stress` + `soak`.

#### System stats (v0.4 + v0.5 milestones, opt-in)

- `system-stats` feature flag (off by default; pulls `sysinfo`).
- `SystemSampler` for repeated RSS + CPU-time captures of the current process.
- `SystemStats::compare(name, before, after, peak_rss_threshold)` returns a `CheckResult` tagged `stress` + `system`.

#### Producer integration

- `StressProducer<F>` adapter implementing `dev_report::Producer`.
- `StressResult::into_report(version, &CompareOptions)` shortcut.
- `CompareOptions` struct configuring `baseline_ops_per_sec`, `ops_drop_pct_threshold`, `baseline_p99`, `p99_regression_pct_threshold`.

#### Builder ergonomics

- `StressRun::iterations_planned` / `threads_planned` accessors.
- `StressRun::track_latency(rate)`.
- `SoakRun::duration` / `checkpoint` / `threads` / `track_latency`.

### Documentation

- All public items have rustdoc with at least one example.
- REPS.md expanded: §4 (latency percentiles definitions), §5 (verdict semantics + required evidence list), §6 (soak tests), §7 (system stats feature), §8 (producer integration).

[0.9.0]: https://github.com/jamesgober/dev-stress/releases/tag/v0.9.0

## [0.1.0] - 2026-05-07

### Added

- Initial crate skeleton.
- `Workload` trait with concurrency-safe `run_once`.
- `StressRun` builder with iterations + threads configuration.
- `StressResult` with `ops_per_sec` and `thread_time_cv` (coefficient
  of variation across thread elapsed times).
- `into_check_result` integration with `dev-report`.
- Smoke tests covering basic execution and verdict integration.

### Note

Name-claim release. Real load patterns (latency percentiles per-op,
soak tests, memory pressure) land in `0.2.x` and beyond.

[Unreleased]: https://github.com/jamesgober/dev-stress/compare/v0.9.3...HEAD
[0.1.0]: https://github.com/jamesgober/dev-stress/releases/tag/v0.1.0
