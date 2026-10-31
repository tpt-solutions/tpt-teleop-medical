//! A live feed from the running system into the scene (spec.txt §6.5).
//!
//! The running system pushes one [`Frame`] per control tick, and any audit
//! events as they happen. [`Feed::snapshot`] turns the buffered history into a
//! [`Scene`] for the window or the HTML export.
//!
//! The buffers are bounded. When full, the oldest frames and events are dropped,
//! so a long run cannot grow memory without limit. Dropping old history is safe
//! for the display, because the window shows recent motion. The audit journal in
//! `tpt-t-medical-compliance` is the record that keeps everything.

use std::collections::VecDeque;

use crate::scene::{Copy3, Event, Force, Scene};

/// One control tick, as the running system reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// When the tick ran, in microseconds.
    pub at_us: u64,
    /// The tool tip position, or `None` when it is unknown (for example after a
    /// sensor fault). Unknown positions are left out of the trajectory, not
    /// filled in.
    pub tool_m: Option<[f64; 3]>,
    /// The force on the tool, in newtons.
    pub force_n: [f64; 3],
    /// The three TMR copies, as the voter reports them.
    pub tmr: [Copy3; 3],
}

/// A bounded buffer of frames and events, with the static scene context.
#[derive(Debug, Clone, PartialEq)]
pub struct Feed {
    frames: VecDeque<Frame>,
    events: VecDeque<Event>,
    frame_capacity: usize,
    event_capacity: usize,
    envelope_m: Option<([f64; 3], [f64; 3])>,
    fixture_m: Vec<[f64; 3]>,
}

impl Feed {
    /// Creates a feed that keeps the newest `frame_capacity` frames and
    /// `event_capacity` events. A capacity of zero keeps nothing, so it is
    /// refused.
    #[must_use]
    pub fn new(frame_capacity: usize, event_capacity: usize) -> Option<Self> {
        (frame_capacity > 0 && event_capacity > 0).then(|| Self {
            frames: VecDeque::with_capacity(frame_capacity),
            events: VecDeque::with_capacity(event_capacity),
            frame_capacity,
            event_capacity,
            envelope_m: None,
            fixture_m: Vec::new(),
        })
    }

    /// Sets the workspace box shown behind the trajectory, as (min, max) corners.
    pub fn set_envelope(&mut self, min_m: [f64; 3], max_m: [f64; 3]) {
        self.envelope_m = Some((min_m, max_m));
    }

    /// Sets the fixture boundary points to draw.
    pub fn set_fixture(&mut self, points_m: Vec<[f64; 3]>) {
        self.fixture_m = points_m;
    }

    /// Adds one tick, dropping the oldest when full.
    pub fn push_frame(&mut self, frame: Frame) {
        if self.frames.len() == self.frame_capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }

    /// Adds one audit event, dropping the oldest when full.
    pub fn push_event(&mut self, at_us: u64, text: impl Into<String>) {
        if self.events.len() == self.event_capacity {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_us,
            text: text.into(),
        });
    }

    /// Returns how many frames are buffered.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// Builds the scene from the buffered history.
    ///
    /// The trajectory is every known tool position, oldest first. The forces and
    /// the TMR states are taken from the newest frame, because the window shows
    /// what the system reports now.
    #[must_use]
    pub fn snapshot(&self) -> Scene {
        let trajectory_m: Vec<[f64; 3]> = self.frames.iter().filter_map(|f| f.tool_m).collect();
        let newest = self.frames.back();
        let forces = match (newest, trajectory_m.last()) {
            (Some(frame), Some(&at_m)) => vec![Force {
                at_m,
                newtons: frame.force_n,
            }],
            _ => Vec::new(),
        };
        Scene {
            trajectory_m,
            envelope_m: self.envelope_m,
            fixture_m: self.fixture_m.clone(),
            forces,
            events: self.events.iter().cloned().collect(),
            tmr: newest.map_or([Copy3::Unknown; 3], |f| f.tmr),
        }
    }
}

