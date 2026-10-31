//! Transport profiles: what each kind of load may pass through and where it may
//! end (spec.txt §5.2).
//!
//! The spec names five loads but does not give their rules, so the rules here
//! are the recommended defaults, and they are conservative:
//!
//! * **Meals, medication and lab samples** travel in clean space only. A route
//!   that touches a dirty or isolation waypoint is refused.
//! * **Linens** may pass through any zone, because soiled linen comes from
//!   dirty areas. The robot is then contaminated, and the route rules require
//!   a UV station before it re-enters clean space.
//! * **Biohazard** may pass through any zone and must end in a non-clean zone,
//!   so it is never left in clean space.
//!
//! The check runs on the route the planner returned. A profile cannot widen the
//! planner's rules, and it can only refuse more routes.

use core::fmt;

use crate::route::{Map, NodeId, Route, RouteError};
use crate::zones::{Robot, Zone};

/// The kind of load a robot carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Clean and soiled linen.
    Linens,
    /// Patient meals.
    Meals,
    /// Medication.
    Medication,
    /// Laboratory samples.
    LabSample,
    /// Biohazardous waste.
    Biohazard,
}

/// Why a route was refused for its profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// The planner found no route at all.
    Route(RouteError),
    /// The route passes a waypoint the load may not enter.
    ZoneNotAllowed {
        /// The waypoint.
        node: NodeId,
        /// The zone it is in.
        zone: Zone,
    },
    /// The load must end in a non-clean zone, and the destination is clean.
    MustEndNonClean,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Route(e) => write!(f, "{e}"),
            Self::ZoneNotAllowed { node, zone } => {
                write!(
                    f,
                    "waypoint {node} is in {zone:?} space, which this load may not enter"
                )
            }
            Self::MustEndNonClean => f.write_str("biohazard must end in a non-clean zone"),
        }
    }
}

impl std::error::Error for ProfileError {}

impl From<RouteError> for ProfileError {
    fn from(e: RouteError) -> Self {
        Self::Route(e)
    }
}

impl Profile {
    /// Returns `true` if this load may enter a waypoint in `zone`.
    #[must_use]
    pub const fn may_enter(self, zone: Zone) -> bool {
        match self {
            Self::Meals | Self::Medication | Self::LabSample => {
                matches!(zone, Zone::Clean)
            }
            Self::Linens | Self::Biohazard => true,
        }
    }

    /// Returns `true` if the load must end its trip in a non-clean zone.
    #[must_use]
    pub const fn must_end_non_clean(self) -> bool {
        matches!(self, Self::Biohazard)
    }
}

/// Plans a route for `robot` carrying `profile`, and checks it against the
/// profile.
///
/// # Errors
///
/// Returns the planner's error, or the first waypoint or destination the
/// profile refuses.
pub fn route_for(
    map: &Map,
    robot: &Robot,
    profile: Profile,
    from: NodeId,
    to: NodeId,
) -> Result<Route, ProfileError> {
    for node in [from, to] {
        let zone = map.zone(node).ok_or(RouteError::UnknownNode(node))?;
        if !profile.may_enter(zone) {
            return Err(ProfileError::ZoneNotAllowed { node, zone });
        }
    }
    // Plan on the map the load may use, so a detour through clean space is found
    // when one exists.
    let allowed = map.restricted(|zone| profile.may_enter(zone));
    let route = allowed.route(robot, from, to)?;
    if profile.must_end_non_clean() && map.zone(to) == Some(Zone::Clean) {
        return Err(ProfileError::MustEndNonClean);
    }
    Ok(route)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::Node;

    /// Ward (1, clean) – utility (2, dirty) – station (3, clean, UV) – store (4,
    /// clean), with a clean-only corridor 1–4 and the utility link 2–3.
    fn hospital() -> Map {
        let clean = Node {
            zone: Zone::Clean,
            uv_station: false,
        };
        let station = Node {
            zone: Zone::Clean,
            uv_station: true,
        };
        let dirty = Node {
            zone: Zone::Dirty,
            uv_station: false,
        };
        let mut m = Map::new();
        assert!(m.add_node(1, clean));
        assert!(m.add_node(2, dirty));
        assert!(m.add_node(3, station));
        assert!(m.add_node(4, clean));
        assert!(m.add_corridor(1, 2, 1_000));
        assert!(m.add_corridor(2, 3, 1_000));
        assert!(m.add_corridor(3, 4, 1_000));
        assert!(m.add_corridor(1, 4, 10_000));
        m
    }

    #[test]
    fn meals_take_the_clean_corridor_only() {
        let m = hospital();
        let r = route_for(&m, &Robot::new(1), Profile::Meals, 1, 4).expect("route");
        assert_eq!(r.nodes, vec![1, 4]);
    }

    #[test]
    fn meals_are_refused_a_route_through_dirty_space() {
        let m = hospital();
        // The only way to node 2 is through clean space, but node 2 is dirty.
        assert_eq!(
            route_for(&m, &Robot::new(1), Profile::Medication, 1, 2),
            Err(ProfileError::ZoneNotAllowed {
                node: 2,
                zone: Zone::Dirty
            })
        );
    }

    #[test]
    fn linens_may_go_through_dirty_space() {
        let m = hospital();
        let r = route_for(&m, &Robot::new(1), Profile::Linens, 1, 2).expect("route");
        assert_eq!(r.nodes, vec![1, 2]);
    }

    #[test]
    fn biohazard_must_end_in_a_non_clean_zone() {
        let m = hospital();
        assert_eq!(
            route_for(&m, &Robot::new(1), Profile::Biohazard, 2, 4),
            Err(ProfileError::MustEndNonClean)
        );
        assert!(route_for(&m, &Robot::new(1), Profile::Biohazard, 1, 2).is_ok());
    }

    #[test]
    fn a_profile_adds_refusals_but_never_removes_the_planner_rules() {
        let m = hospital();
        let mut contaminated = Robot::new(1);
        contaminated.move_to(Zone::Dirty).expect("dirty");
        // Linens may enter clean space, but the planner still needs the station.
        let r = route_for(&m, &contaminated, Profile::Linens, 2, 4).expect("route");
        assert_eq!(r.nodes, vec![2, 3, 4]);
    }

    #[test]
    fn planner_errors_pass_through() {
        let m = hospital();
        assert_eq!(
            route_for(&m, &Robot::new(1), Profile::Linens, 1, 99),
            Err(ProfileError::Route(RouteError::UnknownNode(99)))
        );
    }
}
