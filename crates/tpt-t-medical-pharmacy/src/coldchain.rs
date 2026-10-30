//! Refrigerated storage monitoring (spec.txt §5.3).
//!
//! A short excursion, such as a door left open for a moment, is not an alarm.
//! An excursion that lasts past the grace period is. A reading that is not a
//! finite number counts as out of range, because a faulty sensor must not look
//! like a healthy fridge.

/// Raised when an excursion has lasted longer than the grace period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alert {
    /// When the excursion began, in microseconds on the monitor's clock.
    pub since_us: u64,
}

/// Monitors one refrigerated unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColdChain {
    min_c: f64,
    max_c: f64,
    grace_us: u64,
    excursion_start_us: Option<u64>,
}

impl ColdChain {
    /// Creates a monitor for the range `min_c..=max_c`, with `grace_us` of
    /// tolerance before an excursion becomes an alert.
    ///
    /// Returns `None` if the bounds are not finite or `min_c > max_c`.
    #[must_use]
    pub fn new(min_c: f64, max_c: f64, grace_us: u64) -> Option<Self> {
        let valid = min_c.is_finite() && max_c.is_finite() && min_c <= max_c;
        valid.then_some(Self {
            min_c,
            max_c,
            grace_us,
            excursion_start_us: None,
        })
    }

    /// Records one temperature reading, taken at `now_us`.
    ///
    /// Returns an alert on every reading while an excursion continues past the
    /// grace period. Returns `None` while the unit is in range, or while the
    /// excursion is still within the grace period.
    pub fn record(&mut self, temp_c: f64, now_us: u64) -> Option<Alert> {
        let in_range = temp_c.is_finite() && temp_c >= self.min_c && temp_c <= self.max_c;
        if in_range {
            self.excursion_start_us = None;
            return None;
        }

        let since_us = *self.excursion_start_us.get_or_insert(now_us);
        // A clock that moves backwards gives a zero-length excursion, never a
        // negative one.
        let elapsed = now_us.saturating_sub(since_us);
        (elapsed > self.grace_us).then_some(Alert { since_us })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRACE_US: u64 = 60_000_000; // one minute

    fn fridge() -> ColdChain {
        ColdChain::new(2.0, 8.0, GRACE_US).expect("valid")
    }

    #[test]
    fn in_range_readings_never_alert() {
        let mut f = fridge();
        for t in 0..10 {
            assert_eq!(f.record(4.0, t * 1_000_000), None);
        }
    }

    #[test]
    fn short_excursion_within_grace_is_not_an_alarm() {
        let mut f = fridge();
        assert_eq!(f.record(9.5, 0), None);
        assert_eq!(
            f.record(9.5, GRACE_US),
            None,
            "exactly at grace is still within it"
        );
        assert_eq!(f.record(4.0, GRACE_US + 1), None, "recovered in time");
    }

    #[test]
    fn excursion_past_grace_alerts_with_its_start_time() {
        let mut f = fridge();
        f.record(9.0, 1_000);
        let alert = f.record(9.0, 1_000 + GRACE_US + 1).expect("alert");
        assert_eq!(alert, Alert { since_us: 1_000 });
    }

    #[test]
    fn recovery_resets_the_timer() {
        let mut f = fridge();
        f.record(9.0, 0);
        f.record(4.0, GRACE_US / 2);
        // A new excursion starts from here, so it does not alert yet.
        assert_eq!(f.record(9.0, GRACE_US), None);
    }

    #[test]
    fn non_finite_reading_counts_as_out_of_range() {
        let mut f = fridge();
        f.record(f64::NAN, 0);
        assert!(f.record(f64::NAN, GRACE_US + 1).is_some());
    }

    #[test]
    fn clock_going_backwards_does_not_panic_or_alert_early() {
        let mut f = fridge();
        f.record(9.0, 5_000_000);
        assert_eq!(f.record(9.0, 1_000_000), None);
    }

    #[test]
    fn invalid_bounds_are_refused() {
        assert!(ColdChain::new(8.0, 2.0, GRACE_US).is_none());
        assert!(ColdChain::new(f64::NAN, 8.0, GRACE_US).is_none());
    }
}
