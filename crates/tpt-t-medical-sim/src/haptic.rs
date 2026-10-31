//! Haptic force rendering for the simulator (spec.txt §4.2).
//!
//! The spec does not give a haptic device, so the device model here is the
//! impedance contact model in `tpt-t-medical-haptic`: a spring and damper
//! against a surface, saturated at the device's maximum force. It is a stand-in
//! for a real device, not a characterisation of one.
//!
//! [`HapticRig`] feeds the simulated tool position into [`HapticLoop`]. The loop
//! does the hold logic: a missed deadline, a missing position (for example a
//! sensor failure), a stale sample or a non-finite one all hold the last force
//! rather than dropping it to zero.

use tpt_t_medical_haptic::{HapticInput, HapticLoop, Output, STALE_US, Surface, Tissue};
use tpt_t_medical_ring::SpscRing;

/// Samples the ring can hold. The loop drains it every tick, so a few is enough.
const RING_CAPACITY: usize = 4;

/// Renders the reflected contact force on the tool, one tick at a time.
#[derive(Debug)]
pub struct HapticRig {
    ring: SpscRing<HapticInput>,
    renderer: HapticLoop,
    last: Option<([f64; 3], u64)>,
}

impl HapticRig {
    /// Creates a rig for `surface` and `tissue`, saturating at `max_force_n`.
    /// Returns `None` unless the loop accepts the parameters.
    #[must_use]
    pub fn new(surface: Surface, tissue: Tissue, max_force_n: f64) -> Option<Self> {
        Some(Self {
            ring: SpscRing::with_capacity(RING_CAPACITY),
            renderer: HapticLoop::new(surface, tissue, max_force_n)?,
            last: None,
        })
    }

    /// Renders one tick from the tool position at `now_us`. Pass `None` when
    /// the position is unknown.
    ///
    /// Velocity is a finite difference against the previous position. After a
    /// gap longer than [`STALE_US`] the velocity is zero, because the old
    /// position says nothing about the current speed.
    pub fn render(&mut self, now_us: u64, tool: Option<[f64; 3]>, elapsed_us: u64) -> Output {
        if let Some(position) = tool {
            let velocity = match self.last {
                Some((prev, prev_us)) if now_us > prev_us && now_us - prev_us <= STALE_US => {
                    let dt = seconds(now_us - prev_us);
                    [0, 1, 2].map(|i| (position[i] - prev[i]) / dt)
                }
                _ => [0.0; 3],
            };
            self.last = Some((position, now_us));
            // Drained every tick, so the ring cannot be full here.
            let _ = self.ring.push(HapticInput {
                timestamp_us: now_us,
                position,
                velocity,
            });
        }
        self.renderer.step(&self.ring, now_us, elapsed_us)
    }
}

#[allow(clippy::cast_precision_loss)]
fn seconds(us: u64) -> f64 {
    us as f64 / 1_000_000.0
}

// A held force must be bit-identical, and no contact is exactly zero, so exact
// comparison is intended.
#[allow(clippy::float_cmp)]
#[cfg(test)]
mod tests {
    use super::*;
    use tpt_t_medical_haptic::Status;

    fn floor() -> Surface {
        // Tissue occupies z < 0, with the normal pointing up.
        Surface::new([0.0; 3], [0.0, 0.0, 1.0]).expect("surface")
    }

    fn soft() -> Tissue {
        Tissue::new(1_000.0, 10.0).expect("tissue")
    }

    fn rig() -> HapticRig {
        HapticRig::new(floor(), soft(), 10.0).expect("rig")
    }

    #[test]
    fn no_force_above_the_surface() {
        let mut r = rig();
        let out = r.render(0, Some([0.0, 0.0, 0.01]), 10);
        assert_eq!(out.status, Status::Rendered);
        assert_eq!(out.force, [0.0; 3]);
    }

    #[test]
    fn a_penetrating_tool_is_pushed_back_along_the_normal() {
        // 1 mm deep at 1000 N/m is 1 N.
        let mut r = rig();
        let out = r.render(0, Some([0.0, 0.0, -0.001]), 10);
        assert_eq!(out.status, Status::Rendered);
        assert!((out.force[2] - 1.0).abs() < 1e-9, "{:?}", out.force);
    }

