//! Forward kinematics for a serial 7-DOF arm, using modified (Craig)
//! Denavit–Hartenberg parameters (spec.txt §4.3).
//!
//! The spec does not fix the arm's geometry, so the link table is a parameter.
//! [`Arm::reference`] returns the table of a Panda-class arm, as a reference
//! and a test fixture. A real arm needs its own table from its manufacturer.
//!
//! Only position and orientation are computed. Inverse kinematics and the
//! Jacobian are not here yet.

/// A 3×3 rotation matrix, row-major.
pub type Rotation = [[f64; 3]; 3];

/// One joint's rotation axis and a point on it, in the base frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointFrame {
    /// A point on the joint axis, in metres. The joint's link is attached here.
    pub origin_m: [f64; 3],
    /// Unit direction of the joint axis.
    pub axis: [f64; 3],
}

/// One link, in modified DH form. `a` and `alpha` belong to the link before this
/// joint. `d` is the offset along this joint's axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    /// Link length, in metres.
    pub a: f64,
    /// Link offset, in metres.
    pub d: f64,
    /// Link twist, in radians.
    pub alpha: f64,
}

/// Where the tool flange is, in the base frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Position, in metres.
    pub position_m: [f64; 3],
    /// Orientation.
    pub rotation: Rotation,
}

/// A serial 7-DOF arm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arm {
    links: [Link; 7],
    flange_d_m: f64,
}

/// A 4×4 homogeneous transform.
type Transform = [[f64; 4]; 4];

impl Arm {
    /// Creates an arm from its seven links and the flange offset past the last
    /// joint. Returns `None` if any value is not finite.
    #[must_use]
    pub fn new(links: [Link; 7], flange_d_m: f64) -> Option<Self> {
        let finite = flange_d_m.is_finite()
            && links
                .iter()
                .all(|l| l.a.is_finite() && l.d.is_finite() && l.alpha.is_finite());
        finite.then_some(Self { links, flange_d_m })
    }

    /// The reference geometry of a Panda-class arm, from its published DH
    /// table. This is a fixture, not a claim about the spec's arm.
    #[must_use]
    pub fn reference() -> Self {
        use core::f64::consts::FRAC_PI_2;
        let link = |a, d, alpha| Link { a, d, alpha };
        Self {
            links: [
                link(0.0, 0.333, 0.0),
                link(0.0, 0.0, -FRAC_PI_2),
                link(0.0, 0.316, FRAC_PI_2),
                link(0.0825, 0.0, FRAC_PI_2),
                link(-0.0825, 0.384, -FRAC_PI_2),
                link(0.0, 0.0, FRAC_PI_2),
                link(0.088, 0.0, FRAC_PI_2),
            ],
            flange_d_m: 0.107,
        }
    }

    /// Returns the axis of every joint for joint angles `q`, in radians. Entry
    /// `j` is joint `j + 1`.
    ///
    /// Returns `None` if any angle is not finite.
    #[must_use]
    pub fn joint_frames(&self, q: [f64; 7]) -> Option<[JointFrame; 7]> {
        if !q.iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut t = identity();
        let mut out = [JointFrame {
            origin_m: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
        }; 7];
        for (j, (link, &theta)) in self.links.iter().zip(q.iter()).enumerate() {
            t = mul(&t, &modified_dh(*link, theta));
            out[j] = JointFrame {
                origin_m: [t[0][3], t[1][3], t[2][3]],
                axis: [t[0][2], t[1][2], t[2][2]],
            };
        }
        Some(out)
    }

    /// Returns the flange pose for joint angles `q`, in radians.
    ///
    /// Returns `None` if any angle is not finite, so that a NaN reading is never
    /// turned into a pose.
    #[must_use]
    pub fn forward(&self, q: [f64; 7]) -> Option<Pose> {
        if !q.iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut t = identity();
        for (link, &theta) in self.links.iter().zip(q.iter()) {
            t = mul(&t, &modified_dh(*link, theta));
        }
        t = mul(&t, &translate_z(self.flange_d_m));
        Some(Pose {
            position_m: [t[0][3], t[1][3], t[2][3]],
            rotation: [
                [t[0][0], t[0][1], t[0][2]],
                [t[1][0], t[1][1], t[1][2]],
                [t[2][0], t[2][1], t[2][2]],
            ],
        })
    }
}

