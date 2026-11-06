//! Backend resolution and capture pipeline command lines.
//!
//! Every backend produces the same thing on stdout: a fragmented MP4 stream (see
//! `vinci_replay::fmp4_output_args`) that the replay buffer ingests.
//!
//! | Backend | Process graph                                                              |
//! |---------|----------------------------------------------------------------------------|
//! | WGC     | `ffmpeg gfxcapture(D3D11) → NVENC/AMF/QSV` + WASAPI PCM on stdin           |
//! | DDA     | `ffmpeg ddagrab(D3D11) → NVENC/AMF/QSV` + WASAPI PCM on stdin              |
//! | X11     | `ffmpeg x11grab(XShm) + pulse → NVENC/VAAPI`                               |
//! | Portal  | `gst-launch pipewiresrc(fd) + pulsesrc → VA/NV/x264 → matroska` → `ffmpeg -c copy` |
//! | Test    | `ffmpeg lavfi testsrc2 + sine` (real-time)                                 |

use crate::config::{AudioConfig, BackendKind};
use crate::monitors::MonitorInfo;
use crate::CaptureError;
use serde::{Deserialize, Serialize};
use vinci_encoding::{
    audio_args, plan_video, Codec, EncoderFamily, EncoderInfo, FilterCaps, FrameSource, HwContext, RateControl, VideoParams,
};
use vinci_hw::{DisplayServer, FfmpegInfo, GstInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    BelowNormal,
    #[default]
    Normal,
    AboveNormal,
}

/// Everything needed to build one capture pipeline.
#[derive(Debug, Clone)]
pub struct PipelineSpec {
    pub backend: BackendKind,
    pub monitor: MonitorInfo,
    /// WGC window capture by executable name.
    pub window_exe: Option<String>,
    pub out_size: (u32, u32),
    pub fps: u32,
    pub cursor: bool,
    pub audio: AudioConfig,
    pub encoder: EncoderInfo,
    pub video: VideoParams,
    pub hw: HwContext,
    pub audio_kbps: u32,
    pub frag_ms: u32,
    pub audio_offset_ms: i32,
    pub priority: Priority,
    /// Portal restore token (Wayland) so the user picks the screen only once.
    pub portal_restore_token: Option<String>,
    /// X11 display string, e.g. `:0`.
    pub x11_display: Option<String>,
}

