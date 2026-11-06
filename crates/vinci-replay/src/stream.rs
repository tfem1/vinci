//! Incremental fragmented-MP4 demuxer.
//!
//! Fed with arbitrary-sized chunks read from the encoder's stdout, it emits the init
//! segment (`ftyp`+`moov`) and complete fragments (`moof`+`mdat`) with their timing.
//! Uses `BytesMut::split_to` so fragments are handed over without copying.

use crate::mp4::{self, FragmentInfo, InitInfo, Mp4Error};
use bytes::{Bytes, BytesMut};
use std::sync::Arc;

/// Refuse absurd boxes (corrupt stream) instead of buffering gigabytes.
const MAX_BOX: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Fragment {
    /// `moof` + `mdat` bytes, ready to be appended after an init segment.
    pub data: Bytes,
    /// Starts with a video sync sample (keyframe) — clips can start here.
    pub keyframe: bool,
    /// Video media time of the first sample, seconds.
    pub start: f64,
    /// Video duration of the fragment, seconds.
    pub duration: f64,
    pub sequence: u32,
}

#[derive(Debug, Clone)]
pub enum Fmp4Event {
    Init { data: Bytes, info: Arc<InitInfo> },
    Fragment(Fragment),
}

#[derive(Default)]
pub struct Fmp4Demuxer {
    buf: BytesMut,
    ftyp: Option<Bytes>,
    init: Option<Arc<InitInfo>>,
    pending: Option<(Bytes, FragmentInfo)>,
}

impl Fmp4Demuxer {
    pub fn new() -> Self {
        Self { buf: BytesMut::with_capacity(1 << 20), ..Default::default() }
    }

    pub fn init_info(&self) -> Option<&Arc<InitInfo>> {
        self.init.as_ref()
    }

    /// Bytes waiting for the rest of their box.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    pub fn push(&mut self, data: &[u8], out: &mut Vec<Fmp4Event>) -> Result<(), Mp4Error> {
        self.buf.extend_from_slice(data);
        loop {
            let Some(h) = mp4::read_header(&self.buf)? else { return Ok(()) };
            if h.size > MAX_BOX {
                return Err(Mp4Error::BadSize(h.size));
            }
            if (self.buf.len() as u64) < h.size {
                // Make room for the whole box in one allocation.
                self.buf.reserve(h.size as usize - self.buf.len());
                return Ok(());
            }
            let whole = self.buf.split_to(h.size as usize).freeze();
            let payload = whole.slice(h.header_len..);
            match &h.kind {
                b"ftyp" => {
                    self.ftyp = Some(whole);
                    self.pending = None;
                }
                b"moov" => {
                    let info = Arc::new(mp4::parse_moov(&payload)?);
                    let mut init = Vec::with_capacity(whole.len() + 64);
                    if let Some(f) = &self.ftyp {
                        init.extend_from_slice(f);
                    }
                    init.extend_from_slice(&whole);
                    self.init = Some(info.clone());
                    out.push(Fmp4Event::Init { data: Bytes::from(init), info });
                }
                b"moof" => {
                    let init = self.init.as_ref().ok_or(Mp4Error::NoInit)?;
                    let fi = mp4::parse_moof(&payload, init)?;
                    self.pending = Some((whole, fi));
                }
                b"mdat" => {
                    if let Some((moof, fi)) = self.pending.take() {
                        let mut data = BytesMut::with_capacity(moof.len() + whole.len());
                        data.extend_from_slice(&moof);
                        data.extend_from_slice(&whole);
                        let (keyframe, start, duration) = match &fi.video {
                            Some(v) => (v.first_is_sync && v.sample_count > 0, v.start_secs(), v.duration_secs()),
                            None => (false, fi.audio.as_ref().map(|a| a.start_secs()).unwrap_or(0.0), 0.0),
                        };
                        out.push(Fmp4Event::Fragment(Fragment { data: data.freeze(), keyframe, start, duration, sequence: fi.sequence }));
                    }
                }
                // styp, sidx, prft, free, emsg… are not needed to rebuild a clip.
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4::build::*;

    #[test]
    fn byte_by_byte_and_bulk_give_same_result() {
        let mut stream = init_segment();
        for i in 0..8u32 {
            stream.extend(fragment(i + 1, i as u64 * 7680, 30, i % 4 == 0));
        }
        let mut a = Fmp4Demuxer::new();
        let mut ev_a = Vec::new();
        for b in &stream {
            a.push(std::slice::from_ref(b), &mut ev_a).unwrap();
        }
        let mut b = Fmp4Demuxer::new();
        let mut ev_b = Vec::new();
        b.push(&stream, &mut ev_b).unwrap();
        assert_eq!(ev_a.len(), 9);
        assert_eq!(ev_b.len(), 9);
        let keys: Vec<bool> = ev_a.iter().filter_map(|e| if let Fmp4Event::Fragment(f) = e { Some(f.keyframe) } else { None }).collect();
        assert_eq!(keys, vec![true, false, false, false, true, false, false, false]);
        let total: usize = ev_b
            .iter()
            .map(|e| match e {
                Fmp4Event::Init { data, .. } => data.len(),
                Fmp4Event::Fragment(f) => f.data.len(),
            })
            .sum();
        assert_eq!(total, stream.len(), "every byte accounted for");
        assert_eq!(a.buffered(), 0);
    }

    #[test]
    fn fragment_before_init_is_an_error() {
        let mut d = Fmp4Demuxer::new();
        assert_eq!(d.push(&fragment(1, 0, 1, true), &mut Vec::new()), Err(Mp4Error::NoInit));
    }
}