// Values are copied from the inputs, so exact comparison is intended.
#[allow(clippy::float_cmp)]
#[cfg(test)]
mod tests {
    use super::*;

    fn frame(at_us: u64, tool: Option<[f64; 3]>) -> Frame {
        Frame {
            at_us,
            tool_m: tool,
            force_n: [0.0, 0.0, 1.0],
            tmr: [Copy3::Agree; 3],
        }
    }

    #[test]
    fn a_zero_capacity_feed_is_refused() {
        assert!(Feed::new(0, 4).is_none());
        assert!(Feed::new(4, 0).is_none());
    }

    #[test]
    fn the_trajectory_keeps_known_positions_in_order() {
        let mut feed = Feed::new(10, 10).expect("feed");
        feed.push_frame(frame(0, Some([0.0; 3])));
        feed.push_frame(frame(1_000, None));
        feed.push_frame(frame(2_000, Some([0.1, 0.0, 0.0])));
        assert_eq!(
            feed.snapshot().trajectory_m,
            vec![[0.0; 3], [0.1, 0.0, 0.0]]
        );
    }

    #[test]
    fn the_oldest_frames_are_dropped_when_full() {
        let mut feed = Feed::new(2, 10).expect("feed");
        feed.push_frame(frame(0, Some([0.0; 3])));
        feed.push_frame(frame(1_000, Some([0.1, 0.0, 0.0])));
        feed.push_frame(frame(2_000, Some([0.2, 0.0, 0.0])));
        assert_eq!(feed.frames(), 2);
        assert_eq!(
            feed.snapshot().trajectory_m,
            vec![[0.1, 0.0, 0.0], [0.2, 0.0, 0.0]]
        );
    }

    #[test]
    fn the_force_arrow_sits_at_the_newest_known_position() {
        let mut feed = Feed::new(10, 10).expect("feed");
        feed.push_frame(frame(0, Some([0.05, 0.0, 0.0])));
        feed.push_frame(frame(1_000, None));
        let scene = feed.snapshot();
        assert_eq!(scene.forces.len(), 1);
        assert_eq!(scene.forces[0].at_m, [0.05, 0.0, 0.0]);
    }

    #[test]
    fn an_empty_feed_shows_no_copies_as_unknown() {
        let scene = Feed::new(4, 4).expect("feed").snapshot();
        assert!(scene.trajectory_m.is_empty());
        assert!(scene.forces.is_empty());
        assert_eq!(scene.tmr, [Copy3::Unknown; 3]);
    }

    #[test]
    fn the_newest_tmr_state_is_reported() {
        let mut feed = Feed::new(4, 4).expect("feed");
        feed.push_frame(frame(0, Some([0.0; 3])));
        let mut disagreeing = frame(1_000, Some([0.0; 3]));
        disagreeing.tmr = [Copy3::Agree, Copy3::Disagree, Copy3::Agree];
        feed.push_frame(disagreeing);
        assert_eq!(
            feed.snapshot().tmr,
            [Copy3::Agree, Copy3::Disagree, Copy3::Agree]
        );
    }

    #[test]
    fn events_are_kept_oldest_first_and_bounded() {
        let mut feed = Feed::new(4, 2).expect("feed");
        feed.push_event(1, "engaged");
        feed.push_event(2, "paused");
        feed.push_event(3, "resumed");
        let texts: Vec<String> = feed.snapshot().events.into_iter().map(|e| e.text).collect();
        assert_eq!(texts, vec!["paused", "resumed"]);
    }

    #[test]
    fn envelope_and_fixture_context_is_carried_into_the_scene() {
        let mut feed = Feed::new(4, 4).expect("feed");
        feed.set_envelope([-0.1; 3], [0.1; 3]);
        feed.set_fixture(vec![[0.0, 0.0, 0.0]]);
        let scene = feed.snapshot();
        assert_eq!(scene.envelope_m, Some(([-0.1; 3], [0.1; 3])));
        assert_eq!(scene.fixture_m, vec![[0.0, 0.0, 0.0]]);
    }
}
