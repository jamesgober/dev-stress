use dev_stress::{StressRun, Workload};

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
}
