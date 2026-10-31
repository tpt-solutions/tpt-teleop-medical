//! Host-measured timing budgets for the safety path (spec.txt §4.1, §5.5).
//!
//! These check the per-call cost on the machine that runs them. They do not
//! certify the real-time target: that needs isolated cores and an RT kernel,
//! which this crate does not have. The tests are ignored by default. Run them
//! with `cargo test --release -p tpt-t-medical-safety --test budgets -- --ignored`.

use std::time::{Duration, Instant};

use tpt_t_medical_safety::{Decision, EStopSource, Envelope, Limits, Monitor, Sample, Tmr};

const CALLS: usize = 20_000;

fn limits() -> Limits {
    let envelope = Envelope::new([-0.1; 3], [0.1; 3]).expect("valid envelope");
    Limits::new(envelope, 5.0, 10_000).expect("valid limits")
}

fn sample(now_us: u64) -> Sample {
    Sample {
        now_us,
        tool_position_m: [0.0, 0.0, 0.0],
        tool_force_n: [0.1, 0.0, 0.0],
    }
}

/// Returns the 99th-percentile duration of `calls` runs of `op`.
fn p99(calls: usize, mut op: impl FnMut(u64)) -> Duration {
    let mut samples = Vec::with_capacity(calls);
    for i in 0..calls {
        let start = Instant::now();
        op(i as u64);
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    samples[calls * 99 / 100]
}

#[test]
#[ignore = "timing budget; run with --release --ignored"]
fn tmr_tick_is_under_100_microseconds() {
    let mut tmr = Tmr::new(limits());
    let cost = p99(CALLS, |i| {
        assert_eq!(tmr.evaluate(sample(i * 100)), Decision::Run);
    });
    println!("TMR tick p99: {cost:?}");
    assert!(cost < Duration::from_micros(100), "p99 {cost:?}");
}

#[test]
#[ignore = "timing budget; run with --release --ignored"]
fn tmr_disagreement_response_is_under_100_microseconds() {
    // Each call uses a fresh system where one copy has already been stopped,
    // so the vote disagrees and all three copies latch.
    let mut samples = Vec::with_capacity(CALLS);
    for i in 0..CALLS {
        let mut faulty = Monitor::new(limits());
        faulty.emergency_stop(EStopSource::Voice);
        let mut tmr = Tmr::from_monitors([faulty, Monitor::new(limits()), Monitor::new(limits())]);
        let start = Instant::now();
        let decision = tmr.evaluate(sample(i as u64 * 100));
        samples.push(start.elapsed());
        assert!(matches!(decision, Decision::Stop(_)));
    }
    samples.sort_unstable();
    let cost = samples[CALLS * 99 / 100];
    println!("TMR disagreement p99: {cost:?}");
    assert!(cost < Duration::from_micros(100), "p99 {cost:?}");
}

#[test]
#[ignore = "timing budget; run with --release --ignored"]
fn emergency_stop_takes_effect_on_the_next_tick_within_10_milliseconds() {
    // The stop path is a latch plus one evaluation. Its cost is far inside the
    // 10 ms budget; the wall-clock hardware path is not modelled here.
    let mut monitor = Monitor::new(limits());
    let start = Instant::now();
    monitor.emergency_stop(EStopSource::FootPedal);
    let decision = monitor.evaluate(sample(0));
    let cost = start.elapsed();
    println!("E-stop latch-to-decision: {cost:?}");
    assert_eq!(
        decision,
        Decision::Stop(tpt_t_medical_safety::StopReason::EStop(
            EStopSource::FootPedal
        ))
    );
    assert!(cost < Duration::from_millis(10), "{cost:?}");
}
