//! Two-site telesurgery: the surgeon's console and the patient's arm, joined by
//! two network paths. Commands travel one way and sensor feedback the other. Each
//! path is modelled by the simulator's delay link. The adapter must keep moving
//! on a degraded link, and latch when either path is lost.

// Exact zero torque is the expected halt value, so exact comparison is intended.
#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use tpt_t_medical_edge::Model;
use tpt_t_medical_edge::control::Friction;
use tpt_t_medical_safety::Envelope;
use tpt_t_medical_sim::link::Link;
use tpt_t_medical_surgical::kinematics::Arm;
use tpt_t_medical_surgical::{MotionScaler, SdfGrid};
use tpt_t_medical_teleop::link::LinkState;
use tpt_t_medical_teleop::{Actuators, Adapter, Config, JointReading};

const MS: u64 = 1_000;
const HOME: [f64; 7] = [0.0; 7];
const TOOL_HOME: [f64; 3] = [0.088, 0.0, 0.926];
const SEVEN: usize = 7;

#[derive(Debug, Default)]
struct Recorder;

impl Actuators for Recorder {
    fn apply(&mut self, _torque_nm: [f64; 7]) {}
}

/// A message's payload carries its send time, so the receiver can measure the
/// one-way delay. The simulator link moves opaque joint-sized arrays.
fn payload(sent_us: u64) -> [f64; SEVEN] {
    let mut p = [0.0; SEVEN];
    p[0] = sent_us as f64;
    p
}

fn sent_of(p: [f64; SEVEN]) -> u64 {
    p[0] as u64
}

/// No structure near the arm, so the guard never interferes with this test.
fn far_grid() -> SdfGrid {
    SdfGrid::new([5.0; 3], 0.01, [2, 2, 2], vec![1.0; 8]).expect("grid")
}

fn adapter() -> Adapter<Recorder> {
    let envelope = Envelope::new(
        [
            TOOL_HOME[0] - 0.05,
            TOOL_HOME[1] - 0.05,
            TOOL_HOME[2] - 0.05,
        ],
        [
            TOOL_HOME[0] + 0.05,
            TOOL_HOME[1] + 0.05,
            TOOL_HOME[2] + 0.05,
        ],
    )
    .expect("envelope");
    let masses = [4.0, 3.0, 3.0, 2.5, 2.0, 1.5, 1.0];
    let cfg = Config {
        model: Model {
            arm: Arm::reference(),
            link_masses_kg: masses,
            friction: Friction::new(0.0, 0.0, 0.0).expect("friction"),
            limits_nm: [50.0; 7],
        },
        envelope,
        max_force_n: 10.0,
        watchdog_us: 10 * MS,
        scaler: MotionScaler::new(10.0).expect("ratio"),
        step_m: 0.05,
        stiffness_n_m: 200.0,
        damping_ns_m: 5.0,
        tracking_limit_n: 20.0,
        sdf: far_grid(),
        guard_margin_m: 0.01,
        guard_max_force_n: 3.0,
        override_after_us: 200 * MS,
        retreat_m: 0.001,
        tremor_hz: 10.0,
        sample_hz: 1_000.0,
        ramp_ticks: 50,
    };
    Adapter::new(cfg, Recorder).expect("adapter")
}

/// Runs both paths for `steps` milliseconds, starting at `from_ms`.
fn run(a: &mut Adapter<Recorder>, cmd: &mut Link, feed: &mut Link, from_ms: u64, steps: u64) {
    for ms in from_ms..from_ms + steps {
        let now = ms * MS;
        a.update_joints(JointReading {
            at_us: now,
            q: HOME,
            qdot: [0.0; 7],
        });
        cmd.send(ms, payload(now));
        feed.send(ms, payload(now));
        if let Some(p) = cmd.receive(ms) {
            a.observe_link(sent_of(p), now);
        }
        if let Some(p) = feed.receive(ms) {
            a.observe_feed_link(sent_of(p), now);
        }
        a.tick(now);
    }
}

#[test]
fn a_nominal_two_site_session_stays_nominal_and_running() {
    let mut a = adapter();
    let mut cmd = Link::new(4, 2).expect("link");
    let mut feed = Link::new(4, 2).expect("link");
    run(&mut a, &mut cmd, &mut feed, 0, 300);
    assert_eq!(a.link_state(300 * MS), LinkState::Nominal);
    assert!(!a.is_latched());
}

#[test]
fn a_degraded_command_path_warns_but_keeps_the_arm_running() {
    let mut a = adapter();
    let mut cmd = Link::new(30, 2).expect("link");
    let mut feed = Link::new(4, 2).expect("link");
    run(&mut a, &mut cmd, &mut feed, 0, 300);
    assert_eq!(a.link_state(300 * MS), LinkState::Degraded);
    assert!(
        !a.is_latched(),
        "a degraded command path must not stop the arm"
    );
}

#[test]
fn the_two_site_state_is_the_worse_of_the_two_paths() {
    let mut a = adapter();
    let mut cmd = Link::new(4, 2).expect("link");
    let mut feed = Link::new(30, 2).expect("link");
    run(&mut a, &mut cmd, &mut feed, 0, 300);
    assert_eq!(a.link_state(300 * MS), LinkState::Degraded);
    assert!(!a.is_latched());
}

#[test]
fn a_lost_command_path_latches_the_arm() {
    let mut a = adapter();
    let mut cmd = Link::new(80, 0).expect("link");
    let mut feed = Link::new(4, 2).expect("link");
    run(&mut a, &mut cmd, &mut feed, 0, 200);
    assert!(a.is_latched(), "a command 80 ms late is beyond the horizon");
    assert_eq!(a.last_torque(), [0.0; 7]);
}

#[test]
fn a_lost_feed_path_latches_the_arm_too() {
    // The surgeon cannot see or feel the arm without the feed, so losing it stops
    // the arm just as losing the command path does.
    let mut a = adapter();
    let mut cmd = Link::new(4, 2).expect("link");
    let mut feed = Link::new(60, 0).expect("link");
    run(&mut a, &mut cmd, &mut feed, 0, 200);
    assert!(a.is_latched());
    assert_eq!(a.last_torque(), [0.0; 7]);
}
