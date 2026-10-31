//! Constant-velocity Kalman filter for a tracked point, fusing position reports
//! from several sensors.
//!
//! The state is position and velocity, in metres and metres per second. The
//! motion model assumes white acceleration noise, with a spectral density the
//! caller chooses. Each sensor reports position with its own standard deviation,
//! so an optical reading and an electromagnetic one are fused with the weight
//! their noise deserves.
//!
//! The measurement is linear, because it is a position. The extended Kalman
//! filter reduces to this filter for a linear model. A nonlinear model, such as
//! electromagnetic field distortion modelled as a function of pose, would need
//! the Jacobian step, and that is not here yet.
//!
//! A report whose Mahalanobis distance is above the gate is rejected. A single
//! wild reading then cannot pull the estimate away.

/// A 3-vector.
pub type Vec3 = [f64; 3];
/// A 6×6 matrix, row-major. The state is `[px, py, pz, vx, vy, vz]`.
pub type Mat6 = [[f64; 6]; 6];

/// Chi-squared gate for three degrees of freedom at 99.9 %.
pub const DEFAULT_GATE: f64 = 16.27;

/// What the filter did with one report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// The report was used.
    Accepted {
        /// Squared Mahalanobis distance of the report.
        distance_sq: f64,
    },
    /// The report was too far from the prediction, and was not used.
    Rejected {
        /// Squared Mahalanobis distance of the report.
        distance_sq: f64,
    },
}

/// The filter.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    x: [f64; 6],
    p: Mat6,
    accel_psd: f64,
    gate: f64,
    started: bool,
}

impl Filter {
    /// Creates a filter. `accel_psd` is the acceleration noise spectral density,
    /// in m²/s³, and `gate` is the squared-distance gate. Returns `None` if either
    /// is not positive and finite.
    #[must_use]
    pub fn new(accel_psd: f64, gate: f64) -> Option<Self> {
        let ok = accel_psd.is_finite() && accel_psd > 0.0 && gate.is_finite() && gate > 0.0;
        ok.then_some(Self {
            x: [0.0; 6],
            p: [[0.0; 6]; 6],
            accel_psd,
            gate,
            started: false,
        })
    }

    /// Returns `true` once the first report has started the filter.
    #[must_use]
    pub const fn is_started(&self) -> bool {
        self.started
    }

    /// Returns the estimated position, or `None` before the first report.
    #[must_use]
    pub fn position(&self) -> Option<Vec3> {
        self.started.then(|| [self.x[0], self.x[1], self.x[2]])
    }

    /// Returns the estimated velocity, or `None` before the first report.
    #[must_use]
    pub fn velocity(&self) -> Option<Vec3> {
        self.started.then(|| [self.x[3], self.x[4], self.x[5]])
    }

    /// Returns the position standard deviation, per axis, in metres.
    #[must_use]
    pub fn position_sigma(&self) -> Option<Vec3> {
        self.started
            .then(|| [0, 1, 2].map(|i| self.p[i][i].max(0.0).sqrt()))
    }

    /// Starts the filter at `position` with standard deviation `sigma_m`. The
    /// velocity is taken as unknown, with a variance of 1 m²/s². Returns `None`
    /// for a non-finite position or a non-positive sigma.
    pub fn start(&mut self, position: Vec3, sigma_m: f64) -> Option<()> {
        let sigma_ok = sigma_m.is_finite() && sigma_m > 0.0;
        let position_ok = position.iter().all(|v| v.is_finite());
        if !position_ok || !sigma_ok {
            return None;
        }
        self.x = [position[0], position[1], position[2], 0.0, 0.0, 0.0];
        self.p = [[0.0; 6]; 6];
        for i in 0..3 {
            self.p[i][i] = sigma_m * sigma_m;
            self.p[i + 3][i + 3] = 1.0;
        }
        self.started = true;
        Some(())
    }

    /// Advances the estimate by `dt_s` seconds. Returns `None`, and changes
    /// nothing, if the filter has not started or the step is not positive.
    pub fn predict(&mut self, dt_s: f64) -> Option<()> {
        let dt_ok = dt_s.is_finite() && dt_s > 0.0;
        if !self.started || !dt_ok {
            return None;
        }
        let f = transition(dt_s);
        self.x = mat_vec(&f, self.x);
        let q = process_noise(dt_s, self.accel_psd);
        self.p = add6(&mat_mul(&mat_mul(&f, &self.p), &transpose(&f)), &q);
        Some(())
    }

