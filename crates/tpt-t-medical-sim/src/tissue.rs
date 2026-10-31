//! Mass-spring tissue for the simulator (spec.txt §8).
//!
//! Tissue is a set of point masses joined by springs, with damping. Some points
//! are fixed, as an anchored edge is. Each step applies the spring and damping
//! forces, adds the tool's pulls, and advances the points with semi-implicit
//! Euler.
//!
//! The model is a stand-in for the spec's FEM or mass-spring tissue, and it is
//! not calibrated to any tissue. Its time step is checked against the stiffest
//! point, so an unstable step is refused rather than run.

/// Most points in one tissue.
pub const MAX_NODES: usize = 1_024;
/// Most springs in one tissue.
pub const MAX_SPRINGS: usize = 8_192;

/// A spring between two points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    /// First point.
    pub a: usize,
    /// Second point.
    pub b: usize,
    /// Length with no force, in metres.
    pub rest_m: f64,
    /// Stiffness, in newtons per metre.
    pub stiffness_n_m: f64,
}

/// A mass-spring tissue.
#[derive(Debug, Clone, PartialEq)]
pub struct Tissue {
    pos: Vec<[f64; 3]>,
    vel: Vec<[f64; 3]>,
    fixed: Vec<bool>,
    mass_kg: f64,
    damping_ns_m: f64,
    springs: Vec<Spring>,
    max_dt_s: f64,
}

impl Tissue {
    /// Creates a tissue. Returns `None` for an empty or oversized tissue, a
    /// non-positive mass, a negative damping, a spring that names a missing
    /// point or has invalid values, or a `fixed` list that does not match the
    /// points.
    #[must_use]
    pub fn new(
        pos: Vec<[f64; 3]>,
        fixed: Vec<bool>,
        mass_kg: f64,
        damping_ns_m: f64,
        springs: Vec<Spring>,
    ) -> Option<Self> {
        let n = pos.len();
        let ok = n > 0
            && n <= MAX_NODES
            && springs.len() <= MAX_SPRINGS
            && fixed.len() == n
            && mass_kg.is_finite()
            && mass_kg > 0.0
            && damping_ns_m.is_finite()
            && damping_ns_m >= 0.0
            && pos.iter().flatten().all(|v| v.is_finite())
            && springs.iter().all(|s| {
                s.a < n
                    && s.b < n
                    && s.a != s.b
                    && s.rest_m.is_finite()
                    && s.rest_m >= 0.0
                    && s.stiffness_n_m.is_finite()
                    && s.stiffness_n_m > 0.0
            });
        if !ok {
            return None;
        }
        // The stiffest point sets the largest safe step. Half of the
        // sqrt(m/k) limit leaves room for damping and the tool's pulls.
        let mut stiffness_sum = vec![0.0_f64; n];
        for s in &springs {
            stiffness_sum[s.a] += s.stiffness_n_m;
            stiffness_sum[s.b] += s.stiffness_n_m;
        }
        let stiffest = stiffness_sum.iter().copied().fold(0.0_f64, f64::max);
        let max_dt_s = if stiffest > 0.0 {
            0.5 * (mass_kg / stiffest).sqrt()
        } else {
            f64::INFINITY
        };
        Some(Self {
            vel: vec![[0.0; 3]; n],
            pos,
            fixed,
            mass_kg,
            damping_ns_m,
            springs,
            max_dt_s,
        })
    }

    /// Returns the positions of all points.
    #[must_use]
    pub fn positions(&self) -> &[[f64; 3]] {
        &self.pos
    }

    /// Returns the largest time step the tissue accepts, in seconds.
    #[must_use]
    pub const fn max_dt_s(&self) -> f64 {
        self.max_dt_s
    }

    /// Returns kinetic plus spring energy, in joules.
    #[must_use]
    pub fn energy_j(&self) -> f64 {
        let kinetic: f64 = self
            .vel
            .iter()
            .map(|v| 0.5 * self.mass_kg * v.iter().map(|c| c * c).sum::<f64>())
            .sum();
        let spring: f64 = self
            .springs
            .iter()
            .map(|s| {
                let stretch = length(sub(self.pos[s.b], self.pos[s.a])) - s.rest_m;
                0.5 * s.stiffness_n_m * stretch * stretch
            })
            .sum();
        kinetic + spring
    }

