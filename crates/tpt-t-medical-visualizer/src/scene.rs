//! The scene the operator sees: trajectory, workspace, fixtures, forces, the audit
//! timeline and the three-copy safety indicators (spec §6.5).
//!
//! The model is separate from the window, so it can be checked without a display.
//! [`View::project`] is an orthographic projection with yaw and pitch, which is
//! enough to show the 3D geometry on a 2D screen.

use tpt_t_medical_surgical::kinematics::Arm;

/// Samples in the demo trajectory.
pub const DEMO_SAMPLES: usize = 240;

/// How the 3D scene is turned into the 2D screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Rotation about the vertical axis, in radians.
    pub yaw: f64,
    /// Tilt toward the viewer, in radians.
    pub pitch: f64,
    /// Screen pixels per metre.
    pub px_per_m: f64,
    /// Screen position of the scene origin, in pixels.
    pub centre_px: [f64; 2],
    /// The world point drawn at the screen centre, in metres.
    pub target_m: [f64; 3],
}

impl View {
    /// Projects a scene point to screen pixels. Screen y grows downward.
    #[must_use]
    pub fn project(&self, world: [f64; 3]) -> [f64; 2] {
        let p = [0, 1, 2].map(|i| world[i] - self.target_m[i]);
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let x1 = cy * p[0] - sy * p[1];
        let y1 = sy * p[0] + cy * p[1];
        let z1 = p[2];
        // Height (z) is screen-up at zero pitch. Tilting the view brings depth
        // (y) into the vertical, so a top-down view shows the horizontal plane.
        let up = cp * z1 + sp * y1;
        [
            self.centre_px[0] + self.px_per_m * x1,
            self.centre_px[1] - self.px_per_m * up,
        ]
    }
}

/// The state of one of the three TMR copies, as the voter reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copy3 {
    /// The copy agrees with the majority.
    Agree,
    /// The copy disagrees with the majority.
    Disagree,
    /// No report from the copy.
    Unknown,
}

/// A force on the tool, drawn as an arrow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Force {
    /// Where the arrow starts, in metres.
    pub at_m: [f64; 3],
    /// The force, in newtons.
    pub newtons: [f64; 3],
}

/// One line of the audit timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// When it happened, in microseconds.
    pub at_us: u64,
    /// What happened.
    pub text: String,
}

/// Everything the window draws.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The tool path, in metres.
    pub trajectory_m: Vec<[f64; 3]>,
    /// The workspace box, as (min, max) corners.
    pub envelope_m: Option<([f64; 3], [f64; 3])>,
    /// Points on the fixture boundary, in metres.
    pub fixture_m: Vec<[f64; 3]>,
    /// Forces on the tool.
    pub forces: Vec<Force>,
    /// The audit timeline, oldest first.
    pub events: Vec<Event>,
    /// The three TMR copies.
    pub tmr: [Copy3; 3],
}

/// The twelve edges of an axis-aligned box.
#[must_use]
pub fn box_edges(min: [f64; 3], max: [f64; 3]) -> Vec<([f64; 3], [f64; 3])> {
    let corner = |i: usize| {
        [0, 1, 2].map(|axis| {
            if (i >> axis) & 1 == 1 {
                max[axis]
            } else {
                min[axis]
            }
        })
    };
    let mut edges = Vec::new();
    for i in 0..8_usize {
        for axis in 0..3 {
            // Each corner joins the corner that differs in one bit, once.
            let j = i | (1 << axis);
            if j != i {
                edges.push((corner(i), corner(j)));
            }
        }
    }
    edges
}

/// The smallest box that holds every point. `None` for no points.
#[must_use]
pub fn bounds(points: &[[f64; 3]]) -> Option<([f64; 3], [f64; 3])> {
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(lo, hi), p| {
        (
            [0, 1, 2].map(|i| lo[i].min(p[i])),
            [0, 1, 2].map(|i| hi[i].max(p[i])),
        )
    }))
}

/// A view that fits `points` in a window of the given size.
#[must_use]
pub fn fit(points: &[[f64; 3]], width_px: f64, height_px: f64) -> View {
    let extent = bounds(points).map_or(0.2, |(lo, hi)| {
        (0..3)
            .map(|i| hi[i] - lo[i])
            .fold(0.0_f64, f64::max)
            .max(1e-3)
    });
    let target_m = bounds(points).map_or([0.0; 3], |(lo, hi)| {
        [0, 1, 2].map(|i| f64::midpoint(lo[i], hi[i]))
    });
    View {
        yaw: 0.6,
        pitch: 0.5,
        px_per_m: 0.4 * width_px.min(height_px) / extent,
        centre_px: [width_px / 2.0, height_px / 2.0],
        target_m,
    }
}

