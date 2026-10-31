//! Append-only, hash-chained audit log (spec.txt §4.4).
//!
//! Each entry commits to the one before it. The entry's SHA-256 digest covers
//! its sequence number, the previous entry's digest, and the payload. Changing
//! any entry, or removing or reordering one, breaks verification at the first
//! affected position.
//!
//! Signing is pluggable through [`Signer`] and [`Verifier`]. The [`ed25519`]
//! module provides the Ed25519 backend, built on `ed25519-dalek`
//! (BSD-3-Clause). Hardware-backed keys (TPM, secure enclave) are not
//! implemented yet.
//!
//! The [journal] module persists each entry through a [Storage] before it
//! counts as committed. [`wal::FileWal`] is the file backend. It uses buffered
//! writes with a sync after each append, and it refuses to open a torn tail.
//!
//! Not yet implemented: `O_DIRECT` writes, and rkyv serialization of payloads.
//! Payloads are opaque bytes for now.
//!
//! The log allocates. Appends belong on a logging thread, not on the real-time
//! control path.

#![forbid(unsafe_code)]

pub mod direct;
pub mod ed25519;
pub mod journal;
pub mod wal;

mod codec;

use core::fmt;

use sha2::{Digest, Sha256};

/// A SHA-256 digest.
pub type Hash = [u8; 32];

/// The previous hash of the first entry in any log.
pub const GENESIS: Hash = [0; 32];

/// One record in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Position in the log, starting at 0.
    pub seq: u64,
    /// Digest of the previous entry, or [`GENESIS`] for the first entry.
    pub prev: Hash,
    /// The recorded event, as opaque bytes.
    pub payload: Vec<u8>,
    /// Digest over `seq`, `prev` and `payload`. See [`digest`].
    pub hash: Hash,
    /// Signature over `hash`, produced by a [`Signer`].
    pub signature: Vec<u8>,
}

/// Signs entry digests.
pub trait Signer {
    /// Returns a signature over `hash`.
    fn sign(&self, hash: &Hash) -> Vec<u8>;
}

/// Checks entry signatures.
pub trait Verifier {
    /// Returns `true` if `signature` is a valid signature of `hash`.
    fn verify(&self, hash: &Hash, signature: &[u8]) -> bool;
}

/// Durable storage for encoded entries.
pub trait Storage {
    /// Appends one record. Returns only once the record is durable.
    ///
    /// # Errors
    ///
    /// Returns an error if the record cannot be written or made durable.
    fn append(&mut self, record: &[u8]) -> std::io::Result<()>;

    /// Returns every stored record, oldest first.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage cannot be read or is malformed.
    fn records(&mut self) -> std::io::Result<Vec<Vec<u8>>>;
}

/// Computes an entry digest.
///
/// The payload length is hashed too, so a digest cannot be read as a
/// different split of the same bytes.
#[must_use]
pub fn digest(seq: u64, prev: &Hash, payload: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(seq.to_le_bytes());
    hasher.update(prev);
    hasher.update(
        u64::try_from(payload.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    hasher.update(payload);
    hasher.finalize().into()
}

/// Which check failed, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainError {
    /// Position of the first entry that failed.
    pub index: usize,
    /// The check that failed.
    pub kind: ChainErrorKind,
}

/// The check that a [`ChainError`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainErrorKind {
    /// `seq` does not match the entry's position. An entry was removed,
    /// inserted or reordered.
    Sequence,
    /// `prev` does not match the previous entry's hash. An earlier entry was
    /// changed after this one was written.
    Link,
    /// `hash` does not match the recomputed digest. This entry's contents were
    /// changed.
    Digest,
    /// The signature does not verify.
    Signature,
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            ChainErrorKind::Sequence => "sequence number",
            ChainErrorKind::Link => "link to previous entry",
            ChainErrorKind::Digest => "digest",
            ChainErrorKind::Signature => "signature",
        };
        write!(f, "audit chain broken at entry {}: bad {what}", self.index)
    }
}

impl std::error::Error for ChainError {}

/// An append-only log.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Log {
    entries: Vec<Entry>,
}

