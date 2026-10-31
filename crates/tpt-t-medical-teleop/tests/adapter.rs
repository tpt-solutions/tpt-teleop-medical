//! Exact zeros are the expected values here, so exact comparison is intended.
#![allow(clippy::float_cmp, clippy::cast_possible_truncation)]

//! The adapter driven through `tpt-t-domain-bridge`'s `DomainSession`, which is how
//! tpt-teleop calls it.

use tpt_t_core::Mode;
use tpt_t_core::ser::ControlCommand;
use tpt_t_domain_bridge::wire::{
    FAULT_ACTUATOR, FAULT_GEOFENCE, FAULT_SENSOR, VideoFrame, VideoKind,
};
use tpt_t_domain_bridge::{
    AssistanceReason, DomainSession, DomainTeleopInterface, HandoverState, OperatorId, Result,
    TeleopLink,
};
use tpt_t_medical_edge::Model;
use tpt_t_medical_edge::control::{Friction, gravity_torque};
use tpt_t_medical_safety::Envelope;
use tpt_t_medical_surgical::kinematics::Arm;
use tpt_t_medical_surgical::{MotionScaler, SdfGrid};
use tpt_t_medical_teleop::{Actuators, Adapter, Config, JointReading};

const MASSES: [f64; 7] = [4.0, 3.0, 3.0, 2.5, 2.0, 1.5, 1.0];
const MS: u64 = 1_000;
const WATCHDOG_US: u64 = 10 * MS;
const SURGEON: OperatorId = OperatorId::new([9; 16]);

/// Records every torque set the adapter applies.
#[derive(Debug, Default)]
struct Recorder {
    last: [f64; 7],
    count: usize,
}

impl Actuators for Recorder {
    fn apply(&mut self, torque_nm: [f64; 7]) {
        self.last = torque_nm;
        self.count += 1;
    }
}

/// A signed-distance grid over the workspace, with a ball at `center`.
fn workspace_grid(center: [f64; 3], radius: f64) -> SdfGrid {
    let spacing = 0.005;
    let dims = [41_u32, 41, 31];
    let origin = [0.0, -0.1, 0.85];
    let mut values = Vec::new();
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let p = [
                    origin[0] + f64::from(x) * spacing,
                    origin[1] + f64::from(y) * spacing,
                    origin[2] + f64::from(z) * spacing,
                ];
                let d: f64 = (0..3)
                    .map(|i| (p[i] - center[i]).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    - radius;
                values.push(d as f32);
            }
        }
    }
    SdfGrid::new(origin, spacing, dims, values).expect("grid")
}

/// Home pose of the reference arm: the tool is near (0.088, 0, 0.926).
const HOME: [f64; 7] = [0.0; 7];
const TOOL_HOME: [f64; 3] = [0.088, 0.0, 0.926];

fn config(sdf: SdfGrid) -> Config {
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
    Config {
        model: Model {
            arm: Arm::reference(),
            link_masses_kg: MASSES,
            friction: Friction::new(0.0, 0.0, 0.0).expect("friction"),
            limits_nm: [50.0; 7],
        },
        envelope,
        max_force_n: 10.0,
        watchdog_us: WATCHDOG_US,
        scaler: MotionScaler::new(10.0).expect("ratio"),
        step_m: 0.05,
        stiffness_n_m: 200.0,
        damping_ns_m: 5.0,
        tracking_limit_n: 20.0,
        sdf,
        guard_margin_m: 0.01,
        guard_max_force_n: 3.0,
        override_after_us: 200 * MS,
        retreat_m: 0.001,
        tremor_hz: 10.0,
        sample_hz: 1_000.0,
        ramp_ticks: 50,
    }
}

/// Far from the workspace, so it never interferes.
fn far_grid() -> SdfGrid {
    workspace_grid([0.19, 0.0, 0.93], 0.01)
}

