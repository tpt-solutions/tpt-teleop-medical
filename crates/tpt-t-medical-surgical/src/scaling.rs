//! Motion scaling between the master controller and the tool (spec.txt §5.1).

/// Divides master motion by a ratio. At 10:1, a 10 cm hand movement becomes a
/// 1 cm tool movement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionScaler {
    ratio: f64,
}

impl MotionScaler {
    /// Lowest ratio: 1:1, for microsurgery.
    pub const MIN_RATIO: f64 = 1.0;
    /// Highest ratio: 10:1, for gross dissection.
    pub const MAX_RATIO: f64 = 10.0;

    /// Creates a scaler with a ratio from 1.0 to 10.0 inclusive.
    ///
    /// Returns `None` outside that range, or if the ratio is not finite.
    #[must_use]
    pub fn new(ratio: f64) -> Option<Self> {
        (ratio.is_finite() && (Self::MIN_RATIO..=Self::MAX_RATIO).contains(&ratio))
            .then_some(Self { ratio })
    }

    /// Returns the ratio.
    #[must_use]
    pub const fn ratio(&self) -> f64 {
        self.ratio
    }

    /// Scales a master displacement down to a tool displacement.
    #[must_use]
    pub fn scale(&self, master: [f64; 3]) -> [f64; 3] {
        master.map(|v| v / self.ratio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_centimetres_becomes_one_at_ten_to_one() {
        let s = MotionScaler::new(10.0).expect("in range");
        let tool = s.scale([0.1, 0.0, -0.1]);
        assert!((tool[0] - 0.01).abs() < 1e-12);
        assert!((tool[2] + 0.01).abs() < 1e-12);
    }

    #[test]
    fn one_to_one_is_identity() {
        let s = MotionScaler::new(1.0).expect("in range");
        let tool = s.scale([0.3, -0.2, 0.1]);
        for (got, want) in tool.iter().zip([0.3, -0.2, 0.1]) {
            assert!((got - want).abs() < 1e-12, "got {got}, want {want}");
        }
    }

    #[test]
    fn ratios_outside_the_range_are_refused() {
        assert!(MotionScaler::new(0.5).is_none());
        assert!(MotionScaler::new(10.5).is_none());
        assert!(MotionScaler::new(f64::NAN).is_none());
        assert!(MotionScaler::new(f64::INFINITY).is_none());
    }
}
