# Changelog

## [Unreleased]

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
