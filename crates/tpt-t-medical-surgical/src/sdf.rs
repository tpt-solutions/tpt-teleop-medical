//! Signed-distance grid for virtual fixtures (spec.txt §4.3).
//!
//! The grid is precomputed from the pre-operative segmentation. A lookup takes
//! constant time: it reads eight corner values and blends them trilinearly.
//! The distance is negative inside the structure and positive outside.
//!
//! A point outside the grid returns `None`. The caller must treat that as
//! unknown, and must not assume the point is safe.

/// A regular 3D grid of signed distances, in metres.
#[derive(Debug, Clone, PartialEq)]
pub struct SdfGrid {
    origin: [f64; 3],
    spacing: f64,
    dims: [u32; 3],
    values: Vec<f32>,
}

impl SdfGrid {
    /// Creates a grid. `values` is stored in x-fastest order: the index of
    /// `(x, y, z)` is `x + nx * (y + ny * z)`.
    ///
    /// Returns `None` if the spacing is not positive and finite, if any
    /// dimension is below 2, if `values` does not match the dimensions, or if
    /// any value is not finite.
    #[must_use]
    pub fn new(origin: [f64; 3], spacing: f64, dims: [u32; 3], values: Vec<f32>) -> Option<Self> {
        let [nx, ny, nz] = dims.map(|n| n as usize);
        let expected = nx.checked_mul(ny)?.checked_mul(nz)?;
        let valid = spacing.is_finite()
            && spacing > 0.0
            && origin.iter().all(|v| v.is_finite())
            && dims.iter().all(|&n| n >= 2)
            && values.len() == expected
            && values.iter().all(|v| v.is_finite());
        valid.then_some(Self {
            origin,
            spacing,
            dims,
            values,
        })
    }

    /// Returns the signed distance at `point`, or `None` if the point is
    /// outside the grid or any coordinate is not finite.
    #[must_use]
    pub fn distance(&self, point: [f64; 3]) -> Option<f64> {
        let mut base = [0usize; 3];
        let mut frac = [0.0f64; 3];
        for axis in 0..3 {
            let u = (point[axis] - self.origin[axis]) / self.spacing;
            let last = f64::from(self.dims[axis] - 1);
            // The negated form also rejects NaN.
            if !(0.0..=last).contains(&u) {
                return None;
            }
            let (cell, t) = split_cell(u, self.dims[axis]);
            base[axis] = cell as usize;
            frac[axis] = t;
        }

        let [nx, ny, _] = self.dims.map(|n| n as usize);
        let at = |dx: usize, dy: usize, dz: usize| -> f64 {
            let index = (base[0] + dx) + nx * ((base[1] + dy) + ny * (base[2] + dz));
            f64::from(self.values[index])
        };

        let [tx, ty, tz] = frac;
        let c00 = lerp(at(0, 0, 0), at(1, 0, 0), tx);
        let c10 = lerp(at(0, 1, 0), at(1, 1, 0), tx);
        let c01 = lerp(at(0, 0, 1), at(1, 0, 1), tx);
        let c11 = lerp(at(0, 1, 1), at(1, 1, 1), tx);
        let c0 = lerp(c00, c10, ty);
        let c1 = lerp(c01, c11, ty);
        Some(lerp(c0, c1, tz))
    }
}

/// Splits a grid coordinate into the lower cell index and the fraction within
/// that cell. The index is clamped so that index + 1 is always valid, which
/// makes a point on the far face use the last cell with fraction 1.
///
/// The caller has checked `0 <= u <= n - 1`, so the cast is in range.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn split_cell(u: f64, n: u32) -> (u32, f64) {
    let cell = (u.floor() as u32).min(n - 2);
    (cell, u - f64::from(cell))
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 5×5×5 grid, 1 cm apart, of the signed distance to the plane z = 0.02 m.
    #[allow(clippy::cast_possible_truncation)]
    fn plane_grid() -> SdfGrid {
        let n: u32 = 5;
        let mut values = Vec::new();
        for z in 0..n {
            for _y in 0..n {
                for _x in 0..n {
                    let z_m = f64::from(z) * 0.01;
                    values.push((z_m - 0.02) as f32);
                }
            }
        }
        SdfGrid::new([0.0; 3], 0.01, [n; 3], values).expect("valid grid")
    }

    #[test]
    fn grid_nodes_return_their_stored_values() {
        let g = plane_grid();
        // Values are stored as f32, so compare within f32 precision.
        let d = g.distance([0.0, 0.0, 0.0]).expect("inside");
        assert!((d + 0.02).abs() < 1e-7, "got {d}");
        let d = g.distance([0.02, 0.02, 0.02]).expect("inside");
        assert!(d.abs() < 1e-6);
    }

    #[test]
    fn interpolation_is_exact_for_a_linear_field() {
        // Trilinear interpolation reproduces a linear function exactly.
        let g = plane_grid();
        let d = g.distance([0.013, 0.007, 0.0345]).expect("inside");
        assert!((d - (0.0345 - 0.02)).abs() < 1e-6, "got {d}");
    }

    #[test]
    fn sign_tells_inside_from_outside() {
        let g = plane_grid();
        assert!(g.distance([0.01, 0.01, 0.01]).expect("inside") < 0.0);
        assert!(g.distance([0.01, 0.01, 0.03]).expect("inside") > 0.0);
    }

    #[test]
    fn far_face_is_inside_the_grid() {
        let g = plane_grid();
        assert!(g.distance([0.04, 0.04, 0.04]).is_some());
    }

    #[test]
    fn points_outside_the_grid_are_unknown() {
        let g = plane_grid();
        assert_eq!(g.distance([-0.001, 0.0, 0.0]), None);
        assert_eq!(g.distance([0.0, 0.0, 0.0401]), None);
        assert_eq!(g.distance([f64::NAN, 0.0, 0.0]), None);
    }

    #[test]
    fn malformed_grids_are_refused() {
        assert!(
            SdfGrid::new([0.0; 3], 0.0, [2, 2, 2], vec![0.0; 8]).is_none(),
            "zero spacing"
        );
        assert!(
            SdfGrid::new([0.0; 3], 0.01, [1, 2, 2], vec![0.0; 4]).is_none(),
            "dimension 1"
        );
        assert!(
            SdfGrid::new([0.0; 3], 0.01, [2, 2, 2], vec![0.0; 7]).is_none(),
            "short data"
        );
        assert!(
            SdfGrid::new([0.0; 3], 0.01, [2, 2, 2], vec![f32::NAN; 8]).is_none(),
            "NaN value"
        );
    }
}
