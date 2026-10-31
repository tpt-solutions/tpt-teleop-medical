//! Operator visualizer for tpt-teleop-medical (spec.txt §6.5, Phase 7).
//!
//! [`scene`] holds what is shown, and it is pure data, so it is tested without a
//! display. There is no window toolkit: egui was dropped because its dependency
//! tree includes Apache-2.0-only crates that `deny.toml` rejects. The operator
//! view is a self-refreshing HTML page, from [`html::render_live`].
//!
//! The window shows the tool's trajectory against its workspace box, the fixture
//! boundary, force arrows, the audit timeline, and the state of the three TMR
//! copies. The demonstration scene is a reference arm swept through a slow
//! motion. A live view takes the scene from the running system through [`feed`].

#![forbid(unsafe_code)]

pub mod feed;
pub mod html;
pub mod scene;
