//! Rectified stereo depth for the endoscope (spec.txt §4.4).
//!
//! The two images must already be rectified, so that a point lies on the same
//! row in both. Each pixel of the left image is matched along its row in the
//! right image by the sum of absolute differences over a square window. The
//! disparity is the shift that matches best. Depth follows from
//! `Z = fx · baseline / disparity`.
//!
//! A pixel is kept only if the match is unique enough and it agrees in both
//! directions. The left-right check rejects occlusions and most false matches,
//! so a missing value is reported as `None`, not as a guess.
//!
//! Block matching is O(width · height · disparities · window²). It is exact and
//! simple, and it is not a real-time implementation. The spec's latency targets
//! need the target hardware.

#![allow(
    clippy::many_single_char_names,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use crate::Gray;

/// Parameters of the matcher.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matcher {
    /// Half the window size. The window is `(2·radius + 1)` pixels square.
    pub radius: usize,
    /// Largest disparity searched, in pixels. The search covers `0..=max_disparity`.
    pub max_disparity: usize,
    /// Largest SAD per pixel accepted, from 0 (exact) to 255.
    pub max_mean_error: f64,
}

impl Matcher {
    /// Creates a matcher. Returns `None` for a zero window or search range, or
    /// an error bound outside 0 to 255.
    #[must_use]
    pub fn new(radius: usize, max_disparity: usize, max_mean_error: f64) -> Option<Self> {
        let ok = radius > 0
            && max_disparity > 0
            && max_mean_error.is_finite()
            && (0.0..=255.0).contains(&max_mean_error);
        ok.then_some(Self {
            radius,
            max_disparity,
            max_mean_error,
        })
    }
}

/// A disparity map. `None` marks a pixel with no reliable match.
#[derive(Debug, Clone, PartialEq)]
pub struct Disparity {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Row-major disparities, in pixels.
    pub values: Vec<Option<f32>>,
}

/// Why a stereo pair could not be processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StereoError {
    /// The two images have different sizes.
    SizeMismatch,
    /// The images are too small for the window and the search range.
    TooSmall,
}

impl std::fmt::Display for StereoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SizeMismatch => f.write_str("left and right images have different sizes"),
            Self::TooSmall => f.write_str("images are too small for the window and search range"),
        }
    }
}

impl std::error::Error for StereoError {}

/// Matches a rectified pair and returns the disparity of each left pixel.
///
/// # Errors
///
/// Returns an error if the images differ in size, or are too small for the
/// matcher's window and disparity range.
pub fn disparity(left: &Gray, right: &Gray, m: &Matcher) -> Result<Disparity, StereoError> {
    if left.width != right.width || left.height != right.height {
        return Err(StereoError::SizeMismatch);
    }
    let (w, h) = (left.width, left.height);
    if w < 2 * m.radius + 1 + m.max_disparity || h < 2 * m.radius + 1 {
        return Err(StereoError::TooSmall);
    }
    let forward = best_matches(left, right, m, -1);
    let backward = best_matches(right, left, m, 1);
    let mut values = vec![None; w * h];
    for y in 0..h {
        for x in 0..w {
            let Some(d) = forward[y * w + x] else {
                continue;
            };
            // Follow the match into the right image and check it maps back.
            let xr = x as isize - d as isize;
            let Ok(xr) = usize::try_from(xr) else {
                continue;
            };
            if backward[y * w + xr] == Some(d) {
                values[y * w + x] = Some(d as f32);
            }
        }
    }
    Ok(Disparity {
        width: w,
        height: h,
        values,
    })
}

/// For each pixel of `a`, the disparity into `b` with the lowest error, or `None`
/// if that error is above the bound. A match is taken to be unique only if the best
/// error is clearly lower than the second best.
///
/// `dir` is `-1` when `b` is the right image and `a` the left, since a left pixel
/// at `x` matches the right pixel at `x - d`. It is `+1` for the reverse
/// direction, used by the left-right check.
fn best_matches(a: &Gray, b: &Gray, m: &Matcher, dir: isize) -> Vec<Option<usize>> {
    let (w, h) = (a.width, a.height);
    let r = m.radius;
    let window = ((2 * r + 1) * (2 * r + 1)) as f64;
    let mut out = vec![None; w * h];
    for y in r..h - r {
        for x in r..w - r {
            let mut best = (f64::INFINITY, 0_usize);
            let mut second = f64::INFINITY;
            for d in 0..=m.max_disparity {
                // Every window pixel must land inside `b`.
                let Some(xb) = shifted(x, dir * d as isize, r, w) else {
                    break;
                };
                let sad = window_sad(a, b, x, y, xb as isize - x as isize, r);
                if sad < best.0 {
                    second = best.0;
                    best = (sad, d);
                } else if sad < second {
                    second = sad;
                }
            }
            let mean = best.0 / window;
            let unique = best.0 < second * 0.9 || second.is_infinite();
            if unique && mean <= m.max_mean_error {
                out[y * w + x] = Some(best.1);
            }
        }
    }
    out
}

/// The column in `b` that `x` maps to under `shift`, if the whole window stays
/// inside the image.
fn shifted(x: usize, shift: isize, r: usize, width: usize) -> Option<usize> {
    let xb = x as isize + shift;
    let lo = xb - r as isize;
    let hi = xb + r as isize;
    if lo < 0 || hi >= width as isize {
        None
    } else {
        usize::try_from(xb).ok()
    }
}

