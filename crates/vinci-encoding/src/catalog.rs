//! Codec and encoder catalog.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    pub const ALL: [Codec; 3] = [Codec::H264, Codec::Hevc, Codec::Av1];

    pub fn label(self) -> &'static str {
        match self {
            Codec::H264 => "H.264",
            Codec::Hevc => "H.265 / HEVC",
            Codec::Av1 => "AV1",
        }
    }

    /// Codecs to try, in order, when `self` has no working encoder on this machine.
    pub fn fallback_chain(self) -> &'static [Codec] {
        match self {
            Codec::Av1 => &[Codec::Av1, Codec::Hevc, Codec::H264],
            Codec::Hevc => &[Codec::Hevc, Codec::H264],
            Codec::H264 => &[Codec::H264],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum EncoderFamily {
    /// NVIDIA NVENC (Windows + Linux).
    Nvenc,
    /// AMD AMF (Windows).
    Amf,
    /// Intel Quick Sync (Windows; Linux uses VAAPI for Intel).
    Qsv,
    /// VA-API (Linux: Intel, AMD).
    Vaapi,
    /// CPU fallback (x264 / x265 / SVT-AV1).
    Software,
}

impl EncoderFamily {
    pub fn is_hardware(self) -> bool {
        self != EncoderFamily::Software
    }
    pub fn label(self) -> &'static str {
        match self {
            EncoderFamily::Nvenc => "NVIDIA NVENC",
            EncoderFamily::Amf => "AMD AMF",
            EncoderFamily::Qsv => "Intel Quick Sync",
            EncoderFamily::Vaapi => "VA-API",
            EncoderFamily::Software => "Software (CPU)",
        }
    }
    /// Is this family usable on the platform VINCI was compiled for?
    pub fn on_this_platform(self) -> bool {
        match self {
            EncoderFamily::Nvenc | EncoderFamily::Software => true,
            EncoderFamily::Amf | EncoderFamily::Qsv => cfg!(windows),
            EncoderFamily::Vaapi => cfg!(target_os = "linux"),
        }
    }
    /// Preference rank (lower is better) — hardware first, CPU last.
    pub fn rank(self) -> u8 {
        match self {
            EncoderFamily::Nvenc => 0,
            EncoderFamily::Amf => 1,
            EncoderFamily::Qsv => 2,
            EncoderFamily::Vaapi => 3,
            EncoderFamily::Software => 9,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EncoderInfo {
    /// FFmpeg encoder name.
    pub id: &'static str,
    pub codec: Codec,
    pub family: EncoderFamily,
    pub label: &'static str,
}

macro_rules! enc {
    ($id:literal, $codec:ident, $fam:ident, $label:literal) => {
        EncoderInfo { id: $id, codec: Codec::$codec, family: EncoderFamily::$fam, label: $label }
    };
}

pub static CATALOG: &[EncoderInfo] = &[
    enc!("h264_nvenc", H264, Nvenc, "NVENC H.264"),
    enc!("hevc_nvenc", Hevc, Nvenc, "NVENC HEVC"),
    enc!("av1_nvenc", Av1, Nvenc, "NVENC AV1 (RTX 40+)"),
    enc!("h264_amf", H264, Amf, "AMF H.264"),
    enc!("hevc_amf", Hevc, Amf, "AMF HEVC"),
    enc!("av1_amf", Av1, Amf, "AMF AV1 (RX 7000+)"),
    enc!("h264_qsv", H264, Qsv, "Quick Sync H.264"),
    enc!("hevc_qsv", Hevc, Qsv, "Quick Sync HEVC"),
    enc!("av1_qsv", Av1, Qsv, "Quick Sync AV1 (Arc / Core Ultra)"),
    enc!("h264_vaapi", H264, Vaapi, "VA-API H.264"),
    enc!("hevc_vaapi", Hevc, Vaapi, "VA-API HEVC"),
    enc!("av1_vaapi", Av1, Vaapi, "VA-API AV1"),
    enc!("libx264", H264, Software, "x264 (CPU)"),
    enc!("libx265", Hevc, Software, "x265 (CPU)"),
    enc!("libsvtav1", Av1, Software, "SVT-AV1 (CPU)"),
];

pub fn find(id: &str) -> Option<&'static EncoderInfo> {
    CATALOG.iter().find(|e| e.id == id)
}

pub fn for_codec(codec: Codec) -> impl Iterator<Item = &'static EncoderInfo> {
    CATALOG.iter().filter(move |e| e.codec == codec)
}

/// Encoders compiled into this FFmpeg build and relevant on this platform.
pub fn available_in(ffmpeg: &vinci_hw::FfmpegInfo) -> Vec<&'static EncoderInfo> {
    CATALOG.iter().filter(|e| e.family.on_this_platform() && ffmpeg.has_encoder(e.id)).collect()
}
