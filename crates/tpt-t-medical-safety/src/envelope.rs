//! The workspace envelope: a box the tool must stay inside (spec.txt §5.5).

/// An axis-aligned box, in metres, in the robot base frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Envelope {
    min: [f64; 3],
    max: [f64; 3],
}

impl Envelope {
    /// Creates an envelope from its corners.
    ///
    /// Returns `None` if any `min` component is greater than the matching
    /// `max` component, or if any bound is not finite. A bad envelope is
    /// refused at construction, not discovered during surgery.
    #[must_use]
    pub fn new(min: [f64; 3], max: [f64; 3]) -> Option<Self> {
        let finite = min.iter().chain(max.iter()).all(|v| v.is_finite());
        let ordered = min.iter().zip(max.iter()).all(|(lo, hi)| lo <= hi);
        (finite && ordered).then_some(Self { min, max })
    }

    /// Returns `true` if `point` lies inside the envelope, bounds included.
    ///
    /// A point with any non-finite component is outside. A NaN coordinate must
    /// never pass as "inside".
    #[must_use]
    pub fn contains(&self, point: [f64; 3]) -> bool {
        point
            .iter()
            .zip(self.min.iter().zip(self.max.iter()))
            .all(|(p, (lo, hi))| *p >= *lo && *p <= *hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_1m() -> Envelope {
        Envelope::new([-0.5, -0.5, 0.0], [0.5, 0.5, 1.0]).expect("valid box")
    }

    #[test]
    fn interior_point_is_inside() {
        assert!(box_1m().contains([0.0, 0.0, 0.5]));
    }

    #[test]
    fn boundary_points_are_inside() {
        let env = box_1m();
        assert!(env.contains([0.5, -0.5, 1.0]));
        assert!(env.contains([-0.5, 0.5, 0.0]));
    }

    #[test]
    fn points_just_outside_any_face_are_outside() {
        let env = box_1m();
        assert!(!env.contains([0.5001, 0.0, 0.5]));
        assert!(!env.contains([0.0, -0.5001, 0.5]));
        assert!(!env.contains([0.0, 0.0, -0.0001]));
        assert!(!env.contains([0.0, 0.0, 1.0001]));
    }

    #[test]
    fn non_finite_coordinates_are_outside() {
        let env = box_1m();
        assert!(!env.contains([f64::NAN, 0.0, 0.5]));
        assert!(!env.contains([0.0, f64::INFINITY, 0.5]));
        assert!(!env.contains([0.0, 0.0, f64::NEG_INFINITY]));
    }

    #[test]
    fn inverted_or_non_finite_bounds_are_refused() {
        assert!(Envelope::new([1.0, 0.0, 0.0], [0.0, 1.0, 1.0]).is_none());
        assert!(Envelope::new([f64::NAN, 0.0, 0.0], [1.0, 1.0, 1.0]).is_none());
        assert!(Envelope::new([0.0; 3], [f64::INFINITY, 1.0, 1.0]).is_none());
    }
}
