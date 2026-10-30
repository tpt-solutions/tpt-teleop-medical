//! Tool–tissue contact as an impedance model (spec.txt §4.2).
//!
//! The tissue surface is a plane. When the tool penetrates it, the model pushes
//! back with a spring force proportional to the depth, plus a damping force
//! proportional to how fast the tool is pushing in. Outside the surface, the
//! force is zero.
//!
//! This is a deliberately simple model. It is enough to test the deadline and
//! hold logic in [`crate::HapticLoop`], and it is not a tissue model.

/// A tissue surface, given as a point on the plane and a unit normal that
/// points out of the tissue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    point: [f64; 3],
    normal: [f64; 3],
}

impl Surface {
    /// Creates a surface. The normal is normalised, so any non-zero length is
    /// accepted.
    ///
    /// Returns `None` if any input is not finite, or if the normal has zero
    /// length.
    #[must_use]
    pub fn new(point: [f64; 3], normal: [f64; 3]) -> Option<Self> {
        if !point.iter().chain(normal.iter()).all(|v| v.is_finite()) {
            return None;
        }
        let len = dot(normal, normal).sqrt();
        if len == 0.0 || !len.is_finite() {
            return None;
        }
        Some(Self {
            point,
            normal: normal.map(|v| v / len),
        })
    }
}

/// Stiffness and damping of the tissue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tissue {
    /// Spring constant, in newtons per metre of penetration.
    pub stiffness_n_per_m: f64,
    /// Damping constant, in newton-seconds per metre.
    pub damping_ns_per_m: f64,
}

impl Tissue {
    /// Creates a tissue model. Both constants must be finite and not negative.
    #[must_use]
    pub fn new(stiffness_n_per_m: f64, damping_ns_per_m: f64) -> Option<Self> {
        let valid = [stiffness_n_per_m, damping_ns_per_m]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0);
        valid.then_some(Self {
            stiffness_n_per_m,
            damping_ns_per_m,
        })
    }
}

/// The force on the tool, in newtons, for a tool at `position` moving at
/// `velocity` (m/s). The force points out of the tissue. It is clamped to
/// `max_force_n` in magnitude.
#[must_use]
pub fn contact_force(
    surface: &Surface,
    tissue: &Tissue,
    position: [f64; 3],
    velocity: [f64; 3],
    max_force_n: f64,
) -> [f64; 3] {
    let offset = [
        position[0] - surface.point[0],
        position[1] - surface.point[1],
        position[2] - surface.point[2],
    ];
    let signed_distance = dot(offset, surface.normal);
    if signed_distance >= 0.0 {
        return [0.0; 3];
    }

    let depth = -signed_distance;
    // Only inward motion adds damping. Withdrawing the tool is not resisted.
    let inward_speed = (-dot(velocity, surface.normal)).max(0.0);
    let magnitude = (tissue.stiffness_n_per_m * depth + tissue.damping_ns_per_m * inward_speed)
        .min(max_force_n);
    surface.normal.map(|n| n * magnitude)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> Surface {
        // The tissue occupies z < 0. The normal points up.
        Surface::new([0.0; 3], [0.0, 0.0, 1.0]).expect("valid")
    }

    fn soft() -> Tissue {
        Tissue::new(1_000.0, 10.0).expect("valid")
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn no_force_above_the_surface() {
        let f = contact_force(&floor(), &soft(), [0.0, 0.0, 0.001], [0.0; 3], 10.0);
        assert!(close(f, [0.0; 3]));
    }

    #[test]
    fn spring_force_is_proportional_to_depth() {
        // 1 mm deep at 1000 N/m is 1 N, pushing up along the normal.
        let f = contact_force(&floor(), &soft(), [0.0, 0.0, -0.001], [0.0; 3], 10.0);
        assert!(close(f, [0.0, 0.0, 1.0]), "{f:?}");
    }

    #[test]
    fn pushing_in_adds_damping() {
        // Moving down at 0.1 m/s adds 10 * 0.1 = 1 N on top of the 1 N spring.
        let f = contact_force(
            &floor(),
            &soft(),
            [0.0, 0.0, -0.001],
            [0.0, 0.0, -0.1],
            10.0,
        );
        assert!(close(f, [0.0, 0.0, 2.0]), "{f:?}");
    }

    #[test]
    fn withdrawing_is_not_damped() {
        let f = contact_force(&floor(), &soft(), [0.0, 0.0, -0.001], [0.0, 0.0, 0.5], 10.0);
        assert!(close(f, [0.0, 0.0, 1.0]), "{f:?}");
    }

    #[test]
    fn force_is_clamped_to_the_maximum() {
        let f = contact_force(&floor(), &soft(), [0.0, 0.0, -0.1], [0.0; 3], 5.0);
        assert!(close(f, [0.0, 0.0, 5.0]), "{f:?}");
    }

    #[test]
    fn normal_length_does_not_matter() {
        let scaled = Surface::new([0.0; 3], [0.0, 0.0, 7.0]).expect("valid");
        let a = contact_force(&floor(), &soft(), [0.0, 0.0, -0.002], [0.0; 3], 10.0);
        let b = contact_force(&scaled, &soft(), [0.0, 0.0, -0.002], [0.0; 3], 10.0);
        assert!(close(a, b));
    }

    #[test]
    fn invalid_surface_and_tissue_are_refused() {
        assert!(Surface::new([0.0; 3], [0.0; 3]).is_none(), "zero normal");
        assert!(Surface::new([f64::NAN, 0.0, 0.0], [0.0, 0.0, 1.0]).is_none());
        assert!(Tissue::new(-1.0, 0.0).is_none());
        assert!(Tissue::new(1.0, f64::INFINITY).is_none());
    }
}
