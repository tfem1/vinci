//! Fixed-size circular file used by the disk-backed replay buffer.
//!
//! Records are written back-to-back and wrap to offset 0 when the end is reached. Each
//! record starts with a 16-byte header `{magic, len, seq}` that is re-validated before
//! *and* after reading the payload, so a reader racing with the writer detects an
//! overwritten record instead of producing a corrupt clip.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

const MAGIC: [u8; 4] = *b"VNC1";
pub const HEADER: u64 = 16;

/// The file itself — shared (`Arc`) between the writer and clip exporters.
#[derive(Debug)]
pub struct RingFile {
    file: File,
    path: PathBuf,
    capacity: u64,
}

impl RingFile {
    pub fn create(path: &Path, capacity: u64) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        file.set_len(capacity)?;
        Ok(Self { file, path: path.to_path_buf(), capacity })
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    fn write_at(&self, buf: &[u8], off: u64) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            self.file.write_all_at(buf, off)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            let mut done = 0;
            while done < buf.len() {
                let n = self.file.seek_write(&buf[done..], off + done as u64)?;
                if n == 0 {
                    return Err(io::ErrorKind::WriteZero.into());
                }
                done += n;
            }
            Ok(())
        }
    }

    fn read_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            self.file.read_exact_at(buf, off)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            let mut done = 0;
            while done < buf.len() {
                let n = self.file.seek_read(&mut buf[done..], off + done as u64)?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                done += n;
            }
            Ok(())
        }
    }

    fn header(len: u32, seq: u64) -> [u8; 16] {
        let mut h = [0u8; 16];
        h[0..4].copy_from_slice(&MAGIC);
        h[4..8].copy_from_slice(&len.to_le_bytes());
        h[8..16].copy_from_slice(&seq.to_le_bytes());
        h
    }

    pub fn write_record(&self, off: u64, seq: u64, data: &[u8]) -> io::Result<()> {
        // Payload first, header last: a reader never sees a valid header over stale data.
        self.write_at(data, off + HEADER)?;
        self.write_at(&Self::header(data.len() as u32, seq), off)
    }

    /// Read a record, verifying it still holds sequence `seq`.
    pub fn read_record(&self, off: u64, seq: u64, len: u32) -> io::Result<Vec<u8>> {
        let expect = Self::header(len, seq);
        let mut h = [0u8; 16];
        self.read_at(&mut h, off)?;
        if h != expect {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "replay record was overwritten"));
        }
        let mut data = vec![0u8; len as usize];
        self.read_at(&mut data, off + HEADER)?;
        self.read_at(&mut h, off)?;
        if h != expect {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "replay record was overwritten during read"));
        }
        Ok(data)
    }
}

impl Drop for RingFile {
    fn drop(&mut self) {
        // The ring only ever holds the last few minutes of gameplay: never leave it behind.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub seq: u64,
    pub off: u64,
    pub len: u64, // including header
}

/// Space allocator for the ring (owned by the buffer, mutated under its lock).
#[derive(Debug, Default)]
pub struct RingAlloc {
    capacity: u64,
    regions: VecDeque<Region>,
}

impl RingAlloc {
    pub fn new(capacity: u64) -> Self {
        Self { capacity, regions: VecDeque::new() }
    }

    pub fn used(&self) -> u64 {
        self.regions.iter().map(|r| r.len).sum()
    }

    /// Where a record of `payload` bytes would go, or `None` if the oldest records must
    /// be released first. `Err` if it can never fit.
    pub fn place(&self, payload: u64) -> Result<Option<u64>, ()> {
        let rec = payload + HEADER;
        if rec > self.capacity {
            return Err(());
        }
        let (Some(first), Some(last)) = (self.regions.front(), self.regions.back()) else { return Ok(Some(0)) };
        let tail = first.off;
        let head = last.off + last.len;
        let wrapped = last.off < first.off;
        Ok(if !wrapped {
            if head + rec <= self.capacity {
                Some(head)
            } else if rec <= tail {
                Some(0)
            } else {
                None
            }
        } else if head + rec <= tail {
            Some(head)
        } else {
            None
        })
    }

    pub fn commit(&mut self, seq: u64, off: u64, payload: u64) {
        self.regions.push_back(Region { seq, off, len: payload + HEADER });
    }

    /// Release the oldest record (must be `seq`).
    pub fn release_front(&mut self, seq: u64) {
        if let Some(r) = self.regions.front() {
            debug_assert_eq!(r.seq, seq, "ring released out of order");
            if r.seq == seq {
                self.regions.pop_front();
            }
        }
    }

    pub fn clear(&mut self) {
        self.regions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_wraps_and_respects_tail() {
        let mut a = RingAlloc::new(1000);
        assert_eq!(a.place(2000), Err(()));
        let mut seq = 0;
        let mut put = |a: &mut RingAlloc, n: u64| -> Option<u64> {
            let off = a.place(n).unwrap()?;
            seq += 1;
            a.commit(seq, off, n);
            Some(off)
        };
        assert_eq!(put(&mut a, 284), Some(0)); // 300 with header
        assert_eq!(put(&mut a, 284), Some(300));
        assert_eq!(put(&mut a, 284), Some(600));
        assert_eq!(put(&mut a, 284), None, "full: must release first");
        a.release_front(1);
        assert_eq!(put(&mut a, 284), Some(0), "wraps into released space");
        assert_eq!(put(&mut a, 84), None, "head 300 would hit tail at 300");
        a.release_front(2);
        assert_eq!(put(&mut a, 84), Some(300));
        assert_eq!(a.used(), 300 + 300 + 100);
    }

    #[test]
    fn records_roundtrip_and_detect_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ring.bin");
        {
            let f = RingFile::create(&p, 4096).unwrap();
            f.write_record(0, 1, b"hello").unwrap();
            assert_eq!(f.read_record(0, 1, 5).unwrap(), b"hello");
            f.write_record(0, 9, b"HELLO").unwrap();
            assert!(f.read_record(0, 1, 5).is_err(), "seq mismatch detected");
            assert!(p.exists());
        }
        assert!(!p.exists(), "ring file removed on drop");
    }
}
