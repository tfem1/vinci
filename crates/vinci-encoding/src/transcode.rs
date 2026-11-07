//! Post-capture processing: compression (quality profile or target size), trimming and
//! media inspection. Every job writes to a temporary file next to the destination and
//! is renamed atomically only after FFmpeg succeeds, so a crash never leaves a
//! half-written clip behind.

use crate::args::{audio_args, plan_video, FilterCaps, FrameSource, HwContext, RateControl};
use crate::catalog::{Codec, EncoderFamily, EncoderInfo};
use crate::profile::{audio_kbps, QualityProfile, VideoParams};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use vinci_hw::process::command;
use vinci_hw::FfmpegInfo;

#[derive(Debug, thiserror::Error)]
pub enum TranscodeError {
    #[error("ffprobe is not available")]
    NoFfprobe,
    #[error("could not read media information: {0}")]
    Probe(String),
    #[error("FFmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("cancelled")]
    Cancelled,
    #[error("target size is too small for this clip ({0})")]
    TargetTooSmall(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MediaInfo {
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub size_bytes: u64,
    pub bitrate_kbps: u32,
}

/// Read stream information with ffprobe (JSON output).
pub fn probe_media(ffprobe: &Path, file: &Path) -> Result<MediaInfo, TranscodeError> {
    let out = command(ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| TranscodeError::Probe(e.to_string()))?;
    if !out.status.success() {
        return Err(TranscodeError::Probe(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    parse_ffprobe_json(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| TranscodeError::Probe("unexpected ffprobe output".into()))
}

pub fn parse_ffprobe_json(json: &str) -> Option<MediaInfo> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let streams = v.get("streams")?.as_array()?;
    let video = streams.iter().find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("video"))?;
    let audio = streams.iter().find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("audio"));
    let fmt = v.get("format");
    let num = |val: Option<&serde_json::Value>| -> Option<f64> {
        val.and_then(|x| x.as_str().and_then(|s| s.parse().ok()).or_else(|| x.as_f64()))
    };
    let fps = video
        .get("avg_frame_rate")
        .and_then(|r| r.as_str())
        .and_then(parse_ratio)
        .filter(|f| *f > 0.0)
        .or_else(|| video.get("r_frame_rate").and_then(|r| r.as_str()).and_then(parse_ratio))
        .unwrap_or(0.0);
    let duration = num(fmt.and_then(|f| f.get("duration"))).or_else(|| num(video.get("duration"))).unwrap_or(0.0);
    Some(MediaInfo {
        duration_secs: duration,
        width: video.get("width")?.as_u64()? as u32,
        height: video.get("height")?.as_u64()? as u32,
        fps,
        video_codec: video.get("codec_name").and_then(|c| c.as_str()).unwrap_or("").to_string(),
        audio_codec: audio.and_then(|a| a.get("codec_name")).and_then(|c| c.as_str()).map(String::from),
        size_bytes: num(fmt.and_then(|f| f.get("size"))).unwrap_or(0.0) as u64,
        bitrate_kbps: (num(fmt.and_then(|f| f.get("bit_rate"))).unwrap_or(0.0) / 1000.0) as u32,
    })
}

fn parse_ratio(r: &str) -> Option<f64> {
    let (a, b) = r.split_once('/')?;
    let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
    if b == 0.0 {
        None
    } else {
        Some(a / b)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CompressTarget {
    /// Re-encode with a quality profile (typically `Compact` + HEVC/AV1).
    Profile(QualityProfile),
    /// Fit the clip in N megabytes (10⁶ bytes) — e.g. 10 for Discord.
    SizeMb(f64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompressOptions {
    pub codec: Codec,
    pub target: CompressTarget,
    /// Downscale to at most this height (keeps aspect ratio).
    pub max_height: Option<u32>,
    pub max_fps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressResult {
    pub output: PathBuf,
    pub size_bytes: u64,
    pub attempts: u32,
    pub encoder: String,
}

fn even(v: f64) -> u32 {
    ((v / 2.0).round() as u32 * 2).max(2)
}

/// Output geometry for a given input and height limit.
pub fn fit_size(w: u32, h: u32, max_h: Option<u32>) -> (u32, u32) {
    match max_h {
        Some(mh) if h > mh => (even(w as f64 * mh as f64 / h as f64), even(mh as f64)),
        _ => (even(w as f64), even(h as f64)),
    }
}

/// Bitrate budget (kbit/s for video) for a target size; `None` if impossible.
pub fn size_budget_kbps(size_mb: f64, duration_secs: f64, audio_kbps: u32) -> Option<u32> {
    if duration_secs <= 0.0 {
        return None;
    }
    // 4 % container overhead + safety margin.
    let total_kbps = size_mb * 1_000_000.0 * 8.0 * 0.96 / duration_secs / 1000.0;
    let video = total_kbps - audio_kbps as f64;
    if video < 120.0 {
        None
    } else {
        Some(video as u32)
    }
}

/// Pick a sensible max height for a bitrate budget so tiny targets stay watchable.
pub fn auto_height_for_budget(kbps: u32, src_h: u32) -> u32 {
    let h = match kbps {
        0..=600 => 480,
        601..=1500 => 720,
        1501..=4000 => 1080,
        4001..=9000 => 1440,
        _ => 2160,
    };
    h.min(src_h)
}

pub struct Job<'a> {
    pub ffmpeg: &'a FfmpegInfo,
    pub encoder: &'a EncoderInfo,
    pub hw: &'a HwContext,
    pub cancel: &'a AtomicBool,
}

/// Compress `input` into `output`. `progress` receives 0.0–1.0.
pub fn compress(
    job: &Job,
    input: &Path,
    output: &Path,
    opts: &CompressOptions,
    mut progress: impl FnMut(f32),
) -> Result<CompressResult, TranscodeError> {
    let ffprobe = job.ffmpeg.ffprobe.as_deref().ok_or(TranscodeError::NoFfprobe)?;
    let info = probe_media(ffprobe, input)?;
    let profile = match opts.target {
        CompressTarget::Profile(p) => p,
        CompressTarget::SizeMb(_) => QualityProfile::Compact,
    };
    let a_kbps = match opts.target {
        CompressTarget::SizeMb(mb) if mb < 20.0 => 96,
        _ => audio_kbps(profile).min(160),
    };
    let fps = match opts.max_fps {
        Some(m) if info.fps > m as f64 + 0.5 => m,
        _ => info.fps.round().max(1.0) as u32,
    };
    let tmp = temp_path(output);
    let mut attempts = 0;
    let mut factor = 1.0f64;
    loop {
        attempts += 1;
        let (rc, max_h) = match opts.target {
            CompressTarget::Profile(_) => (RateControl::Quality, opts.max_height),
            CompressTarget::SizeMb(mb) => {
                let budget = size_budget_kbps(mb, info.duration_secs, a_kbps)
                    .ok_or_else(|| TranscodeError::TargetTooSmall(format!("{mb} MB for {:.0} s", info.duration_secs)))?;
                let budget = (budget as f64 * factor) as u32;
                let auto = auto_height_for_budget(budget, info.height);
                (RateControl::Bitrate(budget.max(120)), Some(opts.max_height.map_or(auto, |m| m.min(auto))))
            }
        };
        let (w, h) = fit_size(info.width, info.height, max_h);
        let scale = if (w, h) != (info.width, info.height) { Some((w, h)) } else { None };
        let params = VideoParams { codec: job.encoder.codec, profile, width: w, height: h, fps, keyint_secs: 4.0 };
        let mut plan = plan_video(job.encoder, &params, rc, FrameSource::SystemYuv, scale, FilterCaps::from_ffmpeg(job.ffmpeg), job.hw);
        // Offline encodes can afford slower, more efficient presets.
        if job.encoder.family == EncoderFamily::Software {
            slow_down_software_preset(&mut plan.encoder_args, job.encoder.codec);
        }
        offline_quality(&mut plan.encoder_args, job.encoder.family);
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-y".into(), "-loglevel".into(), "error".into()];
        args.extend(plan.global_args);
        args.push("-i".into());
        args.push(input.display().to_string());
        let mut vf = Vec::new();
        if opts.max_fps.is_some() && fps as f64 + 0.5 < info.fps {
            vf.push(format!("fps={fps}"));
        }
        if let Some(f) = plan.filter {
            vf.push(f);
        }
        if !vf.is_empty() {
            args.push("-vf".into());
            args.push(vf.join(","));
        }
        args.extend(["-map", "0:v:0", "-map", "0:a?"].map(String::from));
        args.extend(plan.encoder_args);
        args.extend(audio_args(a_kbps));
        args.extend(["-movflags", "+faststart", "-f", "mp4", "-progress", "pipe:1", "-nostats"].map(String::from));
        args.push(tmp.display().to_string());

        let res = run_ffmpeg_progress(job.ffmpeg, &args, info.duration_secs, job.cancel, |p| progress(p.min(1.0)));
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        let size = std::fs::metadata(&tmp)?.len();
        if let CompressTarget::SizeMb(mb) = opts.target {
            let limit = (mb * 1_000_000.0) as u64;
            if size > limit && attempts < 3 {
                factor *= (limit as f64 / size as f64) * 0.92;
                log::info!("compress attempt {attempts}: {size} B > {limit} B, retrying with factor {factor:.2}");
                continue;
            }
        }
        std::fs::rename(&tmp, output)?;
        progress(1.0);
        return Ok(CompressResult { output: output.to_path_buf(), size_bytes: size, attempts, encoder: job.encoder.id.to_string() });
    }
}

/// NVENC: the live settings leave quality features off to spare the game's GPU;
/// compressing a clip is offline, so use a slower preset and adaptive quantization.
pub fn offline_quality(args: &mut Vec<String>, family: EncoderFamily) {
    if family != EncoderFamily::Nvenc {
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "-preset") {
        if let Some(v) = args.get_mut(i + 1) {
            *v = "p5".into();
        }
    }
    args.extend(["-spatial-aq".to_string(), "1".to_string()]);
}

fn slow_down_software_preset(args: &mut [String], codec: Codec) {
    if let Some(i) = args.iter().position(|a| a == "-preset") {
        if let Some(v) = args.get_mut(i + 1) {
            *v = match codec {
                Codec::H264 => "medium".into(),
                Codec::Hevc => "fast".into(),
                Codec::Av1 => "7".into(),
            };
        }
    }
}

/// Lossless trim (stream copy). Cuts snap to the previous keyframe for `start`.
pub fn trim(
    ffmpeg: &FfmpegInfo,
    input: &Path,
    output: &Path,
    start_secs: f64,
    end_secs: f64,
    cancel: &AtomicBool,
) -> Result<(), TranscodeError> {
    if end_secs <= start_secs {
        return Err(TranscodeError::Ffmpeg("end must be after start".into()));
    }
    let tmp = temp_path(output);
    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        format!("{start_secs:.3}"),
        "-to".into(),
        format!("{end_secs:.3}"),
        "-i".into(),
        input.display().to_string(),
        "-map".into(),
        "0".into(),
        "-c".into(),
        "copy".into(),
        "-avoid_negative_ts".into(),
        "make_zero".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-f".into(),
        "mp4".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        tmp.display().to_string(),
    ];
    match run_ffmpeg_progress(ffmpeg, &args, end_secs - start_secs, cancel, |_| {}) {
        Ok(()) => {
            std::fs::rename(&tmp, output)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// `clip.mp4` → `.clip.mp4.vinci-tmp` in the same directory (same filesystem ⇒ atomic rename).
pub fn temp_path(output: &Path) -> PathBuf {
    let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "out.mp4".into());
    output.with_file_name(format!(".{name}.vinci-tmp"))
}

/// Run FFmpeg with `-progress pipe:1`, reporting progress and honouring cancellation.
pub fn run_ffmpeg_progress(
    ffmpeg: &FfmpegInfo,
    args: &[String],
    duration_secs: f64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<(), TranscodeError> {
    let mut child = command(&ffmpeg.path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TranscodeError::Ffmpeg(e.to_string()))?;
    let mut stderr = child.stderr.take().expect("stderr piped");
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let stdout = child.stdout.take().expect("stdout piped");
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(TranscodeError::Cancelled);
        }
        if let Some(us) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
            if let (Ok(us), true) = (us.trim().parse::<f64>(), duration_secs > 0.0) {
                progress(((us / 1e6) / duration_secs).clamp(0.0, 1.0) as f32);
            }
        }
    }
    let status = child.wait()?;
    let err = err_reader.join().unwrap_or_default();
    if cancel.load(Ordering::Relaxed) {
        return Err(TranscodeError::Cancelled);
    }
    if status.success() {
        Ok(())
    } else {
        Err(TranscodeError::Ffmpeg(crate::probe::summarize_error(&err)))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn offline_compression_gets_quality_back() {
        let enc = crate::catalog::find("h264_nvenc").unwrap();
        let p = crate::VideoParams {
            codec: enc.codec,
            profile: crate::QualityProfile::Balanced,
            width: 1920,
            height: 1080,
            fps: 60,
            keyint_secs: 4.0,
        };
        let mut a = crate::args::encoder_args(enc, &p, crate::RateControl::Bitrate(8000));
        super::offline_quality(&mut a, enc.family);
        let s = a.join(" ");
        assert!(s.contains("-preset p5") && s.ends_with("-spatial-aq 1"), "{s}");
        let mut x = vec!["-preset".to_string(), "superfast".to_string()];
        super::offline_quality(&mut x, crate::EncoderFamily::Software);
        assert_eq!(x, ["-preset", "superfast"], "only NVENC is changed here");
    }

    use super::*;

    #[test]
    fn budget_math() {
        // 10 MB for a 30 s clip with 96 kbps audio ≈ 2464 kbps video.
        let b = size_budget_kbps(10.0, 30.0, 96).unwrap();
        assert!((2400..2600).contains(&b), "{b}");
        assert!(size_budget_kbps(1.0, 300.0, 128).is_none());
        assert_eq!(auto_height_for_budget(b, 1440), 1080);
        assert_eq!(auto_height_for_budget(500, 1080), 480);
        assert_eq!(fit_size(2560, 1440, Some(1080)), (1920, 1080));
        assert_eq!(fit_size(1920, 1080, Some(1440)), (1920, 1080));
        assert_eq!(fit_size(1366, 768, Some(720)), (1280, 720));
        assert_eq!(temp_path(Path::new("/a/b/clip.mp4")), PathBuf::from("/a/b/.clip.mp4.vinci-tmp"));
    }

    #[test]
    fn ffprobe_json() {
        let j = r#"{"streams":[{"codec_type":"video","codec_name":"hevc","width":1920,"height":1080,"avg_frame_rate":"60/1"},{"codec_type":"audio","codec_name":"aac"}],"format":{"duration":"30.016","size":"30000000","bit_rate":"8000000"}}"#;
        let m = parse_ffprobe_json(j).unwrap();
        assert_eq!((m.width, m.height, m.video_codec.as_str()), (1920, 1080, "hevc"));
        assert_eq!(m.audio_codec.as_deref(), Some("aac"));
        assert!((m.fps - 60.0).abs() < 1e-9 && (m.duration_secs - 30.016).abs() < 1e-9);
        assert_eq!(m.bitrate_kbps, 8000);
    }

    /// End-to-end: generate a 6 s clip, compress it to a size target and trim it.
    #[test]
    fn real_compress_and_trim() {
        let Ok(ff) = vinci_hw::ffmpeg::locate(None) else { return };
        if !ff.has_encoder("libx264") || ff.ffprobe.is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.mp4");
        let st = command(&ff.path)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "6",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "10",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
        let cancel = AtomicBool::new(false);
        let hw = HwContext::default();
        let enc = crate::catalog::find("libx264").unwrap();
        let job = Job { ffmpeg: &ff, encoder: enc, hw: &hw, cancel: &cancel };
        let out = dir.path().join("small.mp4");
        let mut last = 0.0;
        let r = compress(
            &job,
            &src,
            &out,
            &CompressOptions { codec: Codec::H264, target: CompressTarget::SizeMb(1.0), max_height: None, max_fps: None },
            |p| last = p,
        )
        .unwrap();
        assert!(r.size_bytes <= 1_000_000, "size {}", r.size_bytes);
        assert_eq!(last, 1.0);
        let info = probe_media(ff.ffprobe.as_ref().unwrap(), &out).unwrap();
        assert!((info.duration_secs - 6.0).abs() < 0.5);
        assert!(info.audio_codec.is_some());
        assert!(!temp_path(&out).exists());

        let trimmed = dir.path().join("trim.mp4");
        trim(&ff, &src, &trimmed, 1.0, 4.0, &cancel).unwrap();
        let t = probe_media(ff.ffprobe.as_ref().unwrap(), &trimmed).unwrap();
        assert!(t.duration_secs > 2.0 && t.duration_secs < 4.5, "{}", t.duration_secs);
    }
}
