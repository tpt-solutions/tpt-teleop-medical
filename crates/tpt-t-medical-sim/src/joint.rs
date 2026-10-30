//! One simulated joint: a rigid link driven by a motor, under gravity, friction
//! and backlash.
//!
//! The model is a single pendulum term for gravity, `gravity_amp * sin(θ)`.
//! Friction is viscous plus Coulomb. Backlash is applied to the reported
//! output, not to the motor. This is a simulation model for testing control
//! and safety logic. It is not a calibrated model of any arm.

/// Physical parameters of one joint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointParams {
    /// Rotational inertia, in kg·m².
    pub inertia: f64,
    /// Viscous friction, in N·m·s/rad.
    pub damping: f64,
    /// Coulomb friction, in N·m.
    pub coulomb: f64,
    /// Peak gravity torque, `m · g · L`, in N·m.
    pub gravity_amp: f64,
    /// Half-width of the backlash deadband, in rad.
    pub backlash: f64,
}

impl JointParams {
    /// Creates parameters. `inertia` must be positive. The other values must be
    /// finite and not negative.
    #[must_use]
    pub fn new(
        inertia: f64,
        damping: f64,
        coulomb: f64,
        gravity_amp: f64,
        backlash: f64,
    ) -> Option<Self> {
        let valid = inertia.is_finite()
            && inertia > 0.0
            && [damping, coulomb, gravity_amp, backlash]
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0);
        valid.then_some(Self {
            inertia,
            damping,
            coulomb,
            gravity_amp,
            backlash,
        })
    }
}

/// The state of one joint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Joint {
    params: JointParams,
    motor: f64,
    output: f64,
    velocity: f64,
    stalled: bool,
    sensor_failed: bool,
}

impl Joint {
    /// Creates a joint at rest at `initial` rad.
    #[must_use]
    pub const fn new(params: JointParams, initial: f64) -> Self {
        Self {
            params,
            motor: initial,
            output: initial,
            velocity: 0.0,
            stalled: false,
            sensor_failed: false,
        }
    }

    /// Advances the joint by `dt` seconds under `command` (N·m).
    ///
    /// A stalled motor produces no torque. The joint then moves under gravity
    /// and friction alone.
    pub fn step(&mut self, command: f64, dt: f64) {
        let torque = if self.stalled { 0.0 } else { command };
        let gravity = self.params.gravity_amp * self.motor.sin();
        let coulomb = if self.velocity.abs() > 1e-9 {
            self.params.coulomb * self.velocity.signum()
        } else {
            0.0
        };
        let friction = self.params.damping * self.velocity + coulomb;
        let accel = (torque - gravity - friction) / self.params.inertia;

        self.velocity += accel * dt;
        self.motor += self.velocity * dt;
        self.output = backlash(self.motor, self.output, self.params.backlash);
    }

    /// The reported position, or `None` if the sensor has failed.
    #[must_use]
    pub const fn reading(&self) -> Option<f64> {
        if self.sensor_failed {
            None
        } else {
            Some(self.output)
        }
    }

    /// The true motor position. Only the simulator should see this. The
    /// safety logic must use [`Joint::reading`].
    #[must_use]
    pub const fn true_motor_position(&self) -> f64 {
        self.motor
    }

    /// The joint velocity, in rad/s.
    #[must_use]
    pub const fn velocity(&self) -> f64 {
        self.velocity
    }

    /// Sets or clears the motor stall fault.
    pub fn set_stalled(&mut self, stalled: bool) {
        self.stalled = stalled;
    }

    /// Sets or clears the sensor failure fault.
    pub fn set_sensor_failed(&mut self, failed: bool) {
        self.sensor_failed = failed;
    }
}

/// The standard backlash operator. The output follows the input only after the
/// input has moved more than `half_width` away from it.
#[must_use]
pub fn backlash(input: f64, output: f64, half_width: f64) -> f64 {
    if input > output + half_width {
        input - half_width
    } else if input < output - half_width {
        input + half_width
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 0.001;

    fn params(gravity_amp: f64, damping: f64, coulomb: f64) -> JointParams {
        JointParams::new(0.5, damping, coulomb, gravity_amp, 0.0).expect("valid")
    }

    fn run(joint: &mut Joint, command: f64, steps: usize) {
        for _ in 0..steps {
            joint.step(command, DT);
        }
    }

    #[test]
    fn gravity_compensation_holds_the_joint() {
        let p = params(2.0, 0.0, 0.0);
        let start = 0.5;
        let mut j = Joint::new(p, start);
        let hold = p.gravity_amp * start.sin();
        run(&mut j, hold, 1_000);
        let moved = (j.true_motor_position() - start).abs();
        assert!(moved < 1e-3, "drifted {moved} rad under compensation");
    }

    #[test]
    fn without_torque_the_joint_falls_under_gravity() {
        let mut j = Joint::new(params(2.0, 0.0, 0.0), 0.5);
        run(&mut j, 0.0, 1_000);
        assert!(j.true_motor_position() < 0.5, "should fall toward zero");
    }

    #[test]
    fn damping_brings_the_joint_to_rest() {
        let mut j = Joint::new(params(0.0, 5.0, 0.0), 0.0);
        run(&mut j, 1.0, 100);
        let just_after_push = j.velocity().abs();
        run(&mut j, 0.0, 3_000);
        assert!(j.velocity().abs() < just_after_push / 10.0);
    }

    #[test]
    fn stalled_motor_ignores_the_command() {
        let mut j = Joint::new(params(0.0, 0.0, 0.0), 0.0);
        j.set_stalled(true);
        run(&mut j, 10.0, 1_000);
        assert!(j.true_motor_position().abs() < 1e-9);
    }

    #[test]
    fn failed_sensor_reports_nothing() {
        let mut j = Joint::new(params(0.0, 0.0, 0.0), 0.3);
        assert_eq!(j.reading(), Some(0.3));
        j.set_sensor_failed(true);
        assert_eq!(j.reading(), None);
    }

    #[test]
    fn backlash_holds_the_output_inside_the_deadband() {
        assert!(backlash(0.004, 0.0, 0.005).abs() < 1e-12);
        assert!(backlash(-0.004, 0.0, 0.005).abs() < 1e-12);
    }

    #[test]
    fn backlash_follows_the_input_once_it_leaves_the_deadband() {
        assert!((backlash(0.010, 0.0, 0.005) - 0.005).abs() < 1e-12);
        assert!((backlash(-0.010, 0.0, 0.005) + 0.005).abs() < 1e-12);
    }

    #[test]
    fn invalid_parameters_are_refused() {
        assert!(
            JointParams::new(0.0, 0.0, 0.0, 0.0, 0.0).is_none(),
            "zero inertia"
        );
        assert!(
            JointParams::new(1.0, -1.0, 0.0, 0.0, 0.0).is_none(),
            "negative damping"
        );
        assert!(JointParams::new(1.0, 0.0, f64::NAN, 0.0, 0.0).is_none());
    }
}
