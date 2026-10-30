//! Single-occupancy elevator arbitration (spec.txt §5.2).
//!
//! One robot at a time may be in the elevator. Requests that cannot board wait
//! in a queue. Emergency transports go ahead of routine ones, and requests of
//! the same kind are served in the order they arrived.
//!
//! The single-occupancy rule holds by construction. The occupant is set in one
//! place only, and that place checks that the cab is empty.

/// The result of asking for the elevator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// The robot boarded straight away.
    Boarded,
    /// The cab is busy. The robot is queued at `position`, where 0 is next.
    Queued {
        /// Place in the queue.
        position: usize,
    },
    /// The robot is already in the cab.
    AlreadyAboard,
    /// The robot is already queued at `position`. It is not queued twice.
    AlreadyQueued {
        /// Place in the queue.
        position: usize,
    },
}

/// Returned when a robot tries to leave a cab it is not in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotOccupant;

impl std::fmt::Display for NotOccupant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("robot is not in the elevator")
    }
}

impl std::error::Error for NotOccupant {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Request {
    robot: u32,
    emergency: bool,
    seq: u64,
}

/// One elevator cab.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Elevator {
    occupant: Option<u32>,
    queue: Vec<Request>,
    next_seq: u64,
}

impl Elevator {
    /// Creates an empty cab with an empty queue.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            occupant: None,
            queue: Vec::new(),
            next_seq: 0,
        }
    }

    /// Returns the robot in the cab, if any.
    #[must_use]
    pub const fn occupant(&self) -> Option<u32> {
        self.occupant
    }

    /// Asks for the cab on behalf of `robot`. Set `emergency` for an emergency
    /// transport.
    pub fn request(&mut self, robot: u32, emergency: bool) -> Grant {
        if self.occupant == Some(robot) {
            return Grant::AlreadyAboard;
        }
        if let Some(position) = self.queue.iter().position(|r| r.robot == robot) {
            return Grant::AlreadyQueued { position };
        }
        if self.occupant.is_none() && self.queue.is_empty() {
            self.occupant = Some(robot);
            return Grant::Boarded;
        }

        let seq = self.next_seq;
        self.next_seq += 1;
        self.queue.push(Request {
            robot,
            emergency,
            seq,
        });
        // Emergencies first, then arrival order.
        self.queue.sort_by_key(|r| (!r.emergency, r.seq));
        let position = self
            .queue
            .iter()
            .position(|r| r.robot == robot)
            .unwrap_or(self.queue.len() - 1);
        Grant::Queued { position }
    }

    /// Leaves the cab. Returns the robot that boards next, if one is waiting.
    ///
    /// # Errors
    ///
    /// Returns [`NotOccupant`] if `robot` is not the one in the cab.
    pub fn release(&mut self, robot: u32) -> Result<Option<u32>, NotOccupant> {
        if self.occupant != Some(robot) {
            return Err(NotOccupant);
        }
        self.occupant = None;
        if self.queue.is_empty() {
            return Ok(None);
        }
        let next = self.queue.remove(0).robot;
        self.occupant = Some(next);
        Ok(Some(next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_request_boards_and_second_waits() {
        let mut e = Elevator::new();
        assert_eq!(e.request(1, false), Grant::Boarded);
        assert_eq!(e.request(2, false), Grant::Queued { position: 0 });
        assert_eq!(e.occupant(), Some(1));
    }

    #[test]
    fn emergency_jumps_the_routine_queue() {
        let mut e = Elevator::new();
        e.request(1, false);
        e.request(2, false);
        assert_eq!(e.request(3, true), Grant::Queued { position: 0 });
    }

    #[test]
    fn same_priority_is_served_in_arrival_order() {
        let mut e = Elevator::new();
        e.request(1, false);
        e.request(2, false);
        e.request(3, false);
        assert_eq!(e.release(1), Ok(Some(2)));
        assert_eq!(e.release(2), Ok(Some(3)));
        assert_eq!(e.release(3), Ok(None));
    }

    #[test]
    fn only_the_occupant_may_release() {
        let mut e = Elevator::new();
        e.request(1, false);
        e.request(2, false);
        assert_eq!(e.release(2), Err(NotOccupant));
        assert_eq!(e.occupant(), Some(1), "a refused release changes nothing");
    }

    #[test]
    fn a_robot_is_never_queued_twice() {
        let mut e = Elevator::new();
        e.request(1, false);
        e.request(2, false);
        assert_eq!(e.request(2, true), Grant::AlreadyQueued { position: 0 });
        assert_eq!(e.request(1, false), Grant::AlreadyAboard);
    }

    #[test]
    fn at_most_one_robot_is_aboard_at_any_time() {
        let mut e = Elevator::new();
        for robot in 0..20 {
            e.request(robot, robot % 3 == 0);
        }
        let mut aboard = e.occupant().expect("someone boarded");
        for _ in 0..19 {
            let next = e.release(aboard).expect("occupant").expect("waiting");
            assert_eq!(e.occupant(), Some(next));
            aboard = next;
        }
        assert_eq!(e.release(aboard), Ok(None));
        assert_eq!(e.occupant(), None);
    }

    #[test]
    fn empty_cab_boards_the_next_request_immediately() {
        let mut e = Elevator::new();
        e.request(1, false);
        assert_eq!(e.release(1), Ok(None));
        assert_eq!(e.request(2, false), Grant::Boarded);
    }
}
