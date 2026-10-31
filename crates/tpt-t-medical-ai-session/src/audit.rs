//! Journals every session command through `tpt-t-medical-compliance`.
//!
//! A command is written to the journal before it takes effect, whether it is
//! accepted or refused. A refused command is recorded too, because a refused AI
//! proposal is audit evidence. If the write fails, the command does not take
//! effect. Each unit has its own journal (docs/design/ai-session.md §5).

use std::fmt;

use tpt_t_medical_compliance::journal::{Journal, JournalError};
use tpt_t_medical_compliance::{Signer, Storage, Verifier};

use crate::{Actor, Command, Refusal, Session, State, Verdict};

const KIND_SESSION: u8 = b'S';

/// Why a journalled command did not take effect.
#[derive(Debug)]
pub enum AuditError {
    /// The session refused the command. Nothing changed, and the refusal is in
    /// the journal.
    Refused(Refusal),
    /// The journal could not be written. Nothing changed.
    Journal(JournalError),
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(r) => write!(f, "command refused: {r:?}"),
            Self::Journal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AuditError {}

/// One unit's session, with its own journal.
pub struct SessionAudit<S: Storage, G: Signer> {
    unit: u32,
    session: Session,
    journal: Journal<S>,
    signer: G,
}

impl<S: Storage, G: Signer> fmt::Debug for SessionAudit<S, G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionAudit")
            .field("unit", &self.unit)
            .field("state", &self.session.state())
            .field("records", &self.journal.log().entries().len())
            .finish_non_exhaustive()
    }
}

impl<S: Storage, G: Signer> SessionAudit<S, G> {
    /// Opens the unit's journal, verifying its existing records, and records
    /// the startup latch.
    ///
    /// # Errors
    ///
    /// Returns an error if the journal does not verify or cannot be written.
    pub fn open<V: Verifier + ?Sized>(
        unit: u32,
        session: Session,
        store: S,
        verifier: &V,
        signer: G,
    ) -> Result<Self, JournalError> {
        let mut audit = Self {
            unit,
            session,
            journal: Journal::open(store, verifier)?,
            signer,
        };
        audit.write(None, "startup", &Ok(None), 0)?;
        Ok(audit)
    }

    /// Returns the unit this session controls.
    #[must_use]
    pub const fn unit(&self) -> u32 {
        self.unit
    }

    /// Returns the session's current state.
    #[must_use]
    pub const fn state(&self) -> State {
        self.session.state()
    }

    /// Returns the session.
    #[must_use]
    pub const fn session(&self) -> &Session {
        &self.session
    }

    /// Returns the journal.
    #[must_use]
    pub const fn journal(&self) -> &Journal<S> {
        &self.journal
    }

    /// Applies one command, journalled first.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::Refused`] if the session refused the command, or
    /// [`AuditError::Journal`] if the record could not be written. In both cases
    /// the session is unchanged.
    pub fn handle(
        &mut self,
        actor: Actor,
        command: Command,
        now_us: u64,
    ) -> Result<Verdict, AuditError> {
        let mut next = self.session.clone();
        let result = next.step(actor, command, now_us);
        self.write(
            Some(actor),
            &format!("{command:?}"),
            &result.map(Some),
            now_us,
        )
        .map_err(AuditError::Journal)?;
        match result {
            Ok(verdict) => {
                self.session = next;
                Ok(verdict)
            }
            Err(refusal) => Err(AuditError::Refused(refusal)),
        }
    }

    /// Advances time. A change of state is journalled before it takes effect.
    ///
    /// # Errors
    ///
    /// Returns an error if the journal cannot be written. The state is then
    /// unchanged.
    pub fn tick(&mut self, now_us: u64) -> Result<Option<Verdict>, JournalError> {
        let mut next = self.session.clone();
        let Some(verdict) = next.tick(now_us) else {
            return Ok(None);
        };
        self.write(None, "tick", &Ok(Some(verdict)), now_us)?;
        self.session = next;
        Ok(Some(verdict))
    }

    fn write(
        &mut self,
        actor: Option<Actor>,
        what: &str,
        outcome: &Result<Option<Verdict>, Refusal>,
        now_us: u64,
    ) -> Result<(), JournalError> {
        let who = actor.map_or_else(|| "system".to_string(), |a| format!("{a:?}"));
        let payload = format!(
            "unit={} t={now_us}us actor={who} command={what} outcome={outcome:?}",
            self.unit
        );
        let mut bytes = vec![KIND_SESSION];
        bytes.extend_from_slice(payload.as_bytes());
        self.journal.append(&self.signer, bytes).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;
    use tpt_t_medical_safety::{EStopSource, Envelope, Limits};

    use crate::{Config, Proposal};

    const SEED: [u8; 32] = [41; 32];
    const SEC: u64 = 1_000_000;

    /// A store shared between the audit and the test, so the test can read the
    /// records back. `fail` makes every append fail.
    #[derive(Clone, Default)]
    struct SharedStore {
        records: Rc<RefCell<Vec<Vec<u8>>>>,
        fail: Rc<RefCell<bool>>,
    }

    impl Storage for SharedStore {
        fn append(&mut self, record: &[u8]) -> io::Result<()> {
            if *self.fail.borrow() {
                return Err(io::Error::other("disk full"));
            }
            self.records.borrow_mut().push(record.to_vec());
            Ok(())
        }

        fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
            Ok(self.records.borrow().clone())
        }
    }

