//! The surgical arm as a Domain Teleoperation Interface adapter (spec §4.6,
//! `tpt-t-domain-bridge`).
//!
//! Each operator command is turned into a target for the tool, not into a
//! torque directly:
//!
//! 1. the throttle and lateral axes of the [`ControlCommand`] become a master
//!    displacement, one step per full-scale axis,
//! 2. a 10 Hz notch removes physiological tremor from that displacement,
//! 3. the motion scaler divides it down to the tool's motion,
//! 4. the virtual-fixture guard decides whether the tool may move that way.
//!
//! Each tick the adapter pulls the tool toward its target with a spring and
//! damper, runs the result through the edge torque pipeline, and gives the
//! torques to the [`Actuators`]. The safety monitor checks the measured state
//! on every tick, and a stop from it zeroes the torque and latches.
//!
//! The axis mapping is a decision, not from the spec: roll, pitch and yaw are
//! ignored, because orientation is not part of this 3-DOF force model.

use crate::link::{LinkMonitor, LinkState};
use tpt_t_core::ser::ControlCommand;
use tpt_t_domain_bridge::{
    DomainError, DomainState, DomainTeleopInterface, HapticFeedback, OperatorId, Result,
    SensorFeed, VideoFrame,
    wire::{FAULT_ACTUATOR, FAULT_GEOFENCE, FAULT_SENSOR, Pose6DOF},
};
use tpt_t_medical_edge::control::position_jacobian;
use tpt_t_medical_edge::{Loop, Model, State};
use tpt_t_medical_safety::{Decision, Envelope, Limits, Monitor, Sample, StopReason};
use tpt_t_medical_surgical::guard::{Action, Guard};
use tpt_t_medical_surgical::{MotionScaler, NotchFilter, SdfGrid};

/// Joints in the arm.
const JOINTS: usize = 7;
/// Axes of the command that move the tool: throttle, lateral x, lateral y.
const AXIS_THROTTLE: usize = 3;
const AXIS_LATERAL_X: usize = 4;
const AXIS_LATERAL_Y: usize = 5;

/// Where the adapter sends torques. A real arm drives its motors here, and tests
/// record them.
pub trait Actuators {
    /// Applies one set of joint torques, in newton-metres.
    fn apply(&mut self, torque_nm: [f64; JOINTS]);
}

/// One reading of the arm's joints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointReading {
    /// Time of the reading, in microseconds.
    pub at_us: u64,
    /// Joint angles, in radians.
    pub q: [f64; JOINTS],
    /// Joint speeds, in radians per second.
    pub qdot: [f64; JOINTS],
}

/// Everything the adapter needs. The fields are public, so a caller can build
/// the configuration from its own calibration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Arm geometry, masses, friction and torque limits.
    pub model: Model,
    /// Workspace the tool may not leave.
    pub envelope: Envelope,
    /// Force limit for the safety monitor, in newtons.
    pub max_force_n: f64,
    /// Longest gap between joint readings before the sensor counts as lost, in
    /// microseconds.
    pub watchdog_us: u64,
    /// Master-to-tool motion scaling.
    pub scaler: MotionScaler,
    /// Master displacement for a full-scale axis, in metres, before scaling.
    pub step_m: f64,
    /// Tool spring stiffness toward the target, in newtons per metre.
    pub stiffness_n_m: f64,
    /// Tool damping, in newton-seconds per metre.
    pub damping_ns_m: f64,
    /// Largest force the tracking term may command, in newtons.
    pub tracking_limit_n: f64,
    /// The forbidden structures, as a signed-distance grid.
    pub sdf: SdfGrid,
    /// Margin around a structure where force feedback starts, in metres.
    pub guard_margin_m: f64,
    /// Feedback force at the surface, in newtons.
    pub guard_max_force_n: f64,
    /// How long the tool may stay inside before the master is overridden, in
    /// microseconds.
    pub override_after_us: u64,
    /// Retreat per override, in metres.
    pub retreat_m: f64,
    /// Tremor band to remove, as a centre frequency in hertz.
    pub tremor_hz: f64,
    /// Rate of the master input, in hertz.
    pub sample_hz: f64,
    /// Ticks over which the tracking force ramps to zero on disengage.
    pub ramp_ticks: u32,
}

