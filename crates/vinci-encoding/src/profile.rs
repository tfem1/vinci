//! Quality profiles and bitrate model.
//!
//! VINCI uses *quality-targeted* rate control (CQ / CRF / ICQ) wherever the encoder
//! supports it, with a peak-bitrate cap so the replay buffer size stays bounded. The
//! cap and the "estimated size" shown in the UI come from the model below.

use crate::catalog::{Codec, EncoderFamily};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QualityProfile {
    /// Visually lossless for fast-paced games.
    Quality,
    /// Default — excellent quality, ~half the size of typical H.264 recorders.
    #[default]
    Balanced,
    /// Smallest files, still sharp at 1080p (ideal for sharing).
    Compact,
}

impl QualityProfile {
    pub const ALL: [QualityProfile; 3] = [QualityProfile::Quality, QualityProfile::Balanced, QualityProfile::Compact];
    fn idx(self) -> usize {
        match self {
            QualityProfile::Quality => 0,
            QualityProfile::Balanced => 1,
            QualityProfile::Compact => 2,
        }
    }
    /// Preferred codec when the user leaves codec on "Auto".
    pub fn preferred_codec(self) -> Codec {
        match self {
            QualityProfile::Quality | QualityProfile::Balanced => Codec::Hevc,
            QualityProfile::Compact => Codec::Av1,
        }
    }
}

/// Parameters of the video stream being produced.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VideoParams {
    pub codec: Codec,
    pub profile: QualityProfile,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Keyframe interval in seconds — also the cut granularity of saved clips.
    pub keyint_secs: f32,
}

impl VideoParams {
    pub fn gop(&self) -> u32 {
        ((self.fps as f32) * self.keyint_secs).round().max(1.0) as u32
    }
}

/// Reference bitrates (kbit/s) at 1920×1080 @ 60 fps, indexed [profile].
fn base_kbps(codec: Codec) -> [u32; 3] {
    match codec {
        Codec::H264 => [22_000, 14_000, 8_000],
        Codec::Hevc => [16_000, 9_000, 5_500],
        Codec::Av1 => [13_000, 7_500, 4_500],
    }
}

/// Average target bitrate in kbit/s for a stream.
pub fn target_kbps(codec: Codec, profile: QualityProfile, width: u32, height: u32, fps: u32) -> u32 {
    let px = (width.max(16) as f64 * height.max(16) as f64) / (1920.0 * 1080.0);
    let fr = fps.max(1) as f64 / 60.0;
    let k = base_kbps(codec)[profile.idx()] as f64 * px.powf(0.75) * fr.powf(0.6);
    (k.round() as u32).max(500)
}

/// Peak bitrate cap (kbit/s).
pub fn max_kbps(codec: Codec, profile: QualityProfile, width: u32, height: u32, fps: u32) -> u32 {
    let t = target_kbps(codec, profile, width, height, fps) as f64;
    let mult = match profile {
        QualityProfile::Quality => 2.0,
        QualityProfile::Balanced => 1.7,
        QualityProfile::Compact => 1.5,
    };
    (t * mult) as u32
}

pub fn audio_kbps(profile: QualityProfile) -> u32 {
    match profile {
        QualityProfile::Quality => 192,
        QualityProfile::Balanced => 160,
        QualityProfile::Compact => 128,
    }
}

/// Quality value for the encoder family (CQ for NVENC, ICQ for QSV, CRF for CPU).
pub fn quality_value(family: EncoderFamily, codec: Codec, profile: QualityProfile) -> u32 {
    let i = profile.idx();
    let table: [u32; 3] = match (family, codec) {
        (EncoderFamily::Nvenc, Codec::H264) => [19, 23, 27],
        (EncoderFamily::Nvenc, Codec::Hevc) => [21, 25, 29],
        (EncoderFamily::Nvenc, Codec::Av1) => [28, 34, 40], // 0–63 scale
        (EncoderFamily::Qsv, Codec::H264) => [20, 24, 28],
        (EncoderFamily::Qsv, _) => [21, 25, 29],
        (EncoderFamily::Software, Codec::H264) => [20, 23, 26],
        (EncoderFamily::Software, Codec::Hevc) => [22, 25, 28],
        (EncoderFamily::Software, Codec::Av1) => [28, 33, 38],
        // AMF and VA-API use peak-constrained VBR; value unused but kept consistent.
        _ => [20, 24, 28],
    };
    table[i]
}

/// Estimated storage in MB per minute of footage (video + audio).
pub fn estimate_mb_per_minute(codec: Codec, profile: QualityProfile, width: u32, height: u32, fps: u32) -> f64 {
    let kbps = target_kbps(codec, profile, width, height, fps) + audio_kbps(profile);
    kbps as f64 * 60.0 / 8.0 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_is_monotonic() {
        for c in Codec::ALL {
            let q = target_kbps(c, QualityProfile::Quality, 1920, 1080, 60);
            let b = target_kbps(c, QualityProfile::Balanced, 1920, 1080, 60);
            let s = target_kbps(c, QualityProfile::Compact, 1920, 1080, 60);
            assert!(q > b && b > s);
            assert!(target_kbps(c, QualityProfile::Balanced, 3840, 2160, 60) > b);
            assert!(target_kbps(c, QualityProfile::Balanced, 1280, 720, 30) < b);
            assert!(max_kbps(c, QualityProfile::Balanced, 1920, 1080, 60) > b);
        }
        // HEVC/AV1 are always smaller than H.264 for the same profile.
        assert!(
            target_kbps(Codec::Hevc, QualityProfile::Balanced, 1920, 1080, 60)
                < target_kbps(Codec::H264, QualityProfile::Balanced, 1920, 1080, 60)
        );
    }

    #[test]
    fn gop() {
        let p = VideoParams { codec: Codec::H264, profile: QualityProfile::Balanced, width: 1920, height: 1080, fps: 60, keyint_secs: 2.0 };
        assert_eq!(p.gop(), 120);
        assert!((estimate_mb_per_minute(Codec::Hevc, QualityProfile::Balanced, 1920, 1080, 60) - 68.7).abs() < 1.0);
    }
}
