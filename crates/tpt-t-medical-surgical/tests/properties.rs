//! Property checks for the tremor notch filter and the SDF lookup (spec.txt
//! §4.3, §4.5). Cases are seeded, so every run covers the same inputs.

// Generated test inputs are small and bounded, so the lossy casts are intended.
#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use tpt_t_medical_surgical::notch::NotchFilter;
use tpt_t_medical_surgical::sdf::SdfGrid;

const SEEDS: u64 = 64;
const SAMPLE_HZ: f64 = 1_000.0;

/// Minimal xorshift64 generator. Never seeded with zero.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform value in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    /// Uniform value in `[lo, hi)`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

#[test]
fn notch_output_stays_finite_and_bounded_for_bounded_input() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let center = rng.range(1.0, SAMPLE_HZ / 2.0 - 1.0);
        let q = rng.range(0.5, 20.0);
        let mut filter = NotchFilter::new(center, SAMPLE_HZ, q).expect("valid parameters");
        for step in 0..4_000 {
            let y = filter.process(rng.range(-1_000.0, 1_000.0));
            assert!(y.is_finite(), "seed {seed} step {step}");
            assert!(y.abs() < 1.0e5, "seed {seed} step {step}: {y}");
        }
    }
}

#[test]
fn notch_removes_its_centre_frequency_in_steady_state() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let center = rng.range(2.0, 200.0);
        let q = rng.range(1.0, 10.0);
        let filter = NotchFilter::new(center, SAMPLE_HZ, q).expect("valid parameters");
        assert!(
            filter.magnitude_at(center, SAMPLE_HZ) < 1e-9,
            "seed {seed}: centre {center} Hz not removed"
        );
    }
}

#[test]
fn notch_passes_dc_unchanged() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let center = rng.range(2.0, 200.0);
        let q = rng.range(0.5, 20.0);
        let filter = NotchFilter::new(center, SAMPLE_HZ, q).expect("valid parameters");
        assert!(
            (filter.magnitude_at(0.0, SAMPLE_HZ) - 1.0).abs() < 1e-9,
            "seed {seed}"
        );
    }
}

#[test]
fn notch_steady_state_matches_magnitude_at_for_a_sine() {
    // Feed a sine at the test frequency and compare the settled output
    // amplitude with the analytic gain.
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let center = rng.range(5.0, 100.0);
        let q = 2.0;
        let probe = rng.range(1.0, 300.0);
        let mut filter = NotchFilter::new(center, SAMPLE_HZ, q).expect("valid parameters");
        let gain = filter.magnitude_at(probe, SAMPLE_HZ);

        let omega = 2.0 * core::f64::consts::PI * probe / SAMPLE_HZ;
        let mut peak = 0.0_f64;
        for n in 0..8_000 {
            let y = filter.process((omega * f64::from(n)).sin());
            if n >= 6_000 {
                peak = peak.max(y.abs());
            }
        }
        assert!(
            (peak - gain).abs() < 1e-3,
            "seed {seed}: probe {probe} Hz, measured {peak}, analytic {gain}"
        );
    }
}

#[test]
fn notch_rejects_invalid_design_parameters() {
    assert!(NotchFilter::new(10.0, SAMPLE_HZ, 0.0).is_none());
    assert!(NotchFilter::new(10.0, SAMPLE_HZ, -1.0).is_none());
    assert!(NotchFilter::new(0.0, SAMPLE_HZ, 1.0).is_none());
    assert!(NotchFilter::new(SAMPLE_HZ / 2.0, SAMPLE_HZ, 1.0).is_none());
    assert!(NotchFilter::new(10.0, 0.0, 1.0).is_none());
    assert!(NotchFilter::new(f64::NAN, SAMPLE_HZ, 1.0).is_none());
    assert!(NotchFilter::new(10.0, f64::INFINITY, 1.0).is_none());
}

/// Builds a random grid, returning it with the value list and its extremes.
fn random_grid(rng: &mut Rng) -> (SdfGrid, [f64; 3], f64, [u32; 3], f32, f32) {
    let origin = [
        rng.range(-0.05, 0.05),
        rng.range(-0.05, 0.05),
        rng.range(-0.05, 0.05),
    ];
    let spacing = rng.range(0.001, 0.01);
    let dims = [
        2 + u32::try_from(rng.below(6)).unwrap_or(0),
        2 + u32::try_from(rng.below(6)).unwrap_or(0),
        2 + u32::try_from(rng.below(6)).unwrap_or(0),
    ];
    let count = dims.iter().map(|&n| n as usize).product::<usize>();
    let values: Vec<f32> = (0..count).map(|_| rng.range(-0.02, 0.02) as f32).collect();
    let lo = values.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let grid = SdfGrid::new(origin, spacing, dims, values).expect("valid grid");
    (grid, origin, spacing, dims, lo, hi)
}

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(n).unwrap_or(u64::MAX)).unwrap_or(0)
    }
}

