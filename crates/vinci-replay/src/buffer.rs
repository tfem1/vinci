//! The replay buffer: a GOP-aligned circular buffer of encoded fragments.
//!
//! Invariants:
//! * every epoch (one encoder session) starts with a keyframe fragment;
//! * eviction removes whole GOPs from the front, so the buffer always starts on a
//!   keyframe and a clip can be cut at any GOP boundary without re-encoding;
//! * at least `max_duration` seconds are kept whenever available, bounded by
//!   `max_bytes` (RAM mode) or by the ring capacity (disk mode).

use crate::mp4::InitInfo;
use crate::ring::{RingAlloc, RingFile};
use crate::stream::Fragment;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Storage {
    /// Keep encoded fragments in RAM.
    Memory,
    /// Keep them in a preallocated circular file (`capacity` bytes) — minimal RAM.
    Disk { path: PathBuf, capacity: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayConfig {
    pub max_duration_secs: f64,
    pub max_bytes: u64,
    pub storage: Storage,
}

#[derive(Debug, Clone)]
enum Loc {
    Mem(Bytes),
    Disk { off: u64, len: u32 },
}

#[derive(Debug, Clone)]
struct Entry {
    epoch: u64,
    seq: u64,
    keyframe: bool,
    duration: f64,
    bytes: u64,
    loc: Loc,
}

#[derive(Debug, Clone)]
struct Epoch {
    id: u64,
    init: Bytes,
    #[allow(dead_code)]
    info: Arc<InitInfo>,
    started: Instant,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BufferStats {
    pub buffered_secs: f64,
    pub bytes: u64,
    pub fragments: usize,
    pub epochs: usize,
    pub evicted_fragments: u64,
    pub dropped_leading: u64,
    pub disk: bool,
}

pub struct ReplayBuffer {
    cfg: ReplayConfig,
    entries: VecDeque<Entry>,
    epochs: VecDeque<Epoch>,
    ring: Option<(Arc<RingFile>, RingAlloc)>,
    total_bytes: u64,
    total_secs: f64,
    next_seq: u64,
    next_epoch: u64,
    current_epoch_has_key: bool,
    evicted: u64,
    dropped_leading: u64,
    last_push: Option<Instant>,
}

impl ReplayBuffer {
    pub fn new(cfg: ReplayConfig) -> io::Result<Self> {
        let ring = match &cfg.storage {
            Storage::Memory => None,
            Storage::Disk { path, capacity } => {
                let f = Arc::new(RingFile::create(path, *capacity)?);
                Some((f, RingAlloc::new(*capacity)))
            }
        };
        Ok(Self {
            cfg,
            entries: VecDeque::new(),
            epochs: VecDeque::new(),
            ring,
            total_bytes: 0,
            total_secs: 0.0,
            next_seq: 1,
            next_epoch: 1,
            current_epoch_has_key: false,
            evicted: 0,
            dropped_leading: 0,
            last_push: None,
        })
    }

    pub fn config(&self) -> &ReplayConfig {
        &self.cfg
    }

    /// Change the target duration on the fly (no data loss when growing).
    pub fn set_max_duration(&mut self, secs: f64) {
        self.cfg.max_duration_secs = secs;
        self.evict_policy();
    }

    /// A new encoder session started (first start, restart after a crash…).
    pub fn begin_epoch(&mut self, init: Bytes, info: Arc<InitInfo>) {
        let id = self.next_epoch;
        self.next_epoch += 1;
        self.epochs.push_back(Epoch { id, init, info, started: Instant::now() });
        self.current_epoch_has_key = false;
        self.prune_epochs();
    }

    pub fn has_epoch(&self) -> bool {
        !self.epochs.is_empty()
    }

    pub fn last_push(&self) -> Option<Instant> {
        self.last_push
    }

    pub fn push(&mut self, frag: Fragment) -> io::Result<()> {
        let Some(epoch) = self.epochs.back().map(|e| e.id) else {
            return Err(io::Error::other("fragment pushed before init segment"));
        };
        if !self.current_epoch_has_key {
            if !frag.keyframe {
                self.dropped_leading += 1;
                return Ok(());
            }
            self.current_epoch_has_key = true;
        }
        let len = frag.data.len() as u64;
        let seq = self.next_seq;
        self.next_seq += 1;
        let loc = if self.ring.is_some() {
            let off = loop {
                let placed = self.ring.as_ref().unwrap().1.place(len);
                match placed {
                    Err(()) => return Err(io::Error::other(format!("fragment of {len} bytes larger than the disk ring"))),
                    Ok(Some(off)) => break off,
                    Ok(None) => {
                        if !self.evict_gop() {
                            return Err(io::Error::other("disk ring cannot make room"));
                        }
                    }
                }
            };
            let (file, alloc) = self.ring.as_mut().unwrap();
            file.write_record(off, seq, &frag.data)?;
            alloc.commit(seq, off, len);
            Loc::Disk { off, len: len as u32 }
        } else {
            Loc::Mem(frag.data)
        };
        self.entries.push_back(Entry { epoch, seq, keyframe: frag.keyframe, duration: frag.duration.max(0.0), bytes: len, loc });
        self.total_bytes += len;
        self.total_secs += frag.duration.max(0.0);
        self.last_push = Some(Instant::now());
        self.evict_policy();
        Ok(())
    }

    fn evict_policy(&mut self) {
        loop {
            let first_gop = self.first_gop_secs();
            let over_duration = self.total_secs - first_gop >= self.cfg.max_duration_secs;
            let over_bytes = self.total_bytes > self.cfg.max_bytes;
            if !(over_duration || over_bytes) || self.gop_count() <= 1 {
                break;
            }
            self.evict_gop();
        }
    }

    fn gop_count(&self) -> usize {
        self.entries.iter().filter(|e| e.keyframe).count()
    }

    fn first_gop_secs(&self) -> f64 {
        let mut it = self.entries.iter();
        let Some(first) = it.next() else { return 0.0 };
        let mut d = first.duration;
        for e in it {
            if e.keyframe {
                break;
            }
            d += e.duration;
        }
        d
    }

    /// Remove the oldest GOP. Returns false if the buffer is empty.
    fn evict_gop(&mut self) -> bool {
        let Some(first) = self.entries.pop_front() else { return false };
        self.forget(&first);
        while self.entries.front().is_some_and(|e| !e.keyframe) {
            let e = self.entries.pop_front().unwrap();
            self.forget(&e);
        }
        self.prune_epochs();
        true
    }

    fn forget(&mut self, e: &Entry) {
        self.total_bytes -= e.bytes;
        self.total_secs = (self.total_secs - e.duration).max(0.0);
        self.evicted += 1;
        if let (Some((_, alloc)), Loc::Disk { .. }) = (self.ring.as_mut(), &e.loc) {
            alloc.release_front(e.seq);
        }
    }

    fn prune_epochs(&mut self) {
        let oldest_needed = self.entries.front().map(|e| e.epoch);
        while self.epochs.len() > 1 {
            let front = self.epochs.front().unwrap().id;
            if Some(front) == oldest_needed {
                break;
            }
            // Front epoch has no entries left (entries are epoch-ordered).
            if oldest_needed.is_none_or(|o| front < o) {
                self.epochs.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.epochs.clear();
        if let Some((_, a)) = self.ring.as_mut() {
            a.clear();
        }
        self.total_bytes = 0;
        self.total_secs = 0.0;
        self.current_epoch_has_key = false;
    }

    pub fn buffered_secs(&self) -> f64 {
        self.total_secs
    }

    pub fn stats(&self) -> BufferStats {
        BufferStats {
            buffered_secs: self.total_secs,
            bytes: self.total_bytes,
            fragments: self.entries.len(),
            epochs: self.epochs.len(),
            evicted_fragments: self.evicted,
            dropped_leading: self.dropped_leading,
            disk: self.ring.is_some(),
        }
    }

    /// Select the last `secs` seconds, starting at the keyframe at or before
    /// `end - secs`. Cheap: clones `Bytes` handles / disk offsets only.
    pub fn snapshot(&self, secs: f64) -> Option<Snapshot> {
        if self.entries.is_empty() {
            return None;
        }
        let mut acc = 0.0;
        let mut start = 0usize;
        let mut found = false;
        for (i, e) in self.entries.iter().enumerate().rev() {
            acc += e.duration;
            if acc >= secs - 1e-6 {
                // walk back to the GOP start
                let mut j = i;
                while j > 0 && !self.entries[j].keyframe {
                    j -= 1;
                }
                start = j;
                found = true;
                break;
            }
        }
        if !found {
            start = 0;
        }
        let mut parts: Vec<SnapshotPart> = Vec::new();
        let mut duration = 0.0;
        let mut bytes = 0u64;
        for e in self.entries.iter().skip(start) {
            if parts.last().map(|p| p.epoch) != Some(e.epoch) {
                let ep = self.epochs.iter().find(|x| x.id == e.epoch)?;
                parts.push(SnapshotPart { epoch: e.epoch, init: ep.init.clone(), chunks: Vec::new() });
            }
            let chunk = match &e.loc {
                Loc::Mem(b) => Chunk::Mem(b.clone()),
                Loc::Disk { off, len } => Chunk::Disk { off: *off, len: *len, seq: e.seq },
            };
            parts.last_mut().unwrap().chunks.push(chunk);
            duration += e.duration;
            bytes += e.bytes;
        }
        Some(Snapshot { parts, duration, bytes, ring: self.ring.as_ref().map(|(f, _)| f.clone()) })
    }

    /// Wall-clock age of the current epoch.
    pub fn epoch_age(&self) -> Option<std::time::Duration> {
        self.epochs.back().map(|e| e.started.elapsed())
    }
}

#[derive(Debug, Clone)]
pub enum Chunk {
    Mem(Bytes),
    Disk { off: u64, len: u32, seq: u64 },
}

#[derive(Debug, Clone)]
pub struct SnapshotPart {
    pub epoch: u64,
    pub init: Bytes,
    pub chunks: Vec<Chunk>,
}

/// An immutable selection of the buffer, exportable without holding the buffer lock.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub parts: Vec<SnapshotPart>,
    pub duration: f64,
    pub bytes: u64,
    ring: Option<Arc<RingFile>>,
}

impl Snapshot {
    /// Write one part (init + fragments) to `w`.
    pub fn write_part(&self, part: &SnapshotPart, w: &mut impl io::Write) -> io::Result<()> {
        w.write_all(&part.init)?;
        for c in &part.chunks {
            match c {
                Chunk::Mem(b) => w.write_all(b)?,
                Chunk::Disk { off, len, seq } => {
                    let ring = self.ring.as_ref().ok_or_else(|| io::Error::other("disk chunk without ring"))?;
                    w.write_all(&ring.read_record(*off, *seq, *len)?)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4::build::*;
    use crate::stream::{Fmp4Demuxer, Fmp4Event};

    /// Synthetic stream: 0.5 s fragments, keyframe every 4 fragments (2 s GOP).
    fn feed(buf: &mut ReplayBuffer, seconds: u32, first_key: bool) {
        let mut d = Fmp4Demuxer::new();
        let mut ev = Vec::new();
        let mut s = init_segment();
        for i in 0..seconds * 2 {
            let key = if first_key { i % 4 == 0 } else { i % 4 == 2 };
            s.extend(fragment(i + 1, i as u64 * 7680, 30, key));
        }
        d.push(&s, &mut ev).unwrap();
        for e in ev {
            match e {
                Fmp4Event::Init { data, info } => buf.begin_epoch(data, info),
                Fmp4Event::Fragment(f) => buf.push(f).unwrap(),
            }
        }
    }

    fn mem(max: f64) -> ReplayBuffer {
        ReplayBuffer::new(ReplayConfig { max_duration_secs: max, max_bytes: u64::MAX, storage: Storage::Memory }).unwrap()
    }

    #[test]
    fn keeps_requested_duration_gop_aligned() {
        let mut b = mem(10.0);
        feed(&mut b, 60, true);
        let st = b.stats();
        assert!(st.buffered_secs >= 10.0 && st.buffered_secs < 12.0, "{}", st.buffered_secs);
        assert!(b.entries.front().unwrap().keyframe);
        assert_eq!(st.epochs, 1);
        assert!(st.evicted_fragments > 0);
    }

    #[test]
    fn drops_leading_non_keyframes() {
        let mut b = mem(30.0);
        feed(&mut b, 4, false);
        assert_eq!(b.stats().dropped_leading, 2);
        assert!(b.entries.front().unwrap().keyframe);
    }

    #[test]
    fn snapshot_starts_on_keyframe_and_covers_request() {
        let mut b = mem(60.0);
        feed(&mut b, 40, true);
        let s = b.snapshot(5.0).unwrap();
        assert_eq!(s.parts.len(), 1);
        // 5 s requested, GOP = 2 s → starts at the keyframe 6 s before the end.
        assert!((s.duration - 6.0).abs() < 1e-6, "{}", s.duration);
        let all = b.snapshot(1000.0).unwrap();
        assert!((all.duration - 40.0).abs() < 1e-6);
    }

    #[test]
    fn byte_budget_is_enforced() {
        let mut b = ReplayBuffer::new(ReplayConfig { max_duration_secs: 1000.0, max_bytes: 40_000, storage: Storage::Memory }).unwrap();
        feed(&mut b, 60, true);
        assert!(b.stats().bytes <= 40_000);
        assert!(b.stats().buffered_secs > 0.0);
    }

    #[test]
    fn multiple_epochs_and_pruning() {
        let mut b = mem(12.0);
        feed(&mut b, 10, true);
        feed(&mut b, 10, true); // encoder restarted
        let s = b.snapshot(15.0).unwrap();
        assert_eq!(s.parts.len(), 2);
        feed(&mut b, 30, true);
        assert_eq!(b.stats().epochs, 1, "old epochs dropped once fully evicted");
    }

    #[test]
    fn disk_ring_matches_memory() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = mem(8.0);
        let mut d = ReplayBuffer::new(ReplayConfig {
            max_duration_secs: 8.0,
            max_bytes: u64::MAX,
            storage: Storage::Disk { path: dir.path().join("r.ring"), capacity: 80_000 },
        })
        .unwrap();
        feed(&mut m, 120, true);
        feed(&mut d, 120, true);
        assert_eq!(m.stats().buffered_secs, d.stats().buffered_secs);
        let (sm, sd) = (m.snapshot(6.0).unwrap(), d.snapshot(6.0).unwrap());
        let (mut a, mut z) = (Vec::new(), Vec::new());
        sm.write_part(&sm.parts[0], &mut a).unwrap();
        sd.write_part(&sd.parts[0], &mut z).unwrap();
        assert_eq!(a, z, "disk and RAM produce byte-identical clips");
    }

    #[test]
    fn small_disk_ring_still_works() {
        let dir = tempfile::tempdir().unwrap();
        // Ring too small for the requested duration: oldest GOPs are recycled.
        let mut d = ReplayBuffer::new(ReplayConfig {
            max_duration_secs: 300.0,
            max_bytes: u64::MAX,
            storage: Storage::Disk { path: dir.path().join("r.ring"), capacity: 30_000 },
        })
        .unwrap();
        feed(&mut d, 60, true);
        let st = d.stats();
        assert!(st.bytes <= 30_000 && st.buffered_secs >= 2.0, "{st:?}");
        let s = d.snapshot(300.0).unwrap();
        let mut out = Vec::new();
        s.write_part(&s.parts[0], &mut out).unwrap();
    }
}
