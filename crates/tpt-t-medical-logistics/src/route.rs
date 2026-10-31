//! Route planning through corridors, with the infection-control rule built into
//! the search (spec.txt §5.2).
//!
//! The map is a graph of waypoints. Each waypoint has a zone, and a waypoint may
//! be a UV station. A contaminated robot must reach a station before it enters a
//! clean waypoint, so the search tracks a second bit: "still needs UV". Entering
//! a non-clean waypoint sets the bit. Only a station clears it. A clean waypoint
//! that is not a station is refused while the bit is set.
//!
//! The search is Dijkstra over (waypoint, bit) states. A route that the rules
//! forbid is never produced, so a caller cannot get one by asking for a shorter
//! path.
//!
//! The robot runs the UV cycle when it reaches a station. The route only says
//! where to go, and the cycle itself is run by the robot.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use crate::Robot;
use crate::zones::Zone;

/// Most waypoints in one map.
pub const MAX_NODES: usize = 4_096;

/// Identifies a waypoint.
pub type NodeId = u32;

/// One waypoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    /// The zone the waypoint is in.
    pub zone: Zone,
    /// Whether a robot can run a UV-C cycle here. A station must be in a clean
    /// zone, so a robot is never sterilized in a contaminated area.
    pub uv_station: bool,
}

/// A route, from the start to the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// The waypoints in order, including both ends.
    pub nodes: Vec<NodeId>,
    /// Total corridor length, in millimetres.
    pub length_mm: u64,
}

/// Why a route could not be planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    /// The start or destination is not in the map.
    UnknownNode(NodeId),
    /// No route exists that obeys the infection-control rules.
    NoRoute,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownNode(n) => write!(f, "waypoint {n} is not in the map"),
            Self::NoRoute => f.write_str("no route obeys the infection-control rules"),
        }
    }
}

impl std::error::Error for RouteError {}

/// A map of waypoints and the corridors between them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Map {
    nodes: BTreeMap<NodeId, Node>,
    corridors: BTreeMap<NodeId, Vec<(NodeId, u32)>>,
}