/// Pick the concrete backend for a request on this machine.
///
/// On Wayland only the portal backend can see the screen: X11 capture through
/// XWayland would record black frames (or find no monitor at all), so `Auto` never
/// falls back to it — a missing GStreamer piece is reported with the command that
/// installs it instead. `X11` can still be chosen explicitly.
pub fn resolve_backend(
    requested: BackendKind,
    ffmpeg: &FfmpegInfo,
    gst: Option<&GstInfo>,
    display: DisplayServer,
) -> Result<BackendKind, CaptureError> {
    let portal_deps = crate::deps::PortalDeps::check(gst);
    let ok = |b: BackendKind| -> bool {
        match b {
            BackendKind::Wgc => cfg!(windows) && ffmpeg.supports_wgc(),
            BackendKind::Dda => cfg!(windows) && ffmpeg.supports_dda(),
            BackendKind::X11 => cfg!(target_os = "linux") && ffmpeg.has_input_device("x11grab") && std::env::var_os("DISPLAY").is_some(),
            BackendKind::Portal => cfg!(target_os = "linux") && portal_deps.video_ok(),
            BackendKind::Test => ffmpeg.has_input_device("lavfi") || ffmpeg.has_filter("testsrc2"),
            BackendKind::Auto => false,
        }
    };
    let order: &[BackendKind] = match requested {
        BackendKind::Auto => match display {
            DisplayServer::Windows => &[BackendKind::Wgc, BackendKind::Dda],
            DisplayServer::Wayland => &[BackendKind::Portal],
            DisplayServer::X11 => &[BackendKind::X11],
            DisplayServer::Unknown => &[],
        },
        BackendKind::Wgc => &[BackendKind::Wgc, BackendKind::Dda],
        BackendKind::Dda => &[BackendKind::Dda, BackendKind::Wgc],
        BackendKind::X11 => &[BackendKind::X11],
        BackendKind::Portal => &[BackendKind::Portal],
        BackendKind::Test => &[BackendKind::Test],
    };
    if let Some(b) = order.iter().copied().find(|b| ok(*b)) {
        return Ok(b);
    }
    let wants_portal = matches!((requested, display), (BackendKind::Auto, DisplayServer::Wayland) | (BackendKind::Portal, _));
    if wants_portal && cfg!(target_os = "linux") {
        let missing = portal_deps.all();
        return Err(CaptureError::MissingDeps {
            message: format!("Wayland screen capture needs GStreamer plugins that are not installed ({}).", portal_deps.video.join(", ")),
            command: crate::deps::install_command(&missing, vinci_hw::system::distro_family()),
        });
    }
    Err(CaptureError::NoBackend(match (requested, display) {
        (BackendKind::Auto, DisplayServer::Windows) => {
            "this FFmpeg build has neither gfxcapture (WGC) nor ddagrab (DDA); use the FFmpeg bundled with VINCI".into()
        }
        (BackendKind::Auto, DisplayServer::X11) => "FFmpeg was built without x11grab (X11 screen capture)".into(),
        (BackendKind::Auto, DisplayServer::Unknown) => "no graphical session detected (DISPLAY / WAYLAND_DISPLAY unset)".into(),
        (r, _) => format!("{} is not available on this system", r.label()),
    }))
}

#[derive(Debug, Clone, PartialEq)]
pub struct FfmpegCommand {
    pub args: Vec<String>,
    /// Raw f32le/48 kHz/stereo PCM must be written to stdin (Windows WASAPI pump).
    pub stdin_audio: bool,
    pub frame_source: FrameSource,
}

fn push<S: Into<String>>(v: &mut Vec<String>, items: impl IntoIterator<Item = S>) {
    v.extend(items.into_iter().map(Into::into));
}

