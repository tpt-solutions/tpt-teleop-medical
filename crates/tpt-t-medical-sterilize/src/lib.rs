//! UV-C disinfection planning and coverage validation (spec.txt §5.4).
//!
//! The model:
//!
//! * Direct irradiance falls with the square of distance and with the cosine
//!   of the angle of incidence. A surface in shadow gets none.
//! * The dose delivered in a cycle is irradiance × time, in mJ/cm².
//! * **Reflection is not credited.** The spec asks for reflectivity to be
//!   considered, but an estimate of reflected dose is unreliable enough that
//!   crediting it could hide an under-treated surface. Reflectance is recorded,
//!   and it is not used in the dose.
//!
//! The planner gives one cycle time for the whole room. The validator checks
//! the dose a re-scan measured on each surface. Both use the same model, and
//! the validator trusts only what was measured.

#![forbid(unsafe_code)]

/// A UV-C source, characterised by its irradiance at 1 m, normal incidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvSource {
    irradiance_at_1m_mw_cm2: f64,
}

impl UvSource {
    /// Creates a source. The irradiance must be positive and finite.
    #[must_use]
    pub fn new(irradiance_at_1m_mw_cm2: f64) -> Option<Self> {
        (irradiance_at_1m_mw_cm2.is_finite() && irradiance_at_1m_mw_cm2 > 0.0).then_some(Self {
            irradiance_at_1m_mw_cm2,
        })
    }
}

/// A surface in the room, as measured from the robot's current position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    distance_m: f64,
    cos_incidence: f64,
    reflectance: f64,
    shadowed: bool,
}

impl Surface {
    /// Creates a surface.
    ///
    /// `cos_incidence` is in `0..=1`, `reflectance` is in `0..=1`, and
    /// `distance_m` is positive and finite. Returns `None` otherwise.
    #[must_use]
    pub fn new(
        distance_m: f64,
        cos_incidence: f64,
        reflectance: f64,
        shadowed: bool,
    ) -> Option<Self> {
        let valid = distance_m.is_finite()
            && distance_m > 0.0
            && (0.0..=1.0).contains(&cos_incidence)
            && (0.0..=1.0).contains(&reflectance);
        valid.then_some(Self {
            distance_m,
            cos_incidence,
            reflectance,
            shadowed,
        })
    }

    /// Returns the recorded reflectance. It is not used in the dose.
    #[must_use]
    pub const fn reflectance(&self) -> f64 {
        self.reflectance
    }
}

/// Direct irradiance on `surface`, in mW/cm².
#[must_use]
pub fn direct_irradiance(source: &UvSource, surface: &Surface) -> f64 {
    if surface.shadowed {
        return 0.0;
    }
    let falloff = 1.0 / (surface.distance_m * surface.distance_m);
    source.irradiance_at_1m_mw_cm2 * falloff * surface.cos_incidence
}

/// The cycle time, in seconds, that gives every surface at least
/// `required_mj_cm2`.
///
/// Returns `None` if the list is empty, or if any surface gets no direct
/// irradiance, because then no cycle time can treat it. The robot must move to
/// a position where that surface is visible.
#[must_use]
pub fn plan_cycle(source: &UvSource, surfaces: &[Surface], required_mj_cm2: f64) -> Option<f64> {
    if surfaces.is_empty() || !(required_mj_cm2.is_finite() && required_mj_cm2 > 0.0) {
        return None;
    }
    let mut longest: f64 = 0.0;
    for surface in surfaces {
        let e = direct_irradiance(source, surface);
        if e <= 0.0 {
            return None;
        }
        longest = longest.max(required_mj_cm2 / e);
    }
    Some(longest)
}

/// The result of a post-cycle re-scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// Every measured surface received the required dose.
    Complete,
    /// Some surfaces did not. The room must be treated again.
    Incomplete {
        /// How many surfaces fell short.
        under_dosed: usize,
        /// The index of the first surface that fell short.
        first_index: usize,
    },
}