/// Sum of absolute differences between the window at `x` in `a` and the window at
/// `x + shift` in `b`, on the same row.
fn window_sad(a: &Gray, b: &Gray, x: usize, y: usize, shift: isize, r: usize) -> f64 {
    let mut sum = 0.0;
    for dy in 0..=2 * r {
        let row = (y + dy - r) * a.width;
        for dx in 0..=2 * r {
            let xa = x + dx - r;
            let xb = (xa as isize + shift) as usize;
            let va = f64::from(a.data[row + xa]);
            let vb = f64::from(b.data[row + xb]);
            sum += (va - vb).abs();
        }
    }
    sum
}

/// Camera intrinsics of the rectified left camera, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intrinsics {
    /// Focal length, horizontal, in pixels.
    pub fx: f64,
    /// Focal length, vertical, in pixels.
    pub fy: f64,
    /// Principal point, horizontal, in pixels.
    pub cx: f64,
    /// Principal point, vertical, in pixels.
    pub cy: f64,
}

impl Intrinsics {
    /// Creates intrinsics. Returns `None` unless both focal lengths are positive
    /// and every value is finite.
    #[must_use]
    pub fn new(fx: f64, fy: f64, cx: f64, cy: f64) -> Option<Self> {
        let finite = [fx, fy, cx, cy].iter().all(|v| v.is_finite());
        (finite && fx > 0.0 && fy > 0.0).then_some(Self { fx, fy, cx, cy })
    }
}

/// Depth in metres from a disparity, for a rectified pair with the given baseline
/// in metres. Returns `None` for a non-positive disparity, which means no depth.
#[must_use]
pub fn depth_m(disparity_px: f32, intrinsics: &Intrinsics, baseline_m: f64) -> Option<f64> {
    let d = f64::from(disparity_px);
    (d > 0.0 && baseline_m > 0.0 && d.is_finite()).then(|| intrinsics.fx * baseline_m / d)
}

/// Back-projects one pixel with depth into the camera frame, in metres.
#[must_use]
pub fn back_project(u: f64, v: f64, depth_m: f64, k: &Intrinsics) -> [f64; 3] {
    [
        (u - k.cx) * depth_m / k.fx,
        (v - k.cy) * depth_m / k.fy,
        depth_m,
    ]
}

/// Reconstructs the visible surface as points in the camera frame, from a
/// disparity map. Pixels without a depth are skipped.
#[must_use]
pub fn point_cloud(d: &Disparity, k: &Intrinsics, baseline_m: f64) -> Vec<[f64; 3]> {
    let mut points = Vec::new();
    for y in 0..d.height {
        for x in 0..d.width {
            let Some(disp) = d.values[y * d.width + x] else {
                continue;
            };
            if let Some(z) = depth_m(disp, k, baseline_m) {
                points.push(back_project(x as f64, y as f64, z, k));
            }
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic texture, so each window is distinct along its row.
    fn texture(x: usize, y: usize) -> u8 {
        let v = (x * 37 + y * 91 + (x * y) % 17) % 251;
        u8::try_from(v).unwrap_or(0)
    }

    /// A rectified pair where every right pixel is the left pixel `shift` to its
    /// left. The left image is the texture, the right is the texture shifted.
    fn pair(width: usize, height: usize, shift: usize) -> (Gray, Gray) {
        let left: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).map(move |x| texture(x, y)))
            .collect();
        let right: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).map(move |x| texture(x + shift, y)))
            .collect();
        (
            Gray::new(width, height, left).expect("left"),
            Gray::new(width, height, right).expect("right"),
        )
    }

    #[test]
    fn a_known_shift_is_recovered_in_the_interior() {
        let (left, right) = pair(80, 40, 6);
        let m = Matcher::new(3, 12, 8.0).expect("matcher");
        let d = disparity(&left, &right, &m).expect("disparity");
        let mut checked = 0;
        for y in 4..36 {
            for x in 20..60 {
                if let Some(v) = d.values[y * d.width + x] {
                    checked += 1;
                    assert!((v - 6.0).abs() < 0.5, "pixel ({x},{y}) gave {v}");
                }
            }
        }
        assert!(checked > 500, "only {checked} pixels matched");
    }

    #[test]
    fn depth_is_focal_length_times_baseline_over_disparity() {
        let k = Intrinsics::new(500.0, 500.0, 40.0, 20.0).expect("intrinsics");
        let z = depth_m(5.0, &k, 0.004).expect("depth");
        assert!((z - 0.4).abs() < 1e-12, "{z}");
        assert_eq!(depth_m(0.0, &k, 0.004), None);
    }

    #[test]
    fn a_point_cloud_back_projects_each_matched_pixel() {
        let (left, right) = pair(80, 40, 6);
        let m = Matcher::new(3, 12, 8.0).expect("matcher");
        let d = disparity(&left, &right, &m).expect("disparity");
        let k = Intrinsics::new(500.0, 500.0, 40.0, 20.0).expect("intrinsics");
        let cloud = point_cloud(&d, &k, 0.004);
        assert_ne!(cloud.len(), 0);
        // Every point is in front of the camera, at the depth of disparity 6.
        let expected_z = 500.0 * 0.004 / 6.0;
        assert!(cloud.iter().all(|p| (p[2] - expected_z).abs() < 0.05));
    }

    #[test]
    fn mismatched_or_tiny_inputs_are_refused() {
        let (left, _) = pair(80, 40, 6);
        let (other, _) = pair(79, 40, 6);
        let m = Matcher::new(3, 12, 8.0).expect("matcher");
        assert_eq!(disparity(&left, &other, &m), Err(StereoError::SizeMismatch));
        let (small_l, small_r) = pair(10, 10, 1);
        assert_eq!(
            disparity(&small_l, &small_r, &m),
            Err(StereoError::TooSmall)
        );
        assert!(Matcher::new(0, 4, 1.0).is_none());
        assert!(Intrinsics::new(0.0, 1.0, 0.0, 0.0).is_none());
    }
}
