//! The surgical state machine: `Idle → Registered → Active → Paused`, with
//! `Emergency` reachable from every state.
//!
//! # Transition rules
//!
//! | From         | Event           | To           |
//! |--------------|-----------------|--------------|
//! | `Idle`       | `Register`      | `Registered` |
//! | `Registered` | `Activate`      | `Active`     |
//! | `Registered` | `Deregister`    | `Idle`       |
//! | `Active`     | `Pause`         | `Paused`     |
//! | `Paused`     | `Resume`        | `Active`     |
//! | `Paused`     | `Deregister`    | `Idle`       |
//! | any but `Emergency` | `EmergencyStop` | `Emergency` |
//! | `Emergency`  | `ClearEmergency` | `Idle`      |
//!
//! `Emergency` is latched. Nothing except `ClearEmergency` leaves it, and
//! clearing it returns to `Idle`, never straight to `Active`. An operator must
//! register and activate again before commands are accepted.
//!
//! `Active` cannot be deregistered directly. The surgeon must pause first, so
//! an arm in use is never released in one step.

use core::fmt;

/// A surgical-system state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No surgeon registered. No motion commands are accepted.
    Idle,
    /// A surgeon is registered but has not activated control.
    Registered,
    /// Commands are accepted and the arm follows the master.
    Active,
    /// Control is held. The arm stays where it is.
    Paused,
    /// Latched stop. Only `ClearEmergency` leaves this state.
    Emergency,
}

/// An input to the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A surgeon registers with the system.
    Register,
    /// The registered surgeon releases the system.
    Deregister,
    /// The surgeon takes control.
    Activate,
    /// The surgeon holds control.
    Pause,
    /// The surgeon resumes control after a pause.
    Resume,
    /// Any emergency-stop source fired. See `tpt-t-medical-safety`.
    EmergencyStop,
    /// An operator has cleared the emergency.
    ClearEmergency,
}

/// Returned when an event is not allowed in the current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition {
    /// The state the machine was in.
    pub from: State,
    /// The event that was rejected.
    pub event: Event,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "event {:?} is not allowed in state {:?}",
            self.event, self.from
        )
    }
}

impl std::error::Error for InvalidTransition {}

/// Returns the state reached from `state` on `event`, or `None` if the
/// event is not allowed there.
#[must_use]
// Each arm is one row of the transition table. Several rows share a target,
// and merging them would hide the rules.
#[allow(clippy::match_same_arms)]
pub const fn transition(state: State, event: Event) -> Option<State> {
    use Event::{Activate, ClearEmergency, Deregister, EmergencyStop, Pause, Register, Resume};
    use State::{Active, Emergency, Idle, Paused, Registered};

    match (state, event) {
        (Emergency, ClearEmergency) => Some(Idle),
        (Emergency, _) => None,
        (_, EmergencyStop) => Some(Emergency),
        (Idle, Register) => Some(Registered),
        (Registered, Activate) => Some(Active),
        (Registered, Deregister) => Some(Idle),
        (Active, Pause) => Some(Paused),
        (Paused, Resume) => Some(Active),
        (Paused, Deregister) => Some(Idle),
        _ => None,
    }
}

/// The state machine, holding the current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Machine {
    state: State,
}

impl Machine {
    /// Creates a machine in `Idle`.
    #[must_use]
    pub const fn new() -> Self {
        Self { state: State::Idle }
    }

    /// Returns the current state.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Applies `event` and returns the new state.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidTransition`] and leaves the state unchanged when the
    /// event is not allowed in the current state.
    pub fn handle(&mut self, event: Event) -> Result<State, InvalidTransition> {
        let next = transition(self.state, event).ok_or(InvalidTransition {
            from: self.state,
            event,
        })?;
        self.state = next;
        Ok(next)
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [State; 5] = [
        State::Idle,
        State::Registered,
        State::Active,
        State::Paused,
        State::Emergency,
    ];

    const EVENTS: [Event; 7] = [
        Event::Register,
        Event::Deregister,
        Event::Activate,
        Event::Pause,
        Event::Resume,
        Event::EmergencyStop,
        Event::ClearEmergency,
    ];

    /// Every allowed transition, written out by hand. Anything not listed
    /// here must be rejected.
    const ALLOWED: [(State, Event, State); 11] = [
        (State::Idle, Event::Register, State::Registered),
        (State::Registered, Event::Activate, State::Active),
        (State::Registered, Event::Deregister, State::Idle),
        (State::Active, Event::Pause, State::Paused),
        (State::Paused, Event::Resume, State::Active),
        (State::Paused, Event::Deregister, State::Idle),
        (State::Idle, Event::EmergencyStop, State::Emergency),
        (State::Registered, Event::EmergencyStop, State::Emergency),
        (State::Active, Event::EmergencyStop, State::Emergency),
        (State::Paused, Event::EmergencyStop, State::Emergency),
        (State::Emergency, Event::ClearEmergency, State::Idle),
    ];

    #[test]
    fn transition_table_matches_the_documented_rules() {
        let mut checked = 0;
        for &state in &STATES {
            for &event in &EVENTS {
                let expected = ALLOWED
                    .iter()
                    .find(|(s, e, _)| *s == state && *e == event)
                    .map(|&(_, _, next)| next);
                assert_eq!(transition(state, event), expected, "{state:?} + {event:?}");
                checked += 1;
            }
        }
        assert_eq!(checked, STATES.len() * EVENTS.len());
    }

    #[test]
    fn emergency_stop_is_accepted_from_every_non_emergency_state() {
        for &state in &STATES {
            if state == State::Emergency {
                continue;
            }
            assert_eq!(
                transition(state, Event::EmergencyStop),
                Some(State::Emergency),
                "EmergencyStop from {state:?}"
            );
        }
    }

    #[test]
    fn emergency_is_latched_until_cleared() {
        for &event in &EVENTS {
            let expected = if event == Event::ClearEmergency {
                Some(State::Idle)
            } else {
                None
            };
            assert_eq!(transition(State::Emergency, event), expected, "{event:?}");
        }
    }

    #[test]
    fn clearing_emergency_never_resumes_active_control() {
        let mut m = Machine::new();
        m.handle(Event::Register).expect("register");
        m.handle(Event::Activate).expect("activate");
        m.handle(Event::EmergencyStop).expect("stop");
        assert_eq!(m.handle(Event::ClearEmergency), Ok(State::Idle));
        assert_eq!(
            m.handle(Event::Activate),
            Err(InvalidTransition {
                from: State::Idle,
                event: Event::Activate,
            })
        );
    }

    #[test]
    fn active_cannot_be_deregistered_directly() {
        let mut m = Machine::new();
        m.handle(Event::Register).expect("register");
        m.handle(Event::Activate).expect("activate");
        assert!(m.handle(Event::Deregister).is_err());
        assert_eq!(m.state(), State::Active);
    }

    #[test]
    fn rejected_event_leaves_state_unchanged() {
        let mut m = Machine::new();
        let err = m.handle(Event::Pause).expect_err("pause from idle");
        assert_eq!(err.from, State::Idle);
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn full_procedure_path() {
        let mut m = Machine::default();
        let path = [
            Event::Register,
            Event::Activate,
            Event::Pause,
            Event::Resume,
            Event::Pause,
            Event::Deregister,
        ];
        for event in path {
            m.handle(event).expect("valid path");
        }
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn display_names_the_rejected_event() {
        let err = InvalidTransition {
            from: State::Idle,
            event: Event::Pause,
        };
        assert_eq!(err.to_string(), "event Pause is not allowed in state Idle");
    }
}
