//! Tick-driven event loop with one SPSC lane per producer.
//!
//! Each producer (safety monitor, operator input, network link) owns one
//! lane and pushes events into it. The loop owns the consumer side of every
//! lane and applies events to the [`Machine`] when [`EventLoop::tick`] runs.
//!
//! Lane 0 is the emergency lane. It is drained completely at the start of
//! every tick, before any other lane, so an emergency stop never waits behind
//! queued commands. The other lanes share the tick's budget.
//!
//! A lane is a priority, not an authority. Any event may be pushed into any
//! lane. Restricting which producer may clear an emergency is a separate
//! decision and is not made here.

use std::sync::Arc;

use tpt_t_medical_ring::SpscRing;

use crate::state::{Event, Machine, State};

/// Index of the emergency lane, which is drained first on every tick.
pub const EMERGENCY_LANE: usize = 0;

/// What one call to [`EventLoop::tick`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickReport {
    /// Events the machine accepted.
    pub applied: usize,
    /// Events the machine rejected in the current state.
    pub rejected: usize,
}

/// The consumer side of the event loop. Holds the state machine.
#[derive(Debug)]
pub struct EventLoop<const LANES: usize> {
    machine: Machine,
    lanes: [Arc<SpscRing<Event>>; LANES],
}

impl<const LANES: usize> EventLoop<LANES> {
    /// Creates a loop with `LANES` lanes of `lane_capacity` events each.
    ///
    /// Returns the loop and one producer handle per lane, in lane order. All
    /// allocation happens here. `tick` never allocates.
    #[must_use]
    pub fn new(lane_capacity: usize) -> (Self, [Arc<SpscRing<Event>>; LANES]) {
        const { assert!(LANES > 0, "an event loop needs at least the emergency lane") };
        let lanes: [Arc<SpscRing<Event>>; LANES] =
            std::array::from_fn(|_| Arc::new(SpscRing::with_capacity(lane_capacity)));
        let producers = lanes.clone();
        (
            Self {
                machine: Machine::new(),
                lanes,
            },
            producers,
        )
    }

    /// Returns the current state.
    #[must_use]
    pub const fn state(&self) -> State {
        self.machine.state()
    }

    /// Applies pending events.
    ///
    /// The emergency lane is drained up to its capacity, which bounds the
    /// work even if a producer keeps pushing. The remaining lanes share
    /// `budget` events between them, in lane order.
    #[must_use]
    pub fn tick(&mut self, budget: usize) -> TickReport {
        let mut report = TickReport::default();

        let emergency = &self.lanes[EMERGENCY_LANE];
        for _ in 0..emergency.capacity() {
            let Some(event) = emergency.pop() else {
                break;
            };
            apply(&mut self.machine, event, &mut report);
        }

        let mut remaining = budget;
        for lane in self.lanes.iter().skip(EMERGENCY_LANE + 1) {
            let mut taken = 0;
            while taken < remaining {
                let Some(event) = lane.pop() else {
                    break;
                };
                apply(&mut self.machine, event, &mut report);
                taken += 1;
            }
            remaining -= taken;
        }

        report
    }
}

fn apply(machine: &mut Machine, event: Event, report: &mut TickReport) {
    if machine.handle(event).is_ok() {
        report.applied += 1;
    } else {
        report.rejected += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn events_from_a_lane_are_applied_in_order() {
        let (mut ev, [_estop, ops]) = EventLoop::<2>::new(8);
        ops.push(Event::Register).expect("room");
        ops.push(Event::Activate).expect("room");
        let report = ev.tick(10);
        assert_eq!(
            report,
            TickReport {
                applied: 2,
                rejected: 0
            }
        );
        assert_eq!(ev.state(), State::Active);
    }

    #[test]
    fn emergency_lane_is_applied_before_queued_commands() {
        let (mut ev, [estop, ops]) = EventLoop::<2>::new(8);
        ops.push(Event::Register).expect("room");
        ops.push(Event::Activate).expect("room");
        estop.push(Event::EmergencyStop).expect("room");

        let report = ev.tick(10);
        // The stop is applied first, so both queued commands arrive in Emergency.
        assert_eq!(
            report,
            TickReport {
                applied: 1,
                rejected: 2
            }
        );
        assert_eq!(ev.state(), State::Emergency);
    }

    #[test]
    fn budget_limits_only_the_non_emergency_lanes() {
        let (mut ev, [estop, ops]) = EventLoop::<2>::new(8);
        ops.push(Event::Register).expect("room");
        ops.push(Event::Activate).expect("room");
        ops.push(Event::Pause).expect("room");

        let first = ev.tick(2);
        assert_eq!(
            first,
            TickReport {
                applied: 2,
                rejected: 0
            }
        );
        assert_eq!(ops.len(), 1, "the third command waits for the next tick");

        estop.push(Event::EmergencyStop).expect("room");
        let second = ev.tick(2);
        // The stop is applied, then the held Pause is rejected in Emergency.
        assert_eq!(
            second,
            TickReport {
                applied: 1,
                rejected: 1
            }
        );
        assert_eq!(ev.state(), State::Emergency);
    }

    #[test]
    fn emergency_lane_drains_all_queued_events_in_one_tick() {
        let (mut ev, [estop]) = EventLoop::<1>::new(2);
        estop.push(Event::EmergencyStop).expect("room");
        estop.push(Event::ClearEmergency).expect("room");
        let report = ev.tick(0);
        assert_eq!(
            report,
            TickReport {
                applied: 2,
                rejected: 0
            }
        );
        assert_eq!(ev.state(), State::Idle);
    }

    #[test]
    fn full_lane_hands_the_event_back_to_the_producer() {
        let (_ev, [_estop, ops]) = EventLoop::<2>::new(1);
        ops.push(Event::Register).expect("room");
        assert_eq!(ops.push(Event::Activate), Err(Event::Activate));
    }

    #[test]
    fn emergency_from_another_thread_is_applied_on_the_next_tick() {
        let (mut ev, [estop, ops]) = EventLoop::<2>::new(8);
        ops.push(Event::Register).expect("room");
        ops.push(Event::Activate).expect("room");
        assert_eq!(ev.tick(10).applied, 2);

        thread::spawn(move || estop.push(Event::EmergencyStop).expect("room"))
            .join()
            .expect("producer panicked");

        let _ = ev.tick(10);
        assert_eq!(ev.state(), State::Emergency);
    }

    #[test]
    fn every_pushed_event_is_applied_or_rejected_exactly_once() {
        const TOTAL: usize = 50_000;
        const CYCLE: [Event; 5] = [
            Event::Register,
            Event::Activate,
            Event::Pause,
            Event::Resume,
            Event::Deregister,
        ];

        let (mut ev, [_estop, ops]) = EventLoop::<2>::new(64);
        let producer = thread::spawn(move || {
            for i in 0..TOTAL {
                let mut event = CYCLE[i % CYCLE.len()];
                while let Err(back) = ops.push(event) {
                    event = back;
                    thread::yield_now();
                }
            }
        });

        let mut seen = 0;
        while !producer.is_finished() || !all_lanes_empty(&ev) {
            let report = ev.tick(16);
            seen += report.applied + report.rejected;
            if report.applied + report.rejected == 0 {
                thread::yield_now();
            }
        }
        producer.join().expect("producer panicked");
        let report = ev.tick(usize::MAX);
        seen += report.applied + report.rejected;

        assert_eq!(seen, TOTAL);
    }

    fn all_lanes_empty(ev: &EventLoop<2>) -> bool {
        ev.lanes.iter().all(|lane| lane.is_empty())
    }
}
