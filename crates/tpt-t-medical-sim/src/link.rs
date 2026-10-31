//! Command latency and jitter for the network between surgeon and arm.
//!
//! A command sent at one step arrives `latency` steps later, plus a jitter of up
//! to `jitter` steps. The jitter comes from a fixed integer hash of the send
//! step, so a run is repeatable. It is not a random number generator.
//!
//! Commands never overtake one another. If a later send would arrive before an
//! earlier one, it waits for the earlier one. A receiver that asks late gets the
//! newest command that has arrived, and the older ones are dropped.

use std::collections::VecDeque;

use crate::JOINTS;

/// Most commands held in flight at once.
pub const MAX_IN_FLIGHT: usize = 4_096;

/// A one-way command link with latency and jitter, in simulation steps.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    latency_steps: u64,
    jitter_steps: u64,
    in_flight: VecDeque<(u64, [f64; JOINTS])>,
    last_due: u64,
}

impl Link {
    /// Creates a link. Returns `None` if the total delay is more than
    /// `u32::MAX` steps, which no real link would have.
    #[must_use]
    pub fn new(latency_steps: u64, jitter_steps: u64) -> Option<Self> {
        let total = latency_steps.checked_add(jitter_steps)?;
        u32::try_from(total).is_ok().then_some(Self {
            latency_steps,
            jitter_steps,
            in_flight: VecDeque::new(),
            last_due: 0,
        })
    }

    /// Sends a command at `now_step`. Returns `false`, and drops it, if the
    /// link is full.
    pub fn send(&mut self, now_step: u64, command: [f64; JOINTS]) -> bool {
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            return false;
        }
        let wobble = hash(now_step) % (self.jitter_steps + 1);
        let due = (now_step + self.latency_steps + wobble).max(self.last_due);
        self.last_due = due;
        self.in_flight.push_back((due, command));
        true
    }

    /// Returns the newest command that has arrived by `now_step`, if any. The
    /// older arrived commands are dropped.
    pub fn receive(&mut self, now_step: u64) -> Option<[f64; JOINTS]> {
        let mut newest = None;
        while self
            .in_flight
            .front()
            .is_some_and(|(due, _)| *due <= now_step)
        {
            newest = self.in_flight.pop_front().map(|(_, c)| c);
        }
        newest
    }

    /// Returns how many commands are still in flight.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }
}

/// A fixed 64-bit mix (the `SplitMix64` finaliser). It only spreads the jitter, so
/// it needs no more than that.
fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
mod tests {
    use super::*;

    fn cmd(v: f64) -> [f64; JOINTS] {
        [v; JOINTS]
    }

    #[test]
    fn with_no_jitter_a_command_arrives_after_exactly_the_latency() {
        let mut link = Link::new(5, 0).expect("link");
        link.send(0, cmd(1.0));
        assert_eq!(link.receive(4), None);
        assert_eq!(link.receive(5), Some(cmd(1.0)));
    }

    #[test]
    fn every_arrival_is_within_latency_plus_jitter_and_runs_repeat() {
        let run = || {
            let mut link = Link::new(3, 4).expect("link");
            let mut arrivals = Vec::new();
            for send in 0..200_u64 {
                link.send(send, cmd(send as f64));
                for t in send..send + 8 {
                    if let Some(c) = link.receive(t) {
                        arrivals.push((t, c[0]));
                    }
                }
            }
            arrivals
        };
        let first = run();
        assert_eq!(first, run(), "not repeatable");
        // No command arrives before its latency, or after latency plus jitter.
        for (t, value) in &first {
            let sent = *value as u64;
            assert!(*t >= sent + 3, "early: sent {sent}, arrived {t}");
            assert!(*t <= sent + 3 + 4 + 1, "late: sent {sent}, arrived {t}");
        }
    }

    #[test]
    fn commands_do_not_overtake_each_other() {
        let mut link = Link::new(2, 6).expect("link");
        for k in 0..50_u64 {
            link.send(k, cmd(k as f64));
        }
        let mut last = -1.0;
        for t in 0..80_u64 {
            if let Some(c) = link.receive(t) {
                assert!(c[0] > last, "went backwards at {t}");
                last = c[0];
            }
        }
    }

    #[test]
    fn a_late_receiver_gets_the_newest_arrived_command() {
        let mut link = Link::new(1, 0).expect("link");
        link.send(0, cmd(1.0));
        link.send(1, cmd(2.0));
        link.send(2, cmd(3.0));
        assert_eq!(link.receive(10), Some(cmd(3.0)));
        assert_eq!(link.in_flight(), 0);
    }

    #[test]
    fn a_full_link_drops_new_commands() {
        let mut link = Link::new(1_000_000, 0).expect("link");
        for k in 0..MAX_IN_FLIGHT as u64 {
            assert!(link.send(k, cmd(0.0)));
        }
        assert!(!link.send(0, cmd(0.0)));
    }

    #[test]
    fn absurd_delays_are_refused() {
        assert!(Link::new(u64::MAX, 1).is_none());
        assert!(Link::new(u64::from(u32::MAX) + 1, 0).is_none());
    }
}
