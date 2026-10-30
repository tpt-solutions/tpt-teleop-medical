//! Tremor notch filter (spec.txt §4.5, §5.1).
//!
//! A second-order IIR band-stop filter, from the standard RBJ audio-EQ
//! cookbook design. It is applied in direct form II transposed. It holds two
//! state values, allocates nothing, and is stable for any centre frequency
//! below half the sample rate.

use core::f64::consts::PI;

/// A second-order band-stop filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotchFilter {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl NotchFilter {
    /// Creates a notch at `center_hz`, sampled at `sample_hz`, with quality
    /// factor `q`. A larger `q` gives a narrower notch.
    ///
    /// Returns `None` unless `sample_hz > 0`, `0 < center_hz < sample_hz / 2`,
    /// and `q > 0`, all finite.
    #[must_use]
    pub fn new(center_hz: f64, sample_hz: f64, q: f64) -> Option<Self> {
        let valid = sample_hz.is_finite()
            && sample_hz > 0.0
            && center_hz.is_finite()
            && center_hz > 0.0
            && center_hz < sample_hz / 2.0
            && q.is_finite()
            && q > 0.0;
        if !valid {
            return None;
        }

        let w0 = 2.0 * PI * center_hz / sample_hz;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);

        let a0 = 1.0 + alpha;
        Some(Self {
            b: [1.0 / a0, -2.0 * cos_w0 / a0, 1.0 / a0],
            a: [-2.0 * cos_w0 / a0, (1.0 - alpha) / a0],
            z: [0.0; 2],
        })
    }

    /// Filters one sample.
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }

    /// Clears the filter state. Call this when the input is interrupted.
    pub fn reset(&mut self) {
        self.z = [0.0; 2];
    }

    /// Returns the steady-state gain at `freq_hz`, sampled at `sample_hz`.
    #[must_use]
    pub fn magnitude_at(&self, freq_hz: f64, sample_hz: f64) -> f64 {
        let w = 2.0 * PI * freq_hz / sample_hz;
        // z^-1 = e^{-jw} = (cos w, -sin w), and z^-2 is its square.
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = (c1 * c1 - s1 * s1, 2.0 * c1 * s1);

        let num = (
            self.b[0] + self.b[1] * c1 + self.b[2] * c2,
            self.b[1] * s1 + self.b[2] * s2,
        );
        let den = (
            1.0 + self.a[0] * c1 + self.a[1] * c2,
            self.a[0] * s1 + self.a[1] * s2,
        );
        let mag = |(re, im): (f64, f64)| re.hypot(im);
        mag(num) / mag(den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 1_000.0;

    fn tremor_notch() -> NotchFilter {
        NotchFilter::new(10.0, FS, 1.0).expect("valid notch")
    }

    #[test]
    fn centre_frequency_is_removed() {
        assert!(tremor_notch().magnitude_at(10.0, FS) < 1e-9);
    }

    #[test]
    fn voluntary_motion_passes_nearly_unchanged() {
        let f = tremor_notch();
        assert!((f.magnitude_at(0.5, FS) - 1.0).abs() < 0.01, "0.5 Hz");
        assert!(f.magnitude_at(1.0, FS) > 0.98, "1 Hz");
        assert!(f.magnitude_at(50.0, FS) > 0.95, "50 Hz");
    }

    #[test]
    fn dc_passes_at_unity_gain() {
        assert!((tremor_notch().magnitude_at(0.0, FS) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn steady_tremor_sine_is_suppressed_in_the_time_domain() {
        let mut f = tremor_notch();
        let mut last_peak: f64 = 0.0;
        // Run ten seconds. Measure the peak over the final second.
        for n in 0..10_000 {
            let x = (2.0 * PI * 10.0 * f64::from(n) / FS).sin();
            let y = f.process(x);
            if n >= 9_000 {
                last_peak = last_peak.max(y.abs());
            }
        }
        assert!(last_peak < 0.01, "residual tremor amplitude {last_peak}");
    }

    #[test]
    fn reset_clears_the_state() {
        let mut f = tremor_notch();
        for _ in 0..100 {
            f.process(1.0);
        }
        f.reset();
        let fresh = tremor_notch();
        assert_eq!(f, fresh);
    }

    #[test]
    fn invalid_parameters_are_refused() {
        assert!(NotchFilter::new(10.0, FS, 0.0).is_none(), "q = 0");
        assert!(NotchFilter::new(500.0, FS, 1.0).is_none(), "at Nyquist");
        assert!(NotchFilter::new(0.0, FS, 1.0).is_none(), "zero frequency");
        assert!(
            NotchFilter::new(10.0, 0.0, 1.0).is_none(),
            "zero sample rate"
        );
        assert!(
            NotchFilter::new(f64::NAN, FS, 1.0).is_none(),
            "NaN frequency"
        );
    }
}
