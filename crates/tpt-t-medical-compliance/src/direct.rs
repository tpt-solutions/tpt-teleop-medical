//! Block-aligned write-ahead log for direct I/O (spec.txt §4.4, `O_DIRECT`).
//!
//! Direct I/O bypasses the page cache, so the device gets each write as it is
//! issued. It needs two things from the caller: the buffer must start on a
//! block boundary, and each write must be a whole number of blocks. This log
//! meets both. Every record is written as a 4-byte little-endian length, the
//! record bytes, and zero padding up to the next [`BLOCK`] boundary. The
//! buffer is aligned by offsetting into an over-allocated `Vec`, which is safe
//! code.
//!
//! The reader checks the padding. A non-zero byte in it is an error, and the
//! file is not repaired, for the same reason as [`crate::wal`]: the bytes are
//! evidence.
//!
//! What is done per platform:
//!
//! * **Windows**: the write handle is opened with `FILE_FLAG_NO_BUFFERING` and
//!   `FILE_FLAG_WRITE_THROUGH`. The device must accept 4 KiB writes, which every
//!   modern sector size does.
//! * **Other platforms**: the same block format is written, through ordinary
//!   buffered I/O with `sync_data`. Linux `O_DIRECT` needs an architecture
//!   constant that has not been checked on the target hardware, so it is not
//!   set here.
//!
//! Neither direct-I/O path has been measured on `NVMe` hardware. That needs the
//! target device.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::Storage;

/// The block size every write is padded to, in bytes.
pub const BLOCK: usize = 4_096;

/// Windows `FILE_FLAG_NO_BUFFERING`: bypass the cache.
#[cfg(windows)]
const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
/// Windows `FILE_FLAG_WRITE_THROUGH`: do not return until the device has the data.
#[cfg(windows)]
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;

/// A write-ahead log whose writes are whole, aligned blocks.
#[derive(Debug)]
pub struct DirectWal {
    path: PathBuf,
    file: File,
}

impl DirectWal {
    /// Opens the log at `path`, creating it if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened, or if its length is not a
    /// whole number of blocks. A partial block is a torn write, and it is not
    /// truncated here.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = open_direct(path)?;
        let len = fs::metadata(path)?.len();
        if len % BLOCK as u64 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "log length is not a whole number of blocks; recovery is an operator decision",
            ));
        }
        Ok(Self {
            path: path.to_path_buf(),
            file,
        })
    }
}

#[cfg(windows)]
fn open_direct(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    // Plain write access: Windows refuses append-only access with no buffering.
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH)
        .open(path)
}

#[cfg(not(windows))]
fn open_direct(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Returns `len` rounded up to the next block boundary.
const fn round_up(len: usize) -> usize {
    len.div_ceil(BLOCK) * BLOCK
}

/// Builds one block-padded frame in a buffer that starts on a block boundary.
/// Returns the buffer and the offset of the frame inside it.
fn aligned_frame(record: &[u8]) -> io::Result<(Vec<u8>, usize, usize)> {
    let len = u32::try_from(record.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "record too large"))?;
    let used = 4 + record.len();
    let total = round_up(used);
    // Over-allocate by one block, so an aligned start always fits.
    let mut buf = vec![0_u8; total + BLOCK];
    let start = buf.as_ptr().align_offset(BLOCK);
    let frame = &mut buf[start..start + total];
    frame[..4].copy_from_slice(&len.to_le_bytes());
    frame[4..used].copy_from_slice(record);
    Ok((buf, start, total))
}

impl Storage for DirectWal {
    fn append(&mut self, record: &[u8]) -> io::Result<()> {
        let (buf, start, total) = aligned_frame(record)?;
        // The end of the log is always a block boundary, so this seek keeps the
        // write aligned.
        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&buf[start..start + total])?;
        self.file.sync_data()
    }

    fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
        // A separate buffered read keeps the direct handle to writes only.
        let bytes = fs::read(&self.path)?;
        decode(&bytes)
    }
}

/// Decodes a log, checking every length and every padding byte.
fn decode(bytes: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut records = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let header = bytes
            .get(pos..pos + 4)
            .ok_or_else(|| bad("truncated length"))?;
        let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let body_end = pos + 4 + len;
        let body = bytes
            .get(pos + 4..body_end)
            .ok_or_else(|| bad("record runs past the end of the log"))?;
        let next = round_up(body_end);
        let padding = bytes
            .get(body_end..next)
            .ok_or_else(|| bad("padding runs past the end of the log"))?;
        if padding.iter().any(|&b| b != 0) {
            return Err(bad("non-zero padding after a record"));
        }
        records.push(body.to_vec());
        pos = next;
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory for one test's log, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("tpt-direct-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }

        fn log(&self) -> PathBuf {
            self.0.join("audit.log")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn records_round_trip_and_every_write_is_a_whole_block() {
        let scratch = Scratch::new("roundtrip");
        let mut wal = DirectWal::open(&scratch.log()).expect("open");
        wal.append(b"first").expect("append");
        wal.append(&vec![7_u8; 5_000]).expect("append");
        wal.append(b"").expect("append empty");
        assert_eq!(
            fs::metadata(scratch.log()).expect("meta").len() % BLOCK as u64,
            0
        );
        let got = wal.records().expect("records");
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], b"first");
        assert_eq!(got[1].len(), 5_000);
        assert_eq!(got[2].len(), 0);
    }

    #[test]
    fn a_reopened_log_returns_the_same_records() {
        let scratch = Scratch::new("reopen");
        {
            let mut wal = DirectWal::open(&scratch.log()).expect("open");
            wal.append(b"kept").expect("append");
        }
        let mut again = DirectWal::open(&scratch.log()).expect("reopen");
        assert_eq!(again.records().expect("records"), vec![b"kept".to_vec()]);
        again.append(b"more").expect("append after reopen");
        assert_eq!(again.records().expect("records").len(), 2);
    }

    #[test]
    fn a_partial_block_is_refused_and_left_in_place() {
        let scratch = Scratch::new("torn");
        fs::write(scratch.log(), vec![1_u8; 100]).expect("write");
        assert!(DirectWal::open(&scratch.log()).is_err());
        assert_eq!(fs::metadata(scratch.log()).expect("meta").len(), 100);
    }

    #[test]
    fn non_zero_padding_is_detected() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&3_u32.to_le_bytes());
        frame.extend_from_slice(b"abc");
        frame.resize(BLOCK, 0);
        frame[BLOCK - 1] = 0xff;
        assert!(decode(&frame).is_err());
        frame[BLOCK - 1] = 0;
        assert_eq!(decode(&frame).expect("clean").len(), 1);
    }

    #[test]
    fn a_length_that_runs_past_the_end_is_detected() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&5_000_u32.to_le_bytes());
        frame.resize(BLOCK, 0);
        assert!(decode(&frame).is_err());
    }

    #[test]
    fn the_write_buffer_starts_on_a_block_boundary() {
        for n in [0_usize, 1, 4_092, 4_093, 9_000] {
            let record = vec![3_u8; n];
            let (buf, start, total) = aligned_frame(&record).expect("frame");
            assert_eq!((buf.as_ptr().addr() + start) % BLOCK, 0, "record {n}");
            assert_eq!(total % BLOCK, 0);
        }
    }
}
