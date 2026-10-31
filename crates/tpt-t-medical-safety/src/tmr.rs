//! Triple-modular redundancy: three copies of the monitor vote on every
//! decision (spec.txt §4.1).
//!
//! The copies must agree exactly. Two-of-three is not enough: if one copy
//! disagrees, something is wrong with the hardware or the code, and the only
//! safe answer is to stop. Disagreement latches a stop in all three copies.

use crate::{Decision, Limits, Monitor, Sample, StopReason};

/// Returns the common value if all three agree, or `None` otherwise.
#[must_use]
pub fn vote<T: PartialEq + Copy>(a: T, b: T, c: T) -> Option<T> {
    (a == b && b == c).then_some(a)
}

/// Three independent monitors.
///
/// In this crate the three run in sequence on one thread. The spec puts each
/// on its own isolated core, which needs the real-time target to build and
/// measure.
#[derive(Debug, Clone, PartialEq)]
pub struct Tmr {
    monitors: [Monitor; 3],
}

impl Tmr {
    /// Creates three monitors with the same limits.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Self {
            monitors: [
                Monitor::new(limits),
                Monitor::new(limits),
                Monitor::new(limits),
            ],
        }
    }

    /// Creates a system from three monitors. Used by the tests to inject a
    /// fault into one copy.
    #[must_use]
    pub const fn from_monitors(monitors: [Monitor; 3]) -> Self {
        Self { monitors }
    }

    /// Evaluates the sample on all three copies and votes on the result.
    ///
    /// On disagreement, all three copies latch [`StopReason::Disagreement`].
    pub fn evaluate(&mut self, sample: Sample) -> Decision {
        let [a, b, c] = &mut self.monitors;
        let decisions = [a.evaluate(sample), b.evaluate(sample), c.evaluate(sample)];
        if let Some(decision) = vote(decisions[0], decisions[1], decisions[2]) {
            return decision;
        }
        for monitor in &mut self.monitors {
            monitor.latch(StopReason::Disagreement);
        }
        Decision::Stop(StopReason::Disagreement)
    }

    /// Returns the reason the system is latched, if it is. Every stop reaches
    /// all three copies, so the first copy speaks for the system.
    #[must_use]
    pub const fn latched(&self) -> Option<StopReason> {
        self.monitors[0].latched()
    }

    /// Latches an emergency stop in all three copies.
    pub fn emergency_stop(&mut self, source: crate::EStopSource) {
        for monitor in &mut self.monitors {
            monitor.latch(StopReason::EStop(source));
        }
    }

    /// Clears the latch in all three copies. The caller must have confirmed
    /// with the operator that the stop is cleared.
    pub fn reset(&mut self) {
        for monitor in &mut self.monitors {
            monitor.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EStopSource, Envelope};

    fn limits(max_force_n: f64) -> Limits {
        Limits::new(
            Envelope::new([-1.0; 3], [1.0; 3]).expect("valid"),
            max_force_n,
            10_000,
        )
        .expect("valid limits")
    }

    fn sample(now_us: u64, force_n: f64) -> Sample {
        Sample {
            now_us,
            tool_position_m: [0.0; 3],
            tool_force_n: [force_n, 0.0, 0.0],
        }
    }

    #[test]
    fn vote_requires_unanimity() {
        assert_eq!(vote(1, 1, 1), Some(1));
        assert_eq!(vote(1, 1, 2), None);
        assert_eq!(vote(1, 2, 1), None);
        assert_eq!(vote(2, 1, 1), None);
    }

    #[test]
    fn healthy_copies_run() {
        let mut tmr = Tmr::new(limits(5.0));
        assert_eq!(tmr.evaluate(sample(0, 1.0)), Decision::Run);
        assert_eq!(tmr.evaluate(sample(1_000, 1.0)), Decision::Run);
    }

    #[test]
    fn one_faulty_copy_stops_the_system_and_latches_all_three() {
        // The third copy has a tighter force limit, so it alone sees 4 N as a
        // violation. The copies disagree, so the system stops.
        let mut tmr = Tmr::from_monitors([
            Monitor::new(limits(5.0)),
            Monitor::new(limits(5.0)),
            Monitor::new(limits(3.0)),
        ]);
        assert_eq!(
            tmr.evaluate(sample(0, 4.0)),
            Decision::Stop(StopReason::Disagreement)
        );
        // Latched: a safe sample afterwards still stops.
        assert_eq!(
            tmr.evaluate(sample(1_000, 0.0)),
            Decision::Stop(StopReason::Disagreement)
        );
    }

    #[test]
    fn emergency_stop_latches_every_copy_until_reset() {
        let mut tmr = Tmr::new(limits(5.0));
        tmr.emergency_stop(EStopSource::FootPedal);
        assert_eq!(
            tmr.evaluate(sample(0, 0.0)),
            Decision::Stop(StopReason::EStop(EStopSource::FootPedal))
        );
        tmr.reset();
        assert_eq!(tmr.evaluate(sample(1_000, 0.0)), Decision::Run);
    }
}
