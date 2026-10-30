//! Stock selection with first-expired, first-out (FEFO) ordering.

/// One batch of one drug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lot {
    /// Unique lot identifier.
    pub id: u32,
    /// Drug name, as stocked.
    pub drug: String,
    /// Last day the lot may be used, as a day number. A lot is usable on its
    /// expiry day and is expired from the day after.
    pub expiry_day: u32,
    /// Units on hand.
    pub quantity: u32,
}

/// How many units to take from one lot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    /// The lot to take from.
    pub lot_id: u32,
    /// Units to take.
    pub quantity: u32,
}

/// Why a dispense request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispenseError {
    /// Fewer usable units are on hand than were requested. Nothing was taken.
    Insufficient {
        /// Usable units on hand.
        available: u32,
    },
}

impl std::fmt::Display for DispenseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Insufficient { available } => {
                write!(f, "only {available} usable units on hand")
            }
        }
    }
}

impl std::error::Error for DispenseError {}

/// The stock on hand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    lots: Vec<Lot>,
}

impl Inventory {
    /// Creates an empty inventory.
    #[must_use]
    pub const fn new() -> Self {
        Self { lots: Vec::new() }
    }

    /// Adds a lot to the stock.
    pub fn add(&mut self, lot: Lot) {
        self.lots.push(lot);
    }

    /// Returns the usable units of `drug` on `today`.
    #[must_use]
    pub fn usable(&self, drug: &str, today: u32) -> u32 {
        self.lots
            .iter()
            .filter(|l| l.drug == drug && l.expiry_day >= today)
            .map(|l| l.quantity)
            .sum()
    }

    /// Takes `quantity` units of `drug`, earliest-expiring lots first.
    ///
    /// The request is all or nothing. If the usable stock is too small,
    /// nothing is taken. Ties on expiry day go to the lower lot id.
    ///
    /// # Errors
    ///
    /// Returns [`DispenseError::Insufficient`] when the usable stock cannot
    /// cover the request.
    pub fn dispense(
        &mut self,
        drug: &str,
        quantity: u32,
        today: u32,
    ) -> Result<Vec<Pick>, DispenseError> {
        let available = self.usable(drug, today);
        if available < quantity {
            return Err(DispenseError::Insufficient { available });
        }

        let mut order: Vec<usize> = (0..self.lots.len())
            .filter(|&i| {
                let l = &self.lots[i];
                l.drug == drug && l.expiry_day >= today && l.quantity > 0
            })
            .collect();
        order.sort_by_key(|&i| (self.lots[i].expiry_day, self.lots[i].id));

        let mut remaining = quantity;
        let mut picks = Vec::new();
        for i in order {
            if remaining == 0 {
                break;
            }
            let lot = &mut self.lots[i];
            let take = remaining.min(lot.quantity);
            lot.quantity -= take;
            remaining -= take;
            picks.push(Pick {
                lot_id: lot.id,
                quantity: take,
            });
        }
        Ok(picks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: u32 = 100;

    fn lot(id: u32, drug: &str, expiry_day: u32, quantity: u32) -> Lot {
        Lot {
            id,
            drug: drug.to_string(),
            expiry_day,
            quantity,
        }
    }

    #[test]
    fn earliest_expiry_is_dispensed_first() {
        let mut inv = Inventory::new();
        inv.add(lot(1, "heparin", 150, 10));
        inv.add(lot(2, "heparin", 120, 10));
        let picks = inv.dispense("heparin", 5, TODAY).expect("stocked");
        assert_eq!(
            picks,
            vec![Pick {
                lot_id: 2,
                quantity: 5
            }]
        );
    }

    #[test]
    fn a_request_spanning_lots_takes_them_in_expiry_order() {
        let mut inv = Inventory::new();
        inv.add(lot(1, "heparin", 150, 10));
        inv.add(lot(2, "heparin", 120, 4));
        let picks = inv.dispense("heparin", 7, TODAY).expect("stocked");
        assert_eq!(
            picks,
            vec![
                Pick {
                    lot_id: 2,
                    quantity: 4
                },
                Pick {
                    lot_id: 1,
                    quantity: 3
                },
            ]
        );
        assert_eq!(inv.usable("heparin", TODAY), 7);
    }

    #[test]
    fn expired_lots_are_never_dispensed() {
        let mut inv = Inventory::new();
        inv.add(lot(1, "insulin", TODAY - 1, 50));
        inv.add(lot(2, "insulin", TODAY + 30, 5));
        assert_eq!(inv.usable("insulin", TODAY), 5);
        let picks = inv.dispense("insulin", 5, TODAY).expect("stocked");
        assert_eq!(
            picks,
            vec![Pick {
                lot_id: 2,
                quantity: 5
            }]
        );
    }

    #[test]
    fn expiry_day_itself_is_still_usable() {
        let mut inv = Inventory::new();
        inv.add(lot(1, "insulin", TODAY, 3));
        assert_eq!(inv.usable("insulin", TODAY), 3);
    }

    #[test]
    fn insufficient_stock_takes_nothing() {
        let mut inv = Inventory::new();
        inv.add(lot(1, "heparin", 150, 3));
        let before = inv.clone();
        let err = inv.dispense("heparin", 4, TODAY).expect_err("short");
        assert_eq!(err, DispenseError::Insufficient { available: 3 });
        assert_eq!(inv, before, "a refused request must not change stock");
    }

    #[test]
    fn ties_on_expiry_go_to_the_lower_lot_id() {
        let mut inv = Inventory::new();
        inv.add(lot(9, "heparin", 150, 2));
        inv.add(lot(4, "heparin", 150, 2));
        let picks = inv.dispense("heparin", 2, TODAY).expect("stocked");
        assert_eq!(
            picks,
            vec![Pick {
                lot_id: 4,
                quantity: 2
            }]
        );
    }

    #[test]
    fn other_drugs_are_not_touched() {
        let mut inv = Inventory::new();
        inv.add(lot(1, "heparin", 150, 5));
        inv.add(lot(2, "morphine", 110, 5));
        inv.dispense("heparin", 5, TODAY).expect("stocked");
        assert_eq!(inv.usable("morphine", TODAY), 5);
    }
}