    /// Advances the tissue by `dt_s` seconds. `pulls` are forces from the tool,
    /// as `(point, force in newtons)`.
    ///
    /// Returns `None`, and does not move, if the step is not positive, is longer
    /// than [`Tissue::max_dt_s`], or a pull names a missing point.
    pub fn step(&mut self, dt_s: f64, pulls: &[(usize, [f64; 3])]) -> Option<()> {
        let step_ok = dt_s > 0.0 && dt_s <= self.max_dt_s;
        if !step_ok || pulls.iter().any(|(i, _)| *i >= self.pos.len()) {
            return None;
        }
        let mut force = vec![[0.0_f64; 3]; self.pos.len()];
        for s in &self.springs {
            let d = sub(self.pos[s.b], self.pos[s.a]);
            let len = length(d);
            if len == 0.0 {
                continue;
            }
            let f = scale(d, s.stiffness_n_m * (len - s.rest_m) / len);
            force[s.a] = add(force[s.a], f);
            force[s.b] = sub(force[s.b], f);
        }
        for (i, f) in pulls {
            force[*i] = add(force[*i], *f);
        }
        for (i, f) in force.iter().enumerate() {
            if self.fixed[i] {
                continue;
            }
            let damping = scale(self.vel[i], -self.damping_ns_m);
            let total = add(*f, damping);
            self.vel[i] = add(self.vel[i], scale(total, dt_s / self.mass_kg));
            self.pos[i] = add(self.pos[i], scale(self.vel[i], dt_s));
        }
        Some(())
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn length(a: [f64; 3]) -> f64 {
    a.iter().map(|v| v * v).sum::<f64>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spring(a: usize, b: usize, rest: f64, k: f64) -> Spring {
        Spring {
            a,
            b,
            rest_m: rest,
            stiffness_n_m: k,
        }
    }

    /// One fixed point at the origin and one free point, joined by a spring.
    fn pair(rest: f64, k: f64) -> Tissue {
        Tissue::new(
            vec![[0.0; 3], [0.1, 0.0, 0.0]],
            vec![true, false],
            0.01,
            0.2,
            vec![spring(0, 1, rest, k)],
        )
        .expect("tissue")
    }

    #[test]
    fn a_pull_settles_at_hookes_law_stretch() {
        // A 2 N pull on a 100 N/m spring stretches it 2 cm past its rest length.
        let mut t = pair(0.1, 100.0);
        let dt = t.max_dt_s() * 0.5;
        for _ in 0..200_000 {
            t.step(dt, &[(1, [2.0, 0.0, 0.0])]).expect("step");
        }
        let x = t.positions()[1][0];
        assert!((x - 0.12).abs() < 1e-4, "settled at {x}");
    }

    #[test]
    fn with_no_pull_the_energy_only_goes_down() {
        let mut t = pair(0.05, 200.0);
        let dt = t.max_dt_s() * 0.5;
        let mut last = t.energy_j();
        for _ in 0..5_000 {
            t.step(dt, &[]).expect("step");
            let now = t.energy_j();
            assert!(now <= last + 1e-12, "energy rose from {last} to {now}");
            last = now;
        }
        assert!(last < 1e-3, "still {last} J");
    }

    #[test]
    fn an_unstable_step_is_refused_and_nothing_moves() {
        let mut t = pair(0.1, 100.0);
        let before = t.clone();
        assert!(t.step(t.max_dt_s() * 2.0, &[]).is_none());
        assert!(t.step(0.0, &[]).is_none());
        assert_eq!(t, before);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn fixed_points_do_not_move() {
        let mut t = pair(0.1, 100.0);
        let dt = t.max_dt_s() * 0.5;
        for _ in 0..100 {
            t.step(dt, &[(1, [5.0, 0.0, 0.0])]).expect("step");
        }
        assert_eq!(t.positions()[0], [0.0; 3]);
    }

    #[test]
    fn invalid_tissues_are_refused() {
        assert!(Tissue::new(vec![], vec![], 1.0, 0.0, vec![]).is_none());
        assert!(Tissue::new(vec![[0.0; 3]], vec![false], 0.0, 0.0, vec![]).is_none());
        assert!(
            Tissue::new(
                vec![[0.0; 3]],
                vec![false],
                1.0,
                0.0,
                vec![spring(0, 3, 1.0, 1.0)]
            )
            .is_none()
        );
        assert!(
            Tissue::new(
                vec![[0.0; 3]; 2],
                vec![false, false],
                1.0,
                0.0,
                vec![spring(0, 1, 1.0, 0.0)]
            )
            .is_none()
        );
    }
}