/// One link's transform for joint angle `theta`, in modified DH form:
/// `Rx(alpha) · Tx(a) · Rz(theta) · Tz(d)`.
fn modified_dh(link: Link, theta: f64) -> Transform {
    let (st, ct) = theta.sin_cos();
    let (sa, ca) = link.alpha.sin_cos();
    let d = link.d;
    [
        [ct, -st, 0.0, link.a],
        [st * ca, ct * ca, -sa, -sa * d],
        [st * sa, ct * sa, ca, ca * d],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn translate_z(d: f64) -> Transform {
    let mut t = identity();
    t[2][3] = d;
    t
}

fn identity() -> Transform {
    let mut t = [[0.0; 4]; 4];
    for (i, row) in t.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    t
}

fn mul(x: &Transform, y: &Transform) -> Transform {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| x[i][k] * y[k][j]).sum();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn zero_pose_matches_the_reference_arm() {
        let pose = Arm::reference().forward([0.0; 7]).expect("pose");
        // Published zero-configuration flange position for the Panda-class arm.
        assert!(close(pose.position_m[0], 0.088), "{:?}", pose.position_m);
        assert!(close(pose.position_m[1], 0.0), "{:?}", pose.position_m);
        assert!(close(pose.position_m[2], 0.926), "{:?}", pose.position_m);
    }

    #[test]
    fn rotations_stay_orthonormal() {
        let q = [0.3, -0.7, 0.2, -1.4, 0.5, 1.1, -0.4];
        let r = Arm::reference().forward(q).expect("pose").rotation;
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = (0..3).map(|k| r[i][k] * r[j][k]).sum();
                let want = if i == j { 1.0 } else { 0.0 };
                assert!(close(dot, want), "row {i}·{j} = {dot}");
            }
        }
    }

    #[test]
    fn a_single_link_is_a_pure_rotation_about_the_base() {
        // With only the first joint moving, the flange sweeps a circle in the
        // base's x–y plane. It keeps its height and its distance from the axis.
        let arm = Arm::reference();
        let zero = arm.forward([0.0; 7]).expect("zero").position_m;
        let turned = arm
            .forward([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .expect("turned")
            .position_m;
        assert!(close(turned[2], zero[2]));
        let r0 = zero[0].hypot(zero[1]);
        let r1 = turned[0].hypot(turned[1]);
        assert!(close(r0, r1), "{r0} vs {r1}");
    }

    #[test]
    fn joint_axes_are_unit_and_the_first_is_vertical() {
        let arm = Arm::reference();
        let frames = arm
            .joint_frames([0.4, -0.2, 0.1, -1.0, 0.3, 0.9, -0.5])
            .expect("frames");
        for f in frames {
            let len: f64 = f.axis.iter().map(|v| v * v).sum::<f64>().sqrt();
            assert!(close(len, 1.0), "{len}");
        }
        // The first joint sits on the base's vertical axis at zero configuration.
        let first = arm.joint_frames([0.0; 7]).expect("frames")[0];
        assert!(close(first.axis[2], 1.0));
        assert!(close(first.origin_m[2], 0.333));
    }

    #[test]
    fn non_finite_angles_give_no_pose() {
        let arm = Arm::reference();
        assert!(
            arm.forward([f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
                .is_none()
        );
        assert!(
            arm.forward([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, f64::INFINITY])
                .is_none()
        );
    }

    #[test]
    fn non_finite_geometry_is_refused() {
        let link = Link {
            a: 0.0,
            d: 0.0,
            alpha: 0.0,
        };
        assert!(Arm::new([link; 7], f64::NAN).is_none());
        let bad = Link {
            a: f64::INFINITY,
            ..link
        };
        assert!(Arm::new([link, link, link, link, link, link, bad], 0.1).is_none());
    }
}
