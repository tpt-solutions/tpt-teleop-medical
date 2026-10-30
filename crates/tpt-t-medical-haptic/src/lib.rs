//! 1 kHz haptic rendering for tpt-teleop-medical (spec.txt §4.2).
//!
//! [`HapticLoop::step`] runs once per tick. It takes the newest tool sample
//! from a lock-free ring, renders the contact force, and returns it. Three
//! things can go wrong, and each one holds the last force rather than
//! zeroing it:
//!
//! * **Deadline missed.** The tick took longer than [`DEADLINE_US`]. Zeroing
//!   the force would make the surgeon feel a sudden drop, so the previous force
//!   is held.
//! * **No input, or stale input.** The ring is empty, or the newest sample is
//!   older than [`STALE_US`].
//! * **Invalid input.** A sample with any non-finite value is discarded.
//!
//! The loop reads the newest sample, not the oldest. Stale samples are dropped,
//! so the surgeon never feels a force that is out of date.
//!
//! The timing claims (<50 µs execution, 1 ms deadline on a pinned core) are not
//! verified here. This crate can only check the logic that decides when to
//! hold.

#![forbid(unsafe_code)]

pub mod contact;

use tpt_t_medical_ring::SpscRing;

pub use contact::{Surface, Tissue, contact_force};

/// Tick period: 1 kHz.
pub const PERIOD_US: u64 = 1_000;
/// Longest compute time before the tick counts as a missed deadline.
pub const DEADLINE_US: u64 = 1_000;
/// Oldest sample age that is still used.
pub const STALE_US: u64 = 2_000;

/// One tool sample from the tracking or control thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HapticInput {
    /// When the sample was taken, in microseconds on the loop's clock.
    pub timestamp_us: u64,
    /// Tool tip position, in metres.
    pub position: [f64; 3],
    /// Tool tip velocity, in metres per second.
    pub velocity: [f64; 3],
}

impl HapticInput {
    fn is_finite(&self) -> bool {
        self.position
            .iter()
            .chain(self.velocity.iter())
            .all(|v| v.is_finite())
    }
}

/// How this tick's force was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A new force was rendered from a fresh sample.
    Rendered,
    /// The tick missed its deadline. The previous force is held.
    HeldDeadlineMissed,
    /// No sample was waiting. The previous force is held.
    HeldNoInput,
    /// The newest sample is older than [`STALE_US`]. The previous force is held.
    HeldStale,
    /// The newest sample contained a non-finite value. The previous force is
    /// held.
    HeldInvalid,
}

/// What one tick produced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Output {
    /// The force to command, in newtons.
    pub force: [f64; 3],
    /// Why this force was chosen.
    pub status: Status,
}

/// The per-tick haptic renderer. It owns the last force so it can hold it.
#[derive(Debug, Clone, PartialEq)]
pub struct HapticLoop {
    surface: Surface,
    tissue: Tissue,
    max_force_n: f64,
    last_force: [f64; 3],
}

impl HapticLoop {
    /// Creates a renderer. Returns `None` unless `max_force_n` is positive and
    /// finite.
    #[must_use]
    pub fn new(surface: Surface, tissue: Tissue, max_force_n: f64) -> Option<Self> {
        (max_force_n.is_finite() && max_force_n > 0.0).then_some(Self {
            surface,
            tissue,
            max_force_n,
            last_force: [0.0; 3],
        })
    }

    /// Runs one tick.
    ///
    /// `now_us` is the time at the start of the tick. `elapsed_us` is how long
    /// the tick has taken so far. The caller measures it, so the check can run
    /// before any rendering.
    pub fn step(&mut self, ring: &SpscRing<HapticInput>, now_us: u64, elapsed_us: u64) -> Output {
        if elapsed_us > DEADLINE_US {
            return self.hold(Status::HeldDeadlineMissed);
        }

        // Take everything waiting, and keep only the newest sample.
        let mut newest = None;
        while let Some(sample) = ring.pop() {
            newest = Some(sample);
        }
        let Some(sample) = newest else {
            return self.hold(Status::HeldNoInput);
        };
        if !sample.is_finite() {
            return self.hold(Status::HeldInvalid);
        }
        if now_us.saturating_sub(sample.timestamp_us) > STALE_US {
            return self.hold(Status::HeldStale);
        }

        self.last_force = contact_force(
            &self.surface,
            &self.tissue,
            sample.position,
            sample.velocity,
            self.max_force_n,
        );
        Output {
            force: self.last_force,
            status: Status::Rendered,
        }
    }

