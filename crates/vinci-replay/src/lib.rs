//! # vinci-replay — instant replay engine
//!
//! The capture pipeline encodes on the GPU and streams **fragmented MP4** to VINCI.
//! This crate keeps the last N seconds of that stream in a GOP-aligned ring buffer
//! (RAM or a fixed-size disk file) and turns any window of it into a clip **without
//! re-encoding** — saving a 5-minute clip costs a file copy, not a transcode.
//!
//! * [`mp4`] — minimal ISO-BMFF parser (init segment, `moof` timing, keyframe flags).
//! * [`stream`] — incremental demuxer over the encoder's stdout.
//! * [`buffer`] — the replay ring buffer and zero-copy snapshots.
//! * [`ring`] — the fixed-size circular file used in disk mode.
//! * [`export`] — atomic clip writing + `-c copy` faststart remux.

pub mod buffer;
pub mod export;
pub mod mp4;
pub mod ring;
pub mod stream;

pub use buffer::{BufferStats, ReplayBuffer, ReplayConfig, Snapshot, Storage};
pub use export::{export, ExportError, ExportResult};
pub use stream::{Fmp4Demuxer, Fmp4Event, Fragment};

/// FFmpeg muxer arguments that make an encoder emit the stream this crate expects:
/// fragments cut at every keyframe *and* at least every `frag_ms` milliseconds, so a
/// save request never waits more than `frag_ms` for the latest footage.
pub fn fmp4_output_args(frag_ms: u32) -> Vec<String> {
    vec![
        "-f".into(),
        "mp4".into(),
        "-movflags".into(),
        "+frag_keyframe+empty_moov+default_base_moof".into(),
        "-frag_duration".into(),
        (frag_ms as u64 * 1000).to_string(),
        "-flush_packets".into(),
        "1".into(),
        "pipe:1".into(),
    ]
}
