//! From a critical-structure segmentation to a virtual fixture (spec.txt §4.3).
//!
//! The segmentation itself comes from outside this crate: a clinician-reviewed
//! mask of arteries or nerves, produced by a segmentation tool. This module is
//! the glue between that mask and the distance transform in [`crate::volume`]:
//!
//! 1. It checks the mask's sizes, spacings and origin.
//! 2. It resamples the mask onto one isotropic spacing, the smallest of the input
//!    spacings, by nearest neighbour. [`crate::volume::Volume`] takes a single
//!    spacing, so CT with thin slices must be made regular first.
//! 3. It hands the resampled mask to [`crate::volume::signed_distance`], which
//!    gives the signed-distance grid that [`tpt_t_medical_surgical::guard`] reads.
//!
//! The distance is measured between voxel centres, so it is accurate to about
//! one voxel. That is the bound a fixture must tolerate. A structure that fills
//! the whole grid, or is absent from it, has no boundary and is refused.
//!
//! [`window`] is a helper that thresholds Hounsfield units. It is not a
//! segmenter, and it does not replace the reviewed mask.

use tpt_t_medical_surgical::SdfGrid;

use crate::volume::{MAX_VOXELS, Volume, signed_distance};

/// Millimetres to metres. [`SdfGrid`] and [`Volume`] are in metres.
const MM_TO_M: f64 = 1.0e-3;

/// A segmentation mask on its own grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mask<'a> {
    /// Voxels along x, y and z.
    pub dims: [usize; 3],
    /// Voxel size along x, y and z, in millimetres.
    pub spacing_mm: [f64; 3],
    /// Position of voxel `(0, 0, 0)`, in millimetres.
    pub origin_mm: [f64; 3],
    /// `true` inside the structure. Stored x-fastest.
    pub inside: &'a [bool],
}

/// Why a mask cannot become a fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentError {
    /// The mask's sizes, spacings or origin are inconsistent.
    BadMask(&'static str),
    /// The mask is empty, or fills the whole grid, so it has no boundary.
    NoBoundary,
    /// The resampled grid would be above [`MAX_VOXELS`].
    TooLarge {
        /// The voxel count the resampling needs.
        voxels: usize,
    },
    /// The resampled grid is not a valid fixture grid.
    Grid,
}

impl std::fmt::Display for SegmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMask(what) => write!(f, "mask is inconsistent: {what}"),
            Self::NoBoundary => f.write_str("mask is empty or fills the grid"),
            Self::TooLarge { voxels } => write!(f, "{voxels} voxels is above the limit"),
            Self::Grid => f.write_str("resampled mask is not a valid fixture grid"),
        }
    }
}

impl std::error::Error for SegmentError {}

/// Marks voxels whose Hounsfield value lies in `lo..=hi`. Use it to pre-select a
/// tissue range for review. It is not a substitute for the reviewed mask.
#[must_use]
pub fn window(hounsfield: &[f32], lo: f32, hi: f32) -> Vec<bool> {
    hounsfield.iter().map(|&v| v >= lo && v <= hi).collect()
}

/// Converts a mask into the signed-distance fixture grid, in metres.
///
/// # Errors
///
/// Returns an error if the mask is inconsistent, has no boundary, or resamples to
/// a grid above [`MAX_VOXELS`].
pub fn fixture_from_mask(mask: &Mask) -> Result<SdfGrid, SegmentError> {
    validate(mask)?;
    let h_mm = mask
        .spacing_mm
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let (dims, inside) = resample(mask, h_mm)?;
    let origin_m = mask.origin_mm.map(|v| v * MM_TO_M);
    let volume = Volume::new(dims, origin_m, h_mm * MM_TO_M, inside).ok_or(SegmentError::Grid)?;
    signed_distance(&volume).ok_or(SegmentError::NoBoundary)
}

fn validate(mask: &Mask) -> Result<(), SegmentError> {
    let [nx, ny, nz] = mask.dims;
    if nx < 2 || ny < 2 || nz < 2 {
        return Err(SegmentError::BadMask(
            "every axis needs at least two voxels",
        ));
    }
    let expected = nx
        .checked_mul(ny)
        .and_then(|p| p.checked_mul(nz))
        .ok_or(SegmentError::BadMask("voxel count overflows"))?;
    if mask.inside.len() != expected {
        return Err(SegmentError::BadMask(
            "mask length differs from its dimensions",
        ));
    }
    if !mask.spacing_mm.iter().all(|s| s.is_finite() && *s > 0.0)
        || !mask.origin_mm.iter().all(|v| v.is_finite())
    {
        return Err(SegmentError::BadMask(
            "spacing or origin is not finite and positive",
        ));
    }
    Ok(())
}

