//! Operator visualizer for tpt-teleop-medical (spec.txt §6.5, Phase 7).
//!
//! [`scene`] holds what is shown, and it is pure data, so it is tested without a
//! display. The window is in `main.rs`, built with egui (MIT OR Apache-2.0).
//!
//! The window shows the tool's trajectory against its workspace box, the fixture
//! boundary, force arrows, the audit timeline, and the state of the three TMR
//! copies. The demonstration scene is a reference arm swept through a slow
//! motion. A live view takes the scene from the running system through [`feed`].

#![forbid(unsafe_code)]

pub mod feed;
pub mod html;
pub mod scene;