impl Map {
    /// Creates an empty map.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            corridors: BTreeMap::new(),
        }
    }

    /// Adds a waypoint. Returns `false`, and adds nothing, if the identifier is
    /// taken, the map is full, or a UV station is not in a clean zone.
    pub fn add_node(&mut self, id: NodeId, node: Node) -> bool {
        if self.nodes.contains_key(&id) || self.nodes.len() >= MAX_NODES {
            return false;
        }
        if node.uv_station && node.zone != Zone::Clean {
            return false;
        }
        self.nodes.insert(id, node);
        true
    }

    /// Joins two waypoints with a two-way corridor of `length_mm`. Returns
    /// `false` if either waypoint is missing, the length is zero, or the two
    /// are the same waypoint.
    pub fn add_corridor(&mut self, a: NodeId, b: NodeId, length_mm: u32) -> bool {
        if a == b || length_mm == 0 || !self.nodes.contains_key(&a) || !self.nodes.contains_key(&b)
        {
            return false;
        }
        self.corridors.entry(a).or_default().push((b, length_mm));
        self.corridors.entry(b).or_default().push((a, length_mm));
        true
    }

    /// Returns a copy of the map that keeps only waypoints whose zone `keep`
    /// accepts, and the corridors between those waypoints.
    #[must_use]
    pub fn restricted(&self, keep: impl Fn(Zone) -> bool) -> Self {
        let nodes: BTreeMap<NodeId, Node> = self
            .nodes
            .iter()
            .filter(|(_, n)| keep(n.zone))
            .map(|(&id, &n)| (id, n))
            .collect();
        let corridors = self
            .corridors
            .iter()
            .filter(|(a, _)| nodes.contains_key(a))
            .map(|(&a, list)| {
                let kept = list
                    .iter()
                    .filter(|(b, _)| nodes.contains_key(b))
                    .copied()
                    .collect();
                (a, kept)
            })
            .collect();
        Self { nodes, corridors }
    }

    /// Returns the zone of a waypoint, or `None` if it is not in the map.
    #[must_use]
    pub fn zone(&self, id: NodeId) -> Option<Zone> {
        self.nodes.get(&id).map(|n| n.zone)
    }

    /// Plans the shortest route for `robot` from `from` to `to`.
    ///
    /// The robot's current contamination is taken into account. A robot that is
    /// not sterile must pass a UV station before it enters a clean waypoint.
    ///
    /// # Errors
    ///
    /// Returns [`RouteError::UnknownNode`] for a missing endpoint, or
    /// [`RouteError::NoRoute`] when the rules leave no way through.
    pub fn route(&self, robot: &Robot, from: NodeId, to: NodeId) -> Result<Route, RouteError> {
        for id in [from, to] {
            if !self.nodes.contains_key(&id) {
                return Err(RouteError::UnknownNode(id));
            }
        }
        let start = (from, !robot.is_sterile());
        let mut best: BTreeMap<(NodeId, bool), u64> = BTreeMap::new();
        let mut came_from: BTreeMap<(NodeId, bool), (NodeId, bool)> = BTreeMap::new();
        let mut queue = BinaryHeap::new();
        best.insert(start, 0);
        queue.push(Reverse((0_u64, start.0, start.1)));

        while let Some(Reverse((cost, node, dirty_bit))) = queue.pop() {
            let state = (node, dirty_bit);
            if best.get(&state).is_some_and(|&c| cost > c) {
                continue;
            }
            if node == to {
                return Ok(Self::rebuild(&came_from, state, cost));
            }
            for &(next, length) in self.corridors.get(&node).map_or(&[][..], Vec::as_slice) {
                let Some(next_bit) = self.bit_after(next, dirty_bit) else {
                    continue;
                };
                let next_cost = cost + u64::from(length);
                let next_state = (next, next_bit);
                if best.get(&next_state).is_none_or(|&c| next_cost < c) {
                    best.insert(next_state, next_cost);
                    came_from.insert(next_state, state);
                    queue.push(Reverse((next_cost, next, next_bit)));
                }
            }
        }
        Err(RouteError::NoRoute)
    }

    /// Returns the bit after entering `node`, or `None` if the move is refused.
    fn bit_after(&self, node: NodeId, needs_uv: bool) -> Option<bool> {
        let n = self.nodes.get(&node)?;
        match n.zone {
            Zone::Dirty | Zone::Isolation => Some(true),
            Zone::Clean if n.uv_station => Some(false),
            Zone::Clean if needs_uv => None,
            Zone::Clean => Some(false),
        }
    }

    fn rebuild(
        came_from: &BTreeMap<(NodeId, bool), (NodeId, bool)>,
        end: (NodeId, bool),
        cost: u64,
    ) -> Route {
        let mut nodes = vec![end.0];
        let mut at = end;
        while let Some(&prev) = came_from.get(&at) {
            nodes.push(prev.0);
            at = prev;
        }
        nodes.reverse();
        Route {
            nodes,
            length_mm: cost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: Node = Node {
        zone: Zone::Clean,
        uv_station: false,
    };
    const STATION: Node = Node {
        zone: Zone::Clean,
        uv_station: true,
    };
    const DIRTY: Node = Node {
        zone: Zone::Dirty,
        uv_station: false,
    };

    /// Ward (1) – utility (2, dirty) – store (3, clean) – station (4, clean)
    /// – ward (5). The utility area reaches the station directly (2–4), since
    /// a contaminated robot may only step from dirty space into a station. A
    /// short clean corridor 1–5 bypasses the utility area.
    fn hospital() -> Map {
        let mut m = Map::new();
        assert!(m.add_node(1, CLEAN));
        assert!(m.add_node(2, DIRTY));
        assert!(m.add_node(3, CLEAN));
        assert!(m.add_node(4, STATION));
        assert!(m.add_node(5, CLEAN));
        assert!(m.add_corridor(1, 2, 10_000));
        assert!(m.add_corridor(2, 3, 10_000));
        assert!(m.add_corridor(3, 4, 10_000));
        assert!(m.add_corridor(4, 5, 10_000));
        assert!(m.add_corridor(2, 4, 5_000));
        assert!(m.add_corridor(1, 5, 15_000));
        m
    }

    #[test]
    fn a_sterile_robot_takes_the_shortest_clean_route() {
        let m = hospital();
        let r = m.route(&Robot::new(1), 1, 5).expect("route");
        assert_eq!(r.nodes, vec![1, 5]);
        assert_eq!(r.length_mm, 15_000);
    }

    #[test]
    fn a_contaminated_robot_must_pass_a_station_before_clean_space() {
        let mut robot = Robot::new(1);
        robot.move_to(Zone::Dirty).expect("into dirty");
        let m = hospital();
        // Starts in the dirty area and needs to reach the ward. Clean space is
        // closed to it until the station, so the only way is 2 -> 4 -> 5.
        let r = m.route(&robot, 2, 5).expect("route");
        assert_eq!(r.nodes, vec![2, 4, 5]);
        assert_eq!(r.length_mm, 15_000);
    }

    #[test]
    fn a_robot_that_has_been_dirty_cannot_take_the_clean_shortcut() {
        let mut m = Map::new();
        assert!(m.add_node(1, CLEAN));
        assert!(m.add_node(2, DIRTY));
        assert!(m.add_node(3, CLEAN));
        assert!(m.add_corridor(1, 2, 1_000));
        assert!(m.add_corridor(2, 3, 1_000));
        let mut robot = Robot::new(1);
        robot.move_to(Zone::Dirty).expect("dirty");
        // With no station anywhere, the robot cannot re-enter clean space.
        assert_eq!(m.route(&robot, 2, 3), Err(RouteError::NoRoute));
    }

    #[test]
    fn the_uv_bit_is_only_cleared_at_a_station() {
        let m = hospital();
        let mut robot = Robot::new(1);
        robot.move_to(Zone::Dirty).expect("dirty");
        // Going from the dirty area to the clean store directly is refused, so
        // the route must visit the station.
        assert_eq!(m.route(&robot, 2, 3).map(|r| r.nodes), Ok(vec![2, 4, 3]));
    }

    #[test]
    fn unknown_endpoints_are_refused() {
        let m = hospital();
        assert_eq!(
            m.route(&Robot::new(1), 99, 5),
            Err(RouteError::UnknownNode(99))
        );
        assert_eq!(
            m.route(&Robot::new(1), 1, 77),
            Err(RouteError::UnknownNode(77))
        );
    }

    #[test]
    fn a_station_outside_a_clean_zone_is_refused() {
        let mut m = Map::new();
        let bad = Node {
            zone: Zone::Dirty,
            uv_station: true,
        };
        assert!(!m.add_node(1, bad));
    }

    #[test]
    fn bad_corridors_are_refused() {
        let mut m = Map::new();
        assert!(m.add_node(1, CLEAN));
        assert!(m.add_node(2, CLEAN));
        assert!(!m.add_corridor(1, 1, 5));
        assert!(!m.add_corridor(1, 2, 0));
        assert!(!m.add_corridor(1, 9, 5));
        assert!(!m.add_node(1, CLEAN));
    }

    #[test]
    fn a_route_to_self_is_one_waypoint() {
        let m = hospital();
        let r = m.route(&Robot::new(1), 1, 1).expect("route");
        assert_eq!(r.nodes, vec![1]);
        assert_eq!(r.length_mm, 0);
    }
}