/// The adapter. It owns the arm's control state and an [`Actuators`] sink.
#[derive(Debug)]
pub struct Adapter<A: Actuators> {
    cfg: Config,
    edge: Loop,
    notch: [NotchFilter; 3],
    guard: Guard,
    monitor: Monitor,
    joints: Option<JointReading>,
    measured_force_n: [f64; 3],
    feedback_n: f64,
    target_m: Option<[f64; 3]>,
    operator: Option<OperatorId>,
    ramp_left: u32,
    latched: bool,
    link: LinkMonitor,
    feed_link: LinkMonitor,
    fault_flags: u32,
    frames: Vec<VideoFrame>,
    battery_pct: f32,
    last_torque: [f64; JOINTS],
    clock_us: u64,
    actuators: A,
}

impl<A: Actuators> Adapter<A> {
    /// Builds an adapter. Returns `None` if any part of the configuration is
    /// invalid, so the adapter never starts with a partial setup.
    #[must_use]
    pub fn new(cfg: Config, actuators: A) -> Option<Self> {
        let limits = Limits::new(cfg.envelope, cfg.max_force_n, cfg.watchdog_us)?;
        let guard = Guard::new(
            cfg.guard_margin_m,
            cfg.guard_max_force_n,
            cfg.override_after_us,
            cfg.retreat_m,
        )?;
        let notch = [
            NotchFilter::new(cfg.tremor_hz, cfg.sample_hz, 1.0)?,
            NotchFilter::new(cfg.tremor_hz, cfg.sample_hz, 1.0)?,
            NotchFilter::new(cfg.tremor_hz, cfg.sample_hz, 1.0)?,
        ];
        let finite = [
            cfg.step_m,
            cfg.stiffness_n_m,
            cfg.damping_ns_m,
            cfg.tracking_limit_n,
        ]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.0);
        if !finite || cfg.ramp_ticks == 0 {
            return None;
        }
        let edge = Loop::new(cfg.model)?;
        Some(Self {
            edge,
            notch,
            guard,
            monitor: Monitor::new(limits),
            joints: None,
            measured_force_n: [0.0; 3],
            feedback_n: 0.0,
            target_m: None,
            operator: None,
            ramp_left: 0,
            latched: false,
            link: LinkMonitor::new(),
            feed_link: LinkMonitor::new(),
            fault_flags: 0,
            frames: Vec::new(),
            battery_pct: 100.0,
            last_torque: [0.0; JOINTS],
            clock_us: 0,
            actuators,
            cfg,
        })
    }

    /// Records one command's one-way delay, from the surgeon's console to the
    /// patient's arm. A lost link latches the adapter, as a sensor fault does. A
    /// degraded link does not stop motion, and the caller should warn the
    /// surgeon.
    pub fn observe_link(&mut self, sent_us: u64, received_us: u64) -> LinkState {
        let state = self.link.observe(sent_us, received_us);
        self.latch_if_lost(state)
    }

    /// Records one sensor-feed message's one-way delay, from the patient's arm to
    /// the surgeon's console. The surgeon cannot see or feel the arm without it,
    /// so a lost feed latches the adapter just as a lost command link does.
    pub fn observe_feed_link(&mut self, sent_us: u64, received_us: u64) -> LinkState {
        let state = self.feed_link.observe(sent_us, received_us);
        self.latch_if_lost(state)
    }

    fn latch_if_lost(&mut self, state: LinkState) -> LinkState {
        if state == LinkState::Lost {
            self.trip(FAULT_SENSOR);
        }
        state
    }

    /// Returns the state of the two-site link at `now_us`: the worse of the
    /// command path and the feed path.
    #[must_use]
    pub fn link_state(&self, now_us: u64) -> LinkState {
        self.link
            .state_at(now_us)
            .max(self.feed_link.state_at(now_us))
    }

    /// Returns `true` if a path that has reported is now lost to silence.
    fn silent_path_lost(&self, now_us: u64) -> bool {
        let lost = |m: &LinkMonitor| m.samples() > 0 && m.state_at(now_us) == LinkState::Lost;
        lost(&self.link) || lost(&self.feed_link)
    }

    /// Records a joint reading from the arm's sensors.
    pub fn update_joints(&mut self, reading: JointReading) {
        self.clock_us = self.clock_us.max(reading.at_us);
        self.joints = Some(reading);
    }

    /// Records the force the tool is measured to feel, in newtons.
    pub fn update_force(&mut self, force_n: [f64; 3]) {
        self.measured_force_n = force_n;
    }

    /// Queues a video frame for the operator. Frames go out with the next
    /// sensor feed.
    pub fn push_frame(&mut self, frame: VideoFrame) {
        self.frames.push(frame);
    }

    /// Sets the battery level reported to the operator, from 0 to 100.
    pub fn set_battery(&mut self, pct: f32) {
        self.battery_pct = pct.clamp(0.0, 100.0);
    }

    /// Returns the torques last applied.
    #[must_use]
    pub const fn last_torque(&self) -> [f64; JOINTS] {
        self.last_torque
    }

    /// Returns the actuator sink.
    #[must_use]
    pub const fn actuators(&self) -> &A {
        &self.actuators
    }

    /// Returns `true` while a stop is latched.
    #[must_use]
    pub const fn is_latched(&self) -> bool {
        self.latched
    }

    /// Returns the tool target the adapter is moving toward, if an operator
    /// holds the unit.
    #[must_use]
    pub const fn target(&self) -> Option<[f64; 3]> {
        self.target_m
    }

    /// Returns the tool position, if the joints are known.
    #[must_use]
    pub fn tool_position(&self) -> Option<[f64; 3]> {
        let j = self.joints?;
        self.cfg.model.arm.forward(j.q).map(|p| p.position_m)
    }

    /// Runs one control cycle, at `now_us`, and returns the torques applied.
    ///
    /// A latched or sensor-less adapter applies zero torque. A safety stop
    /// found on this tick latches the adapter and applies zero torque from then
    /// on.
    pub fn tick(&mut self, now_us: u64) -> [f64; JOINTS] {
        self.clock_us = self.clock_us.max(now_us);
        if self.latched {
            return self.hold_zero();
        }
        // Once a transport reports on a path, silence past the horizon on that path
        // is a lost link. A deployment without a transport never reports, so it is
        // not checked.
        if self.silent_path_lost(now_us) {
            self.trip(FAULT_SENSOR);
            return self.hold_zero();
        }
        let Some(j) = self.joints else {
            self.trip(FAULT_SENSOR);
            return self.hold_zero();
        };
        if now_us.saturating_sub(j.at_us) > self.cfg.watchdog_us {
            self.trip(FAULT_SENSOR);
            return self.hold_zero();
        }
        let Some(pos) = self.tool_position() else {
            self.trip(FAULT_SENSOR);
            return self.hold_zero();
        };
        if let Decision::Stop(reason) = self.monitor.evaluate(Sample {
            now_us,
            tool_position_m: pos,
            tool_force_n: self.measured_force_n,
        }) {
            self.trip(flag_for(reason));
            return self.hold_zero();
        }

        let force = self.tracking_force(j, pos);
        let state = State {
            q: j.q,
            qdot: j.qdot,
            tool_force_n: force,
            now_us,
        };
        let Some(command) = self.edge.step(state) else {
            self.trip(FAULT_SENSOR);
            return self.hold_zero();
        };
        self.last_torque = command.torque_nm;
        self.actuators.apply(command.torque_nm);
        self.last_torque
    }

    /// The force that pulls the tool to its target, with damping and the
    /// disengage ramp applied. Zero when there is no target.
    fn tracking_force(&mut self, j: JointReading, pos: [f64; 3]) -> [f64; 3] {
        let Some(target) = self.target_m else {
            return [0.0; 3];
        };
        let velocity = position_jacobian(&self.cfg.model.arm, j.q).map_or([0.0; 3], |jac| {
            core::array::from_fn(|k| (0..JOINTS).map(|i| jac[k][i] * j.qdot[i]).sum())
        });
        let raw: [f64; 3] = core::array::from_fn(|k| {
            self.cfg.stiffness_n_m * (target[k] - pos[k]) - self.cfg.damping_ns_m * velocity[k]
        });
        let mut scale = 1.0;
        let magnitude = raw.iter().map(|v| v * v).sum::<f64>().sqrt();
        if magnitude > self.cfg.tracking_limit_n && magnitude > 0.0 {
            scale = self.cfg.tracking_limit_n / magnitude;
        }
        if self.ramp_left > 0 {
            scale *= f64::from(self.ramp_left) / f64::from(self.cfg.ramp_ticks);
            self.ramp_left -= 1;
            if self.ramp_left == 0 {
                self.target_m = None;
            }
        }
        raw.map(|v| v * scale)
    }

    /// Latches a stop with the fault flags for `flags`, and zeroes the output.
    fn trip(&mut self, flags: u32) {
        self.latched = true;
        self.fault_flags |= flags;
        self.target_m = None;
        self.ramp_left = 0;
    }

    fn hold_zero(&mut self) -> [f64; JOINTS] {
        self.last_torque = [0.0; JOINTS];
        self.actuators.apply(self.last_torque);
        self.last_torque
    }

    /// Clears the latch and the fault flags. Called only from a human engage,
    /// which the session allows only after the operator acknowledges a stop.
    fn clear_latch(&mut self) {
        self.latched = false;
        self.fault_flags = 0;
        self.monitor.reset();
    }
}