fn adapter(sdf: SdfGrid) -> Adapter<Recorder> {
    Adapter::new(config(sdf), Recorder::default()).expect("adapter")
}

fn reading(at_us: u64, q: [f64; 7]) -> JointReading {
    JointReading {
        at_us,
        q,
        qdot: [0.0; 7],
    }
}

fn gravity_only() -> [f64; 7] {
    gravity_torque(&Arm::reference(), HOME, MASSES).expect("gravity")
}

fn command(at_us: u64, throttle: f32, lateral_x: f32, lateral_y: f32) -> ControlCommand {
    let mut cmd = ControlCommand::zeroed(Mode::FullTeleop);
    cmd.timestamp_ns = at_us * 1_000;
    cmd.axes[3] = throttle;
    cmd.axes[4] = lateral_x;
    cmd.axes[5] = lateral_y;
    cmd
}

fn close(a: [f64; 7], b: [f64; 7]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

/// A link that accepts every assistance request.
struct Link;

impl TeleopLink for Link {
    fn request_teleop_assistance(&mut self, _: AssistanceReason) -> Result<()> {
        Ok(())
    }
}

#[test]
fn a_new_adapter_holds_the_arm_against_gravity() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    let torque = a.tick(MS);
    assert!(close(torque, gravity_only()), "{torque:?}");
}

#[test]
fn commands_are_refused_before_engage() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    assert!(a.on_control_command(command(MS, 1.0, 0.0, 0.0)).is_err());
}

#[test]
fn engage_starts_the_target_at_the_tool_so_nothing_jumps() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    let target = a.target().expect("target");
    assert!(
        target
            .iter()
            .zip(TOOL_HOME)
            .all(|(t, h)| (t - h).abs() < 1e-9),
        "{target:?}"
    );
    assert!(close(a.tick(MS), gravity_only()));
}

#[test]
fn commands_move_the_target_by_scaled_and_filtered_steps() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    let start = a.target().expect("target");
    for k in 1..=5 {
        a.on_control_command(command(k * MS, 0.0, 1.0, 0.0))
            .expect("command");
    }
    let end = a.target().expect("target");
    // 5 steps of 5 cm master motion at 10:1 is 25 mm of tool motion along x, less
    // whatever the notch takes out of the first few samples.
    let moved = end[0] - start[0];
    assert!(moved > 0.010 && moved < 0.026, "moved {moved} m");
    assert!((end[1] - start[1]).abs() < 1e-9);
}

#[test]
fn a_command_into_a_structure_is_blocked_and_the_target_holds() {
    // A ball whose surface is 2 mm beyond the tool, in the +x direction.
    let mut a = adapter(workspace_grid([0.10, 0.0, TOOL_HOME[2]], 0.01));
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    let before = a.target().expect("target");
    a.on_control_command(command(MS, 0.0, 1.0, 0.0))
        .expect("blocked is not an error");
    assert_eq!(a.target(), Some(before));
}

#[test]
fn staying_inside_a_structure_overrides_the_master_and_retreats() {
    // The tool sits inside a ball. Its centre is 2 mm from the tool.
    let mut a = adapter(workspace_grid(
        [TOOL_HOME[0] - 0.012, 0.0, TOOL_HOME[2]],
        0.02,
    ));
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    let tool = a.tool_position().expect("tool");
    for k in 1..=250_u64 {
        a.on_control_command(command(k * MS, 0.0, -1.0, 0.0))
            .expect("command");
    }
    let target = a.target().expect("target");
    assert!(
        target[0] > tool[0],
        "retreat should be outward (+x): {target:?} vs {tool:?}"
    );
}

