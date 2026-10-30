//! Progressive force for virtual fixtures (spec.txt §4.3).
//!
//! Outside a margin around a forbidden structure, there is no force. Inside
//! the margin, the force rises linearly as the tool approaches, and it reaches
//! the maximum at the surface and beyond.

/// The repulsive force at a signed distance, in newtons.
///
/// `distance_m` is negative inside the structure. Returns `None` unless
/// `margin_m` and `max_force_n` are positive and finite.
///
/// A NaN distance returns the maximum force. An unknown distance must not be
/// treated as safe.
#[must_use]
pub fn repulsive_force(distance_m: f64, margin_m: f64, max_force_n: f64) -> Option<f64> {
    let valid =
        margin_m.is_finite() && margin_m > 0.0 && max_force_n.is_finite() && max_force_n > 0.0;
    if !valid {
        return None;
    }
    if distance_m.is_nan() {
        return Some(max_force_n);
    }
    let depth = ((margin_m - distance_m) / margin_m).clamp(0.0, 1.0);
    Some(max_force_n * depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARGIN: f64 = 0.01;
    const MAX: f64 = 2.0;

    #[test]
    fn no_force_outside_the_margin() {
        assert_eq!(repulsive_force(0.02, MARGIN, MAX), Some(0.0));
        assert_eq!(repulsive_force(MARGIN, MARGIN, MAX), Some(0.0));
    }

    #[test]
    fn force_rises_linearly_inside_the_margin() {
        let half = repulsive_force(0.005, MARGIN, MAX).expect("valid");
        assert!((half - 1.0).abs() < 1e-12);
    }

    #[test]
    fn maximum_at_the_surface_and_beyond() {
        assert_eq!(repulsive_force(0.0, MARGIN, MAX), Some(MAX));
        assert_eq!(repulsive_force(-0.05, MARGIN, MAX), Some(MAX));
    }

    #[test]
    fn force_never_decreases_as_the_tool_approaches() {
        let mut previous = 0.0;
        for step in 0..=100 {
            let d = MARGIN - f64::from(step) * MARGIN / 100.0;
            let f = repulsive_force(d, MARGIN, MAX).expect("valid");
            assert!(f >= previous, "force dropped at distance {d}");
            previous = f;
        }
    }

    #[test]
    fn unknown_distance_gives_maximum_force() {
        assert_eq!(repulsive_force(f64::NAN, MARGIN, MAX), Some(MAX));
    }

    #[test]
    fn invalid_parameters_are_refused() {
        assert!(repulsive_force(0.0, 0.0, MAX).is_none());
        assert!(repulsive_force(0.0, MARGIN, -1.0).is_none());
        assert!(repulsive_force(0.0, f64::NAN, MAX).is_none());
    }
}
