//! Rigid registration of the tracker's frame to the pre-op scan's frame, from
//! paired fiducial points (spec.txt §4.4, "sub-millimetre registration").
//!
//! The rotation and translation are found by Horn's quaternion method. The
//! best rotation is the eigenvector of the largest eigenvalue of a 4×4 matrix,
//! and that eigenvector is found by Jacobi rotations. The method needs no
//! external linear-algebra library.
//!
//! The result maps a tracker point `b` to the scan frame: `a ≈ R·b + t`.

/// A rigid transform: `a ≈ rotation · b + translation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rigid {
    /// The rotation, row-major.
    pub rotation: [[f64; 3]; 3],
    /// The translation, in metres.
    pub translation_m: [f64; 3],
}

impl Rigid {
    /// Maps a tracker point into the scan frame.
    #[must_use]
    pub fn apply(&self, b: [f64; 3]) -> [f64; 3] {
        core::array::from_fn(|i| {
            (0..3).map(|j| self.rotation[i][j] * b[j]).sum::<f64>() + self.translation_m[i]
        })
    }
}

/// Finds the rigid transform that best maps `moving` onto `fixed`, least squares.
///
/// `moving[i]` and `fixed[i]` are the same fiducial, seen by the tracker and in
/// the scan. Returns `None` if the lists differ in length, have fewer than three
/// points, hold a non-finite coordinate, or are all on one line or point, so the
/// rotation about that line is not determined.
#[must_use]
pub fn register(fixed: &[[f64; 3]], moving: &[[f64; 3]]) -> Option<Rigid> {
    let n = fixed.len();
    let finite = fixed.iter().chain(moving).flatten().all(|v| v.is_finite());
    if n != moving.len() || n < 3 || !finite {
        return None;
    }
    let fixed_c = centroid(fixed);
    let moving_c = centroid(moving);

    // S[i][j] = sum over points of moving'_i · fixed'_j.
    let mut s = [[0.0; 3]; 3];
    for (a, b) in fixed.iter().zip(moving) {
        let a = sub(*a, fixed_c);
        let b = sub(*b, moving_c);
        for (i, row) in s.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell += b[i] * a[j];
            }
        }
    }
    let q = best_quaternion(s)?;
    let rotation = quaternion_to_matrix(q);
    let rotated: [f64; 3] =
        core::array::from_fn(|i| (0..3).map(|j| rotation[i][j] * moving_c[j]).sum());
    let translation_m = sub(fixed_c, rotated);
    Some(Rigid {
        rotation,
        translation_m,
    })
}

/// Root-mean-square distance between each mapped moving point and its fixed
/// partner, in metres. Returns `None` for lists of different lengths or none.
#[must_use]
pub fn rms_error(rigid: &Rigid, fixed: &[[f64; 3]], moving: &[[f64; 3]]) -> Option<f64> {
    if fixed.len() != moving.len() || fixed.is_empty() {
        return None;
    }
    let sum: f64 = fixed
        .iter()
        .zip(moving)
        .map(|(a, b)| {
            let m = rigid.apply(*b);
            sub(m, *a).iter().map(|v| v * v).sum::<f64>()
        })
        .sum();
    Some((sum / len_as_f64(fixed.len())).sqrt())
}

#[allow(clippy::cast_precision_loss)]
fn centroid(points: &[[f64; 3]]) -> [f64; 3] {
    let n = len_as_f64(points.len());
    core::array::from_fn(|i| points.iter().map(|p| p[i]).sum::<f64>() / n)
}

