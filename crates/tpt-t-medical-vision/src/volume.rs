//! Critical-structure segmentation to virtual-fixture distance (spec.txt §4.3,
//! §4.4).
//!
//! A segmentation marks each voxel of the pre-op volume as inside a structure or
//! outside it. The signed distance from every voxel to the structure's surface is
//! what the surgical guard reads. It is computed exactly, with a separable
//! distance transform (Felzenszwalb and Huttenlocher, 2012), in time linear in
//! the number of voxels.

#![allow(
    clippy::many_single_char_names,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use tpt_t_medical_surgical::SdfGrid;

/// Largest volume accepted, in voxels. A larger one is refused, not truncated.
pub const MAX_VOXELS: usize = 16_000_000;

/// Stands in for "no feature" in the transform. It is far larger than any
/// squared distance in a valid volume, and it is finite.
const NO_FEATURE: f64 = 1e20;

/// A segmented volume.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    dims: [usize; 3],
    origin_m: [f64; 3],
    spacing_m: f64,
    inside: Vec<bool>,
}

impl Volume {
    /// Creates a volume. `inside` is in x-fastest order. Returns `None` for a
    /// dimension below 2, a non-positive spacing, a size mismatch, or a volume
    /// above [`MAX_VOXELS`].
    #[must_use]
    pub fn new(
        dims: [usize; 3],
        origin_m: [f64; 3],
        spacing_m: f64,
        inside: Vec<bool>,
    ) -> Option<Self> {
        let count = dims[0].checked_mul(dims[1])?.checked_mul(dims[2])?;
        let ok = dims.iter().all(|&d| d >= 2)
            && count <= MAX_VOXELS
            && inside.len() == count
            && spacing_m.is_finite()
            && spacing_m > 0.0
            && origin_m.iter().all(|v| v.is_finite());
        ok.then_some(Self {
            dims,
            origin_m,
            spacing_m,
            inside,
        })
    }
}

/// Computes the signed distance grid of the segmented structure. Negative inside,
/// positive outside, in metres.
///
/// Returns `None` if the structure fills the whole volume, or is absent, because
/// then no surface exists to measure to.
#[must_use]
pub fn signed_distance(volume: &Volume) -> Option<SdfGrid> {
    let any_in = volume.inside.iter().any(|&v| v);
    let any_out = volume.inside.iter().any(|&v| !v);
    if !any_in || !any_out {
        return None;
    }
    // Squared distance from each voxel to the nearest voxel of the other class.
    let to_outside = squared_distance(volume, false);
    let to_inside = squared_distance(volume, true);
    let values: Vec<f32> = volume
        .inside
        .iter()
        .enumerate()
        .map(|(i, &inside)| {
            let d = if inside {
                -to_outside[i].sqrt()
            } else {
                to_inside[i].sqrt()
            };
            (d * volume.spacing_m) as f32
        })
        .collect();
    let dims = volume.dims.map(|d| u32::try_from(d).unwrap_or(u32::MAX));
    SdfGrid::new(volume.origin_m, volume.spacing_m, dims, values)
}

/// Squared distance, in voxels, from each voxel to the nearest voxel whose class
/// is `feature_inside`.
fn squared_distance(volume: &Volume, feature_inside: bool) -> Vec<f64> {
    let [nx, ny, nz] = volume.dims;
    let mut f: Vec<f64> = volume
        .inside
        .iter()
        .map(|&v| if v == feature_inside { 0.0 } else { NO_FEATURE })
        .collect();
    let longest = nx.max(ny).max(nz);
    let mut line = vec![0.0; longest];
    let mut out = vec![0.0; longest];
    let mut v = vec![0_usize; longest];
    let mut z = vec![0.0; longest + 1];

    // Along x, each row is contiguous.
    for k in 0..nz {
        for j in 0..ny {
            let base = nx * (j + ny * k);
            transform_1d(
                &f[base..base + nx],
                &mut out[..nx],
                &mut v[..nx],
                &mut z[..=nx],
            );
            f[base..base + nx].copy_from_slice(&out[..nx]);
        }
    }
    // Along y, with stride nx.
    for k in 0..nz {
        for i in 0..nx {
            for j in 0..ny {
                line[j] = f[i + nx * (j + ny * k)];
            }
            transform_1d(&line[..ny], &mut out[..ny], &mut v[..ny], &mut z[..=ny]);
            for j in 0..ny {
                f[i + nx * (j + ny * k)] = out[j];
            }
        }
    }
    // Along z, with stride nx·ny.
    for j in 0..ny {
        for i in 0..nx {
            for k in 0..nz {
                line[k] = f[i + nx * (j + ny * k)];
            }
            transform_1d(&line[..nz], &mut out[..nz], &mut v[..nz], &mut z[..=nz]);
            for k in 0..nz {
                f[i + nx * (j + ny * k)] = out[k];
            }
        }
    }
    f
}

