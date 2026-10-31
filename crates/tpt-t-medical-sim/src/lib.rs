//! Headless simulator core for tpt-teleop-medical (spec.txt §8).
//!
//! This crate models a seven-joint arm, one joint at a time, at a fixed 1 kHz
//! step. Nothing reads the wall clock and nothing draws random numbers, so a
//! run is repeatable. Faults can be injected and cleared:
//!
//! * [`Fault::SensorFailure`]: a joint stops reporting its position.
//! * [`Fault::MotorStall`]: a joint's motor stops producing torque.
//! * [`Fault::NetworkDrop`]: commands stop arriving. Joints get zero torque
//!   until the link returns.
//!
//! Also modelled:
//!
//! * [`link::Link`]: command latency and deterministic jitter.
//! * [`tissue::Tissue`]: a mass-spring tissue that the tool pushes on.
//! * [`tool_position`]: the tool's position from the joint readings, using the
//!   arm's kinematics.
//!
//! * [`haptic::HapticRig`]: contact force rendered against a surface. The device
//!   model is the impedance model of `tpt-t-medical-haptic`, because the spec
//!   names no haptic device. It is a stand-in, not a device characterisation.
//!
//! Not yet modelled: the visualizer. It is a GUI and is not started.

#![forbid(unsafe_code)]

pub mod haptic;
pub mod joint;
pub mod link;
pub mod tissue;
use core::fmt;

pub use joint::{Joint, JointParams, backlash};

use tpt_t_medical_surgical::kinematics::Arm;

/// Number of joints in the arm.
pub const JOINTS: usize = 7;
/// Simulation step, in microseconds. This is 1 kHz.
pub const STEP_US: u64 = 1_000;

/// A fault that can be injected into the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Joint `index` stops reporting its position.
    SensorFailure(usize),
    /// Joint `index`'s motor stops producing torque.
    MotorStall(usize),
    /// Commands stop reaching the arm.
    NetworkDrop,
}

/// Why a fault could not be injected or cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultError {
    /// The joint index is not in `0..JOINTS`.
    NoSuchJoint {
        /// The index that was asked for.
        index: usize,
    },
}

impl fmt::Display for FaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchJoint { index } => write!(f, "no joint {index}"),
        }
    }
}

impl std::error::Error for FaultError {}

/// What the simulator reports after one step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Readings {
    /// Simulated time after the step, in microseconds.
    pub time_us: u64,
    /// Each joint's reported position, or `None` if its sensor has failed.
    pub positions: [Option<f64>; JOINTS],
    /// Whether this step used the command it was given.
    pub command_applied: bool,
}

/// The arm and its clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Simulator {
    joints: [Joint; JOINTS],
    time_us: u64,
    network_up: bool,
    missing_commands: u64,
}

impl Simulator {
    /// Creates a simulator with each joint at its initial position.
    #[must_use]
    pub const fn new(params: [JointParams; JOINTS], initial: [f64; JOINTS]) -> Self {
        let joints = [
            Joint::new(params[0], initial[0]),
            Joint::new(params[1], initial[1]),
            Joint::new(params[2], initial[2]),
            Joint::new(params[3], initial[3]),
            Joint::new(params[4], initial[4]),
            Joint::new(params[5], initial[5]),
            Joint::new(params[6], initial[6]),
        ];
        Self {
            joints,
            time_us: 0,
            network_up: true,
            missing_commands: 0,
        }
    }

    /// Returns the simulated time, in microseconds.
    #[must_use]
    pub const fn time_us(&self) -> u64 {
        self.time_us
    }

    /// Returns how many steps have run without a usable command.
    #[must_use]
    pub const fn missing_commands(&self) -> u64 {
        self.missing_commands
    }

    /// Returns joint `index`, for inspection.
    ///
    /// # Errors
    ///
    /// Returns [`FaultError::NoSuchJoint`] if the index is out of range.
    pub fn joint(&self, index: usize) -> Result<&Joint, FaultError> {
        self.joints
            .get(index)
            .ok_or(FaultError::NoSuchJoint { index })
    }

    /// Injects `fault`.
    ///
    /// # Errors
    ///
    /// Returns [`FaultError::NoSuchJoint`] for a joint index out of range.
    pub fn inject(&mut self, fault: Fault) -> Result<(), FaultError> {
        self.set_fault(fault, true)
    }

    /// Clears `fault`.
    ///
    /// # Errors
    ///
    /// Returns [`FaultError::NoSuchJoint`] for a joint index out of range.
    pub fn clear(&mut self, fault: Fault) -> Result<(), FaultError> {
        self.set_fault(fault, false)
    }

    fn set_fault(&mut self, fault: Fault, active: bool) -> Result<(), FaultError> {
        match fault {
            Fault::SensorFailure(i) => self.joint_mut(i)?.set_sensor_failed(active),
            Fault::MotorStall(i) => self.joint_mut(i)?.set_stalled(active),
            Fault::NetworkDrop => self.network_up = !active,
        }
        Ok(())
    }

    fn joint_mut(&mut self, index: usize) -> Result<&mut Joint, FaultError> {
        self.joints
            .get_mut(index)
            .ok_or(FaultError::NoSuchJoint { index })
    }

