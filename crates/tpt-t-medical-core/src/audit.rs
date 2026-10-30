//! Couples the event loop to the audit journal.
//!
//! Two rules govern the ordering:
//!
//! * A command is recorded before it is applied. If the record cannot be
//!   written, the command is not applied, and the error is returned.
//! * A safety stop is applied even if the record cannot be written. The stop
//!   matters more than the log, so the error is returned after the machine has
//!   already stopped.
//!
//! After any error from this module, the caller must treat the system as
//! unsafe until the storage problem is fixed and the system is reset.

use tpt_t_medical_compliance::journal::{Journal, JournalError};
use tpt_t_medical_compliance::{Signer, Storage};
use tpt_t_medical_safety::{Decision, EStopSource, StopReason};

use crate::event_loop::{EventLoop, TickReport};
use crate::state::{Event, State};

/// First byte of a record that logs a transition: `[KIND_EVENT, event, state]`.
const KIND_EVENT: u8 = 1;
/// First byte of a record that logs a safety stop: `[KIND_SAFETY, reason]`.
const KIND_SAFETY: u8 = 2;

/// An event loop whose every transition is journalled.
#[derive(Debug)]
pub struct AuditedLoop<const LANES: usize, S: Storage, G: Signer> {
    event_loop: EventLoop<LANES>,
    journal: Journal<S>,
    signer: G,
}

impl<const LANES: usize, S: Storage, G: Signer> AuditedLoop<LANES, S, G> {
    /// Wraps a loop, a journal, and the signer for its records.
    #[must_use]
    pub fn new(event_loop: EventLoop<LANES>, journal: Journal<S>, signer: G) -> Self {
        Self {
            event_loop,
            journal,
            signer,
        }
    }

    /// Returns the current state.
    #[must_use]
    pub const fn state(&self) -> State {
        self.event_loop.state()
    }

    /// Returns the journal, for reading the recorded transitions.
    #[must_use]
    pub const fn journal(&self) -> &Journal<S> {
        &self.journal
    }

    /// Records and applies pending events. See [`EventLoop::tick_with`].
    ///
    /// # Errors
    ///
    /// Returns an error if a record cannot be written. The failing event is
    /// not applied.
    pub fn tick(&mut self, budget: usize) -> Result<TickReport, JournalError> {
        let journal = &mut self.journal;
        let signer = &self.signer;
        self.event_loop.tick_with(budget, |event, state| {
            journal_event(journal, signer, event, state)
        })
    }

    /// Applies a safety decision. A `Run` decision does nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if the stop could not be fully recorded. The stop is
    /// applied regardless. See [`AuditedLoop::stop`].
    pub fn apply_decision(&mut self, decision: Decision) -> Result<(), JournalError> {
        match decision {
            Decision::Run => Ok(()),
            Decision::Stop(reason) => self.stop(reason),
        }
    }

    /// Stops the system for `reason`.
    ///
    /// The reason and the resulting transition are journalled, and the machine
    /// is moved to `Emergency` whatever the storage does. If journalling fails,
    /// the error is returned after the stop has taken effect.
    ///
    /// # Errors
    ///
    /// Returns the first journalling error.
    pub fn stop(&mut self, reason: StopReason) -> Result<(), JournalError> {
        let reason_recorded = self
            .journal
            .append(&self.signer, vec![KIND_SAFETY, reason_code(reason)])
            .map(|_| ());

        let before = self.event_loop.state();
        self.event_loop.apply_now(Event::EmergencyStop);
        let transition_recorded = journal_event(
            &mut self.journal,
            &self.signer,
            Event::EmergencyStop,
            before,
        );

        reason_recorded.and(transition_recorded)
    }
}

fn journal_event<S: Storage, G: Signer + ?Sized>(
    journal: &mut Journal<S>,
    signer: &G,
    event: Event,
    state: State,
) -> Result<(), JournalError> {
    journal
        .append(signer, vec![KIND_EVENT, event.code(), state.code()])
        .map(|_| ())
}