/// A demonstration scene: the reference arm swept through a slow motion, with
/// its workspace, a spherical fixture, two forces, and an audit timeline.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn demo() -> Scene {
    let arm = Arm::reference();
    let trajectory_m: Vec<[f64; 3]> = (0..DEMO_SAMPLES)
        .filter_map(|k| {
            let t = k as f64 / DEMO_SAMPLES as f64 * std::f64::consts::TAU;
            let q = [
                0.4 * t.sin(),
                -0.3 + 0.2 * (2.0 * t).cos(),
                0.2 * t.sin(),
                -1.2,
                0.1 * t.cos(),
                1.0,
                0.0,
            ];
            arm.forward(q).map(|p| p.position_m)
        })
        .collect();
    let centre = trajectory_m.first().copied().unwrap_or([0.0; 3]);
    let fixture_m: Vec<[f64; 3]> = (0..48_i32)
        .map(|k| {
            let a = f64::from(k) / 48.0 * std::f64::consts::TAU;
            [
                centre[0] + 0.05 + 0.02 * a.cos(),
                centre[1] + 0.02 * a.sin(),
                centre[2] + 0.02 * (2.0 * a).sin(),
            ]
        })
        .collect();
    let envelope_m =
        bounds(&trajectory_m).map(|(lo, hi)| (lo.map(|v| v - 0.02), hi.map(|v| v + 0.02)));
    Scene {
        trajectory_m,
        envelope_m,
        fixture_m,
        forces: vec![
            Force {
                at_m: centre,
                newtons: [1.5, 0.0, -0.5],
            },
            Force {
                at_m: centre,
                newtons: [0.0, 2.0, 0.0],
            },
        ],
        events: vec![
            Event {
                at_us: 0,
                text: "Engaged by surgeon".into(),
            },
            Event {
                at_us: 1_200_000,
                text: "Fixture approach: force feedback 1.5 N".into(),
            },
            Event {
                at_us: 2_400_000,
                text: "Copy 3 disagrees with the voter".into(),
            },
        ],
        tmr: [Copy3::Agree, Copy3::Agree, Copy3::Disagree],
    }
}

// The tests compare exact coordinates and bounds, which are exact by design.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            yaw: 0.0,
            pitch: 0.0,
            px_per_m: 100.0,
            centre_px: [200.0, 150.0],
            target_m: [0.0; 3],
        }
    }

    #[test]
    fn the_origin_projects_to_the_centre_of_the_screen() {
        assert_eq!(view().project([0.0; 3]), [200.0, 150.0]);
    }

    #[test]
    fn height_goes_up_the_screen() {
        let p = view().project([0.0, 0.0, 0.1]);
        assert!(p[1] < 150.0, "{p:?}");
    }

    #[test]
    fn a_box_has_twelve_edges_joining_its_corners() {
        let edges = box_edges([0.0; 3], [1.0; 3]);
        assert_eq!(edges.len(), 12);
        for (a, b) in edges {
            let differing = (0..3).filter(|&i| a[i] != b[i]).count();
            assert_eq!(differing, 1, "{a:?} to {b:?}");
        }
    }

    #[test]
    fn bounds_hold_every_point() {
        let pts = [[0.0, 1.0, -1.0], [2.0, -3.0, 0.5]];
        let (lo, hi) = bounds(&pts).expect("bounds");
        assert_eq!(lo, [0.0, -3.0, -1.0]);
        assert_eq!(hi, [2.0, 1.0, 0.5]);
        assert!(bounds(&[]).is_none());
    }

    #[test]
    fn the_demo_scene_is_finite_and_complete() {
        let s = demo();
        assert!(s.trajectory_m.len() > DEMO_SAMPLES / 2);
        assert!(s.trajectory_m.iter().flatten().all(|v| v.is_finite()));
        assert!(s.envelope_m.is_some());
        assert_eq!(s.tmr[2], Copy3::Disagree);
        assert!(s.events.windows(2).all(|w| w[0].at_us <= w[1].at_us));
    }

    #[test]
    fn fitting_keeps_the_scene_on_screen() {
        let s = demo();
        let v = fit(&s.trajectory_m, 800.0, 600.0);
        for p in &s.trajectory_m {
            let q = v.project(*p);
            assert!(
                (0.0..=800.0).contains(&q[0]) && (0.0..=600.0).contains(&q[1]),
                "{q:?}"
            );
        }
    }
}