#[test]
fn a_tool_outside_the_envelope_latches_a_geofence_stop() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    // Joint 1 at 0.9 rad swings the tool about 7 cm to the side of its box.
    a.update_joints(reading(2 * MS, [0.9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
    assert_eq!(a.tick(2 * MS), [0.0; 7]);
    assert!(a.is_latched());
    assert_ne!(a.get_domain_state().fault_flags & FAULT_GEOFENCE, 0);
}

#[test]
fn excessive_force_latches_an_actuator_stop() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.update_force([20.0, 0.0, 0.0]);
    assert_eq!(a.tick(MS), [0.0; 7]);
    assert!(a.is_latched());
    assert_ne!(a.get_domain_state().fault_flags & FAULT_ACTUATOR, 0);
}

#[test]
fn a_silent_sensor_past_the_watchdog_is_a_sensor_fault() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    assert_eq!(a.tick(WATCHDOG_US + 1), [0.0; 7]);
    assert_ne!(a.get_domain_state().fault_flags & FAULT_SENSOR, 0);
}

#[test]
fn a_latched_stop_refuses_commands_until_a_human_engages_again() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    a.update_force([20.0, 0.0, 0.0]);
    a.tick(MS);
    assert!(
        a.on_control_command(command(2 * MS, 1.0, 0.0, 0.0))
            .is_err()
    );
    a.update_force([0.0; 3]);
    a.on_teleop_engage(SURGEON)
        .expect("re-engage clears the latch");
    assert!(!a.is_latched());
    assert_eq!(a.get_domain_state().fault_flags, 0);
}

#[test]
fn disengage_ramps_the_tracking_force_away_and_keeps_the_arm_held() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    a.on_control_command(command(MS, 0.0, 1.0, 0.0))
        .expect("command");
    a.on_teleop_disengage().expect("disengage");
    let mut t = 2 * MS;
    for _ in 0..60 {
        a.update_joints(reading(t, HOME));
        a.tick(t);
        t += MS;
    }
    assert!(
        a.target().is_none(),
        "ramp should finish and clear the target"
    );
    assert!(
        close(a.last_torque(), gravity_only()),
        "gravity must stay on"
    );
}

#[test]
fn the_sensor_feed_carries_frames_telemetry_and_haptics() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.set_battery(63.0);
    a.push_frame(VideoFrame {
        timestamp_us: 1,
        kind: VideoKind::Rgb,
        width: 1,
        height: 1,
        data: vec![1, 2, 3],
    });
    a.update_force([0.5, -0.25, 0.0]);
    let feed = a.get_sensor_feed();
    assert_eq!(feed.video_frames.len(), 1);
    assert!((feed.telemetry.battery_pct - 63.0).abs() < 1e-6);
    assert_eq!(
        feed.telemetry.pose.position.map(|v| (v * 1000.0).round()),
        [88.0, 0.0, 926.0]
    );
    assert!((feed.haptic_data.force_torque[0] - 0.5).abs() < 1e-6);
    assert!(
        a.get_sensor_feed().video_frames.is_empty(),
        "frames go out once"
    );
}

#[test]
fn the_domain_session_drives_a_full_handover() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    let mut session = DomainSession::new(a);
    session
        .request_assistance(&mut Link, AssistanceReason::Degraded)
        .expect("request");
    assert_eq!(session.state(), HandoverState::RequestingTeleop);
    session.engage(SURGEON).expect("engage");
    session
        .command(command(MS, 0.0, 1.0, 0.0))
        .expect("command");
    session.disengage().expect("disengage");
    assert_eq!(session.state(), HandoverState::ReturningToAutonomy);
    session.complete_handover().expect("handover");
    assert_eq!(session.state(), HandoverState::Autonomous);
}

#[test]
fn an_emergency_stop_command_latches_zero_torque_and_is_acknowledged_by_a_human() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    let mut session = DomainSession::new(a);
    session
        .request_assistance(&mut Link, AssistanceReason::Other)
        .expect("request");
    session.engage(SURGEON).expect("engage");
    session
        .command(ControlCommand::zeroed(Mode::EmergencyStop))
        .expect("stop is never refused");
    assert_eq!(session.state(), HandoverState::EmergencyStop);
    assert!(session.adapter().is_latched());
    assert_eq!(session.adapter().last_torque(), [0.0; 7]);
    session.acknowledge_emergency_stop().expect("acknowledge");
    session.engage(SURGEON).expect("engage again");
    assert!(!session.adapter().is_latched());
}

