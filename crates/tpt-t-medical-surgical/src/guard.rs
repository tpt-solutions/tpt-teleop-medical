//! Virtual-fixture guard: keeps the tool out of a forbidden structure, and
//! overrides the surgeon's master when the tool stays inside (spec.txt §4.3, §4.5).
//!
//! Each cycle the guard looks at the operator's requested motion, and returns one
//! of three actions:
//!
//! * [`Action::Follow`]: the motion keeps the tool outside the structure. It is
//!   applied, with the force feedback for the tool's distance.
//! * [`Action::Blocked`]: the motion would enter the structure. It is not
//!   applied, and the feedback force is returned to the surgeon.
//! * [`Action::Override`]: the tool has been inside the structure for longer than
//!   the limit. The master's motion is ignored and the tool retreats along the
//!   outward normal, until it is outside.
//!
//! A point the grid cannot place is treated as inside the structure. The guard
//! never assumes an unknown position is safe.

use crate::fixture::repulsive_force;
use crate::sdf::SdfGrid;

/// Step for the numerical normal, in metres.
const NORMAL_STEP_M: f64 = 0.0005;

/// What the guard decided for one cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// The motion is safe. Apply it, with the feedback force.
    Follow {
        /// The displacement to apply, in metres.
        delta_m: [f64; 3],
        /// The force to feed back to the surgeon, in newtons.
        force_n: f64,
    },
    /// The motion would enter the structure. Do not apply it.
    Blocked {
        /// The force to feed back to the surgeon, in newtons.
        force_n: f64,
    },
    /// The master is overridden. The tool retreats by this displacement.
    Override {
        /// The retreat displacement, in metres.
        retreat_m: [f64; 3],
    },
}

/// Limits for the guard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guard {
    margin_m: f64,
    max_force_n: f64,
    override_after_us: u64,
    retreat_m: f64,
    inside_since_us: Option<u64>,
}

impl Guard {
    /// Creates a guard. Returns `None` unless the margin, the force and the
    /// retreat length are positive and finite, and the override time is not zero.
    #[must_use]
    pub fn new(
        margin_m: f64,
        max_force_n: f64,
        override_after_us: u64,
        retreat_m: f64,
    ) -> Option<Self> {
        let ok = margin_m.is_finite()
            && margin_m > 0.0
            && max_force_n.is_finite()
            && max_force_n > 0.0
            && override_after_us > 0
            && retreat_m.is_finite()
            && retreat_m > 0.0;
        ok.then_some(Self {
            margin_m,
            max_force_n,
            override_after_us,
            retreat_m,
            inside_since_us: None,
        })
    }

    /// Decides the action for one cycle.
    ///
    /// `tool_m` is the tool tip now, and `desired_m` is the displacement the
    /// surgeon's master asks for this cycle.
    pub fn evaluate(
        &mut self,
        now_us: u64,
        sdf: &SdfGrid,
        tool_m: [f64; 3],
        desired_m: [f64; 3],
    ) -> Action {
        let Some(now) = sdf.distance(tool_m) else {
            // Unknown position: full feedback, and no motion.
            self.inside_since_us = None;
            return Action::Blocked {
                force_n: self.max_force_n,
            };
        };
        if now < 0.0 {
            let since = *self.inside_since_us.get_or_insert(now_us);
            if now_us.saturating_sub(since) >= self.override_after_us {
                return Action::Override {
                    retreat_m: self.retreat(sdf, tool_m),
                };
            }
            return Action::Blocked {
                force_n: self.force(now),
            };
        }
        self.inside_since_us = None;

        let ahead = [0, 1, 2].map(|i| tool_m[i] + desired_m[i]);
        match sdf.distance(ahead) {
            Some(next) if next >= 0.0 => Action::Follow {
                delta_m: desired_m,
                force_n: self.force(next),
            },
            // The motion would enter the structure, or lands where the distance is
            // unknown. Either way it is not applied.
            _ => Action::Blocked {
                force_n: self.force(now),
            },
        }
    }

    fn force(&self, distance_m: f64) -> f64 {
        // `new` checked the parameters, so this is never `None`.
        repulsive_force(distance_m, self.margin_m, self.max_force_n).unwrap_or(self.max_force_n)
    }

