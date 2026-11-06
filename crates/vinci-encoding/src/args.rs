//! FFmpeg argument builders: hardware device setup, frame-format adaptation chain and
//! encoder rate-control options for every encoder family.
//!
//! The goal is to keep frames on the GPU from capture to encoder:
//!
//! | Source            | NVENC / AMF           | QSV                                   | VA-API                              | CPU           |
//! |-------------------|-----------------------|---------------------------------------|-------------------------------------|---------------|
//! | D3D11 (WGC / DDA) | direct (zero-copy)    | `scale_d3d11` → `hwmap=qsv`           | n/a                                 | `hwdownload`  |
//! | System RGB (X11)  | direct (GPU converts) | `format=nv12`                         | `hwupload,scale_vaapi=format=nv12`  | `format`      |
//! | System YUV (file) | direct                | direct                                | `format=nv12,hwupload`              | direct        |

use crate::catalog::{Codec, EncoderFamily, EncoderInfo};
use crate::profile::{max_kbps, quality_value, target_kbps, VideoParams};
use std::path::PathBuf;

/// Where the frames handed to the encoder live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSource {
    /// D3D11 textures from `gfxcapture` / `ddagrab` (BGRA).
    D3D11,
    /// CPU memory, packed RGB (x11grab `bgr0`, GStreamer raw).
    SystemRgb,
    /// CPU memory, planar YUV (decoded files, lavfi test sources).
    SystemYuv,
}

/// Which optional GPU filters this FFmpeg build has.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilterCaps {
    pub scale_d3d11: bool,
    pub scale_vaapi: bool,
    pub scale_cuda: bool,
}

impl FilterCaps {
    pub fn from_ffmpeg(f: &vinci_hw::FfmpegInfo) -> Self {
        Self { scale_d3d11: f.has_filter("scale_d3d11"), scale_vaapi: f.has_filter("scale_vaapi"), scale_cuda: f.has_filter("scale_cuda") }
    }
}

/// GPU device selection for encoders that need an explicit device.
#[derive(Debug, Clone, Default)]
pub struct HwContext {
    /// DRM render node for VA-API (`/dev/dri/renderD128`).
    pub vaapi_device: Option<PathBuf>,
    /// DXGI adapter index for D3D11 capture / QSV derivation.
    pub d3d11_adapter: Option<u32>,
}

/// Rate-control intent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RateControl {
    /// Quality-targeted (CQ/CRF/ICQ) with a peak cap derived from the profile.
    Quality,
    /// Average bitrate target in kbit/s (used for "compress to N MB").
    Bitrate(u32),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EncodePlan {
    /// Arguments placed before the inputs (`-init_hw_device`, `-vaapi_device`, …).
    pub global_args: Vec<String>,
    /// Video filter chain (without the leading `-vf`) or `None` when frames go straight in.
    pub filter: Option<String>,
    /// Codec + rate control arguments.
    pub encoder_args: Vec<String>,
}

fn s<I: IntoIterator<Item = T>, T: Into<String>>(it: I) -> Vec<String> {
    it.into_iter().map(Into::into).collect()
}