    #[test]
    fn pushing_in_adds_damping_from_the_finite_difference_velocity() {
        // Moving 0.1 m/s down adds 10 * 0.1 = 1 N to the 1 N spring.
        let mut r = rig();
        r.render(0, Some([0.0, 0.0, 0.0]), 10);
        let out = r.render(1_000, Some([0.0, 0.0, -0.0001]), 10);
        // Depth is 0.1 mm (0.1 N) and inward speed is 0.1 m/s (1 N).
        assert!((out.force[2] - 1.1).abs() < 1e-6, "{:?}", out.force);
    }

    #[test]
    fn the_force_saturates_at_the_device_limit() {
        let mut r = rig();
        let out = r.render(0, Some([0.0, 0.0, -0.5]), 10);
        assert!((out.force[2] - 10.0).abs() < 1e-9, "{:?}", out.force);
    }

    #[test]
    fn a_missed_deadline_holds_the_previous_force() {
        let mut r = rig();
        let first = r.render(0, Some([0.0, 0.0, -0.001]), 10);
        let late = r.render(1_000, Some([0.0, 0.0, -0.002]), 5_000);
        assert_eq!(late.status, Status::HeldDeadlineMissed);
        assert_eq!(late.force, first.force);
    }

    #[test]
    fn an_unknown_position_holds_the_previous_force() {
        let mut r = rig();
        let first = r.render(0, Some([0.0, 0.0, -0.001]), 10);
        let lost = r.render(1_000, None, 10);
        assert_eq!(lost.status, Status::HeldNoInput);
        assert_eq!(lost.force, first.force);
    }

    #[test]
    fn velocity_after_a_stale_gap_is_zero() {
        // A gap longer than the stale limit must not turn an old position into
        // a huge speed.
        let mut r = rig();
        r.render(0, Some([0.0, 0.0, 0.0]), 10);
        let out = r.render(STALE_US + 10_000, Some([0.0, 0.0, -0.0001]), 10);
        // Depth 0.1 mm gives 0.1 N, with no damping term.
        assert!((out.force[2] - 0.1).abs() < 1e-9, "{:?}", out.force);
    }
}

// A held force must be bit-identical, and no contact is exactly zero, so exact
// comparison is intended.
#[allow(clippy::float_cmp)]
#[cfg(test)]
mod simulator_tests {
    use super::*;
    use crate::{Fault, JOINTS, JointParams, Simulator, tool_position};
    use tpt_t_medical_haptic::Status;
    use tpt_t_medical_surgical::kinematics::Arm;

    #[test]
    fn a_simulated_tool_pressed_into_a_surface_feels_the_spring_force() {
        let params = [JointParams::new(0.5, 0.2, 0.0, 0.0, 0.0).expect("params"); JOINTS];
        let mut sim = Simulator::new(params, [0.0; JOINTS]);
        let arm = Arm::reference();

        // The surface sits 2 mm above the tool, so the tool is 2 mm inside it.
        let readings = sim.step(Some([0.0; JOINTS]));
        let tool = tool_position(&arm, &readings).expect("tool at rest");
        let surface =
            Surface::new([tool[0], tool[1], tool[2] + 0.002], [0.0, 0.0, 1.0]).expect("surface");
        let mut rig = HapticRig::new(surface, Tissue::new(1_000.0, 10.0).expect("tissue"), 10.0)
            .expect("rig");

        let out = rig.render(readings.time_us, Some(tool), 10);
        assert_eq!(out.status, Status::Rendered);
        assert!((out.force[2] - 2.0).abs() < 1e-9, "{:?}", out.force);

        // A sensor failure hides the position. The surgeon keeps the last force.
        sim.inject(Fault::SensorFailure(0)).expect("joint");
        let readings = sim.step(Some([0.0; JOINTS]));
        assert_eq!(tool_position(&arm, &readings), None);
        let held = rig.render(readings.time_us, None, 10);
        assert_eq!(held.status, Status::HeldNoInput);
        assert_eq!(held.force, out.force);
    }
}
