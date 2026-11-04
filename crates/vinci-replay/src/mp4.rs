//! Minimal ISO-BMFF (MP4) parsing for fragmented streams.
//!
//! Only what the replay buffer needs: track ids / timescales / handler types and
//! `trex` defaults from the init segment, and per-fragment decode time, duration and
//! "starts with a sync sample" from each `moof`.

use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum Mp4Error {
    #[error("truncated box `{0}`")]
    Truncated(&'static str),
    #[error("invalid box size {0}")]
    BadSize(u64),
    #[error("fragment received before init segment")]
    NoInit,
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxHeader {
    pub kind: [u8; 4],
    /// Total size including the header.
    pub size: u64,
    pub header_len: usize,
}

/// Parse a box header from the start of `buf`. Returns `Ok(None)` if more bytes are needed.
pub fn read_header(buf: &[u8]) -> Result<Option<BoxHeader>, Mp4Error> {
    if buf.len() < 8 {
        return Ok(None);
    }
    let size32 = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as u64;
    let kind: [u8; 4] = buf[4..8].try_into().unwrap();
    let (size, header_len) = match size32 {
        0 => return Err(Mp4Error::Unsupported("box extending to end of stream")),
        1 => {
            if buf.len() < 16 {
                return Ok(None);
            }
            (u64::from_be_bytes(buf[8..16].try_into().unwrap()), 16)
        }
        s => (s, 8),
    };
    if size < header_len as u64 {
        return Err(Mp4Error::BadSize(size));
    }
    Ok(Some(BoxHeader { kind, size, header_len }))
}

/// Iterate over child boxes of a container payload: yields `(kind, payload)`.
pub fn children(payload: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        let rest = payload.get(pos..)?;
        let h = read_header(rest).ok()??;
        let end = usize::try_from(h.size).ok()?;
        if end > rest.len() {
            return None;
        }
        pos += end;
        Some((h.kind, &rest[h.header_len..end]))
    })
}

fn find<'a>(payload: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(payload).find(|(k, _)| k == kind).map(|(_, p)| p)
}