    fn hold(&self, status: Status) -> Output {
        Output {
            force: self.last_force,
            status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> HapticLoop {
        let surface = Surface::new([0.0; 3], [0.0, 0.0, 1.0]).expect("valid");
        let tissue = Tissue::new(1_000.0, 0.0).expect("valid");
        HapticLoop::new(surface, tissue, 10.0).expect("valid")
    }

    fn ring() -> SpscRing<HapticInput> {
        SpscRing::with_capacity(16)
    }

    fn sample(timestamp_us: u64, z: f64) -> HapticInput {
        HapticInput {
            timestamp_us,
            position: [0.0, 0.0, z],
            velocity: [0.0; 3],
        }
    }

    #[test]
    fn fresh_sample_renders_a_force() {
        let mut r = renderer();
        let q = ring();
        q.push(sample(1_000, -0.001)).expect("room");
        let out = r.step(&q, 1_100, 10);
        assert_eq!(out.status, Status::Rendered);
        assert!((out.force[2] - 1.0).abs() < 1e-9, "{:?}", out.force);
    }

    #[test]
    fn missed_deadline_holds_the_previous_force() {
        let mut r = renderer();
        let q = ring();
        q.push(sample(1_000, -0.001)).expect("room");
        let rendered = r.step(&q, 1_100, 10);

        q.push(sample(2_000, -0.005)).expect("room");
        let held = r.step(&q, 2_100, DEADLINE_US + 1);

        assert_eq!(held.status, Status::HeldDeadlineMissed);
        assert_eq!(
            held.force.map(f64::to_bits),
            rendered.force.map(f64::to_bits),
            "held bit-for-bit, not zeroed or recomputed"
        );
    }

    #[test]
    fn deadline_exactly_met_is_not_a_miss() {
        let mut r = renderer();
        let q = ring();
        q.push(sample(1_000, -0.001)).expect("room");
        assert_eq!(r.step(&q, 1_100, DEADLINE_US).status, Status::Rendered);
    }

    #[test]
    fn empty_ring_holds_the_previous_force() {
        let mut r = renderer();
        let q = ring();
        q.push(sample(1_000, -0.001)).expect("room");
        r.step(&q, 1_100, 10);

        let out = r.step(&q, 1_200, 10);
        assert_eq!(out.status, Status::HeldNoInput);
        assert!((out.force[2] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn stale_sample_is_not_rendered() {
        let mut r = renderer();
        let q = ring();
        q.push(sample(0, -0.001)).expect("room");
        let out = r.step(&q, STALE_US + 1, 10);
        assert_eq!(out.status, Status::HeldStale);
        assert_eq!(
            out.force.map(f64::to_bits),
            [0.0f64; 3].map(f64::to_bits),
            "nothing rendered yet, so nothing held"
        );
    }

    #[test]
    fn invalid_sample_is_discarded() {
        let mut r = renderer();
        let q = ring();
        q.push(HapticInput {
            timestamp_us: 1_000,
            position: [f64::NAN, 0.0, -0.001],
            velocity: [0.0; 3],
        })
        .expect("room");
        assert_eq!(r.step(&q, 1_100, 10).status, Status::HeldInvalid);
    }

    #[test]
    fn only_the_newest_sample_is_used() {
        let mut r = renderer();
        let q = ring();
        // Three samples queue up. The last one is 2 mm deep, so the force is 2 N.
        q.push(sample(1_000, 0.0)).expect("room");
        q.push(sample(1_100, -0.001)).expect("room");
        q.push(sample(1_200, -0.002)).expect("room");

        let out = r.step(&q, 1_300, 10);

        assert_eq!(out.status, Status::Rendered);
        assert!((out.force[2] - 2.0).abs() < 1e-9, "{:?}", out.force);
        assert!(q.is_empty(), "the ring is drained each tick");
    }

    #[test]
    fn invalid_parameters_are_refused() {
        let surface = Surface::new([0.0; 3], [0.0, 0.0, 1.0]).expect("valid");
        let tissue = Tissue::new(1.0, 0.0).expect("valid");
        assert!(HapticLoop::new(surface, tissue, 0.0).is_none());
        assert!(HapticLoop::new(surface, tissue, f64::NAN).is_none());
    }
}