    /// Advances one step under `command` (joint torques, N·m).
    ///
    /// The command is used only if the network is up and every value is
    /// finite. Otherwise every joint gets zero torque, and the step is counted
    /// as missing.
    pub fn step(&mut self, command: Option<[f64; JOINTS]>) -> Readings {
        let usable = command.filter(|c| c.iter().all(|v| v.is_finite()));
        let (torques, command_applied) = match usable {
            Some(c) if self.network_up => (c, true),
            _ => {
                self.missing_commands += 1;
                ([0.0; JOINTS], false)
            }
        };

        let dt = step_seconds();
        for (joint, torque) in self.joints.iter_mut().zip(torques) {
            joint.step(torque, dt);
        }
        self.time_us += STEP_US;

        Readings {
            time_us: self.time_us,
            positions: self.joints.map(|j| j.reading()),
            command_applied,
        }
    }
}

/// Returns the tool's position for one set of readings, in the base frame.
///
/// Returns `None` if any joint has no reading, because a sensor failure means
/// the position is unknown and must not be guessed.
#[must_use]
pub fn tool_position(arm: &Arm, readings: &Readings) -> Option<[f64; 3]> {
    let mut q = [0.0; JOINTS];
    for (slot, reading) in q.iter_mut().zip(readings.positions) {
        *slot = reading?;
    }
    arm.forward(q).map(|pose| pose.position_m)
}

#[allow(clippy::cast_precision_loss)]
fn step_seconds() -> f64 {
    STEP_US as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> [JointParams; JOINTS] {
        [JointParams::new(0.5, 0.2, 0.0, 0.0, 0.0).expect("valid"); JOINTS]
    }

    fn sim() -> Simulator {
        Simulator::new(params(), [0.0; JOINTS])
    }

    #[test]
    fn time_advances_by_one_millisecond_per_step() {
        let mut s = sim();
        s.step(Some([0.0; JOINTS]));
        s.step(Some([0.0; JOINTS]));
        assert_eq!(s.time_us(), 2_000);
    }

    #[test]
    fn usable_command_is_applied_and_reported() {
        let mut s = sim();
        let r = s.step(Some([1.0; JOINTS]));
        assert!(r.command_applied);
        assert!(s.joint(0).expect("joint").velocity() > 0.0);
    }

    #[test]
    fn sensor_failure_hides_only_that_joint() {
        let mut s = sim();
        s.inject(Fault::SensorFailure(2)).expect("valid joint");
        let r = s.step(Some([0.0; JOINTS]));
        assert_eq!(r.positions[2], None);
        assert!(r.positions[0].is_some());
    }

    #[test]
    fn motor_stall_stops_one_joint_responding_to_torque() {
        let mut s = sim();
        s.inject(Fault::MotorStall(4)).expect("valid joint");
        for _ in 0..100 {
            s.step(Some([5.0; JOINTS]));
        }
        assert!(s.joint(4).expect("joint").velocity().abs() < 1e-12);
        assert!(s.joint(0).expect("joint").velocity().abs() > 0.1);
    }

    #[test]
    fn network_drop_zeroes_torque_and_counts_missing_steps() {
        let mut s = sim();
        s.inject(Fault::NetworkDrop).expect("valid");
        let r = s.step(Some([5.0; JOINTS]));
        assert!(!r.command_applied);
        assert_eq!(s.missing_commands(), 1);
        assert!(s.joint(0).expect("joint").velocity().abs() < 1e-12);
    }

    #[test]
    fn network_recovers_when_the_fault_is_cleared() {
        let mut s = sim();
        s.inject(Fault::NetworkDrop).expect("valid");
        s.step(Some([5.0; JOINTS]));
        s.clear(Fault::NetworkDrop).expect("valid");
        assert!(s.step(Some([5.0; JOINTS])).command_applied);
    }

    #[test]
    fn missing_command_counts_as_a_missing_step() {
        let mut s = sim();
        let r = s.step(None);
        assert!(!r.command_applied);
        assert_eq!(s.missing_commands(), 1);
    }

    #[test]
    fn non_finite_command_is_refused_not_applied() {
        let mut s = sim();
        let mut bad = [0.0; JOINTS];
        bad[3] = f64::NAN;
        let r = s.step(Some(bad));
        assert!(!r.command_applied);
        assert!(s.joint(3).expect("joint").velocity().is_finite());
    }

    #[test]
    fn fault_on_a_missing_joint_is_an_error() {
        let mut s = sim();
        assert_eq!(
            s.inject(Fault::SensorFailure(JOINTS)),
            Err(FaultError::NoSuchJoint { index: JOINTS })
        );
        assert_eq!(
            s.joint(JOINTS),
            Err(FaultError::NoSuchJoint { index: JOINTS })
        );
    }

    #[test]
    fn identical_runs_are_identical() {
        let run = || {
            let mut s = sim();
            s.inject(Fault::MotorStall(1)).expect("valid");
            for k in 0..500 {
                let t = f64::from(k) * 0.01;
                s.step(Some([t.sin(); JOINTS]));
            }
            s
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn tool_position_is_known_only_while_every_joint_reports() {
        let arm = Arm::reference();
        let mut sim = sim();
        let readings = sim.step(Some([0.0; JOINTS]));
        assert!(tool_position(&arm, &readings).is_some());
        sim.inject(Fault::SensorFailure(2)).expect("inject");
        let readings = sim.step(Some([0.0; JOINTS]));
        assert_eq!(tool_position(&arm, &readings), None);
    }
}
