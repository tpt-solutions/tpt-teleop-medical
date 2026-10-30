//! Narcotic chain-of-custody (spec.txt §5.2).
//!
//! Every change of custodian for a controlled lot is written to the compliance
//! journal, signed and hash-chained. The ledger's holdings are rebuilt by
//! replaying the journal on open. A record that breaks the rules (a transfer by
//! someone who does not hold the lot, for example) stops the open.
//!
//! Rules:
//!
//! * A lot is received once, by one holder, with a quantity.
//! * Only the current holder can transfer the lot, and the person who
//!   authenticates must be that holder.
//! * A transfer moves the whole lot. Partial transfers would need a split
//!   model, and the spec does not describe one.
//!
//! The biometric or badge check happens upstream. This module records the
//! method it is given, and it trusts the caller's claim that the check passed.

use std::collections::BTreeMap;
use std::fmt;

use tpt_t_medical_compliance::journal::{Journal, JournalError};
use tpt_t_medical_compliance::{Signer, Storage, Verifier};

/// How the person handing over the lot was authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Staff badge scanned.
    Badge,
    /// Fingerprint matched.
    Biometric,
}

impl Method {
    const fn code(self) -> u8 {
        match self {
            Self::Badge => 1,
            Self::Biometric => 2,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Badge),
            2 => Some(Self::Biometric),
            _ => None,
        }
    }
}

/// An authenticated identity, as verified upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authentication {
    /// The staff member who was authenticated.
    pub holder: u32,
    /// How they were authenticated.
    pub method: Method,
}

/// Who holds a lot, and how many units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holding {
    /// The current custodian.
    pub holder: u32,
    /// Units held.
    pub quantity: u32,
}

/// Why a custody operation was refused.
#[derive(Debug)]
pub enum CustodyError {
    /// The journal could not be written or verified.
    Journal(JournalError),
    /// No holding exists for this lot.
    NotHeld {
        /// The lot.
        lot: u32,
    },
    /// The lot is already held, so it cannot be received again.
    AlreadyHeld {
        /// The lot.
        lot: u32,
    },
    /// The authenticated person does not hold the lot.
    NotCustodian {
        /// The lot.
        lot: u32,
        /// The person who tried to transfer it.
        attempted_by: u32,
    },
    /// The lot is already held by the recipient.
    SameHolder {
        /// The lot.
        lot: u32,
    },
    /// A journal record does not follow the custody rules. Carries the index of
    /// the record.
    Inconsistent {
        /// Zero-based index of the record.
        record: usize,
    },
    /// A journal record could not be parsed.
    Malformed {
        /// Zero-based index of the record.
        record: usize,
    },
}

impl fmt::Display for CustodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(e) => write!(f, "custody journal: {e}"),
            Self::NotHeld { lot } => write!(f, "lot {lot} is not held"),
            Self::AlreadyHeld { lot } => write!(f, "lot {lot} is already held"),
            Self::NotCustodian { lot, attempted_by } => {
                write!(f, "{attempted_by} does not hold lot {lot}")
            }
            Self::SameHolder { lot } => write!(f, "lot {lot} is already with that holder"),
            Self::Inconsistent { record } => {
                write!(f, "custody record {record} breaks the custody rules")
            }
            Self::Malformed { record } => write!(f, "custody record {record} is malformed"),
        }
    }
}

impl std::error::Error for CustodyError {}

impl From<JournalError> for CustodyError {
    fn from(e: JournalError) -> Self {
        Self::Journal(e)
    }
}

const KIND_RECEIVE: u8 = 1;
const KIND_TRANSFER: u8 = 2;

enum Record {
    Receive {
        lot: u32,
        holder: u32,
        quantity: u32,
        now_us: u64,
    },
    Transfer {
        lot: u32,
        from: u32,
        to: u32,
        method: Method,
        now_us: u64,
    },
}