/// Build the complete plan for one video stream.
///
/// `scale_to` is `Some((w, h))` when the encoded size differs from the size of the
/// frames delivered by the source.
pub fn plan_video(
    enc: &EncoderInfo,
    params: &VideoParams,
    rc: RateControl,
    src: FrameSource,
    scale_to: Option<(u32, u32)>,
    caps: FilterCaps,
    hw: &HwContext,
) -> EncodePlan {
    let mut global = Vec::new();
    let mut chain: Vec<String> = Vec::new();
    let sw_scale = |chain: &mut Vec<String>| {
        if let Some((w, h)) = scale_to {
            chain.push(format!("scale={w}:{h}:flags=bicubic"));
        }
    };

    match (enc.family, src) {
        // ---------------- CPU encoders ----------------
        (EncoderFamily::Software, FrameSource::D3D11) => {
            chain.push("hwdownload".into());
            chain.push("format=bgra".into());
            sw_scale(&mut chain);
            chain.push("format=yuv420p".into());
        }
        (EncoderFamily::Software, _) => {
            sw_scale(&mut chain);
            chain.push("format=yuv420p".into());
        }

        // ---------------- NVENC / AMF: accept D3D11 and RGB directly ----------------
        (EncoderFamily::Nvenc | EncoderFamily::Amf, FrameSource::D3D11) => {
            if let Some((w, h)) = scale_to {
                if caps.scale_d3d11 {
                    chain.push(format!("scale_d3d11=width={w}:height={h}:format=nv12"));
                } else {
                    chain.push("hwdownload".into());
                    chain.push("format=bgra".into());
                    chain.push(format!("scale={w}:{h}:flags=bicubic"));
                    chain.push("format=nv12".into());
                }
            }
        }
        (EncoderFamily::Nvenc, FrameSource::SystemRgb | FrameSource::SystemYuv) => {
            if let Some((w, h)) = scale_to {
                if caps.scale_cuda && src == FrameSource::SystemYuv {
                    chain.push("hwupload_cuda".into());
                    chain.push(format!("scale_cuda={w}:{h}"));
                } else {
                    chain.push(format!("scale={w}:{h}:flags=bilinear"));
                }
            }
        }
        (EncoderFamily::Amf, _) => {
            if let Some((w, h)) = scale_to {
                chain.push(format!("scale={w}:{h}:flags=bilinear"));
            }
        }

        // ---------------- Intel Quick Sync (Windows) ----------------
        (EncoderFamily::Qsv, FrameSource::D3D11) => {
            let adapter = hw.d3d11_adapter.unwrap_or(0);
            global.extend(s([
                "-init_hw_device".to_string(),
                format!("d3d11va=vinci_d3d:{adapter}"),
                "-init_hw_device".into(),
                "qsv=vinci_qsv@vinci_d3d".into(),
            ]));
            if caps.scale_d3d11 {
                let size = scale_to.map(|(w, h)| format!("width={w}:height={h}:")).unwrap_or_default();
                chain.push(format!("scale_d3d11={size}format=nv12"));
                chain.push("hwmap=derive_device=qsv".into());
                chain.push("format=qsv".into());
            } else {
                chain.push("hwdownload".into());
                chain.push("format=bgra".into());
                if let Some((w, h)) = scale_to {
                    chain.push(format!("scale={w}:{h}:flags=bicubic"));
                }
                chain.push("format=nv12".into());
            }
        }
        (EncoderFamily::Qsv, _) => {
            sw_scale(&mut chain);
            chain.push("format=nv12".into());
        }

        // ---------------- VA-API (Linux) ----------------
        (EncoderFamily::Vaapi, _) => {
            let dev = hw.vaapi_device.clone().unwrap_or_else(|| PathBuf::from("/dev/dri/renderD128"));
            global.extend(s(["-vaapi_device".to_string(), dev.display().to_string()]));
            if caps.scale_vaapi {
                // Upload packed RGB as-is and let the GPU video processor convert/scale.
                chain.push(if src == FrameSource::SystemRgb { "format=bgr0".into() } else { "format=nv12".into() });
                chain.push("hwupload".into());
                let size = scale_to.map(|(w, h)| format!("w={w}:h={h}:")).unwrap_or_default();
                chain.push(format!("scale_vaapi={size}format=nv12"));
            } else {
                sw_scale(&mut chain);
                chain.push("format=nv12".into());
                chain.push("hwupload".into());
            }
        }
    }

    EncodePlan {
        global_args: global,
        filter: if chain.is_empty() { None } else { Some(chain.join(",")) },
        encoder_args: encoder_args(enc, params, rc),
    }
}

