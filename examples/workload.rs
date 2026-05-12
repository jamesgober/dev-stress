//! Define a `Workload`, run it under multi-threaded stress, print results.
//!
//! ```text
//! cargo run --example workload --release
//! ```
//!
//! Demonstrates the `Workload` trait + `StressRun::execute` flow. The
//! workload is intentionally trivial; replace `run_once` with the code
//! under test to measure real throughput, contention, and per-thread CV.

use dev_stress::{StressRun, Workload};

#[derive(Clone)]
struct TrivialAdd;

impl Workload for TrivialAdd {
    fn run_once(&self) {
        let mut acc: u64 = 0;
        for i in 0..100 {
            acc = std::hint::black_box(acc.wrapping_add(i));
        }
        std::hint::black_box(acc);
    }
}

fn main() {
    let run = StressRun::new("trivial_add").iterations(100_000).threads(4);

    println!(
        "configured: {} iterations across {} threads",
        run.iterations_planned(),
        run.threads_planned()
    );

    let result = run.execute(&TrivialAdd);

    println!("name:           {}", result.name);
    println!("iterations:     {}", result.iterations);
    println!("threads:        {}", result.threads);
    println!("total_elapsed:  {:?}", result.total_elapsed);
    println!("ops_per_sec:    {:.0}", result.ops_per_sec());
    println!("thread_time_cv: {:.4}", result.thread_time_cv());
}
