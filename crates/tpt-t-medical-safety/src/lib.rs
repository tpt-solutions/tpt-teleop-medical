//! Safety monitor for tpt-teleop-medical (spec.txt §4.1, §5.5).
//!
//! The monitor takes a [`Sample`] of the tool's state and returns a
//! [`Decision`]. The rules:
//!
//! * The tool must stay inside the workspace [`Envelope`].
//! * The force on the tool must not exceed the limit. A NaN or infinite
//!   reading counts as a violation.
//! * The monitor must be evaluated at least once per watchdog timeout. A longer
//!   gap, or a clock that moves backwards, is a violation.
//! * Any stop is latched. Only an explicit [`Monitor::reset`] clears it.
//!
//! The monitor allocates nothing, and it reads no clock. Time comes in on each
//! sample.
//!
//! Not verified here: the spec's timing targets (<100 µs disagreement response,
//! <10 ms E-stop), and the requirement that the three copies run on isolated
//! cores. Those need the real-time target.

#![forbid(unsafe_code)]

pub mod envelope;
pub mod tmr;

pub use envelope::Envelope;
pub use tmr::{Tmr, vote};

/// Limits the monitor enforces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    envelope: Envelope,
    max_force_n: f64,
    watchdog_timeout_us: u64,
}

impl Limits {
    /// Returns the longest gap allowed between two evaluations, in
    /// microseconds.
    #[must_use]
    pub const fn watchdog_timeout_us(&self) -> u64 {
        self.watchdog_timeout_us
    }

    /// Creates limits.
    ///
    /// Returns `None` if `max_force_n` is negative or not finite. A limit that
    /// cannot be enforced is refused, not silently ignored.
    #[must_use]
    pub fn new(envelope: Envelope, max_force_n: f64, watchdog_timeout_us: u64) -> Option<Self> {
        (max_force_n.is_finite() && max_force_n >= 0.0).then_some(Self {
            envelope,
            max_force_n,
            watchdog_timeout_us,
        })
    }
}

/// One reading of the tool's state, taken at `now_us`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Monotonic time of the reading, in microseconds.
    pub now_us: u64,
    /// Tool tip position in the robot base frame, in metres.
    pub tool_position_m: [f64; 3],
    /// Force on the tool, in newtons.
    pub tool_force_n: [f64; 3],
}

/// Where an emergency stop came from. Each source is an independent path
/// (spec.txt §5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EStopSource {
    /// The surgeon's foot pedal.
    FootPedal,
    /// The hand switch on the master controller.
    HandSwitch,
    /// The voice command.
    Voice,
    /// The automatic stop from the safety monitor itself.
    Automatic,
}

/// Why the monitor stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The three copies disagreed.
    Disagreement,
    /// The tool left the workspace envelope.
    OutsideEnvelope,
    /// The force on the tool exceeded the limit, or was not a finite number.
    ForceExceeded,
    /// The monitor was not evaluated in time, or the clock moved backwards.
    WatchdogExpired,
    /// An emergency stop was requested.
    EStop(EStopSource),
}

/// The monitor's output for one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Motion may continue.
    Run,
    /// The system must go to its safe state.
    Stop(StopReason),
}

/// One copy of the safety monitor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    limits: Limits,
    last_us: Option<u64>,
    latched: Option<StopReason>,
}

impl Monitor {
    /// Creates a monitor that has not yet seen a sample.
    #[must_use]
    pub const fn new(limits: Limits) -> Self {
        Self {
            limits,
            last_us: None,
            latched: None,
        }
    }

    /// Checks one sample.
    ///
    /// If the monitor is already latched, it returns the latched stop without
    /// looking at the sample.
    pub fn evaluate(&mut self, sample: Sample) -> Decision {
        if let Some(reason) = self.latched {
            return Decision::Stop(reason);
        }
        let violation = self.watchdog(sample.now_us).or_else(|| self.check(sample));
        self.last_us = Some(sample.now_us);
        match violation {
            Some(reason) => {
                self.latched = Some(reason);
                Decision::Stop(reason)
            }
            None => Decision::Run,
        }
    }

    /// Latches a stop. The next [`Monitor::evaluate`] returns it until reset.
    pub fn emergency_stop(&mut self, source: EStopSource) {
        self.latch(StopReason::EStop(source));
    }

    /// Clears the latch. The caller must have confirmed with the operator that
    /// the stop is cleared.
    pub fn reset(&mut self) {
        self.latched = None;
    }

    /// Returns the latched stop, if any.
    #[must_use]
    pub const fn latched(&self) -> Option<StopReason> {
        self.latched
    }

    pub(crate) fn latch(&mut self, reason: StopReason) {
        if self.latched.is_none() {
            self.latched = Some(reason);
        }
    }

    fn watchdog(&self, now_us: u64) -> Option<StopReason> {
        let last = self.last_us?;
        match now_us.checked_sub(last) {
            Some(gap) if gap <= self.limits.watchdog_timeout_us => None,
            _ => Some(StopReason::WatchdogExpired),
        }
    }