/// A count as . Point lists are far smaller than 2^53, so this is exact.
#[allow(clippy::cast_precision_loss)]
fn len_as_f64(n: usize) -> f64 {
    n as f64
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The unit quaternion `(w, x, y, z)` for the best rotation, from Horn's
/// symmetric 4×4 matrix. Its eigenvector for the largest eigenvalue is the
/// quaternion.
fn best_quaternion(s: [[f64; 3]; 3]) -> Option<[f64; 4]> {
    let [[sxx, sxy, sxz], [syx, syy, syz], [szx, szy, szz]] = s;
    let n = [
        [sxx + syy + szz, syz - szy, szx - sxz, sxy - syx],
        [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
        [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
        [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz],
    ];
    let (values, vectors) = jacobi_eigen(n);
    let (best, _) = values
        .iter()
        .enumerate()
        .fold(
            (0, f64::NEG_INFINITY),
            |acc, (i, &v)| {
                if v > acc.1 { (i, v) } else { acc }
            },
        );
    let q: [f64; 4] = core::array::from_fn(|k| vectors[k][best]);
    let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    // A zero eigenvector means the points carry no rotation information.
    if !norm.is_finite() || norm < 1e-12 {
        return None;
    }
    Some(q.map(|v| v / norm))
}

#[allow(clippy::many_single_char_names)]
fn quaternion_to_matrix([w, x, y, z]: [f64; 4]) -> [[f64; 3]; 3] {
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// Eigenvalues and eigenvectors (as columns) of a symmetric 4×4 matrix, by
/// cyclic Jacobi rotations.
#[allow(clippy::many_single_char_names)]
fn jacobi_eigen(mut a: [[f64; 4]; 4]) -> ([f64; 4], [[f64; 4]; 4]) {
    let mut v = [[0.0; 4]; 4];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _sweep in 0..100 {
        let off: f64 = (0..4)
            .flat_map(|p| (0..4).filter(move |&q| q != p).map(move |q| (p, q)))
            .map(|(p, q)| a[p][q] * a[p][q])
            .sum();
        if off < 1e-30 {
            break;
        }
        for p in 0..4 {
            for q in (p + 1)..4 {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                rotate(&mut a, &mut v, p, q, c, s);
            }
        }
    }
    (core::array::from_fn(|i| a[i][i]), v)
}

/// Applies one Jacobi rotation in the (p, q) plane, to the columns and then the
/// rows of , and to the columns of the eigenvector matrix .
#[allow(clippy::many_single_char_names)]
fn rotate(a: &mut [[f64; 4]; 4], v: &mut [[f64; 4]; 4], p: usize, q: usize, c: f64, s: f64) {
    rotate_columns(a, p, q, c, s);
    let (rp, rq) = (a[p], a[q]);
    a[p] = core::array::from_fn(|k| c * rp[k] - s * rq[k]);
    a[q] = core::array::from_fn(|k| s * rp[k] + c * rq[k]);
    rotate_columns(v, p, q, c, s);
}

#[allow(clippy::many_single_char_names)]
fn rotate_columns(m: &mut [[f64; 4]; 4], p: usize, q: usize, c: f64, s: f64) {
    for row in m.iter_mut() {
        let (x, y) = (row[p], row[q]);
        row[p] = c * x - s * y;
        row[q] = s * x + c * y;
    }
}

#[cfg(test)]
#[allow(clippy::many_single_char_names)]
mod tests {
    use super::*;

    /// Maps 53 bits of `seed` to a value in `[0, 1)`.
    #[allow(clippy::cast_precision_loss)]
    fn unit_from_bits(seed: u64) -> f64 {
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    }

    /// A rotation of `angle` radians about the unit axis `axis`.
    fn axis_angle(axis: [f64; 3], angle: f64) -> [[f64; 3]; 3] {
        let [x, y, z] = axis;
        let (s, c) = angle.sin_cos();
        let t = 1.0 - c;
        [
            [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
            [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
            [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
        ]
    }

    fn apply(r: &[[f64; 3]; 3], p: [f64; 3], t: [f64; 3]) -> [f64; 3] {
        core::array::from_fn(|i| (0..3).map(|j| r[i][j] * p[j]).sum::<f64>() + t[i])
    }

    fn fiducials() -> Vec<[f64; 3]> {
        vec![
            [0.00, 0.00, 0.00],
            [0.08, 0.01, 0.00],
            [0.02, 0.09, 0.01],
            [0.01, 0.02, 0.07],
            [-0.05, 0.04, 0.03],
            [0.03, -0.06, 0.05],
        ]
    }

    #[test]
    fn a_known_rotation_and_translation_are_recovered() {
        let r = axis_angle([1.0, 2.0, 2.0].map(|v: f64| v / 3.0), 0.7);
        let t = [0.12, -0.04, 0.3];
        let fixed = fiducials();
        // The moving points are the fixed ones taken back through the transform.
        let moving: Vec<[f64; 3]> = fixed
            .iter()
            .map(|a| {
                let back = sub(*a, t);
                core::array::from_fn(|i| (0..3).map(|j| r[j][i] * back[j]).sum())
            })
            .collect();
        let rigid = register(&fixed, &moving).expect("register");
        for (a, b) in fixed.iter().zip(&moving) {
            let mapped = rigid.apply(*b);
            for i in 0..3 {
                assert!((mapped[i] - a[i]).abs() < 1e-9, "{mapped:?} vs {a:?}");
            }
        }
    }

    #[test]
    fn registration_with_fiducial_noise_stays_within_half_a_millimetre() {
        // Each fiducial is off by up to 0.1 mm in the tracker's frame.
        let r = axis_angle([0.0, 0.0, 1.0], 0.3);
        let t = [0.05, 0.02, -0.01];
        let fixed = fiducials();
        let mut seed = 99_u64;
        let mut jitter = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (unit_from_bits(seed) - 0.5) * 0.0002
        };
        let moving: Vec<[f64; 3]> = fixed
            .iter()
            .map(|a| {
                let back = sub(*a, t);
                let noisy: [f64; 3] = core::array::from_fn(|i| back[i] + jitter());
                core::array::from_fn(|i| (0..3).map(|j| r[j][i] * noisy[j]).sum())
            })
            .collect();
        let rigid = register(&fixed, &moving).expect("register");
        let err = rms_error(&rigid, &fixed, &moving).expect("rms");
        assert!(err < 0.0005, "rms {err} m");
        // The transform itself is close to the truth, not just the residual.
        let check = apply(&r, [0.0; 3], t);
        let mapped = rigid.apply([0.0; 3]);
        assert!(sub(mapped, check).iter().all(|v| v.abs() < 0.0005));
    }

    #[test]
    fn rotation_matrices_are_orthonormal() {
        let fixed = fiducials();
        let moving: Vec<[f64; 3]> = fixed.iter().map(|p| [p[1], -p[0], p[2]]).collect();
        let rigid = register(&fixed, &moving).expect("register");
        let r = rigid.rotation;
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = (0..3).map(|k| r[i][k] * r[j][k]).sum();
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn degenerate_inputs_are_refused() {
        let fixed = fiducials();
        assert!(register(&fixed[..2], &fixed[..2]).is_none());
        assert!(register(&fixed, &fixed[..3]).is_none());
        let mut bad = fixed.clone();
        bad[0][0] = f64::NAN;
        assert!(register(&bad, &fixed).is_none());
    }
}