/// Codec + rate-control arguments for one encoder.
pub fn encoder_args(enc: &EncoderInfo, p: &VideoParams, rc: RateControl) -> Vec<String> {
    use crate::profile::QualityProfile as Q;
    let gop = p.gop().to_string();
    let (target, peak) = match rc {
        RateControl::Quality => {
            (target_kbps(p.codec, p.profile, p.width, p.height, p.fps), max_kbps(p.codec, p.profile, p.width, p.height, p.fps))
        }
        RateControl::Bitrate(k) => (k, (k as f64 * 1.25) as u32),
    };
    let k = |v: u32| format!("{v}k");
    let q = quality_value(enc.family, p.codec, p.profile).to_string();
    let mut a = s(["-c:v", enc.id]);

    match enc.family {
        EncoderFamily::Nvenc => {
            let preset = match p.profile {
                Q::Quality => "p5",
                Q::Balanced => "p4",
                Q::Compact => "p5",
            };
            a.extend(s(["-preset", preset, "-tune", "hq", "-rc", "vbr"]));
            match rc {
                RateControl::Quality => a.extend(s(["-cq".to_string(), q, "-b:v".into(), "0".into()])),
                RateControl::Bitrate(_) => a.extend(s(["-b:v".to_string(), k(target)])),
            }
            a.extend(s(["-maxrate".to_string(), k(peak), "-bufsize".into(), k(peak * 2), "-spatial-aq".into(), "1".into()]));
        }
        EncoderFamily::Amf => {
            let quality = match p.profile {
                Q::Quality => "quality",
                _ => "balanced",
            };
            a.extend(s(["-usage", "transcoding", "-quality", quality, "-rc", "vbr_peak"]));
            a.extend(s(["-b:v".to_string(), k(target), "-maxrate".into(), k(peak), "-bufsize".into(), k(peak * 2)]));
        }
        EncoderFamily::Qsv => {
            let preset = match p.profile {
                Q::Quality => "medium",
                _ => "faster",
            };
            a.extend(s(["-preset", preset, "-look_ahead", "0", "-async_depth", "4"]));
            match (rc, p.codec) {
                (RateControl::Quality, Codec::H264 | Codec::Hevc) => a.extend(s(["-global_quality".to_string(), q])),
                _ => a.extend(s(["-b:v".to_string(), k(target), "-maxrate".into(), k(peak), "-bufsize".into(), k(peak * 2)])),
            }
        }
        EncoderFamily::Vaapi => {
            a.extend(s([
                "-rc_mode".to_string(),
                "VBR".into(),
                "-b:v".into(),
                k(target),
                "-maxrate".into(),
                k(peak),
                "-bufsize".into(),
                k(peak * 2),
            ]));
            if p.codec == Codec::H264 {
                a.extend(s(["-profile:v", "high"]));
            }
        }
        EncoderFamily::Software => {
            let (preset, rc_args) = match (p.codec, rc) {
                (Codec::H264, RateControl::Quality) => ("veryfast", s(["-crf".to_string(), q])),
                (Codec::Hevc, RateControl::Quality) => ("ultrafast", s(["-crf".to_string(), q])),
                (Codec::Av1, RateControl::Quality) => ("10", s(["-crf".to_string(), q])),
                (Codec::H264, RateControl::Bitrate(_)) => ("veryfast", s(["-b:v".to_string(), k(target)])),
                (Codec::Hevc, RateControl::Bitrate(_)) => ("ultrafast", s(["-b:v".to_string(), k(target)])),
                (Codec::Av1, RateControl::Bitrate(_)) => ("10", s(["-b:v".to_string(), k(target)])),
            };
            a.extend(s(["-preset", preset]));
            a.extend(rc_args);
            if p.codec != Codec::Av1 {
                a.extend(s(["-maxrate".to_string(), k(peak), "-bufsize".into(), k(peak * 2)]));
            }
            if p.codec == Codec::Hevc {
                a.extend(s(["-x265-params", "log-level=error"]));
            }
        }
    }
    a.extend(s(["-g".to_string(), gop.clone()]));
    if enc.family == EncoderFamily::Software && p.codec == Codec::H264 {
        a.extend(s(["-keyint_min".to_string(), gop]));
    }
    // hvc1 tag makes HEVC-in-MP4 playable in QuickTime/Safari/Windows players.
    if p.codec == Codec::Hevc {
        a.extend(s(["-tag:v", "hvc1"]));
    }
    a
}

