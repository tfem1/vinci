//! Real encoder validation.
//!
//! An encoder being *compiled into* FFmpeg says nothing about the machine: NVENC needs an
//! NVIDIA driver, AV1 NVENC needs an RTX 40, VA-API needs a working Mesa/iHD driver…
//! Each candidate is therefore validated with a short real encode (15 frames, 1280×720)
//! and only encoders that succeed are offered to the user.

use crate::args::{plan_video, FilterCaps, FrameSource, HwContext, RateControl};
use crate::catalog::{available_in, Codec, EncoderFamily, EncoderInfo};
use crate::profile::{QualityProfile, VideoParams};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant};
use vinci_hw::process::command;
use vinci_hw::FfmpegInfo;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbeResult {
    pub id: String,
    pub codec: Codec,
    pub family: EncoderFamily,
    pub ok: bool,
    pub error: Option<String>,
    pub millis: u64,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Build the probe command line (exposed for tests and diagnostics).
pub fn probe_args(enc: &EncoderInfo, caps: FilterCaps, hw: &HwContext) -> Vec<String> {
    let params = VideoParams { codec: enc.codec, profile: QualityProfile::Balanced, width: 1280, height: 720, fps: 30, keyint_secs: 1.0 };
    let plan = plan_video(enc, &params, RateControl::Quality, FrameSource::SystemYuv, None, caps, hw);
    let mut a: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-loglevel".into(), "error".into()];
    a.extend(plan.global_args);
    a.extend(["-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30,format=yuv420p"].map(String::from));
    a.extend(["-frames:v", "15"].map(String::from));
    if let Some(f) = plan.filter {
        a.push("-vf".into());
        a.push(f);
    }
    a.extend(plan.encoder_args);
    a.extend(["-f", "null", "-"].map(String::from));
    a
}

pub fn probe_encoder(ffmpeg: &FfmpegInfo, enc: &EncoderInfo, hw: &HwContext) -> ProbeResult {
    let start = Instant::now();
    let args = probe_args(enc, FilterCaps::from_ffmpeg(ffmpeg), hw);
    let result = run_with_timeout(ffmpeg, &args, PROBE_TIMEOUT);
    let (ok, error) = match result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e)),
    };
    log::info!("encoder probe {}: {} ({} ms)", enc.id, if ok { "OK" } else { "unavailable" }, start.elapsed().as_millis());
    ProbeResult { id: enc.id.to_string(), codec: enc.codec, family: enc.family, ok, error, millis: start.elapsed().as_millis() as u64 }
}

