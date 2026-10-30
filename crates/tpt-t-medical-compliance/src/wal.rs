//! File-backed write-ahead log.
//!
//! Each record is written as a 4-byte little-endian length followed by the
//! record bytes. Every append is followed by `sync_data`, so the call returns
//! only after the record has reached the device.
//!
//! Opening a file with a partial record at the end is an error. The file is
//! not truncated. A torn tail may be the last record of a crash, or it may be
//! a sign of tampering, and deleting it silently would destroy that evidence.
//! Recovery is an operator decision.
//!
//! This backend uses ordinary buffered I/O. `O_DIRECT` (spec.txt §4.4) is not
//! implemented yet, and it is Linux-specific.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::Storage;

/// A WAL stored in one file.
#[derive(Debug)]
pub struct FileWal {
    file: File,
}

impl FileWal {
    /// Opens the WAL at `path`, creating it if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns any error from opening the file.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;
        Ok(Self { file })
    }
}

impl Storage for FileWal {
    fn append(&mut self, record: &[u8]) -> io::Result<()> {
        let len = u32::try_from(record.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "record too large"))?;
        let mut frame = Vec::with_capacity(4 + record.len());
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(record);
        self.file.write_all(&frame)?;
        self.file.sync_data()
    }

    fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        self.file.read_to_end(&mut bytes)?;

        let mut records = Vec::new();
        let mut pos = 0;
        while pos < bytes.len() {
            let torn = || {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("incomplete record at byte {pos}; refusing to open"),
                )
            };
            let header = bytes.get(pos..pos + 4).ok_or_else(torn)?;
            let len = usize::try_from(u32::from_le_bytes(header.try_into().map_err(|_| torn())?))
                .map_err(|_| torn())?;
            let start = pos + 4;
            let record = bytes.get(start..start + len).ok_or_else(torn)?;
            records.push(record.to_vec());
            pos = start + len;
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let path = env::temp_dir().join(format!("tpt-wal-{}-{name}.bin", std::process::id()));
        let _ = fs::remove_file(&path);
        path
    }

    #[test]
    fn records_round_trip_across_reopen() {
        let path = temp_path("round-trip");
        {
            let mut wal = FileWal::open(&path).expect("open");
            wal.append(b"one").expect("append");
            wal.append(b"").expect("append empty");
            wal.append(b"three").expect("append");
        }
        let mut wal = FileWal::open(&path).expect("reopen");
        let records = wal.records().expect("read");
        assert_eq!(
            records,
            vec![b"one".to_vec(), Vec::new(), b"three".to_vec()]
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn partial_trailing_record_is_refused_and_file_untouched() {
        let path = temp_path("torn");
        {
            let mut wal = FileWal::open(&path).expect("open");
            wal.append(b"complete").expect("append");
        }
        let before = fs::read(&path).expect("read");
        // A length header that promises 10 bytes, with only 2 present.
        let mut torn = before.clone();
        torn.extend_from_slice(&10u32.to_le_bytes());
        torn.extend_from_slice(b"ab");
        fs::write(&path, &torn).expect("write torn file");

        let mut wal = FileWal::open(&path).expect("open");
        let err = wal.records().expect_err("torn tail");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            fs::read(&path).expect("read"),
            torn,
            "file must not be truncated"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn truncated_length_header_is_refused() {
        let path = temp_path("torn-header");
        fs::write(&path, [1u8, 0]).expect("write");
        let mut wal = FileWal::open(&path).expect("open");
        assert_eq!(
            wal.records().expect_err("short header").kind(),
            io::ErrorKind::InvalidData
        );
        let _ = fs::remove_file(&path);
    }
}
