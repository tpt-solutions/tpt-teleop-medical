//! A [`Log`] whose every entry is persisted before it counts as committed.
//!
//! On open, the journal reads the stored records, decodes them, and checks
//! the whole chain. A record that cannot be decoded, or a chain that does not
//! verify, stops the open. Nothing is repaired automatically.

use std::fmt;
use std::io;

use crate::codec;
use crate::{ChainError, Log, Signer, Storage, Verifier};

/// Why a journal could not be opened or appended to.
#[derive(Debug)]
pub enum JournalError {
    /// The storage failed, or its framing is malformed.
    Io(io::Error),
    /// Record `record` is not a valid encoded entry.
    Corrupt {
        /// Zero-based index of the bad record.
        record: usize,
    },
    /// The chain or a signature does not verify.
    Chain(ChainError),
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "journal storage error: {e}"),
            Self::Corrupt { record } => write!(f, "record {record} is not a valid entry"),
            Self::Chain(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for JournalError {}

impl From<io::Error> for JournalError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ChainError> for JournalError {
    fn from(e: ChainError) -> Self {
        Self::Chain(e)
    }
}

/// A verified log backed by durable storage.
#[derive(Debug)]
pub struct Journal<S: Storage> {
    log: Log,
    store: S,
}

impl<S: Storage> Journal<S> {
    /// Opens the journal and verifies every stored entry.
    ///
    /// # Errors
    ///
    /// Returns an error if storage cannot be read, a record cannot be decoded,
    /// or the chain does not verify.
    pub fn open<V: Verifier + ?Sized>(mut store: S, verifier: &V) -> Result<Self, JournalError> {
        let records = store.records()?;
        let mut entries = Vec::with_capacity(records.len());
        for (record, bytes) in records.iter().enumerate() {
            let entry = codec::decode(bytes).ok_or(JournalError::Corrupt { record })?;
            entries.push(entry);
        }
        let log = Log::from_entries(entries, verifier)?;
        Ok(Self { log, store })
    }

    /// Returns the verified log.
    #[must_use]
    pub const fn log(&self) -> &Log {
        &self.log
    }

    /// Signs and persists `payload`, then adds it to the log. Returns its
    /// sequence number.
    ///
    /// If storage fails, the log is unchanged and the error is returned.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry cannot be encoded or written.
    pub fn append<G: Signer + ?Sized>(
        &mut self,
        signer: &G,
        payload: Vec<u8>,
    ) -> Result<u64, JournalError> {
        let entry = self.log.seal(signer, payload);
        self.store.append(&codec::encode(&entry)?)?;
        Ok(self.log.push(entry)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChainErrorKind;
    use crate::ed25519::Ed25519Signer;
    use crate::wal::FileWal;
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::path::{Path, PathBuf};

    const SEED: [u8; 32] = [9; 32];

    fn temp_path(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!("tpt-journal-{}-{name}.bin", std::process::id()));
        let _ = fs::remove_file(&path);
        path
    }

    fn write_three(path: &Path) {
        let signer = Ed25519Signer::from_seed(&SEED);
        let mut journal =
            Journal::open(FileWal::open(path).expect("open"), &signer.verifier()).expect("fresh");
        for payload in [&b"register"[..], b"activate", b"pause"] {
            journal.append(&signer, payload.to_vec()).expect("append");
        }
    }

    #[test]
    fn appended_entries_survive_reopen_and_verify() {
        let path = temp_path("reopen");
        write_three(&path);

        let signer = Ed25519Signer::from_seed(&SEED);
        let journal =
            Journal::open(FileWal::open(&path).expect("open"), &signer.verifier()).expect("reopen");
        assert_eq!(journal.log().entries().len(), 3);
        assert_eq!(journal.log().entries()[2].payload, b"pause");
        assert_eq!(journal.log().verify(&signer.verifier()), Ok(()));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn reopened_journal_continues_the_chain() {
        let path = temp_path("continue");
        write_three(&path);

        let signer = Ed25519Signer::from_seed(&SEED);
        let mut journal =
            Journal::open(FileWal::open(&path).expect("open"), &signer.verifier()).expect("reopen");
        let seq = journal
            .append(&signer, b"resume".to_vec())
            .expect("append after reopen");
        assert_eq!(seq, 3);
        assert_eq!(journal.log().verify(&signer.verifier()), Ok(()));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn modified_byte_on_disk_stops_the_open_at_that_entry() {
        let path = temp_path("tamper");
        write_three(&path);

        // The last byte of the file is in the last entry's payload.
        let mut bytes = fs::read(&path).expect("read");
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        fs::write(&path, &bytes).expect("write");

        let signer = Ed25519Signer::from_seed(&SEED);
        let err = Journal::open(FileWal::open(&path).expect("open"), &signer.verifier())
            .expect_err("tampered");
        assert!(matches!(
            err,
            JournalError::Chain(ChainError {
                index: 2,
                kind: ChainErrorKind::Digest
            })
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn torn_tail_stops_the_open_without_repairing_the_file() {
        let path = temp_path("torn");
        write_three(&path);
        let before = fs::read(&path).expect("read");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut f| io::Write::write_all(&mut f, &[9, 0, 0, 0, 1]))
            .expect("append garbage");

        let signer = Ed25519Signer::from_seed(&SEED);
        let err = Journal::open(FileWal::open(&path).expect("open"), &signer.verifier())
            .expect_err("torn");
        assert!(matches!(err, JournalError::Io(ref e) if e.kind() == io::ErrorKind::InvalidData));
        assert!(
            fs::metadata(&path).expect("metadata").len() > before.len() as u64,
            "the torn bytes must stay on disk for an operator to inspect"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn undecodable_record_is_reported_by_index() {
        let path = temp_path("corrupt");
        {
            let mut wal = FileWal::open(&path).expect("open");
            wal.append(b"not an entry").expect("append");
        }
        let signer = Ed25519Signer::from_seed(&SEED);
        let err = Journal::open(FileWal::open(&path).expect("open"), &signer.verifier())
            .expect_err("corrupt");
        assert!(matches!(err, JournalError::Corrupt { record: 0 }));
        let _ = fs::remove_file(&path);
    }
}
