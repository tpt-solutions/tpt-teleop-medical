//! Augmented-reality overlay: pre-operative points placed in the live image
//! (spec.txt §4.4).
//!
//! The registration from `tpt-t-medical-track` gives the pre-op scan's frame in
//! the tracker's frame. The caller supplies the transform from the scan to the
//! camera, and this module projects each point through the pinhole model. Points
//! behind the camera, and points that fall outside the image, are dropped.

#![allow(
    clippy::many_single_char_names,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use tpt_t_medical_track::Rigid;

use crate::stereo::Intrinsics;

/// The camera that sees the overlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Intrinsics of the rectified left camera.
    pub intrinsics: Intrinsics,
    /// Image width, in pixels.
    pub width: usize,
    /// Image height, in pixels.
    pub height: usize,
}

/// One overlay point in the image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Horizontal pixel position.
    pub u: f64,
    /// Vertical pixel position.
    pub v: f64,
    /// Depth from the camera, in metres. It can drive a depth cue.
    pub depth_m: f64,
}

/// Projects one camera-frame point. Returns `None` if the point is not in front of
/// the camera, or is outside the image.
#[must_use]
pub fn project(p: [f64; 3], camera: &Camera) -> Option<Placed> {
    let [x, y, z] = p;
    if !(z > 0.0 && z.is_finite()) {
        return None;
    }
    let k = &camera.intrinsics;
    let u = k.fx * x / z + k.cx;
    let v = k.fy * y / z + k.cy;
    let inside =
        (0.0..camera.width as f64).contains(&u) && (0.0..camera.height as f64).contains(&v);
    inside.then_some(Placed { u, v, depth_m: z })
}

/// Places scan-frame points in the image. `scan_to_camera` maps a point of the
/// pre-op scan into the camera frame, so it is the registration composed with the
/// tracker-to-camera calibration.
#[must_use]
pub fn overlay(points_scan_m: &[[f64; 3]], scan_to_camera: &Rigid, camera: &Camera) -> Vec<Placed> {
    points_scan_m
        .iter()
        .filter_map(|p| project(scan_to_camera.apply(*p), camera))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            intrinsics: Intrinsics::new(500.0, 500.0, 320.0, 240.0).expect("intrinsics"),
            width: 640,
            height: 480,
        }
    }

    fn identity() -> Rigid {
        Rigid {
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation_m: [0.0; 3],
        }
    }

    #[test]
    fn a_point_on_the_optical_axis_projects_to_the_principal_point() {
        let placed = project([0.0, 0.0, 0.1], &camera()).expect("visible");
        assert!((placed.u - 320.0).abs() < 1e-9 && (placed.v - 240.0).abs() < 1e-9);
        assert!((placed.depth_m - 0.1).abs() < 1e-12);
    }

    #[test]
    fn points_behind_or_outside_the_image_are_dropped() {
        assert!(project([0.0, 0.0, -0.1], &camera()).is_none());
        assert!(project([1.0, 0.0, 0.1], &camera()).is_none());
        assert!(project([f64::NAN, 0.0, 0.1], &camera()).is_none());
    }

    #[test]
    fn the_registration_moves_scan_points_into_the_camera_frame() {
        // The scan sits 10 cm in front of the camera.
        let mut to_camera = identity();
        to_camera.translation_m = [0.0, 0.0, 0.1];
        let placed = overlay(&[[0.0, 0.0, 0.0], [0.5, 0.0, 0.0]], &to_camera, &camera());
        assert_eq!(placed.len(), 1, "the 0.5 m offset leaves the image");
        assert!((placed[0].depth_m - 0.1).abs() < 1e-12);
    }
}