impl Log {
    /// Creates an empty log.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Returns the entries, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Returns the hash of the newest entry, or [`GENESIS`] for an empty log.
    #[must_use]
    pub fn head(&self) -> Hash {
        self.entries.last().map_or(GENESIS, |e| e.hash)
    }

    /// Builds the entry that would follow the current head, without adding it.
    ///
    /// Persist the entry first, then pass it to [`Log::push`]. That way the
    /// in-memory log never holds an entry that is not on disk.
    #[must_use]
    pub fn seal<S: Signer + ?Sized>(&self, signer: &S, payload: Vec<u8>) -> Entry {
        let seq = u64::try_from(self.entries.len()).unwrap_or(u64::MAX);
        let prev = self.head();
        let hash = digest(seq, &prev, &payload);
        let signature = signer.sign(&hash);
        Entry {
            seq,
            prev,
            payload,
            hash,
            signature,
        }
    }

    /// Adds a sealed entry to the end of the log.
    ///
    /// The entry's position, link and digest are checked. Its signature is not,
    /// because the caller produced it.
    ///
    /// # Errors
    ///
    /// Returns the failed check if the entry does not follow the head.
    pub fn push(&mut self, entry: Entry) -> Result<u64, ChainError> {
        let index = self.entries.len();
        let fail = |kind| ChainError { index, kind };
        if entry.seq != u64::try_from(index).unwrap_or(u64::MAX) {
            return Err(fail(ChainErrorKind::Sequence));
        }
        if entry.prev != self.head() {
            return Err(fail(ChainErrorKind::Link));
        }
        if entry.hash != digest(entry.seq, &entry.prev, &entry.payload) {
            return Err(fail(ChainErrorKind::Digest));
        }
        let seq = entry.seq;
        self.entries.push(entry);
        Ok(seq)
    }

    /// Creates a log from entries that must all verify, such as entries read
    /// back from disk.
    ///
    /// # Errors
    ///
    /// Returns the first entry that fails a check.
    pub fn from_entries<V: Verifier + ?Sized>(
        entries: Vec<Entry>,
        verifier: &V,
    ) -> Result<Self, ChainError> {
        verify_entries(&entries, verifier)?;
        Ok(Self { entries })
    }

    /// Appends `payload` and returns its sequence number.
    pub fn append<S: Signer + ?Sized>(&mut self, signer: &S, payload: Vec<u8>) -> u64 {
        let entry = self.seal(signer, payload);
        let seq = entry.seq;
        self.entries.push(entry);
        seq
    }

    /// Checks the whole log.
    ///
    /// # Errors
    ///
    /// Returns the first entry that fails a check.
    pub fn verify<V: Verifier + ?Sized>(&self, verifier: &V) -> Result<(), ChainError> {
        verify_entries(&self.entries, verifier)
    }
}

