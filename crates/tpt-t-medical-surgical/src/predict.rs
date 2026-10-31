//! Predictive display: the tool position shown on the surgeon's screen (spec.txt
//! §4.6).
//!
//! The remote robot reports its position late, by the network delay. The display
//! extrapolates from the last report, using the velocity between the last two
//! reports. That hides jitter of up to the horizon (50 ms by default).
//!
//! When a new report disagrees with the prediction, the display does not jump.
//! The gap is kept as an offset, and the offset decays exponentially toward the
//! reported track.
//!
//! Extrapolation stops at the horizon. A long gap in the stream holds the
//! display at the horizon point instead of carrying it further, so a stalled
//! link cannot drift the image.

/// Predicts and smooths a tool position from delayed reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Predictor {
    tau_us: u64,
    horizon_us: u64,
    anchor: Option<Anchor>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Anchor {
    at_us: u64,
    position_m: [f64; 3],
    velocity_m_s: [f64; 3],
    offset_m: [f64; 3],
}

impl Predictor {
    /// Default extrapolation horizon, in microseconds (spec.txt §4.6: 50 ms).
    pub const DEFAULT_HORIZON_US: u64 = 50_000;

    /// Creates a predictor. `tau_us` is the time constant with which a
    /// misprediction decays. `horizon_us` is the most the display extrapolates.
    /// Returns `None` if either is zero.
    #[must_use]
    pub const fn new(tau_us: u64, horizon_us: u64) -> Option<Self> {
        if tau_us == 0 || horizon_us == 0 {
            return None;
        }
        Some(Self {
            tau_us,
            horizon_us,
            anchor: None,
        })
    }

    /// Takes one report of the tool's position, measured at `at_us`.
    ///
    /// A report not later than the last one is ignored, and `false` is returned.
    /// A report with a non-finite coordinate is also ignored.
    #[allow(clippy::cast_precision_loss)]
    pub fn observe(&mut self, at_us: u64, position_m: [f64; 3]) -> bool {
        if !position_m.iter().all(|v| v.is_finite()) {
            return false;
        }
        match self.anchor {
            None => {
                self.anchor = Some(Anchor {
                    at_us,
                    position_m,
                    velocity_m_s: [0.0; 3],
                    offset_m: [0.0; 3],
                });
                true
            }
            Some(prev) => {
                let Some(elapsed_us) = at_us.checked_sub(prev.at_us).filter(|&e| e > 0) else {
                    return false;
                };
                // What was on screen at the moment of this report. The new track
                // starts from there, so the image does not jump.
                let shown = self.display(at_us).unwrap_or(position_m);
                let secs = elapsed_us as f64 / 1e6;
                let velocity_m_s = [0, 1, 2].map(|i| (position_m[i] - prev.position_m[i]) / secs);
                self.anchor = Some(Anchor {
                    at_us,
                    position_m,
                    velocity_m_s,
                    offset_m: [0, 1, 2].map(|i| shown[i] - position_m[i]),
                });
                true
            }
        }
    }

    /// Returns the position to show at `now_us`, or `None` before the first
    /// report.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn display(&self, now_us: u64) -> Option<[f64; 3]> {
        let a = self.anchor?;
        let since_report = now_us.saturating_sub(a.at_us);
        let extrapolated_us = since_report.min(self.horizon_us);
        let secs = extrapolated_us as f64 / 1e6;
        // The offset decays from the time of the last report.
        let decay = (-(since_report as f64) / self.tau_us as f64).exp();
        Some([0, 1, 2].map(|i| a.position_m[i] + a.velocity_m_s[i] * secs + a.offset_m[i] * decay))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000;

    fn predictor() -> Predictor {
        Predictor::new(20 * MS, Predictor::DEFAULT_HORIZON_US).expect("valid")
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn nothing_is_shown_before_the_first_report() {
        assert_eq!(predictor().display(0), None);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn a_steady_motion_is_predicted_within_the_horizon() {
        // The tool moves at 0.1 m/s along x, reported every 10 ms.
        let mut p = predictor();
        for k in 0..=10u64 {
            let t = k * 10 * MS;
            p.observe(t, [0.1 * t as f64 / 1e6, 0.0, 0.0]);
        }
        // 30 ms after the last report at 100 ms, the display leads by 3 mm. The
        // start-up offset has decayed to a few micrometres by then.
        let shown = p.display(130 * MS).expect("shown");
        assert!((shown[0] - 0.013).abs() < 1e-5, "{shown:?}");
    }

    #[test]
    fn extrapolation_stops_at_the_horizon() {
        let mut p = predictor();
        p.observe(0, [0.0; 3]);
        p.observe(10 * MS, [0.01, 0.0, 0.0]);
        // Once the start-up offset has decayed, a long gap holds the display at
        // the horizon point: 10 mm plus 50 ms at the 1 m/s that the report implies.
        let settled = p.display(10 * MS + 1_000_000).expect("settled");
        let much_later = p.display(10 * MS + 10_000_000).expect("much later");
        assert!((settled[0] - 0.06).abs() < 1e-9, "{settled:?}");
        assert!(
            (settled[0] - much_later[0]).abs() < 1e-9,
            "{settled:?} vs {much_later:?}"
        );
    }

    #[test]
    fn a_misprediction_does_not_jump_the_image() {
        let mut p = predictor();
        p.observe(0, [0.0; 3]);
        p.observe(10 * MS, [0.01, 0.0, 0.0]);
        let before = p.display(40 * MS).expect("before");
        // The next report is 5 mm off the predicted track.
        p.observe(40 * MS, [before[0] + 0.005, 0.0, 0.0]);
        let after = p.display(40 * MS).expect("after");
        assert!(close(before, after), "{before:?} vs {after:?}");
    }

    #[test]
    fn the_offset_decays_back_to_the_reported_track() {
        let mut p = predictor();
        p.observe(0, [0.0; 3]);
        p.observe(10 * MS, [0.0; 3]);
        // Report a 1 cm jump, then report it again so the velocity settles.
        p.observe(20 * MS, [0.01, 0.0, 0.0]);
        p.observe(30 * MS, [0.01, 0.0, 0.0]);
        let soon = p.display(35 * MS).expect("soon")[0];
        let later = p.display(230 * MS).expect("later")[0];
        assert!((later - 0.01).abs() < 1e-4, "settled at {later}");
        assert!(soon > later, "no decay: {soon} then {later}");
    }

    #[test]
    fn stale_and_non_finite_reports_are_ignored() {
        let mut p = predictor();
        assert!(p.observe(10 * MS, [0.0; 3]));
        assert!(!p.observe(10 * MS, [1.0, 0.0, 0.0]));
        assert!(!p.observe(5 * MS, [1.0, 0.0, 0.0]));
        assert!(!p.observe(20 * MS, [f64::NAN, 0.0, 0.0]));
        assert_eq!(p.display(10 * MS), Some([0.0; 3]));
    }

    #[test]
    fn zero_time_constant_or_horizon_is_refused() {
        assert!(Predictor::new(0, 1).is_none());
        assert!(Predictor::new(1, 0).is_none());
    }
}