/// FFmpeg pipeline for the WGC / DDA / X11 / Test backends.
pub fn ffmpeg_command(spec: &PipelineSpec, ffmpeg: &FfmpegInfo) -> Result<FfmpegCommand, CaptureError> {
    let caps = FilterCaps::from_ffmpeg(ffmpeg);
    let (sw, sh) = (spec.monitor.width.max(2), spec.monitor.height.max(2));
    let fps = spec.fps.clamp(10, 240);
    let windows_audio = cfg!(windows) && matches!(spec.backend, BackendKind::Wgc | BackendKind::Dda) && spec.audio.any();
    let mut hw = spec.hw.clone();
    if matches!(spec.backend, BackendKind::Wgc | BackendKind::Dda) {
        hw.d3d11_adapter = Some(spec.monitor.adapter);
    }

    let (frame_source, scale_to) = match spec.backend {
        BackendKind::Wgc => (FrameSource::D3D11, None), // gfxcapture scales on the GPU itself
        BackendKind::Dda => (FrameSource::D3D11, (spec.out_size != (sw, sh)).then_some(spec.out_size)),
        BackendKind::X11 => (FrameSource::SystemRgb, (spec.out_size != (sw, sh)).then_some(spec.out_size)),
        BackendKind::Test => (FrameSource::SystemYuv, None),
        BackendKind::Portal | BackendKind::Auto => {
            return Err(CaptureError::NoBackend("ffmpeg_command called for a non-FFmpeg backend".into()))
        }
    };
    let plan = plan_video(&spec.encoder, &spec.video, RateControl::Quality, frame_source, scale_to, caps, &hw);

    let mut a: Vec<String> = Vec::new();
    push(&mut a, ["-hide_banner", "-loglevel", "warning", "-nostats"]);
    if !windows_audio {
        a.push("-nostdin".into());
    }
    let mut global = plan.global_args.clone();
    if frame_source == FrameSource::D3D11 {
        if !global.iter().any(|g| g.starts_with("d3d11va=vinci_d3d")) {
            let mut init = vec!["-init_hw_device".to_string(), format!("d3d11va=vinci_d3d:{}", spec.monitor.adapter)];
            init.extend(global);
            global = init;
        }
        push(&mut global, ["-filter_hw_device", "vinci_d3d"]);
    }
    a.extend(global);

    // ---- inputs ----
    let mut input_idx = 0usize;
    let mut video_label: Option<String> = None; // input pad (e.g. "0:v") when the source is an input
    let mut graph: Vec<String> = Vec::new();
    match spec.backend {
        BackendKind::X11 => {
            let disp = spec.x11_display.clone().or_else(|| std::env::var("DISPLAY").ok()).unwrap_or_else(|| ":0".into());
            // Keep the raw-frame queue small (16 × 8 MB at 1080p): if the encoder falls
            // behind we drop frames instead of buffering gigabytes of raw video.
            // Measured: `-fps_mode passthrough` + PulseAudio stalls FFmpeg 6.1's
            // single-threaded main loop (~20 fps), so the default CFR mode is kept.
            push(&mut a, ["-thread_queue_size", "16", "-f", "x11grab", "-framerate"]);
            a.push(fps.to_string());
            push(&mut a, ["-draw_mouse", if spec.cursor { "1" } else { "0" }, "-video_size"]);
            a.push(format!("{sw}x{sh}"));
            a.push("-i".into());
            a.push(format!("{disp}+{},{}", spec.monitor.x, spec.monitor.y));
            video_label = Some(format!("{input_idx}:v"));
            input_idx += 1;
        }
        BackendKind::Test => {
            push(&mut a, ["-re", "-f", "lavfi", "-i"]);
            let (w, h) = spec.out_size;
            a.push(format!("testsrc2=size={w}x{h}:rate={fps}"));
            video_label = Some(format!("{input_idx}:v"));
            input_idx += 1;
        }
        _ => {}
    }

    let mut audio_inputs: Vec<(usize, f32)> = Vec::new();
    if spec.audio.any() {
        let offset = |a: &mut Vec<String>| {
            if spec.audio_offset_ms != 0 {
                a.push("-itsoffset".into());
                a.push(format!("{:.3}", spec.audio_offset_ms as f64 / 1000.0));
            }
        };
        if windows_audio {
            // System + mic are captured and mixed natively (WASAPI) and piped in.
            offset(&mut a);
            push(&mut a, ["-thread_queue_size", "1024", "-f", "f32le", "-ar", "48000", "-ac", "2", "-i", "pipe:0"]);
            audio_inputs.push((input_idx, 1.0));
            input_idx += 1;
        } else if spec.backend == BackendKind::Test {
            offset(&mut a);
            push(&mut a, ["-re", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:beep_factor=4"]);
            audio_inputs.push((input_idx, spec.audio.system_volume));
            input_idx += 1;
        } else {
            let mut pulse = |a: &mut Vec<String>, dev: &str, vol: f32| {
                offset(a);
                push(
                    a,
                    [
                        "-thread_queue_size",
                        "1024",
                        "-f",
                        "pulse",
                        "-sample_rate",
                        "48000",
                        "-channels",
                        "2",
                        "-fragment_size",
                        "3840",
                        "-i",
                        dev,
                    ],
                );
                audio_inputs.push((input_idx, vol));
                input_idx += 1;
            };
            if spec.audio.system {
                pulse(&mut a, spec.audio.system_device.as_deref().unwrap_or("@DEFAULT_MONITOR@"), spec.audio.system_volume);
            }
            if spec.audio.microphone {
                pulse(&mut a, spec.audio.mic_device.as_deref().unwrap_or("@DEFAULT_SOURCE@"), spec.audio.mic_volume);
            }
        }
    }
    let _ = input_idx;

    // ---- filter graph ----
    let chain = plan.filter.clone();
    let video_map = match spec.backend {
        BackendKind::Wgc => {
            let mut src = String::from("gfxcapture=");
            if let Some(exe) = &spec.window_exe {
                src.push_str(&format!("window_exe={}", wgc_exe_regex(exe)));
            } else if spec.monitor.handle != 0 {
                src.push_str(&format!("hmonitor={}", spec.monitor.handle));
            } else {
                src.push_str(&format!("monitor_idx={}", spec.monitor.index));
            }
            src.push_str(&format!(":max_framerate={fps}:capture_cursor={}", spec.cursor as u8));
            if spec.window_exe.is_none() && spec.out_size != (sw, sh) {
                src.push_str(&format!(":width={}:height={}:resize_mode=scale_aspect:scale_mode=bicubic", spec.out_size.0, spec.out_size.1));
            }
            src.push_str(&format!(",fps={fps}"));
            if let Some(c) = &chain {
                src.push(',');
                src.push_str(c);
            }
            graph.push(format!("{src}[v]"));
            "[v]".to_string()
        }
        BackendKind::Dda => {
            let mut src = format!("ddagrab=output_idx={}:framerate={fps}:draw_mouse={}", spec.monitor.output, spec.cursor as u8);
            if let Some(c) = &chain {
                src.push(',');
                src.push_str(c);
            }
            graph.push(format!("{src}[v]"));
            "[v]".to_string()
        }
        _ => {
            let label = video_label.clone().unwrap_or_else(|| "0:v".into());
            match &chain {
                Some(c) => {
                    graph.push(format!("[{label}]{c}[v]"));
                    "[v]".to_string()
                }
                None => label,
            }
        }
    };

    let audio_map = match audio_inputs.as_slice() {
        [] => None,
        [(i, v)] if (*v - 1.0).abs() < 1e-3 => Some(format!("{i}:a")),
        [(i, v)] => {
            graph.push(format!("[{i}:a]volume={v:.2}[a]"));
            Some("[a]".into())
        }
        many => {
            let mut labels = String::new();
            for (n, (i, v)) in many.iter().enumerate() {
                graph.push(format!("[{i}:a]volume={v:.2}[a{n}]"));
                labels.push_str(&format!("[a{n}]"));
            }
            graph.push(format!("{labels}amix=inputs={}:duration=longest:normalize=0[a]", many.len()));
            Some("[a]".into())
        }
    };

    if !graph.is_empty() {
        a.push("-filter_complex".into());
        a.push(graph.join(";"));
    }
    a.push("-map".into());
    a.push(video_map);
    if let Some(m) = audio_map {
        a.push("-map".into());
        a.push(m);
    }
    a.extend(plan.encoder_args);
    if spec.encoder.id == "libx264" {
        // Measured on 1080p60: default look-ahead/frame threads use ~420 MB; this keeps
        // ~300 MB for +4 % size (`zerolatency` would be 136 MB but files 2.7× larger).
        let threads = std::thread::available_parallelism().map(|n| n.get().min(6)).unwrap_or(4);
        push(&mut a, ["-x264-params".to_string(), "rc-lookahead=10:sync-lookahead=0".into(), "-threads".into(), threads.to_string()]);
    }
    if spec.audio.any() {
        a.extend(audio_args(spec.audio_kbps));
    }
    a.extend(vinci_replay::fmp4_output_args(spec.frag_ms));
    Ok(FfmpegCommand { args: a, stdin_audio: windows_audio, frame_source })
}

/// `VALORANT-Win64-Shipping.exe` → `(?i)^VALORANT.Win64.Shipping.exe$`
/// Regex metacharacters and filtergraph-special characters are replaced by `.`.
pub fn wgc_exe_regex(exe: &str) -> String {
    let safe: String = exe.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == ' ' { c } else { '.' }).collect();
    format!("(?i)^{safe}$")
}

/// GStreamer pipeline (Wayland portal) — writes Matroska to stdout.
pub fn gst_portal_args(spec: &PipelineSpec, gst: &GstInfo, pipewire_fd: i32, node_id: u32) -> (Vec<String>, Codec) {
    let (w, h) = spec.out_size;
    let fps = spec.fps.clamp(10, 240);
    let v = &spec.video;
    let kbps = vinci_encoding::profile::target_kbps(v.codec, v.profile, w, h, fps);
    let gop = v.gop();
    let mut p: Vec<String> = vec!["-q".into(), "-e".into()];
    let mut el = |s: String| {
        if !p.is_empty() && p.len() > 2 {
            p.push("!".into());
        }
        p.extend(s.split_whitespace().map(String::from));
    };
    el(format!("pipewiresrc fd={pipewire_fd} path={node_id} do-timestamp=true keepalive-time=1000 always-copy=false"));
    el("videorate".into());
    el(format!("video/x-raw,framerate={fps}/1"));
    let (elements, actual_codec) = gst_video_encoder(spec, gst, w, h, kbps, gop);
    for e in elements {
        el(e);
    }
    el("queue max-size-time=2000000000 max-size-buffers=0 max-size-bytes=0".into());
    el("mux.".into());
    let mut audio_parts: Vec<String> = Vec::new();
    let aac = ["avenc_aac", "fdkaacenc", "voaacenc"].into_iter().find(|e| gst.has(e));
    let audio_deps = crate::deps::PortalDeps::check(Some(gst));
    if spec.audio.any() {
        if let (Some(enc), true) = (aac, audio_deps.audio_ok()) {
            let kb = spec.audio_kbps * 1000;
            let src = |dev: &str, vol: f32| {
                format!("pulsesrc device={dev} do-timestamp=true provide-clock=false ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! volume volume={vol:.2}")
            };
            match (spec.audio.system, spec.audio.microphone) {
                (true, true) => {
                    audio_parts.push(format!("audiomixer name=amix ! audioconvert ! {enc} bitrate={kb} ! aacparse ! queue ! mux."));
                    audio_parts.push(format!(
                        "{} ! queue ! amix.",
                        src(spec.audio.system_device.as_deref().unwrap_or("@DEFAULT_MONITOR@"), spec.audio.system_volume)
                    ));
                    audio_parts.push(format!(
                        "{} ! queue ! amix.",
                        src(spec.audio.mic_device.as_deref().unwrap_or("@DEFAULT_SOURCE@"), spec.audio.mic_volume)
                    ));
                }
                (sys, _) => {
                    let dev = if sys {
                        spec.audio.system_device.as_deref().unwrap_or("@DEFAULT_MONITOR@")
                    } else {
                        spec.audio.mic_device.as_deref().unwrap_or("@DEFAULT_SOURCE@")
                    };
                    let vol = if sys { spec.audio.system_volume } else { spec.audio.mic_volume };
                    audio_parts.push(format!("{} ! {enc} bitrate={kb} ! aacparse ! queue ! mux.", src(dev, vol)));
                }
            }
        } else {
            log::warn!("GStreamer audio elements missing ({}): recording without audio", audio_deps.audio.join(", "));
        }
    }
    p.extend("matroskamux name=mux streamable=true ! fdsink fd=1 sync=false".split_whitespace().map(String::from));
    for part in audio_parts {
        p.extend(part.split_whitespace().map(String::from));
    }
    (p, actual_codec)
}

/// Chooses the GStreamer encoder matching the FFmpeg encoder family that was validated.
/// Returns the element chain and the codec it really produces.
fn gst_video_encoder(spec: &PipelineSpec, gst: &GstInfo, w: u32, h: u32, kbps: u32, gop: u32) -> (Vec<String>, Codec) {
    let codec = spec.video.codec;
    let (va, vaapi, nv, sw, parse) = match codec {
        Codec::H264 => ("vah264enc", "vaapih264enc", "nvh264enc", "x264enc", "h264parse"),
        Codec::Hevc => ("vah265enc", "vaapih265enc", "nvh265enc", "x265enc", "h265parse"),
        Codec::Av1 => ("vaav1enc", "", "nvav1enc", "", "av1parse"),
    };
    let fam = spec.encoder.family;
    if fam == EncoderFamily::Vaapi && gst.has(va) && gst.has("vapostproc") {
        return (
            vec![
                "vapostproc".into(),
                format!("video/x-raw(memory:VAMemory),format=NV12,width={w},height={h}"),
                format!("{va} rate-control=vbr bitrate={kbps} key-int-max={gop}"),
                parse.into(),
            ],
            codec,
        );
    }
    if fam == EncoderFamily::Vaapi && !vaapi.is_empty() && gst.has(vaapi) {
        return (
            vec![
                "vaapipostproc".into(),
                format!("video/x-raw(memory:VASurface),format=NV12,width={w},height={h}"),
                format!("{vaapi} rate-control=vbr bitrate={kbps} keyframe-period={gop}"),
                parse.into(),
            ],
            codec,
        );
    }
    if fam == EncoderFamily::Nvenc && gst.has(nv) {
        return (
            vec![
                "videoconvert".into(),
                "videoscale".into(),
                format!("video/x-raw,format=NV12,width={w},height={h}"),
                format!("{nv} rc-mode=vbr bitrate={kbps} gop-size={gop}"),
                parse.into(),
            ],
            codec,
        );
    }
    // CPU fallback (always H.264 when the codec's software encoder is missing).
    let (sw, parse, actual) = if !sw.is_empty() && gst.has(sw) { (sw, parse, codec) } else { ("x264enc", "h264parse", Codec::H264) };
    let opts = if sw == "x264enc" {
        format!("speed-preset=veryfast bitrate={kbps} key-int-max={gop}")
    } else {
        format!("speed-preset=ultrafast bitrate={kbps} key-int-max={gop}")
    };
    (
        vec![
            "videoconvert".into(),
            "videoscale".into(),
            format!("video/x-raw,format=I420,width={w},height={h}"),
            format!("{sw} {opts}"),
            parse.into(),
        ],
        actual,
    )
}

/// FFmpeg stage that turns the GStreamer Matroska stream into fragmented MP4 (copy only).
pub fn remux_mkv_to_fmp4_args(frag_ms: u32, hevc: bool) -> Vec<String> {
    let mut a: Vec<String> =
        ["-hide_banner", "-loglevel", "warning", "-nostats", "-f", "matroska", "-i", "pipe:0", "-map", "0", "-c", "copy"]
            .map(String::from)
            .to_vec();
    if hevc {
        push(&mut a, ["-tag:v", "hvc1"]);
    }
    a.extend(vinci_replay::fmp4_output_args(frag_ms));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use vinci_encoding::{catalog::find, QualityProfile};

    fn ff(filters: &[&str], devices: &[&str]) -> FfmpegInfo {
        FfmpegInfo {
            path: "ffmpeg".into(),
            ffprobe: None,
            source: vinci_hw::FfmpegSource::System,
            version: "8.1".into(),
            release: Some((8, 1)),
            lavfi: (11, 0, 0),
            encoders: BTreeSet::new(),
            filters: filters.iter().map(|s| s.to_string()).collect(),
            input_devices: devices.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn spec(backend: BackendKind, enc: &str, out: (u32, u32)) -> PipelineSpec {
        let e = *find(enc).unwrap();
        PipelineSpec {
            backend,
            monitor: MonitorInfo {
                index: 1,
                name: "D".into(),
                x: 1920,
                y: 0,
                width: 2560,
                height: 1440,
                primary: false,
                adapter: 0,
                output: 1,
                handle: 65537,
            },
            window_exe: None,
            out_size: out,
            fps: 60,
            cursor: true,
            audio: AudioConfig::default(),
            encoder: e,
            video: VideoParams {
                codec: e.codec,
                profile: QualityProfile::Balanced,
                width: out.0,
                height: out.1,
                fps: 60,
                keyint_secs: 2.0,
            },
            hw: HwContext::default(),
            audio_kbps: 160,
            frag_ms: 500,
            audio_offset_ms: 0,
            priority: Priority::Normal,
            portal_restore_token: None,
            x11_display: Some(":0".into()),
        }
    }

    #[test]
    fn wgc_scales_inside_capture_and_feeds_nvenc_directly() {
        let f = ff(&["gfxcapture", "ddagrab", "scale_d3d11"], &[]);
        let c = ffmpeg_command(&spec(BackendKind::Wgc, "hevc_nvenc", (1920, 1080)), &f).unwrap();
        let s = c.args.join(" ");
        assert!(s.contains("-init_hw_device d3d11va=vinci_d3d:0 -filter_hw_device vinci_d3d"), "{s}");
        assert!(s.contains("gfxcapture=hmonitor=65537:max_framerate=60:capture_cursor=1:width=1920:height=1080:resize_mode=scale_aspect:scale_mode=bicubic,fps=60[v]"), "{s}");
        assert!(s.contains("-map [v]") && s.contains("-c:v hevc_nvenc"));
        assert!(s.ends_with("-flush_packets 1 pipe:1"));
        assert_eq!(c.frame_source, FrameSource::D3D11);
        if cfg!(windows) {
            assert!(c.stdin_audio && s.contains("-f f32le -ar 48000 -ac 2 -i pipe:0") && !s.contains("-nostdin"));
        }
    }

    #[test]
    fn dda_scales_with_d3d11_video_processor() {
        let f = ff(&["ddagrab", "scale_d3d11"], &[]);
        let s = ffmpeg_command(&spec(BackendKind::Dda, "h264_amf", (1920, 1080)), &f).unwrap().args.join(" ");
        assert!(s.contains("ddagrab=output_idx=1:framerate=60:draw_mouse=1,scale_d3d11=width=1920:height=1080:format=nv12[v]"), "{s}");
    }

    #[test]
    fn x11_with_vaapi_and_two_audio_sources() {
        let f = ff(&["scale_vaapi"], &["x11grab", "pulse"]);
        let mut sp = spec(BackendKind::X11, "h264_vaapi", (1920, 1080));
        sp.audio.microphone = true;
        sp.audio.mic_volume = 0.8;
        let c = ffmpeg_command(&sp, &f).unwrap();
        let s = c.args.join(" ");
        assert!(s.contains("-f x11grab -framerate 60 -draw_mouse 1 -video_size 2560x1440 -i :0+1920,0"), "{s}");
        assert!(s.contains("-i @DEFAULT_MONITOR@") && s.contains("-i @DEFAULT_SOURCE@"));
        assert!(s.contains("[0:v]format=bgr0,hwupload,scale_vaapi=w=1920:h=1080:format=nv12[v]"), "{s}");
        assert!(s.contains("[1:a]volume=1.00[a0];[2:a]volume=0.80[a1];[a0][a1]amix=inputs=2:duration=longest:normalize=0[a]"), "{s}");
        assert!(s.contains("-vaapi_device"));
        assert!(!c.stdin_audio && s.contains("-nostdin"));
    }

    #[test]
    fn wgc_window_regex_is_filtergraph_safe() {
        assert_eq!(wgc_exe_regex("VALORANT-Win64-Shipping.exe"), "(?i)^VALORANT.Win64.Shipping.exe$");
        assert_eq!(wgc_exe_regex("a:b,c[d]';.exe"), "(?i)^a.b.c.d....exe$");
    }

    #[test]
    fn backend_resolution() {
        let f = ff(&["ddagrab"], &["lavfi"]);
        assert_eq!(resolve_backend(BackendKind::Test, &f, None, DisplayServer::Unknown).unwrap(), BackendKind::Test);
        assert!(resolve_backend(BackendKind::Auto, &f, None, DisplayServer::Unknown).is_err());
        if cfg!(windows) {
            assert_eq!(
                resolve_backend(BackendKind::Auto, &f, None, DisplayServer::Windows).unwrap(),
                BackendKind::Dda,
                "falls back to DDA without gfxcapture"
            );
        }
        let full: Vec<String> = crate::deps::PORTAL_VIDEO.iter().map(|s| s.to_string()).collect();
        let gst = GstInfo { launch: "gst-launch-1.0".into(), elements: full.into_iter().collect() };
        if cfg!(target_os = "linux") {
            assert_eq!(resolve_backend(BackendKind::Auto, &f, Some(&gst), DisplayServer::Wayland).unwrap(), BackendKind::Portal);
            // Wayland without the PipeWire plugin: never X11 through XWayland (black / "no monitor found"),
            // but an explanation naming what is missing.
            let f11 = ff(&[], &["x11grab", "lavfi"]);
            let partial = GstInfo { launch: "gst-launch-1.0".into(), elements: ["videoconvert".to_string()].into_iter().collect() };
            match resolve_backend(BackendKind::Auto, &f11, Some(&partial), DisplayServer::Wayland) {
                Err(CaptureError::MissingDeps { message, .. }) => assert!(message.contains("pipewiresrc"), "{message}"),
                other => panic!("expected MissingDeps, got {other:?}"),
            }
            assert!(matches!(
                resolve_backend(BackendKind::Auto, &f11, None, DisplayServer::Wayland),
                Err(CaptureError::MissingDeps { .. })
            ));
        }
    }

    #[test]
    fn gst_pipeline_shape() {
        let gst = GstInfo {
            launch: "gst-launch-1.0".into(),
            elements: ["pipewiresrc", "vah265enc", "vapostproc", "x264enc"]
                .iter()
                .chain(crate::deps::PORTAL_AUDIO)
                .map(|s| s.to_string())
                .collect(),
        };
        let sp = spec(BackendKind::Portal, "hevc_vaapi", (1920, 1080));
        let (a, codec) = gst_portal_args(&sp, &gst, 3, 42);
        let a = a.join(" ");
        assert_eq!(codec, Codec::Hevc);
        assert!(a.starts_with("-q -e pipewiresrc fd=3 path=42"), "{a}");
        assert!(
            a.contains("vapostproc ! video/x-raw(memory:VAMemory),format=NV12,width=1920,height=1080 ! vah265enc rate-control=vbr"),
            "{a}"
        );
        assert!(a.contains("matroskamux name=mux streamable=true ! fdsink fd=1"));
        assert!(a.contains("pulsesrc device=@DEFAULT_MONITOR@") && a.contains("avenc_aac bitrate=160000"), "{a}");
        // Unknown encoder family → x264 fallback
        let sp = spec(BackendKind::Portal, "libx265", (1280, 720));
        let gst2 = GstInfo { launch: "x".into(), elements: ["pipewiresrc", "x264enc"].iter().map(|s| s.to_string()).collect() };
        let (a2, c2) = gst_portal_args(&sp, &gst2, 3, 1);
        assert!(a2.join(" ").contains("x264enc speed-preset=veryfast"));
        assert_eq!(c2, Codec::H264, "real codec reported when falling back");
        let r = remux_mkv_to_fmp4_args(500, true).join(" ");
        assert!(r.contains("-f matroska -i pipe:0 -map 0 -c copy -tag:v hvc1 -f mp4"));
    }
}