#[test]
fn sdf_lookup_is_a_convex_blend_of_the_grid_values() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let (grid, origin, spacing, dims, lo, hi) = random_grid(&mut rng);
        let extent = [
            origin[0] + spacing * f64::from(dims[0] - 1),
            origin[1] + spacing * f64::from(dims[1] - 1),
            origin[2] + spacing * f64::from(dims[2] - 1),
        ];
        for _ in 0..200 {
            let p = [
                rng.range(origin[0], extent[0]),
                rng.range(origin[1], extent[1]),
                rng.range(origin[2], extent[2]),
            ];
            let d = grid.distance(p).expect("inside the grid");
            assert!(
                d >= f64::from(lo) - 1e-9 && d <= f64::from(hi) + 1e-9,
                "seed {seed}: {d} outside [{lo}, {hi}]"
            );
        }
    }
}

#[test]
fn sdf_is_exact_for_a_linear_field() {
    // Trilinear interpolation reproduces a linear function exactly, up to the
    // f32 rounding of the stored samples.
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let origin = [0.0, 0.0, 0.0];
        let spacing = rng.range(0.002, 0.01);
        let dims = [4_u32, 5, 3];
        let coef = [
            rng.range(-1.0, 1.0),
            rng.range(-1.0, 1.0),
            rng.range(-1.0, 1.0),
        ];
        let offset = rng.range(-0.01, 0.01);
        let mut values = Vec::new();
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let p = [
                        f64::from(x) * spacing,
                        f64::from(y) * spacing,
                        f64::from(z) * spacing,
                    ];
                    let v = coef[0] * p[0] + coef[1] * p[1] + coef[2] * p[2] + offset;
                    values.push(v as f32);
                }
            }
        }
        let grid = SdfGrid::new(origin, spacing, dims, values).expect("valid grid");
        for _ in 0..200 {
            let p = [
                rng.range(0.0, spacing * f64::from(dims[0] - 1)),
                rng.range(0.0, spacing * f64::from(dims[1] - 1)),
                rng.range(0.0, spacing * f64::from(dims[2] - 1)),
            ];
            let want = coef[0] * p[0] + coef[1] * p[1] + coef[2] * p[2] + offset;
            let got = grid.distance(p).expect("inside the grid");
            assert!((got - want).abs() < 1e-6, "seed {seed}: {got} vs {want}");
        }
    }
}

#[test]
fn sdf_refuses_points_outside_the_grid_and_non_finite_points() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let (grid, origin, spacing, dims, _, _) = random_grid(&mut rng);
        let extent = origin[0] + spacing * f64::from(dims[0] - 1);
        let eps = spacing * 0.01;
        let inside = [origin[0], origin[1], origin[2]];
        assert!(grid.distance(inside).is_some(), "seed {seed}");
        assert!(
            grid.distance([extent + eps, inside[1], inside[2]])
                .is_none()
        );
        assert!(
            grid.distance([origin[0] - eps, inside[1], inside[2]])
                .is_none()
        );
        assert!(grid.distance([f64::NAN, inside[1], inside[2]]).is_none());
        assert!(
            grid.distance([inside[0], f64::INFINITY, inside[2]])
                .is_none()
        );
    }
}

#[test]
fn sdf_rejects_malformed_grids() {
    let ok = vec![0.0_f32; 8];
    assert!(SdfGrid::new([0.0; 3], 0.0, [2, 2, 2], ok.clone()).is_none());
    assert!(SdfGrid::new([0.0; 3], -1.0, [2, 2, 2], ok.clone()).is_none());
    assert!(SdfGrid::new([0.0; 3], 0.01, [1, 2, 2], vec![0.0; 4]).is_none());
    assert!(SdfGrid::new([0.0; 3], 0.01, [2, 2, 3], ok.clone()).is_none());
    let mut nan = ok;
    nan[3] = f32::NAN;
    assert!(SdfGrid::new([0.0; 3], 0.01, [2, 2, 2], nan).is_none());
}
