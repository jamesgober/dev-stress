# Changelog

## [Unreleased]

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

[Unreleased]: https://github.com/jamesgober/dev-stress/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jamesgober/dev-stress/releases/tag/v0.1.0