/// Checks a re-scan. `measured_mj_cm2[i]` is the dose the re-scan found on
/// `surfaces[i]`. Each is compared with the requirement, and the model's
/// prediction is not used.
///
/// Returns `None` if the two lists differ in length, or if a measurement is not
/// a finite number. Those inputs cannot be checked, so they are not reported as
/// complete.
#[must_use]
pub fn verify_coverage(measured_mj_cm2: &[f64], required_mj_cm2: f64) -> Option<Coverage> {
    if !required_mj_cm2.is_finite() || required_mj_cm2 <= 0.0 {
        return None;
    }
    if measured_mj_cm2.iter().any(|m| !m.is_finite()) {
        return None;
    }
    let short: Vec<usize> = measured_mj_cm2
        .iter()
        .enumerate()
        .filter(|(_, m)| **m < required_mj_cm2)
        .map(|(i, _)| i)
        .collect();
    Some(match short.first() {
        None => Coverage::Complete,
        Some(&first_index) => Coverage::Incomplete {
            under_dosed: short.len(),
            first_index,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: f64 = 40.0; // mJ/cm²

    fn lamp() -> UvSource {
        UvSource::new(1.0).expect("valid")
    }

    fn face_on(distance_m: f64) -> Surface {
        Surface::new(distance_m, 1.0, 0.1, false).expect("valid")
    }

    #[test]
    fn irradiance_falls_with_the_square_of_distance() {
        let near = direct_irradiance(&lamp(), &face_on(1.0));
        let far = direct_irradiance(&lamp(), &face_on(2.0));
        assert!((near - 1.0).abs() < 1e-12);
        assert!((far - 0.25).abs() < 1e-12);
    }

    #[test]
    fn grazing_incidence_gives_less_irradiance() {
        let s = Surface::new(1.0, 0.5, 0.0, false).expect("valid");
        assert!((direct_irradiance(&lamp(), &s) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn shadowed_surface_gets_nothing() {
        let s = Surface::new(1.0, 1.0, 0.0, true).expect("valid");
        assert!(direct_irradiance(&lamp(), &s).abs() < 1e-12);
    }

    #[test]
    fn cycle_time_is_set_by_the_farthest_surface() {
        // At 1 m, 40 mJ/cm² needs 40 s. At 2 m, irradiance is 0.25, so 160 s.
        let surfaces = [face_on(1.0), face_on(2.0)];
        let t = plan_cycle(&lamp(), &surfaces, REQUIRED).expect("reachable");
        assert!((t - 160.0).abs() < 1e-9, "{t}");
    }

    #[test]
    fn reflectance_does_not_shorten_the_cycle() {
        let dull = Surface::new(1.0, 1.0, 0.0, false).expect("valid");
        let shiny = Surface::new(1.0, 1.0, 0.9, false).expect("valid");
        assert_eq!(
            plan_cycle(&lamp(), &[dull], REQUIRED),
            plan_cycle(&lamp(), &[shiny], REQUIRED)
        );
    }

    #[test]
    fn a_shadowed_surface_cannot_be_planned() {
        let hidden = Surface::new(1.0, 1.0, 0.0, true).expect("valid");
        assert_eq!(plan_cycle(&lamp(), &[face_on(1.0), hidden], REQUIRED), None);
    }

    #[test]
    fn empty_surface_list_cannot_be_planned() {
        assert_eq!(plan_cycle(&lamp(), &[], REQUIRED), None);
    }

    #[test]
    fn complete_coverage_is_reported() {
        assert_eq!(
            verify_coverage(&[40.0, 55.0, 41.2], REQUIRED),
            Some(Coverage::Complete)
        );
    }

    #[test]
    fn short_surfaces_are_counted_and_the_first_is_named() {
        assert_eq!(
            verify_coverage(&[50.0, 12.0, 30.0, 39.9], REQUIRED),
            Some(Coverage::Incomplete {
                under_dosed: 3,
                first_index: 1
            })
        );
    }

    #[test]
    fn non_finite_measurement_is_not_reported_as_complete() {
        assert_eq!(verify_coverage(&[40.0, f64::NAN], REQUIRED), None);
    }

    #[test]
    fn invalid_inputs_are_refused() {
        assert!(UvSource::new(0.0).is_none());
        assert!(
            Surface::new(0.0, 1.0, 0.0, false).is_none(),
            "zero distance"
        );
        assert!(
            Surface::new(1.0, 1.5, 0.0, false).is_none(),
            "cosine above 1"
        );
        assert!(
            Surface::new(1.0, 1.0, -0.1, false).is_none(),
            "negative reflectance"
        );
    }
}