/// Stable code for each stop reason, used in audit records. Do not renumber.
const fn reason_code(reason: StopReason) -> u8 {
    match reason {
        StopReason::Disagreement => 1,
        StopReason::OutsideEnvelope => 2,
        StopReason::ForceExceeded => 3,
        StopReason::WatchdogExpired => 4,
        StopReason::EStop(EStopSource::FootPedal) => 10,
        StopReason::EStop(EStopSource::HandSwitch) => 11,
        StopReason::EStop(EStopSource::Voice) => 12,
        StopReason::EStop(EStopSource::Automatic) => 13,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_loop::EventLoop;
    use std::cell::Cell;
    use std::io;
    use std::rc::Rc;
    use std::sync::Arc;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;
    use tpt_t_medical_ring::SpscRing;

    const SEED: [u8; 32] = [5; 32];

    /// In-memory storage with a switch that makes every append fail.
    struct MemStore {
        stored: Vec<Vec<u8>>,
        fail: Rc<Cell<bool>>,
    }

    impl Storage for MemStore {
        fn append(&mut self, record: &[u8]) -> io::Result<()> {
            if self.fail.get() {
                return Err(io::Error::other("storage unavailable"));
            }
            self.stored.push(record.to_vec());
            Ok(())
        }

        fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
            Ok(self.stored.clone())
        }
    }

    type Fixture = (
        AuditedLoop<2, MemStore, Ed25519Signer>,
        [Arc<SpscRing<Event>>; 2],
    );

    fn fixture(fail: &Rc<Cell<bool>>) -> Fixture {
        let signer = Ed25519Signer::from_seed(&SEED);
        let store = MemStore {
            stored: Vec::new(),
            fail: Rc::clone(fail),
        };
        let journal = Journal::open(store, &signer.verifier()).expect("fresh journal");
        let (event_loop, producers) = EventLoop::<2>::new(8);
        (AuditedLoop::new(event_loop, journal, signer), producers)
    }

    fn payloads(aud: &AuditedLoop<2, MemStore, Ed25519Signer>) -> Vec<Vec<u8>> {
        aud.journal()
            .log()
            .entries()
            .iter()
            .map(|e| e.payload.clone())
            .collect()
    }

    #[test]
    fn tick_records_each_transition_with_its_state() {
        let fail = Rc::new(Cell::new(false));
        let (mut aud, producers) = fixture(&fail);
        producers[1].push(Event::Register).expect("room");
        producers[1].push(Event::Activate).expect("room");

        aud.tick(10).expect("recorded");

        assert_eq!(aud.state(), State::Active);
        assert_eq!(
            payloads(&aud),
            vec![
                vec![KIND_EVENT, Event::Register.code(), State::Idle.code()],
                vec![KIND_EVENT, Event::Activate.code(), State::Registered.code()],
            ]
        );
    }

    #[test]
    fn failed_record_leaves_the_state_unchanged_and_drops_the_event() {
        let fail = Rc::new(Cell::new(true));
        let (mut aud, producers) = fixture(&fail);
        producers[1].push(Event::Register).expect("room");

        assert!(aud.tick(10).is_err());
        assert_eq!(aud.state(), State::Idle);

        // The event was taken from its lane before the write failed, so it is
        // not retried. The error is the signal that it was lost.
        fail.set(false);
        let report = aud.tick(10).expect("storage healthy again");
        assert_eq!(report.applied, 0);
        assert_eq!(aud.state(), State::Idle);
    }

    #[test]
    fn stop_records_the_reason_then_the_transition() {
        let fail = Rc::new(Cell::new(false));
        let (mut aud, producers) = fixture(&fail);
        producers[1].push(Event::Register).expect("room");
        producers[1].push(Event::Activate).expect("room");
        aud.tick(10).expect("recorded");

        aud.stop(StopReason::ForceExceeded).expect("recorded");

        assert_eq!(aud.state(), State::Emergency);
        let all = payloads(&aud);
        let tail = &all[all.len() - 2..];
        assert_eq!(tail[0], vec![KIND_SAFETY, 3]);
        assert_eq!(
            tail[1],
            vec![
                KIND_EVENT,
                Event::EmergencyStop.code(),
                State::Active.code()
            ]
        );
    }

    #[test]
    fn stop_takes_effect_even_when_storage_fails() {
        let fail = Rc::new(Cell::new(true));
        let (mut aud, _producers) = fixture(&fail);

        let err = aud.stop(StopReason::EStop(EStopSource::FootPedal));

        assert!(err.is_err(), "the failed record must be reported");
        assert_eq!(aud.state(), State::Emergency, "the stop must still apply");
    }

    #[test]
    fn run_decision_changes_nothing() {
        let fail = Rc::new(Cell::new(false));
        let (mut aud, _producers) = fixture(&fail);

        aud.apply_decision(Decision::Run).expect("no-op");

        assert_eq!(aud.state(), State::Idle);
        assert!(payloads(&aud).is_empty());
    }

    #[test]
    fn stop_decision_is_routed_to_stop() {
        let fail = Rc::new(Cell::new(false));
        let (mut aud, _producers) = fixture(&fail);

        aud.apply_decision(Decision::Stop(StopReason::Disagreement))
            .expect("recorded");

        assert_eq!(aud.state(), State::Emergency);
        assert_eq!(payloads(&aud)[0], vec![KIND_SAFETY, 1]);
    }
}