/// Checks a slice of entries as a chain, without requiring a [`Log`].
///
/// Use this to check an exported or archived log.
///
/// # Errors
///
/// Returns the first entry that fails a check.
pub fn verify_entries<V: Verifier + ?Sized>(
    entries: &[Entry],
    verifier: &V,
) -> Result<(), ChainError> {
    let mut prev = GENESIS;
    for (index, entry) in entries.iter().enumerate() {
        let fail = |kind| ChainError { index, kind };
        if entry.seq != u64::try_from(index).unwrap_or(u64::MAX) {
            return Err(fail(ChainErrorKind::Sequence));
        }
        if entry.prev != prev {
            return Err(fail(ChainErrorKind::Link));
        }
        if entry.hash != digest(entry.seq, &entry.prev, &entry.payload) {
            return Err(fail(ChainErrorKind::Digest));
        }
        if !verifier.verify(&entry.hash, &entry.signature) {
            return Err(fail(ChainErrorKind::Signature));
        }
        prev = entry.hash;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only signer. It is a keyed digest, not a public-key signature, and
    /// it exists only to exercise the wiring. Production uses an asymmetric
    /// scheme.
    struct TestKey(&'static [u8]);

    impl TestKey {
        fn tag(&self, hash: &Hash) -> Vec<u8> {
            let mut h = Sha256::new();
            h.update(self.0);
            h.update(hash);
            h.finalize().to_vec()
        }
    }

    impl Signer for TestKey {
        fn sign(&self, hash: &Hash) -> Vec<u8> {
            self.tag(hash)
        }
    }

    impl Verifier for TestKey {
        fn verify(&self, hash: &Hash, signature: &[u8]) -> bool {
            self.tag(hash) == signature
        }
    }

    fn sample_log() -> Log {
        let key = TestKey(b"test-key");
        let mut log = Log::new();
        for payload in [&b"pause"[..], b"resume", b"emergency-stop"] {
            log.append(&key, payload.to_vec());
        }
        log
    }

    #[test]
    fn empty_log_verifies_and_has_genesis_head() {
        let log = Log::new();
        assert_eq!(log.head(), GENESIS);
        assert_eq!(log.verify(&TestKey(b"test-key")), Ok(()));
    }

    #[test]
    fn appended_log_verifies() {
        let log = sample_log();
        assert_eq!(log.entries().len(), 3);
        assert_eq!(log.verify(&TestKey(b"test-key")), Ok(()));
    }

    #[test]
    fn each_entry_links_to_the_one_before() {
        let log = sample_log();
        let entries = log.entries();
        assert_eq!(entries[0].prev, GENESIS);
        assert_eq!(entries[1].prev, entries[0].hash);
        assert_eq!(entries[2].prev, entries[1].hash);
        assert_eq!(log.head(), entries[2].hash);
    }

    #[test]
    fn changed_payload_is_a_digest_failure_at_that_entry() {
        let mut entries = sample_log().entries().to_vec();
        entries[1].payload = b"resume-at-speed".to_vec();
        assert_eq!(
            verify_entries(&entries, &TestKey(b"test-key")),
            Err(ChainError {
                index: 1,
                kind: ChainErrorKind::Digest
            })
        );
    }

    #[test]
    fn recomputed_entry_still_breaks_the_next_link() {
        // An attacker who rewrites entry 1 and recomputes its digest and
        // signature still breaks entry 2's link to it.
        let key = TestKey(b"test-key");
        let mut entries = sample_log().entries().to_vec();
        entries[1].payload = b"resume-at-speed".to_vec();
        entries[1].hash = digest(entries[1].seq, &entries[1].prev, &entries[1].payload);
        entries[1].signature = key.sign(&entries[1].hash);
        assert_eq!(
            verify_entries(&entries, &key),
            Err(ChainError {
                index: 2,
                kind: ChainErrorKind::Link
            })
        );
    }

    #[test]
    fn removed_entry_is_a_sequence_failure() {
        let mut entries = sample_log().entries().to_vec();
        entries.remove(1);
        assert_eq!(
            verify_entries(&entries, &TestKey(b"test-key")),
            Err(ChainError {
                index: 1,
                kind: ChainErrorKind::Sequence
            })
        );
    }

    #[test]
    fn reordered_entries_are_a_sequence_failure() {
        let mut entries = sample_log().entries().to_vec();
        entries.swap(0, 1);
        assert_eq!(
            verify_entries(&entries, &TestKey(b"test-key")),
            Err(ChainError {
                index: 0,
                kind: ChainErrorKind::Sequence
            })
        );
    }

    #[test]
    fn wrong_key_is_a_signature_failure() {
        let log = sample_log();
        assert_eq!(
            log.verify(&TestKey(b"other-key")),
            Err(ChainError {
                index: 0,
                kind: ChainErrorKind::Signature
            })
        );
    }

    #[test]
    fn digest_depends_on_every_field() {
        let base = digest(0, &GENESIS, b"x");
        assert_ne!(base, digest(1, &GENESIS, b"x"));
        assert_ne!(base, digest(0, &[1; 32], b"x"));
        assert_ne!(base, digest(0, &GENESIS, b"y"));
    }

    #[test]
    fn error_message_names_the_entry_and_check() {
        let err = ChainError {
            index: 7,
            kind: ChainErrorKind::Link,
        };
        assert_eq!(
            err.to_string(),
            "audit chain broken at entry 7: bad link to previous entry"
        );
    }
}