/// Maps a safety stop to the fault flag the operator sees.
const fn flag_for(reason: StopReason) -> u32 {
    match reason {
        StopReason::OutsideEnvelope => FAULT_GEOFENCE,
        StopReason::ForceExceeded => FAULT_ACTUATOR,
        StopReason::WatchdogExpired | StopReason::Disagreement => FAULT_SENSOR,
        StopReason::EStop(_) => 0,
    }
}

impl<A: Actuators> Adapter<A> {
    /// Applies one checked autonomous displacement to the operator's target.
    ///
    /// Used when the AI session is in full-autonomy mode and has returned
    /// `Verdict::Commanded` for a proposal. The displacement is already tool
    /// motion, so it skips the tremor notch and the motion scaler, which act on
    /// operator input. It then goes through the same virtual-fixture guard, and
    /// the same latch and target checks, as operator motion.
    ///
    /// The caller must pass only displacements the session has accepted.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is latched, no operator is engaged, the
    /// target is still ramping down, the displacement is not finite, or the tool
    /// position is unknown.
    pub fn on_autonomous_motion(&mut self, delta_m: [f64; 3]) -> Result<()> {
        if self.latched {
            return Err(DomainError::adapter("stop is latched"));
        }
        if self.operator.is_none() || self.target_m.is_none() || self.ramp_left > 0 {
            return Err(DomainError::adapter("no operator target to move"));
        }
        if !delta_m.iter().all(|v| v.is_finite()) {
            return Err(DomainError::adapter("non-finite autonomous displacement"));
        }
        let now_us = self.clock_us;
        self.move_target(now_us, delta_m)
    }

