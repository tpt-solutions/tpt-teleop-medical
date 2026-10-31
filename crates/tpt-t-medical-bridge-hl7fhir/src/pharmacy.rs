//! Feeds medication orders into the pharmacy (spec.txt §5.3).
//!
//! An order is dispensed only when it is `active`, asks for a whole positive
//! quantity, and the stock covers it. The stock is taken with
//! [`Inventory::dispense`], which is all or nothing and first-expired,
//! first-out. A refused order leaves the stock unchanged.
//!
//! The caller journals the order with [`crate::audit::BridgeAudit`] before it
//! calls [`dispense_order`].

use core::fmt;

use tpt_t_medical_pharmacy::{DispenseError, Inventory, Pick};

use crate::fhir::MedicationOrder;

/// Why an order was not dispensed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The order is not `active`, so it is not dispensed.
    NotActive,
    /// The order gives no quantity to dispense.
    NoQuantity,
    /// The quantity is not a whole number of units, or is not positive.
    QuantityNotWhole,
    /// The quantity is too large to dispense in one request.
    QuantityTooLarge,
    /// The stock cannot cover the quantity. Nothing was taken.
    Stock(DispenseError),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotActive => f.write_str("order is not active"),
            Self::NoQuantity => f.write_str("order has no quantity to dispense"),
            Self::QuantityNotWhole => f.write_str("quantity is not a whole positive number"),
            Self::QuantityTooLarge => f.write_str("quantity is too large for one dispense"),
            Self::Stock(e) => write!(f, "stock refused: {e}"),
        }
    }
}

impl std::error::Error for Refusal {}

/// Dispenses the stock for one medication order.
///
/// `today` is the day number used for expiry checks, as in
/// [`Inventory::dispense`].
///
/// # Errors
///
/// Returns the first reason the order cannot be dispensed. The stock is
/// unchanged in that case.
pub fn dispense_order(
    order: &MedicationOrder,
    inventory: &mut Inventory,
    today: u32,
) -> Result<Vec<Pick>, Refusal> {
    if order.status != "active" {
        return Err(Refusal::NotActive);
    }
    let quantity = order.quantity.ok_or(Refusal::NoQuantity)?;
    let units = whole_units(quantity)?;
    inventory
        .dispense(&order.drug, units, today)
        .map_err(Refusal::Stock)
}

/// Converts a validated quantity to units. Only whole, positive values that fit
/// in `u32` are accepted.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole_units(quantity: f64) -> Result<u32, Refusal> {
    if quantity.fract() != 0.0 || quantity < 1.0 {
        return Err(Refusal::QuantityNotWhole);
    }
    if quantity > f64::from(u32::MAX) {
        return Err(Refusal::QuantityTooLarge);
    }
    Ok(quantity as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fhir::parse_medication_request;
    use tpt_t_medical_pharmacy::Lot;

    const TODAY: u32 = 100;

    fn order(status: &str, quantity: Option<f64>) -> MedicationOrder {
        let mut v = serde_json::json!({
            "resourceType": "MedicationRequest",
            "id": "rx-1",
            "status": status,
            "subject": { "reference": "Patient/MRN-1" },
            "medicationCodeableConcept": { "text": "heparin" },
        });
        if let Some(q) = quantity {
            v["dispenseRequest"] = serde_json::json!({ "quantity": { "value": q } });
        }
        let bytes = serde_json::to_vec(&v).expect("json");
        parse_medication_request(&bytes).expect("valid order")
    }

    fn stock() -> Inventory {
        let mut inv = Inventory::new();
        inv.add(Lot {
            id: 1,
            drug: "heparin".into(),
            expiry_day: 150,
            quantity: 5,
        });
        inv.add(Lot {
            id: 2,
            drug: "heparin".into(),
            expiry_day: 120,
            quantity: 3,
        });
        inv
    }

    #[test]
    fn active_order_takes_earliest_expiring_lot_first() {
        let mut inv = stock();
        let picks = dispense_order(&order("active", Some(4.0)), &mut inv, TODAY).expect("ok");
        assert_eq!(
            picks,
            vec![
                Pick {
                    lot_id: 2,
                    quantity: 3
                },
                Pick {
                    lot_id: 1,
                    quantity: 1
                }
            ]
        );
        assert_eq!(inv.usable("heparin", TODAY), 4);
    }

    #[test]
    fn inactive_orders_are_not_dispensed() {
        let mut inv = stock();
        for status in [
            "on-hold",
            "cancelled",
            "completed",
            "draft",
            "entered-in-error",
        ] {
            assert_eq!(
                dispense_order(&order(status, Some(1.0)), &mut inv, TODAY),
                Err(Refusal::NotActive),
                "{status}"
            );
        }
        assert_eq!(inv.usable("heparin", TODAY), 8);
    }

    #[test]
    fn order_without_quantity_is_refused() {
        let mut inv = stock();
        assert_eq!(
            dispense_order(&order("active", None), &mut inv, TODAY),
            Err(Refusal::NoQuantity)
        );
    }

    #[test]
    fn fractional_and_zero_quantities_are_refused() {
        let mut inv = stock();
        assert_eq!(
            dispense_order(&order("active", Some(1.5)), &mut inv, TODAY),
            Err(Refusal::QuantityNotWhole)
        );
        assert_eq!(
            dispense_order(&order("active", Some(0.5)), &mut inv, TODAY),
            Err(Refusal::QuantityNotWhole)
        );
        assert_eq!(inv.usable("heparin", TODAY), 8);
    }

    #[test]
    fn quantity_above_u32_is_refused() {
        let mut inv = stock();
        assert_eq!(
            dispense_order(&order("active", Some(1e12)), &mut inv, TODAY),
            Err(Refusal::QuantityTooLarge)
        );
    }

    #[test]
    fn short_stock_refuses_and_takes_nothing() {
        let mut inv = stock();
        assert_eq!(
            dispense_order(&order("active", Some(9.0)), &mut inv, TODAY),
            Err(Refusal::Stock(DispenseError::Insufficient { available: 8 }))
        );
        assert_eq!(inv.usable("heparin", TODAY), 8);
    }

    #[test]
    fn expired_lots_do_not_count_toward_the_order() {
        let mut inv = stock();
        assert_eq!(
            dispense_order(&order("active", Some(6.0)), &mut inv, 121),
            Err(Refusal::Stock(DispenseError::Insufficient { available: 5 }))
        );
    }
}
