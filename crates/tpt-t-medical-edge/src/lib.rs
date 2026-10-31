//! 1 kHz control-law pipeline for tpt-teleop-medical (spec.txt §4.3, Phase 3).
//!
//! Each cycle, [`Loop::step`] turns the arm's state and the tool force into
//! joint torques:
//!
//! 1. the tool force becomes joint torques through the Jacobian,
//! 2. gravity compensation is added,
//! 3. friction is added,
//! 4. each joint is clamped to its limit.
//!
//! If a cycle arrives after the deadline, the loop holds the last command
//! rather than computing a new one from a stale state. The spec's hold-last-value
//! rule applies here as well.
//!
//! The pipeline is arithmetic. The 1 kHz period and the 1 ms deadline are
//! checked against the clock the caller supplies. They are not measured on a
//! real-time core, and nothing here pins a thread or sets a scheduler policy.
//! Those need the target hardware.

#![forbid(unsafe_code)]

pub mod control;

use tpt_t_medical_surgical::kinematics::Arm;

pub use control::{Friction, GRAVITY_M_S2};

use control::{friction_torque, gravity_torque, joint_torque_from_force, saturate};

/// The control period at 1 kHz, in microseconds.
pub const PERIOD_US: u64 = 1_000;

/// The longest a cycle may arrive after the previous one before it counts as
/// missed, in microseconds. This is the 1 ms hard deadline in the spec.
pub const DEADLINE_US: u64 = 1_000;

/// One cycle's output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Command {
    /// Torque for each joint, in newton-metres, after the limits.
    pub torque_nm: [f64; 7],
    /// `true` if a joint was clamped to its limit.
    pub saturated: bool,
    /// `true` if this cycle missed its deadline, so the torque is held from the
    /// last good cycle.
    pub held: bool,
}

/// The arm's state for one cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    /// Joint angles, in radians.
    pub q: [f64; 7],
    /// Joint speeds, in radians per second.
    pub qdot: [f64; 7],
    /// Force on the tool, in newtons.
    pub tool_force_n: [f64; 3],
    /// Time of the reading, in microseconds.
    pub now_us: u64,
}

/// The model and limits the pipeline uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Model {
    /// The arm's geometry.
    pub arm: Arm,
    /// Mass at each joint's link, in kilograms.
    pub link_masses_kg: [f64; 7],
    /// Friction in every joint.
    pub friction: Friction,
    /// Torque limit for each joint, in newton-metres.
    pub limits_nm: [f64; 7],
}

/// Runs the pipeline once per cycle, and holds the last command on a missed
/// deadline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loop {
    model: Model,
    last: Option<(u64, [f64; 7])>,
}

impl Loop {
    /// Creates a loop. Returns `None` if the model's limits are invalid. A
    /// limit must be finite and not negative.
    #[must_use]
    pub fn new(model: Model) -> Option<Self> {
        model
            .limits_nm
            .iter()
            .all(|l| l.is_finite() && *l >= 0.0)
            .then_some(Self { model, last: None })
    }

    /// Computes the torque for one cycle.
    ///
    /// Returns `None` if the state holds a non-finite value, and there is no
    /// earlier command to hold. The caller must then stop the arm.
    pub fn step(&mut self, state: State) -> Option<Command> {
        if let Some((at, held)) = self.last {
            if state.now_us.saturating_sub(at) > DEADLINE_US {
                return Some(Command {
                    torque_nm: held,
                    saturated: false,
                    held: true,
                });
            }
        }
        let torque = self.compute(state)?;
        let (limited, saturated) = saturate(torque, self.model.limits_nm)?;
        self.last = Some((state.now_us, limited));
        Some(Command {
            torque_nm: limited,
            saturated,
            held: false,
        })
    }

    fn compute(&self, state: State) -> Option<[f64; 7]> {
        let m = &self.model;
        let force = joint_torque_from_force(&m.arm, state.q, state.tool_force_n)?;
        let gravity = gravity_torque(&m.arm, state.q, m.link_masses_kg)?;
        let friction = friction_torque(state.qdot, m.friction)?;
        Some(core::array::from_fn(|j| {
            force[j] + gravity[j] + friction[j]
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: [f64; 7] = [0.1, -0.4, 0.2, -1.1, 0.0, 1.0, 0.3];
    const MASSES: [f64; 7] = [4.0, 3.0, 3.0, 2.5, 2.0, 1.5, 1.0];

    fn model(limit: f64) -> Model {
        Model {
            arm: Arm::reference(),
            link_masses_kg: MASSES,
            friction: Friction::new(0.2, 0.05, 0.01).expect("friction"),
            limits_nm: [limit; 7],
        }
    }

    fn state(now_us: u64) -> State {
        State {
            q: Q,
            qdot: [0.0; 7],
            tool_force_n: [0.0; 3],
            now_us,
        }
    }

    #[test]
    fn at_rest_the_loop_holds_the_arm_against_gravity() {
        let mut l = Loop::new(model(1000.0)).expect("loop");
        let cmd = l.step(state(0)).expect("command");
        assert!(!cmd.held && !cmd.saturated);
        // With no motion and no tool force, the command is gravity alone.
        let gravity = gravity_torque(&Arm::reference(), Q, MASSES).expect("gravity");
        for (j, (got, want)) in cmd.torque_nm.iter().zip(gravity).enumerate() {
            assert!((got - want).abs() < 1e-9, "joint {j}");
        }
    }

    #[test]
    fn large_torques_are_clamped_and_flagged() {
        let mut l = Loop::new(model(0.5)).expect("loop");
        let mut s = state(0);
        s.tool_force_n = [50.0, 0.0, 0.0];
        let cmd = l.step(s).expect("command");
        assert!(cmd.saturated);
        assert!(cmd.torque_nm.iter().all(|t| t.abs() <= 0.5 + 1e-12));
    }

    // Holding means the same bits come back, so exact comparison is intended.
    #[allow(clippy::float_cmp)]
    #[test]
    fn a_missed_deadline_holds_the_last_command() {
        let mut l = Loop::new(model(100.0)).expect("loop");
        let first = l.step(state(0)).expect("first");
        // The next cycle arrives 5 ms late, with a very different state.
        let mut late = state(5_000);
        late.tool_force_n = [9.0, 9.0, 9.0];
        let held = l.step(late).expect("held");
        assert!(held.held);
        assert_eq!(held.torque_nm, first.torque_nm);
    }

    #[allow(clippy::float_cmp)]
    #[test]
    fn an_on_time_cycle_is_computed_fresh() {
        let mut l = Loop::new(model(100.0)).expect("loop");
        let first = l.step(state(0)).expect("first");
        let mut next = state(PERIOD_US);
        next.tool_force_n = [2.0, 0.0, 0.0];
        let second = l.step(next).expect("second");
        assert!(!second.held);
        assert_ne!(second.torque_nm, first.torque_nm);
    }

    #[test]
    fn a_non_finite_state_gives_no_command() {
        let mut l = Loop::new(model(100.0)).expect("loop");
        let mut s = state(0);
        s.qdot[3] = f64::NAN;
        assert!(l.step(s).is_none());
    }

    #[test]
    fn invalid_limits_are_refused_at_construction() {
        let mut m = model(1.0);
        m.limits_nm[2] = -1.0;
        assert!(Loop::new(m).is_none());
        m.limits_nm[2] = f64::NAN;
        assert!(Loop::new(m).is_none());
    }
}