/// One dimension of the squared-distance transform: the lower envelope of the
/// parabolas `(q - p)² + f[p]`.
#[allow(clippy::cast_precision_loss)]
fn transform_1d(f: &[f64], out: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k = 0_usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let mut s = intersection(f, v[k], q);
        while s <= z[k] {
            k -= 1;
            s = intersection(f, v[k], q);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, slot) in out.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let d = q as f64 - v[k] as f64;
        *slot = d * d + f[v[k]];
    }
}

/// Where the parabolas rooted at `p` and `q` cross.
#[allow(clippy::cast_precision_loss)]
fn intersection(f: &[f64], p: usize, q: usize) -> f64 {
    let (pf, qf) = (p as f64, q as f64);
    ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * qf - 2.0 * pf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ball of `radius` voxels, centred in a cube of `n` voxels a side.
    #[allow(clippy::cast_precision_loss)]
    fn ball(n: usize, radius: f64) -> Volume {
        let c = (n as f64 - 1.0) / 2.0;
        let inside = (0..n)
            .flat_map(|k| (0..n).flat_map(move |j| (0..n).map(move |i| (i, j, k))))
            .map(|(i, j, k)| {
                let d2 = [i, j, k]
                    .iter()
                    .map(|&v| (v as f64 - c).powi(2))
                    .sum::<f64>();
                d2 <= radius * radius
            })
            .collect();
        Volume::new([n; 3], [0.0; 3], 0.001, inside).expect("volume")
    }

    #[test]
    fn distances_match_the_ball_in_every_direction() {
        let v = ball(40, 8.0);
        let sdf = signed_distance(&v).expect("sdf");
        // The centre of the ball is 8 mm inside, to within a voxel.
        let centre = sdf.distance([0.0195, 0.0195, 0.0195]).expect("inside");
        assert!((centre + 0.008).abs() < 0.0015, "centre {centre}");
        // 4 mm outside the surface, along +x. The surface is 8 mm from the centre.
        let outside = sdf
            .distance([0.0195 + 0.008 + 0.004, 0.0195, 0.0195])
            .expect("outside");
        assert!((outside - 0.004).abs() < 0.0015, "outside {outside}");
    }

    #[test]
    fn the_surface_is_near_zero() {
        let v = ball(40, 8.0);
        let sdf = signed_distance(&v).expect("sdf");
        let on_surface = sdf
            .distance([0.0195 + 0.008, 0.0195, 0.0195])
            .expect("surface");
        assert!(on_surface.abs() < 0.0015, "{on_surface}");
    }

    #[test]
    fn a_structure_that_fills_or_misses_the_volume_has_no_distance() {
        let all = Volume::new([4; 3], [0.0; 3], 0.001, vec![true; 64]).expect("volume");
        let none = Volume::new([4; 3], [0.0; 3], 0.001, vec![false; 64]).expect("volume");
        assert!(signed_distance(&all).is_none());
        assert!(signed_distance(&none).is_none());
    }

    #[test]
    fn invalid_volumes_are_refused() {
        assert!(Volume::new([1, 4, 4], [0.0; 3], 0.001, vec![false; 16]).is_none());
        assert!(Volume::new([4; 3], [0.0; 3], 0.0, vec![false; 64]).is_none());
        assert!(Volume::new([4; 3], [0.0; 3], 0.001, vec![false; 10]).is_none());
    }
}