    fn session() -> Session {
        let envelope = Envelope::new([-0.1; 3], [0.1; 3]).expect("box");
        let limits = Limits::new(envelope, 5.0, 2 * SEC).expect("limits");
        let config = Config::new(60 * SEC, 2 * SEC, SEC / 10, SEC / 2, 0.005).expect("config");
        Session::new(7, limits, config).expect("session")
    }

    fn open(store: &SharedStore) -> SessionAudit<SharedStore, Ed25519Signer> {
        let signer = Ed25519Signer::from_seed(&SEED);
        SessionAudit::open(3, session(), store.clone(), &signer.verifier(), signer).expect("open")
    }

    fn proposal() -> Proposal {
        Proposal {
            delta_m: [0.001, 0.0, 0.0],
            tool_position_m: [0.0; 3],
            tool_force_n: [0.0; 3],
            observed_delta_m: [0.0; 3],
        }
    }

    #[test]
    fn opening_latches_and_journals_the_startup() {
        let store = SharedStore::default();
        let audit = open(&store);
        assert_eq!(audit.state(), State::EmergencyStop);
        assert_eq!(audit.journal().log().entries().len(), 1);
    }

    #[test]
    fn accepted_and_refused_commands_are_both_journalled() {
        let store = SharedStore::default();
        let mut audit = open(&store);
        audit
            .handle(Actor::Surgeon(7), Command::Reset, SEC)
            .expect("reset");
        let refused = audit.handle(Actor::Ai, Command::Engage, SEC);
        assert!(matches!(
            refused,
            Err(AuditError::Refused(Refusal::NotPermitted))
        ));
        // startup, reset, and the refused engage.
        let entries = audit.journal().log().entries();
        assert_eq!(entries.len(), 3);
        let refused_text = String::from_utf8_lossy(&entries[2].payload).into_owned();
        assert!(refused_text.contains("NotPermitted"), "{refused_text}");
        assert_eq!(audit.state(), State::Autonomous);
    }

    #[test]
    fn a_journal_failure_leaves_the_session_unchanged() {
        let store = SharedStore::default();
        let mut audit = open(&store);
        *store.fail.borrow_mut() = true;
        let before = audit.session().clone();
        let result = audit.handle(Actor::Surgeon(7), Command::Reset, SEC);
        assert!(matches!(result, Err(AuditError::Journal(_))));
        assert_eq!(audit.session(), &before);
        assert_eq!(audit.state(), State::EmergencyStop);
    }

    #[test]
    fn a_failed_tick_write_leaves_the_state_unchanged() {
        let store = SharedStore::default();
        let mut audit = open(&store);
        audit
            .handle(Actor::Surgeon(7), Command::Reset, 0)
            .expect("reset");
        audit
            .handle(Actor::Ai, Command::RequestAssistance, 0)
            .expect("request");
        audit
            .handle(Actor::Surgeon(7), Command::Engage, 0)
            .expect("engage");
        *store.fail.borrow_mut() = true;
        // Silent for longer than the heartbeat, so this tick would fault.
        assert!(audit.tick(SEC).is_err());
        assert_eq!(audit.state(), State::TeleopActive);
    }

    #[test]
    fn a_fault_is_journalled_and_the_unit_stays_latched() {
        let store = SharedStore::default();
        let mut audit = open(&store);
        audit
            .handle(Actor::Surgeon(7), Command::Reset, 0)
            .expect("reset");
        audit
            .handle(Actor::Ai, Command::RequestAssistance, 0)
            .expect("request");
        audit
            .handle(Actor::Surgeon(7), Command::Engage, 0)
            .expect("engage");
        audit
            .handle(Actor::Ai, Command::EmergencyStop(EStopSource::Voice), SEC)
            .expect("stop");
        assert_eq!(audit.state(), State::EmergencyStop);
        let entries = audit.journal().log().entries();
        // startup, reset, request, engage, stop.
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn restart_latches_again_and_the_journal_still_verifies() {
        let store = SharedStore::default();
        {
            let mut audit = open(&store);
            audit
                .handle(Actor::Surgeon(7), Command::Reset, 0)
                .expect("reset");
        }
        let reopened = open(&store);
        assert_eq!(reopened.state(), State::EmergencyStop);
        assert_eq!(reopened.journal().log().entries().len(), 3);
    }

    #[test]
    fn proposals_go_through_the_journal_as_advice() {
        let store = SharedStore::default();
        let mut audit = open(&store);
        audit
            .handle(Actor::Surgeon(7), Command::Reset, 0)
            .expect("reset");
        audit
            .handle(Actor::Ai, Command::RequestAssistance, 0)
            .expect("request");
        audit
            .handle(Actor::Surgeon(7), Command::Engage, 0)
            .expect("engage");
        let verdict = audit
            .handle(Actor::Ai, Command::Propose(proposal()), SEC)
            .expect("advised");
        assert!(matches!(verdict, Verdict::Advised { .. }));
        let last = audit.journal().log().entries().last().expect("entry");
        let text = String::from_utf8_lossy(&last.payload).into_owned();
        assert!(text.contains("Advised"), "{text}");
    }
}
