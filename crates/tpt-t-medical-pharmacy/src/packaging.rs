//! Unit-dose packaging: each dose is checked by barcode and by weight before it
//! goes into the package, and the package seals once every dose has passed
//! (spec.txt §5.3).
//!
//! A scan must match the expected product code exactly. A weight must fall
//! within [`DEFAULT_WEIGHT_TOLERANCE`] of the expected unit weight. A refused
//! scan changes nothing, and the rejected dose must be removed by hand.

use core::fmt;

/// The largest deviation from the expected unit weight that is accepted,
/// as a fraction. 5% is the recommended default. It is a constant, so it is
/// changed in one place.
pub const DEFAULT_WEIGHT_TOLERANCE: f64 = 0.05;

/// What the package should contain.
#[derive(Debug, Clone, PartialEq)]
pub struct Expected {
    /// The product code (GTIN) printed on the barcode.
    pub gtin: String,
    /// Number of unit doses in the package.
    pub units: u32,
    /// Weight of one unit dose, in milligrams.
    pub unit_weight_mg: f64,
}

/// Why a dose was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyError {
    /// The barcode does not match the expected product.
    WrongItem,
    /// The weight is not a finite, positive number.
    BadWeight,
    /// The weight is outside the tolerance around the expected unit weight.
    WeightOutOfTolerance {
        /// The expected unit weight, in milligrams.
        expected_mg: f64,
        /// The measured weight, in milligrams.
        measured_mg: f64,
    },
    /// The package already holds every dose it needs.
    AlreadySealed,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongItem => f.write_str("barcode does not match the expected product"),
            Self::BadWeight => f.write_str("weight is not a finite positive number"),
            Self::WeightOutOfTolerance {
                expected_mg,
                measured_mg,
            } => write!(
                f,
                "weight {measured_mg} mg is outside the tolerance around {expected_mg} mg"
            ),
            Self::AlreadySealed => f.write_str("package is already sealed"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// A package being built, one verified dose at a time.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    expected: Expected,
    verified: u32,
}

/// The result of a scan that was accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// This many doses are still needed.
    Open {
        /// Doses still to verify.
        remaining: u32,
    },
    /// The package is complete and sealed.
    Sealed {
        /// Doses in the package.
        units: u32,
    },
}

impl Package {
    /// Starts a package. Returns `None` for an empty product code, zero units,
    /// or a unit weight that is not finite and positive.
    #[must_use]
    pub fn new(expected: Expected) -> Option<Self> {
        let ok = !expected.gtin.is_empty()
            && expected.units > 0
            && expected.unit_weight_mg.is_finite()
            && expected.unit_weight_mg > 0.0;
        ok.then_some(Self {
            expected,
            verified: 0,
        })
    }

    /// Returns the number of verified doses.
    #[must_use]
    pub const fn verified(&self) -> u32 {
        self.verified
    }

    /// Verifies one dose by barcode and weight.
    ///
    /// # Errors
    ///
    /// Returns the reason the dose was refused. The package is unchanged.
    pub fn verify_dose(&mut self, barcode: &str, weighed_mg: f64) -> Result<Progress, VerifyError> {
        if self.is_sealed() {
            return Err(VerifyError::AlreadySealed);
        }
        if barcode != self.expected.gtin {
            return Err(VerifyError::WrongItem);
        }
        if !weighed_mg.is_finite() || weighed_mg <= 0.0 {
            return Err(VerifyError::BadWeight);
        }
        let expected_mg = self.expected.unit_weight_mg;
        let allowed = expected_mg * DEFAULT_WEIGHT_TOLERANCE;
        if (weighed_mg - expected_mg).abs() > allowed {
            return Err(VerifyError::WeightOutOfTolerance {
                expected_mg,
                measured_mg: weighed_mg,
            });
        }
        self.verified += 1;
        Ok(self.progress())
    }

    /// Returns `true` once every dose is verified.
    #[must_use]
    pub const fn is_sealed(&self) -> bool {
        self.verified >= self.expected.units
    }

    fn progress(&self) -> Progress {
        if self.is_sealed() {
            Progress::Sealed {
                units: self.expected.units,
            }
        } else {
            Progress::Open {
                remaining: self.expected.units - self.verified,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(units: u32) -> Package {
        Package::new(Expected {
            gtin: "00312345678906".into(),
            units,
            unit_weight_mg: 500.0,
        })
        .expect("valid")
    }

    #[test]
    fn a_package_seals_after_every_dose_passes() {
        let mut p = package(2);
        assert_eq!(
            p.verify_dose("00312345678906", 500.0),
            Ok(Progress::Open { remaining: 1 })
        );
        assert_eq!(
            p.verify_dose("00312345678906", 510.0),
            Ok(Progress::Sealed { units: 2 })
        );
        assert!(p.is_sealed());
    }

    #[test]
    fn a_sealed_package_takes_no_more_doses() {
        let mut p = package(1);
        p.verify_dose("00312345678906", 500.0).expect("dose");
        assert_eq!(
            p.verify_dose("00312345678906", 500.0),
            Err(VerifyError::AlreadySealed)
        );
        assert_eq!(p.verified(), 1);
    }

    #[test]
    fn the_wrong_barcode_is_refused_and_counts_nothing() {
        let mut p = package(1);
        assert_eq!(
            p.verify_dose("00999999999999", 500.0),
            Err(VerifyError::WrongItem)
        );
        assert_eq!(p.verify_dose("", 500.0), Err(VerifyError::WrongItem));
        assert_eq!(p.verified(), 0);
    }

    #[test]
    fn weight_outside_tolerance_is_refused_at_both_edges() {
        let mut p = package(1);
        assert!(p.verify_dose("00312345678906", 474.9).is_err());
        assert!(p.verify_dose("00312345678906", 525.1).is_err());
        assert_eq!(p.verified(), 0);
        assert!(p.verify_dose("00312345678906", 475.0).is_ok());
    }

    #[test]
    fn bad_weights_are_refused() {
        let mut p = package(1);
        for w in [f64::NAN, f64::INFINITY, 0.0, -500.0] {
            assert_eq!(
                p.verify_dose("00312345678906", w),
                Err(VerifyError::BadWeight),
                "{w}"
            );
        }
    }

    #[test]
    fn invalid_expectations_are_refused() {
        let base = Expected {
            gtin: "1".into(),
            units: 1,
            unit_weight_mg: 1.0,
        };
        assert!(
            Package::new(Expected {
                gtin: String::new(),
                ..base.clone()
            })
            .is_none()
        );
        assert!(
            Package::new(Expected {
                units: 0,
                ..base.clone()
            })
            .is_none()
        );
        assert!(
            Package::new(Expected {
                unit_weight_mg: f64::NAN,
                ..base.clone()
            })
            .is_none()
        );
        assert!(
            Package::new(Expected {
                unit_weight_mg: 0.0,
                ..base
            })
            .is_none()
        );
    }
}