impl Record {
    fn encode(&self) -> Vec<u8> {
        match *self {
            Self::Receive {
                lot,
                holder,
                quantity,
                now_us,
            } => {
                let mut out = vec![KIND_RECEIVE];
                out.extend_from_slice(&lot.to_le_bytes());
                out.extend_from_slice(&holder.to_le_bytes());
                out.extend_from_slice(&quantity.to_le_bytes());
                out.extend_from_slice(&now_us.to_le_bytes());
                out
            }
            Self::Transfer {
                lot,
                from,
                to,
                method,
                now_us,
            } => {
                let mut out = vec![KIND_TRANSFER];
                out.extend_from_slice(&lot.to_le_bytes());
                out.extend_from_slice(&from.to_le_bytes());
                out.extend_from_slice(&to.to_le_bytes());
                out.push(method.code());
                out.extend_from_slice(&now_us.to_le_bytes());
                out
            }
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let (&kind, rest) = bytes.split_first()?;
        let u32_at = |i: usize| -> Option<u32> {
            Some(u32::from_le_bytes(rest.get(i..i + 4)?.try_into().ok()?))
        };
        let u64_at = |i: usize| -> Option<u64> {
            Some(u64::from_le_bytes(rest.get(i..i + 8)?.try_into().ok()?))
        };
        match kind {
            KIND_RECEIVE if rest.len() == 20 => Some(Self::Receive {
                lot: u32_at(0)?,
                holder: u32_at(4)?,
                quantity: u32_at(8)?,
                now_us: u64_at(12)?,
            }),
            KIND_TRANSFER if rest.len() == 21 => Some(Self::Transfer {
                lot: u32_at(0)?,
                from: u32_at(4)?,
                to: u32_at(8)?,
                method: Method::from_code(*rest.get(12)?)?,
                now_us: u64_at(13)?,
            }),
            _ => None,
        }
    }
}

impl<S: Storage, G: Signer> fmt::Debug for CustodyLedger<S, G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The signer is not shown, so key material cannot leak into logs.
        f.debug_struct("CustodyLedger")
            .field("holdings", &self.holdings)
            .finish_non_exhaustive()
    }
}

/// The custody ledger. Holdings are rebuilt from the journal on open.
pub struct CustodyLedger<S: Storage, G: Signer> {
    journal: Journal<S>,
    signer: G,
    holdings: BTreeMap<u32, Holding>,
}

impl<S: Storage, G: Signer> CustodyLedger<S, G> {
    /// Opens the ledger, verifying the journal and replaying every record.
    ///
    /// # Errors
    ///
    /// Returns an error if the journal does not verify, a record cannot be
    /// parsed, or a record breaks the custody rules.
    pub fn open<V: Verifier + ?Sized>(
        store: S,
        verifier: &V,
        signer: G,
    ) -> Result<Self, CustodyError> {
        let journal = Journal::open(store, verifier)?;
        let mut holdings = BTreeMap::new();
        for (index, entry) in journal.log().entries().iter().enumerate() {
            let record =
                Record::decode(&entry.payload).ok_or(CustodyError::Malformed { record: index })?;
            apply(&mut holdings, &record).ok_or(CustodyError::Inconsistent { record: index })?;
        }
        Ok(Self {
            journal,
            signer,
            holdings,
        })
    }

    /// Returns the current holding of `lot`, if it is held.
    #[must_use]
    pub fn holding(&self, lot: u32) -> Option<Holding> {
        self.holdings.get(&lot).copied()
    }

    /// Records the first receipt of `lot`.
    ///
    /// # Errors
    ///
    /// Returns [`CustodyError::AlreadyHeld`] if the lot is already held, or a
    /// journal error if the record cannot be written.
    pub fn receive(
        &mut self,
        lot: u32,
        holder: u32,
        quantity: u32,
        now_us: u64,
    ) -> Result<u64, CustodyError> {
        if self.holdings.contains_key(&lot) {
            return Err(CustodyError::AlreadyHeld { lot });
        }
        self.commit(&Record::Receive {
            lot,
            holder,
            quantity,
            now_us,
        })
    }