/// Audio encoder arguments (AAC-LC, universally playable in MP4).
pub fn audio_args(kbps: u32) -> Vec<String> {
    s(["-c:a".to_string(), "aac".into(), "-b:a".into(), format!("{kbps}k"), "-ar".into(), "48000".into(), "-ac".into(), "2".into()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::find;
    use crate::profile::QualityProfile;

    fn params(codec: Codec) -> VideoParams {
        VideoParams { codec, profile: QualityProfile::Balanced, width: 1920, height: 1080, fps: 60, keyint_secs: 2.0 }
    }

    #[test]
    fn nvenc_d3d11_zero_copy() {
        let enc = find("hevc_nvenc").unwrap();
        let plan = plan_video(
            enc,
            &params(Codec::Hevc),
            RateControl::Quality,
            FrameSource::D3D11,
            None,
            FilterCaps::default(),
            &HwContext::default(),
        );
        assert_eq!(plan.filter, None, "no download / conversion for NVENC from D3D11");
        let a = plan.encoder_args.join(" ");
        assert!(a.contains("-rc vbr -cq 25 -b:v 0"), "{a}");
        assert!(a.contains("-g 120") && a.contains("-tag:v hvc1"));
    }

    #[test]
    fn nvenc_d3d11_scale_uses_gpu_when_possible() {
        let enc = find("h264_nvenc").unwrap();
        let caps = FilterCaps { scale_d3d11: true, ..Default::default() };
        let plan =
            plan_video(enc, &params(Codec::H264), RateControl::Quality, FrameSource::D3D11, Some((1280, 720)), caps, &HwContext::default());
        assert_eq!(plan.filter.as_deref(), Some("scale_d3d11=width=1280:height=720:format=nv12"));
        let plan = plan_video(
            enc,
            &params(Codec::H264),
            RateControl::Quality,
            FrameSource::D3D11,
            Some((1280, 720)),
            FilterCaps::default(),
            &HwContext::default(),
        );
        assert!(plan.filter.unwrap().starts_with("hwdownload,format=bgra,scale=1280:720"));
    }

    #[test]
    fn qsv_derives_from_d3d11() {
        let enc = find("h264_qsv").unwrap();
        let caps = FilterCaps { scale_d3d11: true, ..Default::default() };
        let hw = HwContext { d3d11_adapter: Some(1), ..Default::default() };
        let plan = plan_video(enc, &params(Codec::H264), RateControl::Quality, FrameSource::D3D11, None, caps, &hw);
        assert_eq!(plan.global_args, vec!["-init_hw_device", "d3d11va=vinci_d3d:1", "-init_hw_device", "qsv=vinci_qsv@vinci_d3d"]);
        assert_eq!(plan.filter.as_deref(), Some("scale_d3d11=format=nv12,hwmap=derive_device=qsv,format=qsv"));
        assert!(plan.encoder_args.join(" ").contains("-global_quality 24"));
    }

    #[test]
    fn vaapi_uploads_rgb_and_converts_on_gpu() {
        let enc = find("h264_vaapi").unwrap();
        let caps = FilterCaps { scale_vaapi: true, ..Default::default() };
        let hw = HwContext { vaapi_device: Some("/dev/dri/renderD129".into()), ..Default::default() };
        let plan = plan_video(enc, &params(Codec::H264), RateControl::Quality, FrameSource::SystemRgb, Some((1280, 720)), caps, &hw);
        assert_eq!(plan.global_args, vec!["-vaapi_device", "/dev/dri/renderD129"]);
        assert_eq!(plan.filter.as_deref(), Some("format=bgr0,hwupload,scale_vaapi=w=1280:h=720:format=nv12"));
        assert!(plan.encoder_args.join(" ").contains("-rc_mode VBR"));
    }

    #[test]
    fn software_fallback() {
        let enc = find("libx264").unwrap();
        let plan = plan_video(
            enc,
            &params(Codec::H264),
            RateControl::Bitrate(3000),
            FrameSource::D3D11,
            None,
            FilterCaps::default(),
            &HwContext::default(),
        );
        assert_eq!(plan.filter.as_deref(), Some("hwdownload,format=bgra,format=yuv420p"));
        let a = plan.encoder_args.join(" ");
        assert!(a.contains("-preset veryfast -b:v 3000k -maxrate 3750k"), "{a}");
    }

    #[test]
    fn amf_uses_peak_vbr() {
        let enc = find("av1_amf").unwrap();
        let a = encoder_args(enc, &params(Codec::Av1), RateControl::Quality).join(" ");
        assert!(a.contains("-rc vbr_peak") && a.contains("-quality balanced"), "{a}");
    }
}
