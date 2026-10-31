//! Feeds ADT events into the transport layer (spec.txt §5.2).
//!
//! An admit or a transfer names where the patient now is. The bridge maps that
//! location code to a waypoint in the logistics map, and the transport planner
//! routes to it. A discharge clears the destination, so no robot is sent to a
//! bed that is now empty.
//!
//! A location code that is not in the ward table is refused. The bridge does not
//! guess a nearby waypoint, because a wrong destination sends a robot into the
//! wrong room.

use std::collections::BTreeMap;

use tpt_t_medical_logistics::route::NodeId;

use crate::{AdtEvent, AdtKind};

/// Maps HL7 location codes (PV1-3) to logistics waypoints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wards {
    codes: BTreeMap<String, NodeId>,
}

impl Wards {
    /// Creates an empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            codes: BTreeMap::new(),
        }
    }

    /// Adds a location code. Returns `false`, and adds nothing, if the code is
    /// already mapped.
    pub fn add(&mut self, code: &str, node: NodeId) -> bool {
        if self.codes.contains_key(code) {
            return false;
        }
        self.codes.insert(code.to_string(), node);
        true
    }

    /// Returns the waypoint for a location code.
    #[must_use]
    pub fn node(&self, code: &str) -> Option<NodeId> {
        self.codes.get(code).copied()
    }
}

/// What the transport layer should do for a patient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// Route transports for this patient to this waypoint.
    Destination(NodeId),
    /// Clear the patient's destination. Nothing more is sent to them.
    Clear,
}

/// Why an event could not be turned into a transport instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationError {
    /// An admit or transfer has no location (PV1 is missing or empty).
    MissingLocation,
    /// The location code is not in the ward table.
    UnknownLocation,
}

impl std::fmt::Display for LocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingLocation => f.write_str("admit or transfer has no location"),
            Self::UnknownLocation => f.write_str("location code is not in the ward table"),
        }
    }
}

impl std::error::Error for LocationError {}

/// Turns an ADT event into a transport instruction.
///
/// # Errors
///
/// Returns an error for an admit or transfer whose location is missing or not in
/// the ward table.
pub fn transport_for(event: &AdtEvent, wards: &Wards) -> Result<Transport, LocationError> {
    match event.kind {
        AdtKind::Discharge => Ok(Transport::Clear),
        AdtKind::Admit | AdtKind::Transfer => {
            let code = event
                .location
                .as_deref()
                .ok_or(LocationError::MissingLocation)?;
            wards
                .node(code)
                .map(Transport::Destination)
                .ok_or(LocationError::UnknownLocation)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn wards() -> Wards {
        let mut w = Wards::new();
        assert!(w.add("WARD4", 40));
        assert!(w.add("ICU", 10));
        w
    }

    fn event(trigger: &str, pv1: Option<&str>) -> AdtEvent {
        let mut msg = format!("MSH|^~\\&|A|B|C|D|t||ADT^{trigger}|1|P|2.5\rPID|1||MRN-9");
        if let Some(pv1) = pv1 {
            msg.push('\r');
            msg.push_str(pv1);
        }
        parse(msg.as_bytes()).expect("valid")
    }

    #[test]
    fn admit_routes_to_the_ward() {
        let e = event("A01", Some("PV1|1|I|WARD4^401^A"));
        assert_eq!(transport_for(&e, &wards()), Ok(Transport::Destination(40)));
    }

    #[test]
    fn transfer_routes_to_the_new_location() {
        let e = event("A02", Some("PV1|1|I|ICU^1"));
        assert_eq!(transport_for(&e, &wards()), Ok(Transport::Destination(10)));
    }

    #[test]
    fn discharge_clears_the_destination() {
        let e = event("A03", Some("PV1|1|I|WARD4^401^A"));
        assert_eq!(transport_for(&e, &wards()), Ok(Transport::Clear));
    }

    #[test]
    fn unknown_and_missing_locations_are_refused_not_guessed() {
        assert_eq!(
            transport_for(&event("A01", Some("PV1|1|I|NOWHERE")), &wards()),
            Err(LocationError::UnknownLocation)
        );
        assert_eq!(
            transport_for(&event("A01", None), &wards()),
            Err(LocationError::MissingLocation)
        );
    }

    #[test]
    fn a_code_cannot_be_mapped_twice() {
        let mut w = wards();
        assert!(!w.add("ICU", 99));
        assert_eq!(w.node("ICU"), Some(10));
    }
}