    /// Transfers `lot` to `to`. The authenticated person must hold the lot.
    ///
    /// Nothing is written unless every check passes.
    ///
    /// # Errors
    ///
    /// Returns an error if the lot is not held, the authenticated person is
    /// not its holder, the recipient already holds it, or the journal write
    /// fails.
    pub fn transfer(
        &mut self,
        lot: u32,
        to: u32,
        auth: Authentication,
        now_us: u64,
    ) -> Result<u64, CustodyError> {
        let current = self.holding(lot).ok_or(CustodyError::NotHeld { lot })?;
        if auth.holder != current.holder {
            return Err(CustodyError::NotCustodian {
                lot,
                attempted_by: auth.holder,
            });
        }
        if to == current.holder {
            return Err(CustodyError::SameHolder { lot });
        }
        self.commit(&Record::Transfer {
            lot,
            from: current.holder,
            to,
            method: auth.method,
            now_us,
        })
    }

    fn commit(&mut self, record: &Record) -> Result<u64, CustodyError> {
        let seq = self.journal.append(&self.signer, record.encode())?;
        // The record is durable, so apply it to the in-memory view.
        apply(&mut self.holdings, record).ok_or(CustodyError::Inconsistent {
            record: usize::try_from(seq).unwrap_or(usize::MAX),
        })?;
        Ok(seq)
    }
}

