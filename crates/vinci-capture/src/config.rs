//! User-facing capture configuration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// Best native backend for this platform/session.
    #[default]
    Auto,
    /// Windows.Graphics.Capture (Windows 10 1903+, FFmpeg 8.1 `gfxcapture`).
    Wgc,
    /// DXGI Desktop Duplication (`ddagrab`) — captures fullscreen-exclusive games.
    Dda,
    /// X11 shared-memory capture (`x11grab`).
    X11,
    /// xdg-desktop-portal ScreenCast + PipeWire (Wayland).
    Portal,
    /// Synthetic test pattern + tone (CI, demos, troubleshooting).
    Test,
}

impl BackendKind {
    pub fn label(self) -> &'static str {
        match self {
            BackendKind::Auto => "Auto",
            BackendKind::Wgc => "Windows Graphics Capture",
            BackendKind::Dda => "Desktop Duplication",
            BackendKind::X11 => "X11 (XShm)",
            BackendKind::Portal => "PipeWire portal",
            BackendKind::Test => "Test pattern",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureTarget {
    /// A whole monitor, by index in [`crate::monitors::list_monitors`].
    Monitor { index: u32 },
    /// A single game window by executable name (WGC only; falls back to the monitor).
    Window { exe: String },
    /// Follow the detected game's window when possible, otherwise the primary monitor.
    FollowGame,
}

impl Default for CaptureTarget {
    fn default() -> Self {
        CaptureTarget::Monitor { index: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    #[default]
    Native,
    P720,
    P1080,
    P1440,
    P2160,
}

impl Resolution {
    pub fn max_height(self) -> Option<u32> {
        match self {
            Resolution::Native => None,
            Resolution::P720 => Some(720),
            Resolution::P1080 => Some(1080),
            Resolution::P1440 => Some(1440),
            Resolution::P2160 => Some(2160),
        }
    }

    /// Output size for a source of `w`×`h` (never upscales, keeps even dimensions).
    pub fn output_size(self, w: u32, h: u32) -> (u32, u32) {
        let even = |v: u32| (v / 2 * 2).max(2);
        match self.max_height() {
            Some(mh) if h > mh => (even(((w as u64 * mh as u64) / h as u64) as u32), even(mh)),
            _ => (even(w), even(h)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Capture what you hear (desktop / game audio).
    pub system: bool,
    pub microphone: bool,
    /// Platform device id (PulseAudio source name / WASAPI endpoint id); `None` = default.
    pub system_device: Option<String>,
    pub mic_device: Option<String>,
    pub system_volume: f32,
    pub mic_volume: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { system: true, microphone: false, system_device: None, mic_device: None, system_volume: 1.0, mic_volume: 1.0 }
    }
}

impl AudioConfig {
    pub fn any(&self) -> bool {
        self.system || self.microphone
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    pub backend: BackendKind,
    pub target: CaptureTarget,
    pub resolution: Resolution,
    pub fps: u32,
    pub cursor: bool,
    pub audio: AudioConfig,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self { backend: BackendKind::Auto, target: CaptureTarget::default(), resolution: Resolution::Native, fps: 60, cursor: true, audio: AudioConfig::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_sizes() {
        assert_eq!(Resolution::Native.output_size(2560, 1440), (2560, 1440));
        assert_eq!(Resolution::P1080.output_size(2560, 1440), (1920, 1080));
        assert_eq!(Resolution::P1080.output_size(1280, 720), (1280, 720), "never upscale");
        assert_eq!(Resolution::P720.output_size(3440, 1440), (1720, 720), "ultrawide keeps aspect");
        assert_eq!(Resolution::P720.output_size(1366, 768), (1280, 720));
        let cfg: CaptureConfig = serde_json::from_str(r#"{"fps":120}"#).unwrap();
        assert_eq!(cfg.fps, 120);
        assert!(cfg.audio.system);
    }
}