#[test]
fn autonomous_motion_moves_the_target_by_exactly_the_checked_displacement() {
    // The displacement is already tool motion, so the notch and the 10:1 scaler
    // must not touch it. A 1 mm step lands as 1 mm.
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    let start = a.target().expect("target");
    a.on_autonomous_motion([0.001, 0.0, 0.0])
        .expect("autonomous step");
    let end = a.target().expect("target");
    assert!((end[0] - start[0] - 0.001).abs() < 1e-12, "{end:?}");
    assert!((end[1] - start[1]).abs() < 1e-12);
    assert!((end[2] - start[2]).abs() < 1e-12);
}

#[test]
fn autonomous_motion_is_refused_before_engage_and_while_latched() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    assert!(a.on_autonomous_motion([0.001, 0.0, 0.0]).is_err());
    a.on_teleop_engage(SURGEON).expect("engage");
    // Joint 1 at 0.9 rad swings the tool out of its box, which latches a stop.
    a.update_joints(reading(2 * MS, [0.9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
    a.tick(2 * MS);
    assert!(a.is_latched());
    assert!(a.on_autonomous_motion([0.001, 0.0, 0.0]).is_err());
}

#[test]
fn autonomous_motion_into_a_structure_is_blocked_by_the_guard() {
    // A ball whose surface is 2 mm beyond the tool, in the +x direction.
    let mut a = adapter(workspace_grid([0.10, 0.0, TOOL_HOME[2]], 0.01));
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    let before = a.target().expect("target");
    a.on_autonomous_motion([0.005, 0.0, 0.0])
        .expect("blocked is not an error");
    assert_eq!(a.target(), Some(before));
}

#[test]
fn a_non_finite_autonomous_displacement_is_refused() {
    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    let before = a.target().expect("target");
    assert!(a.on_autonomous_motion([f64::NAN, 0.0, 0.0]).is_err());
    assert_eq!(a.target(), Some(before));
}

#[test]
fn an_autonomous_session_drives_the_adapter_through_the_checked_path() {
    use tpt_t_medical_ai_session::{Actor, Command, Mode, Proposal, Session, Verdict};
    use tpt_t_medical_safety::Limits;

    const SEC: u64 = 1_000 * MS;
    const SURGEON_ID: u32 = 7;

    // The session's envelope and timing match the adapter's workspace box.
    let envelope =
        Envelope::new(TOOL_HOME.map(|v| v - 0.05), TOOL_HOME.map(|v| v + 0.05)).expect("envelope");
    let limits = Limits::new(envelope, 5.0, 2 * SEC).expect("limits");
    let timing = tpt_t_medical_ai_session::Config::new(60 * SEC, 2 * SEC, SEC / 10, SEC / 2, 0.005)
        .expect("config");
    let mut session = Session::new(SURGEON_ID, limits, timing).expect("session");

    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");

    session
        .step(Actor::Surgeon(SURGEON_ID), Command::Reset, 0)
        .expect("reset");
    session
        .step(Actor::Ai, Command::RequestAssistance, 0)
        .expect("request");
    session
        .step(
            Actor::Surgeon(SURGEON_ID),
            Command::SetMode(Mode::FullAutonomy),
            0,
        )
        .expect("mode");
    session
        .step(Actor::Surgeon(SURGEON_ID), Command::Engage, 0)
        .expect("engage");

    let start = a.target().expect("target");
    let mut commanded = 0;
    // One second of control at 1 kHz, with a 0.5 mm proposal every 200 ms.
    for k in 1..=1_000_u64 {
        let t = k * MS;
        a.update_joints(reading(t, HOME));
        a.tick(t);
        if k % 200 == 0 {
            let proposal = Proposal {
                delta_m: [0.0005, 0.0, 0.0],
                tool_position_m: a.tool_position().expect("tool"),
                tool_force_n: [0.0; 3],
                observed_delta_m: [0.0; 3],
            };
            let verdict = session
                .step(Actor::Ai, Command::Propose(proposal), t)
                .expect("proposal");
            // Only a commanded motion moves the arm. The count below catches any other verdict.
            if let Verdict::Commanded { delta_m } = verdict {
                a.on_autonomous_motion(delta_m).expect("applied");
                commanded += 1;
            }
        }
    }

    assert_eq!(commanded, 5);
    assert!(!a.is_latched());
    let end = a.target().expect("target");
    assert!((end[0] - start[0] - 0.0025).abs() < 1e-9, "{end:?}");
}

#[test]
fn a_command_sent_over_the_transport_moves_the_arm_target() {
    use std::thread;
    use std::time::{Duration, Instant};
    use tpt_t_link::mux::UdpMux;
    use tpt_t_medical_teleop::transport::{Arm, Operator};
    use tpt_t_sec::{DeviceIdentity, Role, TrustStore};

    let timeout = Duration::from_secs(2);
    let base = Instant::now();
    let elapsed_ns = || u64::try_from(base.elapsed().as_nanos()).unwrap_or(u64::MAX);

    let operator_id = DeviceIdentity::generate(1, Role::Operator).expect("identity");
    let arm_id = DeviceIdentity::generate(2, Role::Admin).expect("identity");
    let mut operator_trust = TrustStore::new();
    operator_trust.enroll(2, arm_id.public_key(), Role::Admin);
    let mut arm_trust = TrustStore::new();
    arm_trust.enroll(1, operator_id.public_key(), Role::Operator);

    let arm_command = UdpMux::bind_loopback().expect("bind");
    let arm_feed = UdpMux::bind_loopback().expect("bind");
    let command_addr = arm_command.local_addr().expect("addr");
    let feed_addr = arm_feed.local_addr().expect("addr");

    let (mut operator, mut arm_end) = thread::scope(|s| {
        let accepting = s.spawn(move || {
            Arm::accept(
                &arm_id,
                &arm_trust,
                arm_command,
                arm_feed,
                0,
                timeout,
                Duration::from_secs(1),
            )
            .expect("arm accepts")
        });
        let operator = Operator::connect(
            &operator_id,
            &operator_trust,
            2,
            command_addr,
            feed_addr,
            UdpMux::bind_loopback().expect("bind"),
            UdpMux::bind_loopback().expect("bind"),
            0,
            timeout,
        )
        .expect("operator connects");
        (operator, accepting.join().expect("arm thread"))
    });

    let mut a = adapter(far_grid());
    a.update_joints(reading(0, HOME));
    a.on_teleop_engage(SURGEON).expect("engage");
    let start = a.target().expect("target");

    // Lateral x at full scale, sent by the surgeon's side of the link.
    let mut cmd = ControlCommand::zeroed(Mode::FullTeleop);
    cmd.axes[4] = 1.0;
    operator.send_command(cmd, elapsed_ns()).expect("send");

    let deadline = Instant::now() + timeout;
    let received = loop {
        if let Some(cmd) = arm_end.recv_command().expect("recv") {
            break cmd;
        }
        assert!(Instant::now() < deadline, "no command arrived");
        thread::sleep(Duration::from_millis(1));
    };
    let received_us = elapsed_ns() / 1_000;
    a.observe_link(received.timestamp_ns / 1_000, received_us);
    a.on_control_command(received).expect("applied");

    assert!(!a.is_latched());
    let end = a.target().expect("target");
    assert!(end[0] > start[0], "{start:?} -> {end:?}");
}