    /// Fuses one position report with standard deviation `sigma_m`.
    ///
    /// Returns `None` if the filter has not started, or the report or sigma is
    /// not finite and positive. Otherwise it returns whether the report was used.
    pub fn update(&mut self, z: Vec3, sigma_m: f64) -> Option<Outcome> {
        let sigma_ok = sigma_m.is_finite() && sigma_m > 0.0;
        let report_ok = z.iter().all(|v| v.is_finite());
        if !self.started || !report_ok || !sigma_ok {
            return None;
        }
        // Innovation covariance S = P_pp + R, a 3×3 matrix.
        let mut s = [[0.0; 3]; 3];
        for (i, row) in s.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = self.p[i][j];
            }
            row[i] += sigma_m * sigma_m;
        }
        let s_inv = invert3(s)?;
        let y = [0, 1, 2].map(|i| z[i] - self.x[i]);
        let distance_sq: f64 = (0..3)
            .map(|i| (0..3).map(|j| y[i] * s_inv[i][j] * y[j]).sum::<f64>())
            .sum();
        if distance_sq > self.gate {
            return Some(Outcome::Rejected { distance_sq });
        }

        // Kalman gain K = P_:p · S⁻¹, a 6×3 matrix.
        let mut k = [[0.0; 3]; 6];
        for (i, row) in k.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|m| self.p[i][m] * s_inv[m][j]).sum();
            }
        }
        for (i, row) in self.x.iter_mut().enumerate() {
            *row += (0..3).map(|j| k[i][j] * y[j]).sum::<f64>();
        }
        // P = (I - K H) P, where H selects the position rows.
        let hp: [[f64; 6]; 3] = [self.p[0], self.p[1], self.p[2]];
        for (i, row) in self.p.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell -= (0..3).map(|m| k[i][m] * hp[m][j]).sum::<f64>();
            }
        }
        self.p = symmetrize(&self.p);
        Some(Outcome::Accepted { distance_sq })
    }
}

fn transition(dt: f64) -> Mat6 {
    let mut f = identity();
    for i in 0..3 {
        f[i][i + 3] = dt;
    }
    f
}

fn process_noise(dt: f64, psd: f64) -> Mat6 {
    let mut q = [[0.0; 6]; 6];
    let (dt2, dt3, dt4) = (dt * dt, dt * dt * dt, dt * dt * dt * dt);
    for i in 0..3 {
        q[i][i] = psd * dt4 / 4.0;
        q[i][i + 3] = psd * dt3 / 2.0;
        q[i + 3][i] = psd * dt3 / 2.0;
        q[i + 3][i + 3] = psd * dt2;
    }
    q
}

fn identity() -> Mat6 {
    let mut m = [[0.0; 6]; 6];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    m
}

fn mat_mul(a: &Mat6, b: &Mat6) -> Mat6 {
    let mut out = [[0.0; 6]; 6];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..6).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn mat_vec(a: &Mat6, x: [f64; 6]) -> [f64; 6] {
    core::array::from_fn(|i| (0..6).map(|k| a[i][k] * x[k]).sum())
}

fn transpose(a: &Mat6) -> Mat6 {
    core::array::from_fn(|i| core::array::from_fn(|j| a[j][i]))
}

fn add6(a: &Mat6, b: &Mat6) -> Mat6 {
    core::array::from_fn(|i| core::array::from_fn(|j| a[i][j] + b[i][j]))
}

fn symmetrize(a: &Mat6) -> Mat6 {
    core::array::from_fn(|i| core::array::from_fn(|j| f64::midpoint(a[i][j], a[j][i])))
}