    /// Moves the target by one tool displacement, after the virtual-fixture
    /// guard decides whether the tool may move that way.
    fn move_target(&mut self, now_us: u64, delta: [f64; 3]) -> Result<()> {
        let Some(tool) = self.tool_position() else {
            return Err(DomainError::adapter("no joint state"));
        };
        let action = self.guard.evaluate(now_us, &self.cfg.sdf, tool, delta);
        if let Some(target) = self.target_m {
            match action {
                Action::Follow { delta_m, force_n } => {
                    // The guard checks the measured tool. The target is where the
                    // arm is commanded, and it can lead the tool, so it is checked
                    // as well. A lagging tool must not let the target into a structure.
                    let next: [f64; 3] = core::array::from_fn(|k| target[k] + delta_m[k]);
                    if self.cfg.sdf.distance(next).is_some_and(|d| d >= 0.0) {
                        self.feedback_n = force_n;
                        self.target_m = Some(next);
                    } else {
                        // Not applied. Full feedback, as the guard gives for an
                        // unknown position.
                        self.feedback_n = self.cfg.guard_max_force_n;
                    }
                }
                Action::Blocked { force_n } => {
                    // The motion would enter the structure. It is not applied, and
                    // the surgeon feels the force.
                    self.feedback_n = force_n;
                }
                Action::Override { retreat_m } => {
                    self.feedback_n = self.cfg.guard_max_force_n;
                    self.target_m = Some(core::array::from_fn(|k| tool[k] + retreat_m[k]));
                }
            }
        }
        Ok(())
    }
}

