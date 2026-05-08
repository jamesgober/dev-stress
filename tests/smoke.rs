use dev_report::Producer;
use dev_stress::{CompareOptions, SoakRun, StressProducer, StressRun, Workload};
use std::time::Duration;

#[derive(Clone)]
struct Noop;
impl Workload for Noop {
    fn run_once(&self) {
        std::hint::black_box(1 + 1);
    }
}

#[test]
fn smoke_single_thread() {
    let run = StressRun::new("noop").iterations(1_000).threads(1);
    let r = run.execute(&Noop);
    assert_eq!(r.iterations, 1_000);
    assert!(r.ops_per_sec() > 0.0);
}

#[test]
fn smoke_multi_thread() {
    let run = StressRun::new("noop").iterations(10_000).threads(4);
    let r = run.execute(&Noop);
    assert_eq!(r.iterations, 10_000);
    assert_eq!(r.threads, 4);
    assert_eq!(r.thread_times.len(), 4);
}

#[test]
fn smoke_check_result_integration() {
    let run = StressRun::new("noop").iterations(100).threads(2);
    let r = run.execute(&Noop);
    let c = r.into_check_result(None);
    assert!(matches!(c.verdict, dev_report::Verdict::Pass));
    assert!(c.has_tag("stress"));
}

#[test]
fn smoke_check_carries_numeric_evidence() {
    let run = StressRun::new("noop").iterations(100).threads(2);
    let r = run.execute(&Noop);
    let c = r.into_check_result(None);
    let labels: Vec<&str> = c.evidence.iter().map(|e| e.label.as_str()).collect();
    for required in &[
        "iterations",
        "threads",
        "ops_per_sec",
        "thread_time_cv",
        "total_elapsed_ms",
    ] {
        assert!(labels.contains(required), "missing evidence: {}", required);
    }
}

#[test]
fn smoke_latency_tracking_round_trip() {
    let run = StressRun::new("hot")
        .iterations(500)
        .threads(2)
        .track_latency(1);
    let r = run.execute(&Noop);
    let lat = r.latency.as_ref().expect("latency tracked");
    assert!(lat.samples_count > 0);
}

#[test]
fn smoke_soak_runs_for_duration() {
    let r = SoakRun::new("steady")
        .duration(Duration::from_millis(120))
        .checkpoint(Duration::from_millis(40))
        .threads(2)
        .execute(&Noop);
    assert!(!r.checkpoints.is_empty());
    let c = r.into_check_result(20.0);
    assert!(c.has_tag("soak"));
    assert!(c.has_tag("stress"));
}

#[test]
fn smoke_producer_emits_report() {
    let producer = StressProducer::new(
        || {
            StressRun::new("noop")
                .iterations(50)
                .threads(1)
                .execute(&Noop)
        },
        "0.1.0",
        CompareOptions::default(),
    );
    let report = producer.produce();
    assert_eq!(report.checks.len(), 1);
    assert_eq!(report.producer.as_deref(), Some("dev-stress"));
}