/// Inverts a 3×3 matrix by cofactors. Returns `None` if it is singular.
fn invert3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv,
        ],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic, approximately Gaussian noise source: the sum of twelve
    /// uniform values, centred and scaled. The filter is checked against a run
    /// that always gives the same numbers.
    struct Noise(u64);

    impl Noise {
        #[allow(clippy::cast_precision_loss)]
        fn next_unit(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1_u64 << 53) as f64
        }

        fn gaussian(&mut self) -> f64 {
            (0..12).map(|_| self.next_unit()).sum::<f64>() - 6.0
        }
    }

    fn norm(v: Vec3) -> f64 {
        v.iter().map(|c| c * c).sum::<f64>().sqrt()
    }

    #[test]
    fn nothing_happens_before_the_first_report() {
        let mut f = Filter::new(1.0, DEFAULT_GATE).expect("filter");
        assert_eq!(f.predict(0.01), None);
        assert_eq!(f.update([0.0; 3], 0.001), None);
        assert_eq!(f.position(), None);
    }

    #[test]
    fn fused_reports_settle_within_half_a_millimetre_of_a_still_target() {
        // Optical at 0.2 mm and electromagnetic at 0.5 mm, both at 100 Hz for 10 s.
        let truth = [0.10, -0.05, 0.02];
        let mut f = Filter::new(1e-4, DEFAULT_GATE).expect("filter");
        let mut noise = Noise(7);
        f.start(truth, 0.0005).expect("start");
        for _ in 0..1_000 {
            f.predict(0.01).expect("predict");
            let optical: Vec3 = core::array::from_fn(|i| truth[i] + 0.0002 * noise.gaussian());
            let em: Vec3 = core::array::from_fn(|i| truth[i] + 0.0005 * noise.gaussian());
            f.update(optical, 0.0002).expect("optical");
            f.update(em, 0.0005).expect("em");
        }
        let p = f.position().expect("position");
        let err = norm([p[0] - truth[0], p[1] - truth[1], p[2] - truth[2]]);
        assert!(err < 0.0005, "error {err} m");
    }

    #[test]
    fn a_moving_target_gives_its_velocity() {
        let mut f = Filter::new(1e-3, DEFAULT_GATE).expect("filter");
        let mut noise = Noise(11);
        f.start([0.0; 3], 0.001).expect("start");
        for k in 1..=2_000 {
            let t = f64::from(k) * 0.01;
            f.predict(0.01).expect("predict");
            let z: Vec3 = [0.02 * t, 0.0, 0.0].map(|c| c + 0.0002 * noise.gaussian());
            f.update(z, 0.0002).expect("update");
        }
        let v = f.velocity().expect("velocity");
        assert!((v[0] - 0.02).abs() < 0.002, "vx {}", v[0]);
    }

    #[test]
    fn a_wild_report_is_rejected_and_the_estimate_holds() {
        let mut f = Filter::new(1e-4, DEFAULT_GATE).expect("filter");
        f.start([0.0; 3], 0.0005).expect("start");
        f.predict(0.01).expect("predict");
        let before = f.position().expect("before");
        let outcome = f.update([0.1, 0.0, 0.0], 0.0002).expect("update");
        assert!(matches!(outcome, Outcome::Rejected { .. }), "{outcome:?}");
        assert_eq!(f.position(), Some(before));
    }

    #[test]
    fn uncertainty_shrinks_with_reports_and_grows_without_them() {
        let mut f = Filter::new(1e-4, DEFAULT_GATE).expect("filter");
        f.start([0.0; 3], 0.001).expect("start");
        let start = f.position_sigma().expect("sigma")[0];
        f.update([0.0; 3], 0.0005).expect("update");
        let after_report = f.position_sigma().expect("sigma")[0];
        assert!(after_report < start);
        for _ in 0..100 {
            f.predict(0.01).expect("predict");
        }
        assert!(f.position_sigma().expect("sigma")[0] > after_report);
    }

    #[test]
    fn invalid_inputs_are_refused() {
        assert!(Filter::new(0.0, DEFAULT_GATE).is_none());
        let mut f = Filter::new(1.0, DEFAULT_GATE).expect("filter");
        assert!(f.start([f64::NAN, 0.0, 0.0], 0.001).is_none());
        assert!(f.start([0.0; 3], 0.0).is_none());
        f.start([0.0; 3], 0.001).expect("start");
        assert!(f.predict(0.0).is_none());
        assert!(f.update([0.0; 3], -1.0).is_none());
    }
}