impl<A: Actuators> DomainTeleopInterface for Adapter<A> {
    fn on_teleop_engage(&mut self, operator: OperatorId) -> Result<()> {
        let Some(pos) = self.tool_position() else {
            return Err(DomainError::adapter("no joint state; cannot engage"));
        };
        // The session only reaches here after an acknowledged stop or a request,
        // so a human has reset the unit. The target starts at the current tool
        // position, so engaging does not move the arm.
        self.clear_latch();
        self.target_m = Some(pos);
        self.operator = Some(operator);
        self.ramp_left = 0;
        Ok(())
    }

    fn on_teleop_disengage(&mut self) -> Result<()> {
        self.operator = None;
        if self.target_m.is_some() {
            self.ramp_left = self.cfg.ramp_ticks;
        }
        Ok(())
    }

    fn on_control_command(&mut self, cmd: ControlCommand) -> Result<()> {
        if self.latched {
            return Err(DomainError::adapter("stop is latched"));
        }
        if self.target_m.is_none() || self.ramp_left > 0 {
            return Err(DomainError::adapter("no operator target to move"));
        }
        let axes = cmd.axes.map(f64::from);
        let raw = [AXIS_LATERAL_X, AXIS_LATERAL_Y, AXIS_THROTTLE].map(|i| axes[i]);
        if !raw.iter().all(|v| v.is_finite()) {
            return Err(DomainError::adapter("non-finite command axis"));
        }
        let now_us = if cmd.timestamp_ns > 0 {
            cmd.timestamp_ns / 1_000
        } else {
            self.clock_us
        };

        let master_m = raw.map(|a| a.clamp(-1.0, 1.0) * self.cfg.step_m);
        let filtered = [0, 1, 2].map(|i| self.notch[i].process(master_m[i]));
        let delta = self.cfg.scaler.scale(filtered);

        self.move_target(now_us, delta)
    }

    #[allow(clippy::cast_possible_truncation)]
    fn get_domain_state(&self) -> DomainState {
        let pose = self
            .joints
            .and_then(|j| self.cfg.model.arm.forward(j.q))
            .map_or(Pose6DOF::IDENTITY, |p| Pose6DOF {
                position: p.position_m.map(|v| v as f32),
                orientation: [0.0, 0.0, 0.0, 1.0],
            });
        DomainState {
            timestamp_us: self.clock_us,
            pose,
            battery_pct: self.battery_pct,
            fault_flags: self.fault_flags,
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn get_sensor_feed(&mut self) -> SensorFeed {
        let telemetry = self.get_domain_state();
        let [fx, fy, fz] = self.measured_force_n.map(|v| v as f32);
        SensorFeed {
            timestamp_us: self.clock_us,
            video_frames: std::mem::take(&mut self.frames),
            point_clouds: Vec::new(),
            telemetry,
            haptic_data: HapticFeedback {
                force_torque: [fx, fy, fz, 0.0, 0.0, self.feedback_n as f32],
            },
            audio: None,
        }
    }

    fn on_emergency_stop(&mut self) {
        // An operator stop is latched like any other, with no fault flag.
        self.latched = true;
        self.target_m = None;
        self.ramp_left = 0;
        self.hold_zero();
    }
}