/// Resamples the mask to spacing `h_mm` by nearest neighbour. The new grid covers
/// the same extent as the old one, so its last voxel sits where the old one did.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn resample(mask: &Mask, h_mm: f64) -> Result<([usize; 3], Vec<bool>), SegmentError> {
    let mut dims = [0_usize; 3];
    for (axis, dim) in dims.iter_mut().enumerate() {
        let extent = (mask.dims[axis] - 1) as f64 * mask.spacing_mm[axis];
        *dim = (extent / h_mm).round() as usize + 1;
    }
    let voxels = dims
        .iter()
        .try_fold(1_usize, |acc, &n| acc.checked_mul(n))
        .ok_or(SegmentError::TooLarge { voxels: usize::MAX })?;
    if voxels > MAX_VOXELS {
        return Err(SegmentError::TooLarge { voxels });
    }

    let [sx, sy, sz] = mask.dims;
    let mut out = Vec::with_capacity(voxels);
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let at_mm = [x, y, z].map(|i| i as f64 * h_mm);
                let idx = [0, 1, 2].map(|a| {
                    let n = [sx, sy, sz][a];
                    let v = (at_mm[a] / mask.spacing_mm[a]).round();
                    (v.max(0.0) as usize).min(n - 1)
                });
                out.push(mask.inside[idx[0] + sx * (idx[1] + sy * idx[2])]);
            }
        }
    }
    Ok((dims, out))
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::cast_precision_loss)]
mod tests {
    use super::*;

    /// A solid ball of voxels within `radius_mm` of its centre, on an `n^3` grid.
    fn ball(n: usize, spacing: [f64; 3], centre_vox: [f64; 3], radius_mm: f64) -> Vec<bool> {
        let mut out = Vec::with_capacity(n * n * n);
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let p = [x as f64, y as f64, z as f64];
                    let d2: f64 = (0..3)
                        .map(|a| ((p[a] - centre_vox[a]) * spacing[a]).powi(2))
                        .sum();
                    out.push(d2.sqrt() <= radius_mm);
                }
            }
        }
        out
    }

    fn mask(dims: [usize; 3], spacing: [f64; 3], inside: &[bool]) -> Mask<'_> {
        Mask {
            dims,
            spacing_mm: spacing,
            origin_mm: [0.0; 3],
            inside,
        }
    }

    #[test]
    fn the_distance_inside_and_outside_matches_the_sphere() {
        let n = 41;
        let inside = ball(n, [1.0; 3], [20.0; 3], 8.0);
        let grid = fixture_from_mask(&mask([n; 3], [1.0; 3], &inside)).expect("fixture");
        // Queries are in metres. The centre is 8 mm inside, to within a voxel.
        let inner = grid.distance([20.0e-3; 3]).expect("inside the grid");
        assert!((inner + 8.0e-3).abs() <= 1.0e-3, "centre {inner}");
        // A point 5 mm outside the sphere is 5 mm positive, to within a voxel.
        let outer = grid.distance([20.0e-3, 20.0e-3, 33.0e-3]).expect("outside");
        assert!((outer - 5.0e-3).abs() <= 1.0e-3, "outside {outer}");
    }

    #[test]
    fn anisotropic_input_is_resampled_to_the_finest_spacing() {
        // 2 mm slices and 0.5 mm pixels: the fixture spacing is 0.5 mm.
        let n = 21;
        let spacing = [0.5, 0.5, 2.0];
        let inside = ball(n, spacing, [10.0, 10.0, 10.0], 3.0);
        let grid = fixture_from_mask(&mask([n; 3], spacing, &inside)).expect("fixture");
        let centre_m =
            [10.0 * spacing[0], 10.0 * spacing[1], 10.0 * spacing[2]].map(|v| v * MM_TO_M);
        let inner = grid.distance(centre_m).expect("inside");
        assert!(inner < -2.0e-3 && inner > -4.0e-3, "centre {inner}");
    }

    #[test]
    fn an_empty_or_full_mask_has_no_boundary() {
        let n = 4;
        let none = vec![false; n * n * n];
        let all = vec![true; n * n * n];
        assert_eq!(
            fixture_from_mask(&mask([n; 3], [1.0; 3], &none)),
            Err(SegmentError::NoBoundary)
        );
        assert_eq!(
            fixture_from_mask(&mask([n; 3], [1.0; 3], &all)),
            Err(SegmentError::NoBoundary)
        );
    }

    #[test]
    fn a_mask_of_the_wrong_length_is_refused() {
        let inside = vec![true; 10];
        assert!(matches!(
            fixture_from_mask(&mask([4; 3], [1.0; 3], &inside)),
            Err(SegmentError::BadMask(_))
        ));
    }

    #[test]
    fn a_grid_above_the_voxel_limit_is_refused_before_allocating() {
        // One structure voxel, so the mask has a boundary. The finest spacing is
        // 1e-6 mm, so the 1 mm extent resamples to about a million voxels per axis.
        let mut inside = vec![false; 8];
        inside[0] = true;
        let fine = Mask {
            dims: [2, 2, 2],
            spacing_mm: [1.0, 1.0, 1.0e-6],
            origin_mm: [0.0; 3],
            inside: &inside,
        };
        assert!(matches!(
            fixture_from_mask(&fine),
            Err(SegmentError::TooLarge { .. })
        ));
    }

    #[test]
    fn the_window_selects_values_inclusively() {
        let v = [-1000.0_f32, 0.0, 150.0, 300.0, 1200.0];
        assert_eq!(
            window(&v, 100.0, 300.0),
            vec![false, false, true, true, false]
        );
    }
}