    /// The outward normal at `tool_m`, scaled to the retreat length. Zero when the
    /// gradient cannot be found.
    fn retreat(&self, sdf: &SdfGrid, tool_m: [f64; 3]) -> [f64; 3] {
        let mut grad = [0.0; 3];
        for (i, slot) in grad.iter_mut().enumerate() {
            let mut up = tool_m;
            let mut down = tool_m;
            up[i] += NORMAL_STEP_M;
            down[i] -= NORMAL_STEP_M;
            if let (Some(a), Some(b)) = (sdf.distance(up), sdf.distance(down)) {
                *slot = (a - b) / (2.0 * NORMAL_STEP_M);
            }
        }
        let len = grad.iter().map(|v| v * v).sum::<f64>().sqrt();
        if len < 1e-12 {
            return [0.0; 3];
        }
        grad.map(|v| v / len * self.retreat_m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: u64 = 1_000_000;

    /// A ball of radius 5 cm at the origin, on a grid spanning ±10 cm.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn ball() -> SdfGrid {
        let spacing = 0.005;
        let n = 41_u32;
        let mut values = Vec::new();
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let p = [x, y, z].map(|i| -0.1 + f64::from(i) * spacing);
                    let d = p.iter().map(|v| v * v).sum::<f64>().sqrt() - 0.05;
                    values.push(d as f32);
                }
            }
        }
        SdfGrid::new([-0.1; 3], spacing, [n; 3], values).expect("grid")
    }

    fn guard() -> Guard {
        // A 1 cm margin, 3 N at the surface, override after 200 ms, retreat 1 mm.
        Guard::new(0.01, 3.0, 200_000, 0.001).expect("guard")
    }

    fn follow(a: Action) -> Option<([f64; 3], f64)> {
        match a {
            Action::Follow { delta_m, force_n } => Some((delta_m, force_n)),
            Action::Blocked { .. } | Action::Override { .. } => None,
        }
    }

    fn blocked_force(a: Action) -> Option<f64> {
        match a {
            Action::Blocked { force_n } => Some(force_n),
            Action::Follow { .. } | Action::Override { .. } => None,
        }
    }

    fn retreat(a: Action) -> Option<[f64; 3]> {
        match a {
            Action::Override { retreat_m } => Some(retreat_m),
            Action::Follow { .. } | Action::Blocked { .. } => None,
        }
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn free_motion_is_followed_without_force() {
        let mut g = guard();
        let action = g.evaluate(0, &ball(), [0.09, 0.0, 0.0], [0.0, 0.001, 0.0]);
        let (delta, force) = follow(action).expect("followed");
        assert_eq!(delta, [0.0, 0.001, 0.0]);
        assert!(force.abs() < 1e-12);
    }

    #[test]
    fn motion_into_the_structure_is_blocked_and_pushes_back() {
        let mut g = guard();
        // 5.5 mm from the surface, moving 1 cm further in.
        let action = g.evaluate(0, &ball(), [0.0555, 0.0, 0.0], [-0.01, 0.0, 0.0]);
        assert!(blocked_force(action).expect("blocked") > 0.0);
    }

    #[test]
    fn force_rises_as_the_tool_nears_the_surface() {
        let mut g = guard();
        let grid = ball();
        let far = follow(g.evaluate(0, &grid, [0.08, 0.0, 0.0], [0.0; 3]))
            .expect("far")
            .1;
        let near = follow(g.evaluate(0, &grid, [0.0525, 0.0, 0.0], [0.0; 3]))
            .expect("near")
            .1;
        // Inside the margin the surgeon feels force; outside it there is none.
        assert!(near > far);
    }

    #[test]
    fn staying_inside_too_long_overrides_the_master_and_retreats_outward() {
        let mut g = guard();
        let grid = ball();
        let tool = [0.03, 0.0, 0.0];
        // Inside from t = 0 to just before the limit: blocked.
        assert!(blocked_force(g.evaluate(0, &grid, tool, [0.0; 3])).is_some());
        assert!(blocked_force(g.evaluate(199_000, &grid, tool, [0.0; 3])).is_some());
        let retreat_m = retreat(g.evaluate(200_000, &grid, tool, [0.0; 3])).expect("override");
        assert!(
            retreat_m[0] > 0.0,
            "retreat should point out: {retreat_m:?}"
        );
        assert!((retreat_m[0] - 0.001).abs() < 1e-4);
    }

    #[test]
    fn leaving_the_structure_resets_the_override_timer() {
        let mut g = guard();
        let grid = ball();
        g.evaluate(0, &grid, [0.03, 0.0, 0.0], [0.0; 3]);
        g.evaluate(100_000, &grid, [0.09, 0.0, 0.0], [0.0; 3]);
        // Back inside much later: the timer starts again from here.
        assert!(blocked_force(g.evaluate(SEC, &grid, [0.03, 0.0, 0.0], [0.0; 3])).is_some());
    }

    #[test]
    fn an_unknown_position_gets_full_feedback_and_no_motion() {
        let mut g = guard();
        let force = blocked_force(g.evaluate(0, &ball(), [5.0, 0.0, 0.0], [0.0, 0.001, 0.0]));
        assert!((force.expect("blocked") - 3.0).abs() < 1e-12);
    }

    #[test]
    fn invalid_settings_are_refused() {
        assert!(Guard::new(0.0, 1.0, 1, 0.001).is_none());
        assert!(Guard::new(0.01, 1.0, 0, 0.001).is_none());
        assert!(Guard::new(0.01, 1.0, 1, f64::NAN).is_none());
    }
}
