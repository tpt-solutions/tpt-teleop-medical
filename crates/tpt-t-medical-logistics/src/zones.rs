//! Infection-control zones (spec.txt §5.2).
//!
//! A robot that has been in a non-clean zone is contaminated until a verified
//! UV-C cycle clears it. It may re-enter a clean zone only after that. The
//! sterile flag is cleared whenever the robot enters a non-clean zone, and only
//! a complete UV coverage result sets it again.

use tpt_t_medical_sterilize::Coverage;

/// A hospital zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// Clean corridors and rooms.
    Clean,
    /// Dirty utility areas.
    Dirty,
    /// An isolation room. Anything that enters it must be sterilized before it
    /// goes back to a clean zone.
    Isolation,
}

/// Why a zone move was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneError {
    /// The robot has left a non-clean zone and has not been sterilized since.
    NeedsSterilization,
}

impl std::fmt::Display for ZoneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("robot must be UV-sterilized before re-entering a clean zone")
    }
}

impl std::error::Error for ZoneError {}

/// A delivery robot and its infection-control state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Robot {
    id: u32,
    zone: Zone,
    sterile: bool,
}

impl Robot {
    /// Creates a robot in a clean zone. It is assumed to start sterile.
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self {
            id,
            zone: Zone::Clean,
            sterile: true,
        }
    }

    /// Returns the robot's identifier.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns the zone the robot is in.
    #[must_use]
    pub const fn zone(&self) -> Zone {
        self.zone
    }

    /// Returns `true` if the robot may enter a clean zone.
    #[must_use]
    pub const fn is_sterile(&self) -> bool {
        self.sterile
    }

    /// Moves the robot to `zone`.
    ///
    /// Moving to a non-clean zone always succeeds, and it clears the sterile
    /// flag. Moving to a clean zone is refused unless the robot is sterile.
    ///
    /// # Errors
    ///
    /// Returns [`ZoneError::NeedsSterilization`] when entering a clean zone
    /// from a non-clean zone without a completed UV cycle since.
    pub fn move_to(&mut self, zone: Zone) -> Result<(), ZoneError> {
        match zone {
            Zone::Clean => {
                if self.zone != Zone::Clean && !self.sterile {
                    return Err(ZoneError::NeedsSterilization);
                }
            }
            Zone::Dirty | Zone::Isolation => {
                self.sterile = false;
            }
        }
        self.zone = zone;
        Ok(())
    }

    /// Records the result of a UV-C cycle run on the robot.
    ///
    /// Only [`Coverage::Complete`] clears the contamination. An incomplete
    /// cycle leaves the robot as it was, and it must be run again.
    pub fn record_uv_cycle(&mut self, coverage: Coverage) {
        if coverage == Coverage::Complete {
            self.sterile = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete() -> Coverage {
        Coverage::Complete
    }

    fn incomplete() -> Coverage {
        Coverage::Incomplete {
            under_dosed: 1,
            first_index: 0,
        }
    }

    #[test]
    fn robot_moves_freely_within_clean_zones() {
        let mut r = Robot::new(1);
        assert_eq!(r.move_to(Zone::Clean), Ok(()));
        assert!(r.is_sterile());
    }

    #[test]
    fn leaving_clean_zone_contaminates_the_robot() {
        let mut r = Robot::new(1);
        r.move_to(Zone::Isolation)
            .expect("isolation is always allowed");
        assert!(!r.is_sterile());
    }

    #[test]
    fn returning_to_clean_without_sterilization_is_refused() {
        let mut r = Robot::new(1);
        r.move_to(Zone::Isolation).expect("allowed");
        assert_eq!(r.move_to(Zone::Clean), Err(ZoneError::NeedsSterilization));
        assert_eq!(r.zone(), Zone::Isolation, "refused move changes nothing");
    }

    #[test]
    fn dirty_zone_also_needs_sterilization_before_clean() {
        let mut r = Robot::new(1);
        r.move_to(Zone::Dirty).expect("allowed");
        assert_eq!(r.move_to(Zone::Clean), Err(ZoneError::NeedsSterilization));
    }

    #[test]
    fn complete_uv_cycle_allows_return_to_clean() {
        let mut r = Robot::new(1);
        r.move_to(Zone::Isolation).expect("allowed");
        r.record_uv_cycle(complete());
        assert_eq!(r.move_to(Zone::Clean), Ok(()));
    }

    #[test]
    fn incomplete_uv_cycle_does_not_clear_contamination() {
        let mut r = Robot::new(1);
        r.move_to(Zone::Isolation).expect("allowed");
        r.record_uv_cycle(incomplete());
        assert_eq!(r.move_to(Zone::Clean), Err(ZoneError::NeedsSterilization));
    }

    #[test]
    fn staying_in_a_non_clean_zone_keeps_the_contamination_flag() {
        let mut r = Robot::new(1);
        r.move_to(Zone::Isolation).expect("allowed");
        r.move_to(Zone::Isolation).expect("allowed");
        assert!(!r.is_sterile());
    }
}
