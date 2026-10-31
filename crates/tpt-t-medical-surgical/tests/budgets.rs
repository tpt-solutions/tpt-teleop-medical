//! Host-measured timing budgets for the surgical filters (spec.txt §4.3, §4.5).
//!
//! These check the per-call cost on the machine that runs them. They do not
//! certify the real-time target. The tests are ignored by default. Run them
//! with `cargo test --release -p tpt-t-medical-surgical --test budgets -- --ignored`.

use std::time::{Duration, Instant};

use tpt_t_medical_surgical::notch::NotchFilter;
use tpt_t_medical_surgical::sdf::SdfGrid;

const CALLS: usize = 200_000;

/// Returns the 99th-percentile duration of `calls` runs of `op`.
fn p99(calls: usize, mut op: impl FnMut(usize)) -> Duration {
    let mut samples = Vec::with_capacity(calls);
    for i in 0..calls {
        let start = Instant::now();
        op(i);
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    samples[calls * 99 / 100]
}

#[test]
#[ignore = "timing budget; run with --release --ignored"]
fn sdf_lookup_is_under_20_microseconds() {
    // A 64^3 grid, about 1 MB of f32, looked up at varying points inside it.
    let dims = [64_u32; 3];
    let values: Vec<f32> = (0..64 * 64 * 64)
        .map(|i| (i % 97) as f32 * 1.0e-4)
        .collect();
    let grid = SdfGrid::new([0.0; 3], 1.0e-3, dims, values).expect("valid grid");
    let span = 1.0e-3 * 63.0;
    let cost = p99(CALLS, |i| {
        let t = (i % 1000) as f64 / 1000.0;
        let p = [t * span, (1.0 - t) * span, 0.5 * span];
        assert!(grid.distance(p).is_some());
    });
    println!("SDF lookup p99: {cost:?}");
    assert!(cost < Duration::from_micros(20), "p99 {cost:?}");
}

#[test]
#[ignore = "timing budget; run with --release --ignored"]
fn notch_sample_is_under_5_microseconds() {
    let mut filter = NotchFilter::new(10.0, 1_000.0, 4.0).expect("valid notch");
    let cost = p99(CALLS, |i| {
        let x = ((i % 360) as f64).to_radians().sin();
        std::hint::black_box(filter.process(x));
    });
    println!("notch sample p99: {cost:?}");
    assert!(cost < Duration::from_micros(5), "p99 {cost:?}");
}