fn run_with_timeout(ffmpeg: &FfmpegInfo, args: &[String], timeout: Duration) -> Result<(), String> {
    let mut child = command(&ffmpeg.path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let err = reader.join().unwrap_or_default();
    match status {
        Some(st) if st.success() => Ok(()),
        Some(_) => Err(summarize_error(&err)),
        None => Err("timed out".into()),
    }
}

/// Keep the most meaningful line of an FFmpeg error dump.
pub fn summarize_error(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let interesting = lines
        .iter()
        .find(|l| {
            let l = l.to_lowercase();
            l.contains("cannot load")
                || l.contains("no capable devices")
                || l.contains("driver")
                || l.contains("not supported")
                || l.contains("failed")
        })
        .or(lines.first());
    interesting.map(|s| s.chars().take(240).collect()).unwrap_or_else(|| "encoder failed".into())
}

/// Probe every relevant encoder in parallel (bounded to 4 concurrent FFmpeg processes).
pub fn probe_all(ffmpeg: &FfmpegInfo, hw: &HwContext) -> Vec<ProbeResult> {
    let encoders = available_in(ffmpeg);
    let mut results = Vec::with_capacity(encoders.len());
    for chunk in encoders.chunks(4) {
        let handles: Vec<_> = chunk
            .iter()
            .map(|enc| {
                let ff = ffmpeg.clone();
                let hw = hw.clone();
                let enc = **enc;
                std::thread::spawn(move || probe_encoder(&ff, &enc, &hw))
            })
            .collect();
        for h in handles {
            if let Ok(r) = h.join() {
                results.push(r);
            }
        }
    }
    results
}

/// Pick the best working encoder.
///
/// * `preferred_id` — explicit user choice; honoured when it works.
/// * otherwise hardware encoders are tried first, following the codec fallback chain
///   (AV1 → HEVC → H.264);
/// * CPU fallback always prefers **x264**: real-time x265/SVT-AV1 at 1080p60 costs
///   several cores, which would hurt the game far more than the bitrate savings help.
pub fn select(
    probes: &[ProbeResult],
    codec: Codec,
    preferred_id: Option<&str>,
    allow_software: bool,
) -> Option<(&'static EncoderInfo, bool)> {
    let ok = |id: &str| probes.iter().any(|p| p.id == id && p.ok);
    if let Some(id) = preferred_id {
        if ok(id) {
            if let Some(e) = crate::catalog::find(id) {
                return Some((e, false));
            }
        }
    }
    let explicit_miss = preferred_id.is_some();
    for (i, c) in codec.fallback_chain().iter().enumerate() {
        let mut cands: Vec<&'static EncoderInfo> = crate::catalog::for_codec(*c).filter(|e| e.family.is_hardware() && ok(e.id)).collect();
        cands.sort_by_key(|e| e.family.rank());
        if let Some(e) = cands.first() {
            return Some((e, i > 0 || explicit_miss));
        }
    }
    if allow_software {
        for id in ["libx264", "libx265", "libsvtav1"] {
            if ok(id) {
                let e = crate::catalog::find(id)?;
                return Some((e, e.codec != codec || explicit_miss));
            }
        }
    }
    None
}

/// Offline jobs (compression) can afford slower CPU encoders: same order as
/// [`select`] but keeps the requested codec in software too.
pub fn select_offline(probes: &[ProbeResult], codec: Codec) -> Option<&'static EncoderInfo> {
    let ok = |id: &str| probes.iter().any(|p| p.id == id && p.ok);
    for c in codec.fallback_chain() {
        let mut cands: Vec<&'static EncoderInfo> = crate::catalog::for_codec(*c).filter(|e| ok(e.id)).collect();
        cands.sort_by_key(|e| e.family.rank());
        if let Some(e) = cands.first() {
            return Some(e);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(id: &str, ok: bool) -> ProbeResult {
        let e = crate::catalog::find(id).unwrap();
        ProbeResult { id: id.into(), codec: e.codec, family: e.family, ok, error: None, millis: 1 }
    }

    #[test]
    fn selection_prefers_hardware_and_falls_back() {
        let probes = vec![pr("libx264", true), pr("libx265", true), pr("hevc_nvenc", true), pr("av1_nvenc", false), pr("h264_nvenc", true)];
        // AV1 requested, NVENC AV1 not available (pre-RTX40) → HEVC NVENC, flagged as fallback.
        let (e, fb) = select(&probes, Codec::Av1, None, true).unwrap();
        assert_eq!((e.id, fb), ("hevc_nvenc", true));
        // Explicit user choice wins.
        assert_eq!(select(&probes, Codec::Hevc, Some("libx265"), true).unwrap().0.id, "libx265");
        // Broken explicit choice falls back.
        assert_eq!(select(&probes, Codec::Hevc, Some("hevc_amf"), true).unwrap().0.id, "hevc_nvenc");
        // CPU only machine: real-time capture uses x264 even when HEVC/AV1 work in software.
        let cpu = vec![pr("libx264", true), pr("libx265", true), pr("libsvtav1", true)];
        assert_eq!(select(&cpu, Codec::Hevc, None, true).unwrap().0.id, "libx264");
        assert_eq!(select(&cpu, Codec::Av1, None, true).unwrap().0.id, "libx264");
        assert!(select(&cpu, Codec::Av1, None, false).is_none());
        // ...but an explicit choice is respected, and offline jobs keep the codec.
        assert_eq!(select(&cpu, Codec::Hevc, Some("libx265"), true).unwrap().0.id, "libx265");
        assert_eq!(select_offline(&cpu, Codec::Hevc).unwrap().id, "libx265");
        // Hardware of a fallback codec beats software of the requested codec.
        let mixed = vec![pr("libsvtav1", true), pr("h264_nvenc", true), pr("libx264", true)];
        assert_eq!(select(&mixed, Codec::Av1, None, true).unwrap().0.id, "h264_nvenc");
    }

    #[test]
    fn probe_args_shape() {
        let enc = crate::catalog::find("h264_vaapi").unwrap();
        let a = probe_args(enc, FilterCaps { scale_vaapi: true, ..Default::default() }, &HwContext::default()).join(" ");
        assert!(a.contains("-vaapi_device /dev/dri/renderD128 -f lavfi"), "{a}");
        assert!(a.ends_with("-f null -"));
    }

    #[test]
    fn real_probe_software_encoders() {
        let Ok(ff) = vinci_hw::ffmpeg::locate(None) else { return };
        if !ff.has_encoder("libx264") {
            return;
        }
        let r = probe_encoder(&ff, crate::catalog::find("libx264").unwrap(), &HwContext::default());
        assert!(r.ok, "libx264 probe failed: {:?}", r.error);
        // A hardware encoder on a machine without that GPU must fail cleanly, not hang.
        if ff.has_encoder("h264_nvenc") && !std::path::Path::new("/dev/nvidia0").exists() {
            let r = probe_encoder(&ff, crate::catalog::find("h264_nvenc").unwrap(), &HwContext::default());
            assert!(!r.ok);
            assert!(r.error.is_some());
        }
    }
}