    fn check(&self, sample: Sample) -> Option<StopReason> {
        if !self.limits.envelope.contains(sample.tool_position_m) {
            return Some(StopReason::OutsideEnvelope);
        }
        // Compare squared magnitudes to avoid a square root. A NaN reading is
        // refused explicitly, because a NaN comparison is false and would pass.
        let [fx, fy, fz] = sample.tool_force_n;
        let force_sq = fx * fx + fy * fy + fz * fz;
        let limit = self.limits.max_force_n;
        if force_sq.is_nan() || force_sq > limit * limit {
            return Some(StopReason::ForceExceeded);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> Envelope {
        Envelope::new([-0.1; 3], [0.1; 3]).expect("valid box")
    }

    fn limits() -> Limits {
        Limits::new(envelope(), 5.0, 10_000).expect("valid limits")
    }

    fn sample(now_us: u64) -> Sample {
        Sample {
            now_us,
            tool_position_m: [0.0; 3],
            tool_force_n: [0.0; 3],
        }
    }

    #[test]
    fn nominal_samples_run() {
        let mut m = Monitor::new(limits());
        assert_eq!(m.evaluate(sample(0)), Decision::Run);
        assert_eq!(m.evaluate(sample(5_000)), Decision::Run);
    }

    #[test]
    fn force_at_the_limit_is_allowed_and_above_it_is_not() {
        // [3, 4, 0] has magnitude exactly 5.
        let at_limit = Sample {
            tool_force_n: [3.0, 4.0, 0.0],
            ..sample(0)
        };
        assert_eq!(Monitor::new(limits()).evaluate(at_limit), Decision::Run);

        let over = Sample {
            tool_force_n: [3.0, 4.0, 0.1],
            ..sample(0)
        };
        assert_eq!(
            Monitor::new(limits()).evaluate(over),
            Decision::Stop(StopReason::ForceExceeded)
        );
    }

    #[test]
    fn non_finite_force_is_a_violation() {
        for bad in [f64::NAN, f64::INFINITY] {
            let s = Sample {
                tool_force_n: [bad, 0.0, 0.0],
                ..sample(0)
            };
            assert_eq!(
                Monitor::new(limits()).evaluate(s),
                Decision::Stop(StopReason::ForceExceeded),
                "force component {bad}"
            );
        }
    }

    #[test]
    fn leaving_the_envelope_stops_and_stays_latched() {
        let mut m = Monitor::new(limits());
        let outside = Sample {
            tool_position_m: [0.2, 0.0, 0.0],
            ..sample(0)
        };
        assert_eq!(
            m.evaluate(outside),
            Decision::Stop(StopReason::OutsideEnvelope)
        );
        // Back inside, but still latched.
        assert_eq!(
            m.evaluate(sample(1_000)),
            Decision::Stop(StopReason::OutsideEnvelope)
        );
    }

    #[test]
    fn missed_evaluation_past_the_timeout_expires_the_watchdog() {
        let mut m = Monitor::new(limits());
        assert_eq!(m.evaluate(sample(0)), Decision::Run);
        assert_eq!(
            m.evaluate(sample(10_001)),
            Decision::Stop(StopReason::WatchdogExpired)
        );
    }

    #[test]
    fn gap_exactly_at_the_timeout_is_allowed() {
        let mut m = Monitor::new(limits());
        assert_eq!(m.evaluate(sample(0)), Decision::Run);
        assert_eq!(m.evaluate(sample(10_000)), Decision::Run);
    }

    #[test]
    fn clock_moving_backwards_expires_the_watchdog() {
        let mut m = Monitor::new(limits());
        assert_eq!(m.evaluate(sample(5_000)), Decision::Run);
        assert_eq!(
            m.evaluate(sample(1_000)),
            Decision::Stop(StopReason::WatchdogExpired)
        );
    }

    #[test]
    fn emergency_stop_latches_until_reset() {
        let mut m = Monitor::new(limits());
        m.emergency_stop(EStopSource::HandSwitch);
        assert_eq!(
            m.evaluate(sample(0)),
            Decision::Stop(StopReason::EStop(EStopSource::HandSwitch))
        );
        m.reset();
        assert_eq!(m.latched(), None);
        assert_eq!(m.evaluate(sample(1_000)), Decision::Run);
    }

    #[test]
    fn first_stop_reason_is_kept_when_a_second_arrives() {
        let mut m = Monitor::new(limits());
        m.emergency_stop(EStopSource::FootPedal);
        m.emergency_stop(EStopSource::Voice);
        assert_eq!(m.latched(), Some(StopReason::EStop(EStopSource::FootPedal)));
    }

    #[test]
    fn limits_refuse_a_negative_or_non_finite_force_limit() {
        assert!(Limits::new(envelope(), -1.0, 10_000).is_none());
        assert!(Limits::new(envelope(), f64::NAN, 10_000).is_none());
        assert!(Limits::new(envelope(), f64::INFINITY, 10_000).is_none());
        assert!(Limits::new(envelope(), 0.0, 10_000).is_some());
    }
}
