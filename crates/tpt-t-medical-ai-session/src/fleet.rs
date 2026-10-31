//! Fleet dispatch over several unit sessions (docs/design/ai-session.md §5).
//!
//! The dispatcher's whole surface is [`Fleet::request_assistance`]. It cannot
//! engage, override, reset or propose, because those methods do not exist on
//! the fleet. Every command names one unit. Nothing is broadcast, so a command
//! for unit A never reaches unit B.

use std::collections::BTreeMap;
use std::fmt;

use tpt_t_medical_compliance::journal::JournalError;
use tpt_t_medical_compliance::{Signer, Storage};

use crate::audit::{AuditError, SessionAudit};
use crate::{Actor, Command, State, Verdict};

/// Why a fleet command did not take effect.
#[derive(Debug)]
pub enum FleetError {
    /// No unit with that identifier is in the fleet.
    UnknownUnit(u32),
    /// The unit refused or could not journal the command.
    Unit(AuditError),
}

impl fmt::Display for FleetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownUnit(u) => write!(f, "unit {u} is not in the fleet"),
            Self::Unit(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FleetError {}

/// The sessions of several units, keyed by unit identifier.
pub struct Fleet<S: Storage, G: Signer> {
    units: BTreeMap<u32, SessionAudit<S, G>>,
}

impl<S: Storage, G: Signer> fmt::Debug for Fleet<S, G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fleet")
            .field("units", &self.units.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl<S: Storage, G: Signer> Default for Fleet<S, G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Storage, G: Signer> Fleet<S, G> {
    /// Creates an empty fleet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            units: BTreeMap::new(),
        }
    }

    /// Adds a unit. Returns `false`, and adds nothing, if a unit with the same
    /// identifier is already present.
    pub fn add(&mut self, unit: SessionAudit<S, G>) -> bool {
        let id = unit.unit();
        if self.units.contains_key(&id) {
            return false;
        }
        self.units.insert(id, unit);
        true
    }

    /// Returns the identifiers of every unit, in ascending order.
    pub fn unit_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.units.keys().copied()
    }

    /// Returns one unit's session and journal, for read-only views.
    #[must_use]
    pub fn unit(&self, unit: u32) -> Option<&SessionAudit<S, G>> {
        self.units.get(&unit)
    }

    /// Returns the state of one unit.
    #[must_use]
    pub fn state(&self, unit: u32) -> Option<State> {
        self.units.get(&unit).map(SessionAudit::state)
    }

    /// Returns the number of units.
    #[must_use]
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Returns `true` if the fleet has no units.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Asks a surgeon to take up one unit. This is the dispatcher's only
    /// command. The dispatcher may be a human or the AI.
    ///
    /// # Errors
    ///
    /// Returns [`FleetError::UnknownUnit`] for an unknown unit, or
    /// [`FleetError::Unit`] if that unit refused or could not journal it.
    pub fn request_assistance(
        &mut self,
        unit: u32,
        by: Actor,
        now_us: u64,
    ) -> Result<Verdict, FleetError> {
        let audit = self
            .units
            .get_mut(&unit)
            .ok_or(FleetError::UnknownUnit(unit))?;
        audit
            .handle(by, Command::RequestAssistance, now_us)
            .map_err(FleetError::Unit)
    }

    /// Advances every unit's clock. Stops at the first journal failure, and
    /// returns the transitions made before it.
    ///
    /// # Errors
    ///
    /// Returns the unit and the journal error for the first unit that could not
    /// record its change.
    pub fn tick_all(&mut self, now_us: u64) -> Result<Vec<(u32, Verdict)>, (u32, JournalError)> {
        let mut changes = Vec::new();
        for (&id, audit) in &mut self.units {
            match audit.tick(now_us) {
                Ok(Some(verdict)) => changes.push((id, verdict)),
                Ok(None) => {}
                Err(e) => return Err((id, e)),
            }
        }
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Session};
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;
    use tpt_t_medical_safety::{Envelope, Limits};

    const SEC: u64 = 1_000_000;

    #[derive(Clone, Default)]
    struct SharedStore(Rc<RefCell<Vec<Vec<u8>>>>);

    impl Storage for SharedStore {
        fn append(&mut self, record: &[u8]) -> io::Result<()> {
            self.0.borrow_mut().push(record.to_vec());
            Ok(())
        }

        fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
            Ok(self.0.borrow().clone())
        }
    }

    fn unit(id: u32, surgeon: u32) -> SessionAudit<SharedStore, Ed25519Signer> {
        let envelope = Envelope::new([-0.1; 3], [0.1; 3]).expect("box");
        let limits = Limits::new(envelope, 5.0, 2 * SEC).expect("limits");
        let config = Config::new(60 * SEC, 2 * SEC, SEC / 10, SEC / 2, 0.005).expect("config");
        let session = Session::new(surgeon, limits, config).expect("session");
        let signer = Ed25519Signer::from_seed(&[u8::try_from(id % 256).unwrap_or(0); 32]);
        SessionAudit::open(
            id,
            session,
            SharedStore::default(),
            &signer.verifier(),
            signer,
        )
        .expect("open")
    }

    fn fleet() -> Fleet<SharedStore, Ed25519Signer> {
        let mut f = Fleet::new();
        assert!(f.add(unit(1, 70)));
        assert!(f.add(unit(2, 71)));
        f
    }

    /// Clears the startup latch on a unit, as its surgeon would.
    fn reset(f: &mut Fleet<SharedStore, Ed25519Signer>, id: u32, surgeon: u32) {
        f.units
            .get_mut(&id)
            .expect("unit")
            .handle(Actor::Surgeon(surgeon), Command::Reset, 0)
            .expect("reset");
    }

    #[test]
    fn duplicate_units_are_refused() {
        let mut f = fleet();
        assert!(!f.add(unit(1, 99)));
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn a_dispatcher_can_request_one_unit_and_no_other_unit_changes() {
        let mut f = fleet();
        reset(&mut f, 1, 70);
        f.request_assistance(1, Actor::Dispatcher(9), 0)
            .expect("request");
        assert_eq!(f.state(1), Some(State::RequestingTeleop));
        assert_eq!(f.state(2), Some(State::EmergencyStop));
    }

    #[test]
    fn the_ai_can_request_assistance_too() {
        let mut f = fleet();
        reset(&mut f, 2, 71);
        f.request_assistance(2, Actor::Ai, 0).expect("request");
        assert_eq!(f.state(2), Some(State::RequestingTeleop));
    }

    #[test]
    fn requests_for_unknown_units_are_refused() {
        let mut f = fleet();
        assert!(matches!(
            f.request_assistance(42, Actor::Ai, 0),
            Err(FleetError::UnknownUnit(42))
        ));
    }

    #[test]
    fn a_request_on_a_latched_unit_is_refused_and_changes_nothing() {
        let mut f = fleet();
        assert!(matches!(
            f.request_assistance(1, Actor::Ai, 0),
            Err(FleetError::Unit(AuditError::Refused(_)))
        ));
        assert_eq!(f.state(1), Some(State::EmergencyStop));
    }

    #[test]
    fn tick_all_reports_each_unit_that_changed() {
        let mut f = fleet();
        // Unit 1 is reset, requested and engaged, so it is the only one that can
        // time out.
        reset(&mut f, 1, 70);
        {
            let u1 = f.units.get_mut(&1).expect("unit 1");
            u1.handle(Actor::Ai, Command::RequestAssistance, 0)
                .expect("request");
            u1.handle(Actor::Surgeon(70), Command::Engage, 0)
                .expect("engage");
        }
        let changes = f.tick_all(SEC).expect("tick");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, 1);
        assert_eq!(f.state(1), Some(State::EmergencyStop));
        assert_eq!(f.state(2), Some(State::EmergencyStop));
    }
}