fn be32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4).map(|s| u32::from_be_bytes(s.try_into().unwrap()))
}
fn be64(b: &[u8], off: usize) -> Option<u64> {
    b.get(off..off + 8).map(|s| u64::from_be_bytes(s.try_into().unwrap()))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackInfo {
    pub id: u32,
    pub timescale: u32,
    pub is_video: bool,
    pub is_audio: bool,
    pub default_sample_duration: u32,
    pub default_sample_flags: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitInfo {
    pub tracks: Vec<TrackInfo>,
}

impl InitInfo {
    pub fn video(&self) -> Option<&TrackInfo> {
        self.tracks.iter().find(|t| t.is_video)
    }
    pub fn track(&self, id: u32) -> Option<&TrackInfo> {
        self.tracks.iter().find(|t| t.id == id)
    }
}

/// Parse the payload of a `moov` box.
pub fn parse_moov(moov: &[u8]) -> Result<InitInfo, Mp4Error> {
    let mut info = InitInfo::default();
    for (kind, trak) in children(moov) {
        if &kind != b"trak" {
            continue;
        }
        let tkhd = find(trak, b"tkhd").ok_or(Mp4Error::Truncated("tkhd"))?;
        let id = match tkhd.first() {
            Some(1) => be32(tkhd, 20),
            _ => be32(tkhd, 12),
        }
        .ok_or(Mp4Error::Truncated("tkhd"))?;
        let mdia = find(trak, b"mdia").ok_or(Mp4Error::Truncated("mdia"))?;
        let mdhd = find(mdia, b"mdhd").ok_or(Mp4Error::Truncated("mdhd"))?;
        let timescale = match mdhd.first() {
            Some(1) => be32(mdhd, 20),
            _ => be32(mdhd, 12),
        }
        .ok_or(Mp4Error::Truncated("mdhd"))?;
        let handler = find(mdia, b"hdlr").and_then(|h| h.get(8..12)).unwrap_or(b"    ");
        info.tracks.push(TrackInfo { id, timescale: timescale.max(1), is_video: handler == b"vide", is_audio: handler == b"soun", ..Default::default() });
    }
    if let Some(mvex) = find(moov, b"mvex") {
        for (kind, trex) in children(mvex) {
            if &kind != b"trex" {
                continue;
            }
            let (Some(id), Some(dur), Some(flags)) = (be32(trex, 4), be32(trex, 12), be32(trex, 20)) else { continue };
            if let Some(t) = info.tracks.iter_mut().find(|t| t.id == id) {
                t.default_sample_duration = dur;
                t.default_sample_flags = flags;
            }
        }
    }
    if info.tracks.is_empty() {
        return Err(Mp4Error::Truncated("trak"));
    }
    Ok(info)
}

/// `sample_is_non_sync_sample` bit of ISO-BMFF sample flags.
pub const NON_SYNC: u32 = 0x0001_0000;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackTiming {
    pub track_id: u32,
    pub base_decode_time: u64,
    pub duration_ticks: u64,
    pub timescale: u32,
    pub sample_count: u32,
    pub first_is_sync: bool,
}

impl TrackTiming {
    pub fn start_secs(&self) -> f64 {
        self.base_decode_time as f64 / self.timescale as f64
    }
    pub fn duration_secs(&self) -> f64 {
        self.duration_ticks as f64 / self.timescale as f64
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FragmentInfo {
    pub sequence: u32,
    pub video: Option<TrackTiming>,
    pub audio: Option<TrackTiming>,
}

/// Parse the payload of a `moof` box.
pub fn parse_moof(moof: &[u8], init: &InitInfo) -> Result<FragmentInfo, Mp4Error> {
    let mut out = FragmentInfo { sequence: find(moof, b"mfhd").and_then(|m| be32(m, 4)).unwrap_or(0), ..Default::default() };
    for (kind, traf) in children(moof) {
        if &kind != b"traf" {
            continue;
        }
        let tfhd = find(traf, b"tfhd").ok_or(Mp4Error::Truncated("tfhd"))?;
        let tf_flags = be32(tfhd, 0).ok_or(Mp4Error::Truncated("tfhd"))? & 0x00ff_ffff;
        let track_id = be32(tfhd, 4).ok_or(Mp4Error::Truncated("tfhd"))?;
        let Some(track) = init.track(track_id) else { continue };
        let mut off = 8;
        if tf_flags & 0x01 != 0 {
            off += 8;
        }
        if tf_flags & 0x02 != 0 {
            off += 4;
        }
        let mut default_dur = track.default_sample_duration;
        let mut default_flags = track.default_sample_flags;
        if tf_flags & 0x08 != 0 {
            default_dur = be32(tfhd, off).ok_or(Mp4Error::Truncated("tfhd"))?;
            off += 4;
        }
        if tf_flags & 0x10 != 0 {
            off += 4;
        }
        if tf_flags & 0x20 != 0 {
            default_flags = be32(tfhd, off).ok_or(Mp4Error::Truncated("tfhd"))?;
        }
        let base = match find(traf, b"tfdt") {
            Some(t) if t.first() == Some(&1) => be64(t, 4).ok_or(Mp4Error::Truncated("tfdt"))?,
            Some(t) => be32(t, 4).ok_or(Mp4Error::Truncated("tfdt"))? as u64,
            None => 0,
        };
        let mut timing = TrackTiming { track_id, base_decode_time: base, timescale: track.timescale, ..Default::default() };
        let mut first_trun = true;
        for (k, trun) in children(traf) {
            if &k != b"trun" {
                continue;
            }
            let flags = be32(trun, 0).ok_or(Mp4Error::Truncated("trun"))? & 0x00ff_ffff;
            let count = be32(trun, 4).ok_or(Mp4Error::Truncated("trun"))?;
            let mut p = 8;
            if flags & 0x01 != 0 {
                p += 4;
            }
            let mut first_flags = None;
            if flags & 0x04 != 0 {
                first_flags = Some(be32(trun, p).ok_or(Mp4Error::Truncated("trun"))?);
                p += 4;
            }
            let per = [0x100, 0x200, 0x400, 0x800].iter().filter(|b| flags & **b != 0).count() * 4;
            if count as usize * per > trun.len().saturating_sub(p) {
                return Err(Mp4Error::Truncated("trun samples"));
            }
            for i in 0..count as usize {
                let mut q = p + i * per;
                let dur = if flags & 0x100 != 0 {
                    let d = be32(trun, q).unwrap();
                    q += 4;
                    d
                } else {
                    default_dur
                };
                if flags & 0x200 != 0 {
                    q += 4;
                }
                let sflags = if flags & 0x400 != 0 { be32(trun, q).unwrap() } else { default_flags };
                if first_trun && i == 0 {
                    let f = first_flags.unwrap_or(sflags);
                    timing.first_is_sync = f & NON_SYNC == 0;
                }
                timing.duration_ticks += dur as u64;
            }
            timing.sample_count += count;
            first_trun = false;
        }
        if track.is_video {
            out.video = Some(timing);
        } else if track.is_audio {
            out.audio = Some(timing);
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod build {
    //! Tiny MP4 box writer used by tests to forge streams.
    pub fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }
    pub fn full(version: u8, flags: u32, rest: &[u8]) -> Vec<u8> {
        let mut v = vec![version];
        v.extend_from_slice(&flags.to_be_bytes()[1..]);
        v.extend_from_slice(rest);
        v
    }
    pub fn trak(id: u32, timescale: u32, handler: &[u8; 4]) -> Vec<u8> {
        let mut tkhd = vec![0u8; 8];
        tkhd.extend_from_slice(&id.to_be_bytes());
        tkhd.extend_from_slice(&[0u8; 68]);
        let mut mdhd = vec![0u8; 8];
        mdhd.extend_from_slice(&timescale.to_be_bytes());
        mdhd.extend_from_slice(&[0u8; 8]);
        let mut hdlr = vec![0u8; 4];
        hdlr.extend_from_slice(handler);
        hdlr.extend_from_slice(&[0u8; 13]);
        let mdia = [bx(b"mdhd", &full(0, 0, &mdhd)), bx(b"hdlr", &full(0, 0, &hdlr))].concat();
        [bx(b"tkhd", &full(0, 3, &tkhd)), bx(b"mdia", &mdia)].concat()
    }
    /// Init segment: video track 1 (timescale 15360), audio track 2 (48000).
    pub fn init_segment() -> Vec<u8> {
        let trex = |id: u32, dur: u32, flags: u32| {
            let mut p = id.to_be_bytes().to_vec();
            p.extend_from_slice(&1u32.to_be_bytes());
            p.extend_from_slice(&dur.to_be_bytes());
            p.extend_from_slice(&0u32.to_be_bytes());
            p.extend_from_slice(&flags.to_be_bytes());
            bx(b"trex", &full(0, 0, &p))
        };
        let mvex = bx(b"mvex", &[trex(1, 256, super::NON_SYNC), trex(2, 1024, 0)].concat());
        let moov = bx(b"moov", &[bx(b"trak", &trak(1, 15360, b"vide")), bx(b"trak", &trak(2, 48000, b"soun")), mvex].concat());
        [bx(b"ftyp", b"isom\0\0\x02\0isomiso6"), moov].concat()
    }
    /// One fragment (moof+mdat): `video_samples` × 256 ticks, first sample sync if `key`.
    pub fn fragment(seq: u32, base_video: u64, video_samples: u32, key: bool) -> Vec<u8> {
        let mfhd = bx(b"mfhd", &full(0, 0, &seq.to_be_bytes()));
        // video traf: tfhd default-base-is-moof, tfdt v1, trun with first-sample-flags.
        let tfhd_v = bx(b"tfhd", &full(0, 0x020000, &1u32.to_be_bytes()));
        let tfdt_v = bx(b"tfdt", &full(1, 0, &base_video.to_be_bytes()));
        let mut trun_p = video_samples.to_be_bytes().to_vec();
        trun_p.extend_from_slice(&0u32.to_be_bytes()); // data offset
        trun_p.extend_from_slice(&(if key { 0x0200_0000u32 } else { super::NON_SYNC }).to_be_bytes());
        for _ in 0..video_samples {
            trun_p.extend_from_slice(&100u32.to_be_bytes()); // sample size
        }
        let trun_v = bx(b"trun", &full(0, 0x01 | 0x04 | 0x200, &trun_p));
        let traf_v = bx(b"traf", &[tfhd_v, tfdt_v, trun_v].concat());
        // audio traf with per-sample durations.
        let tfhd_a = bx(b"tfhd", &full(0, 0x020000, &2u32.to_be_bytes()));
        let tfdt_a = bx(b"tfdt", &full(0, 0, &((base_video * 48000 / 15360) as u32).to_be_bytes()));
        let mut ta = 2u32.to_be_bytes().to_vec();
        ta.extend_from_slice(&0u32.to_be_bytes());
        for _ in 0..2 {
            ta.extend_from_slice(&1024u32.to_be_bytes());
            ta.extend_from_slice(&10u32.to_be_bytes());
        }
        let trun_a = bx(b"trun", &full(0, 0x01 | 0x100 | 0x200, &ta));
        let traf_a = bx(b"traf", &[tfhd_a, tfdt_a, trun_a].concat());
        let moof = bx(b"moof", &[mfhd, traf_v, traf_a].concat());
        let mdat = bx(b"mdat", &vec![0xAB; (video_samples * 100 + 20) as usize]);
        [moof, mdat].concat()
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn init_and_fragment() {
        let init = init_segment();
        let ftyp = read_header(&init).unwrap().unwrap();
        let moov_bytes = &init[ftyp.size as usize..];
        let h = read_header(moov_bytes).unwrap().unwrap();
        assert_eq!(&h.kind, b"moov");
        let info = parse_moov(&moov_bytes[h.header_len..h.size as usize]).unwrap();
        assert_eq!(info.tracks.len(), 2);
        let v = info.video().unwrap();
        assert_eq!((v.id, v.timescale, v.default_sample_duration), (1, 15360, 256));

        let frag = fragment(7, 15360 * 3, 30, true);
        let mh = read_header(&frag).unwrap().unwrap();
        let fi = parse_moof(&frag[mh.header_len..mh.size as usize], &info).unwrap();
        assert_eq!(fi.sequence, 7);
        let vt = fi.video.unwrap();
        assert!(vt.first_is_sync);
        assert_eq!(vt.sample_count, 30);
        assert!((vt.start_secs() - 3.0).abs() < 1e-9);
        assert!((vt.duration_secs() - 0.5).abs() < 1e-9);
        let at = fi.audio.unwrap();
        assert_eq!(at.duration_ticks, 2048);

        let frag2 = fragment(8, 0, 30, false);
        let mh = read_header(&frag2).unwrap().unwrap();
        let fi2 = parse_moof(&frag2[mh.header_len..mh.size as usize], &info).unwrap();
        assert!(!fi2.video.unwrap().first_is_sync);
    }

    #[test]
    fn headers() {
        assert_eq!(read_header(&[0, 0, 0]).unwrap(), None);
        assert!(read_header(&[0, 0, 0, 0, b'm', b'd', b'a', b't']).is_err());
        assert!(read_header(&[0, 0, 0, 4, b'm', b'd', b'a', b't']).is_err());
        let mut large = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        assert_eq!(read_header(&large).unwrap(), None);
        large.extend_from_slice(&20u64.to_be_bytes());
        assert_eq!(read_header(&large).unwrap().unwrap().size, 20);
    }
}