/// Applies one record to the holdings. Returns `None` if the record breaks the
/// custody rules.
fn apply(holdings: &mut BTreeMap<u32, Holding>, record: &Record) -> Option<()> {
    match *record {
        Record::Receive {
            lot,
            holder,
            quantity,
            ..
        } => {
            if holdings.contains_key(&lot) {
                return None;
            }
            holdings.insert(lot, Holding { holder, quantity });
        }
        Record::Transfer { lot, from, to, .. } => {
            let held = holdings.get_mut(&lot)?;
            if held.holder != from || to == from {
                return None;
            }
            held.holder = to;
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;

    const SEED: [u8; 32] = [17; 32];
    const NURSE_A: u32 = 100;
    const NURSE_B: u32 = 200;
    const PHARMACIST: u32 = 300;
    const LOT: u32 = 7;

    /// Storage shared between a test and the ledger it opens, so a test can
    /// reopen the same records.
    #[derive(Clone, Default)]
    struct SharedStore(Rc<RefCell<Vec<Vec<u8>>>>);

    impl Storage for SharedStore {
        fn append(&mut self, record: &[u8]) -> io::Result<()> {
            self.0.borrow_mut().push(record.to_vec());
            Ok(())
        }

        fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
            Ok(self.0.borrow().clone())
        }
    }

    fn ledger(store: &SharedStore) -> CustodyLedger<SharedStore, Ed25519Signer> {
        let signer = Ed25519Signer::from_seed(&SEED);
        let verifier = Ed25519Signer::from_seed(&SEED).verifier();
        CustodyLedger::open(store.clone(), &verifier, signer).expect("open")
    }

    fn auth(holder: u32) -> Authentication {
        Authentication {
            holder,
            method: Method::Biometric,
        }
    }

    #[test]
    fn receive_then_transfer_moves_custody() {
        let store = SharedStore::default();
        let mut l = ledger(&store);
        l.receive(LOT, PHARMACIST, 20, 1).expect("receive");
        l.transfer(LOT, NURSE_A, auth(PHARMACIST), 2)
            .expect("transfer");
        assert_eq!(
            l.holding(LOT),
            Some(Holding {
                holder: NURSE_A,
                quantity: 20
            })
        );
    }

    #[test]
    fn only_the_current_holder_can_transfer() {
        let store = SharedStore::default();
        let mut l = ledger(&store);
        l.receive(LOT, PHARMACIST, 20, 1).expect("receive");
        let err = l
            .transfer(LOT, NURSE_B, auth(NURSE_A), 2)
            .expect_err("not the holder");
        assert!(matches!(
            err,
            CustodyError::NotCustodian {
                attempted_by: NURSE_A,
                ..
            }
        ));
        assert_eq!(l.holding(LOT).expect("held").holder, PHARMACIST);
    }

    #[test]
    fn refused_transfer_writes_nothing_to_the_journal() {
        let store = SharedStore::default();
        let mut l = ledger(&store);
        l.receive(LOT, PHARMACIST, 20, 1).expect("receive");
        let before = store.0.borrow().len();
        let _ = l.transfer(LOT, NURSE_B, auth(NURSE_A), 2);
        let _ = l.transfer(LOT, PHARMACIST, auth(PHARMACIST), 3);
        assert_eq!(store.0.borrow().len(), before);
    }

    #[test]
    fn a_lot_cannot_be_received_twice() {
        let store = SharedStore::default();
        let mut l = ledger(&store);
        l.receive(LOT, PHARMACIST, 20, 1).expect("receive");
        assert!(matches!(
            l.receive(LOT, NURSE_A, 5, 2),
            Err(CustodyError::AlreadyHeld { lot: LOT })
        ));
    }

    #[test]
    fn transfer_of_an_unheld_lot_is_refused() {
        let store = SharedStore::default();
        let mut l = ledger(&store);
        assert!(matches!(
            l.transfer(LOT, NURSE_A, auth(PHARMACIST), 1),
            Err(CustodyError::NotHeld { lot: LOT })
        ));
    }

    #[test]
    fn custody_survives_reopen_by_replaying_the_journal() {
        let store = SharedStore::default();
        {
            let mut l = ledger(&store);
            l.receive(LOT, PHARMACIST, 20, 1).expect("receive");
            l.transfer(LOT, NURSE_A, auth(PHARMACIST), 2)
                .expect("transfer");
            l.transfer(LOT, NURSE_B, auth(NURSE_A), 3)
                .expect("transfer");
        }
        let reopened = ledger(&store);
        assert_eq!(reopened.holding(LOT).expect("held").holder, NURSE_B);
    }

    #[test]
    fn a_signed_but_rule_breaking_record_stops_the_open() {
        // A transfer by someone who never held the lot is validly signed, so the
        // journal accepts it. The custody replay must still refuse it.
        let store = SharedStore::default();
        {
            let mut l = ledger(&store);
            l.receive(LOT, PHARMACIST, 20, 1).expect("receive");
        }
        let signer = Ed25519Signer::from_seed(&SEED);
        let mut journal = Journal::open(store.clone(), &signer.verifier()).expect("journal");
        let forged = Record::Transfer {
            lot: LOT,
            from: NURSE_A,
            to: NURSE_B,
            method: Method::Badge,
            now_us: 2,
        };
        journal
            .append(&signer, forged.encode())
            .expect("journal accepts a validly signed record");

        let verifier = Ed25519Signer::from_seed(&SEED).verifier();
        let err = CustodyLedger::open(store.clone(), &verifier, Ed25519Signer::from_seed(&SEED))
            .expect_err("custody replay must refuse it");
        assert!(matches!(err, CustodyError::Inconsistent { record: 1 }));
    }

    #[test]
    fn records_round_trip_through_their_encoding() {
        let receive = Record::Receive {
            lot: 1,
            holder: 2,
            quantity: 3,
            now_us: 4,
        };
        let transfer = Record::Transfer {
            lot: 1,
            from: 2,
            to: 5,
            method: Method::Badge,
            now_us: 6,
        };
        for record in [receive, transfer] {
            let bytes = record.encode();
            assert!(Record::decode(&bytes).is_some());
            assert!(
                Record::decode(&bytes[..bytes.len() - 1]).is_none(),
                "truncated"
            );
        }
    }
}
