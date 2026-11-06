//! User settings (persisted as JSON in the config directory).
//!
//! Every struct uses `#[serde(default)]`, so settings files from older versions load
//! with new fields defaulted, and unknown fields from newer versions are ignored.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vinci_capture::{CaptureConfig, Priority};
use vinci_encoding::{Codec, QualityProfile};
use vinci_games::GameEntry;
use vinci_hotkeys::{Binding, HotkeyAction};
use vinci_update::Channel;

pub const SETTINGS_VERSION: u32 = 1;
pub const MIN_BUFFER_SECS: u32 = 15;
pub const MAX_BUFFER_SECS: u32 = 600;
pub const CLIP_PRESETS: [u32; 5] = [30, 60, 120, 180, 300];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CodecChoice {
    #[default]
    Auto,
    H264,
    Hevc,
    Av1,
}

impl CodecChoice {
    pub fn resolve(self, profile: QualityProfile) -> Codec {
        match self {
            CodecChoice::Auto => profile.preferred_codec(),
            CodecChoice::H264 => Codec::H264,
            CodecChoice::Hevc => Codec::Hevc,
            CodecChoice::Av1 => Codec::Av1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BufferStorage {
    /// RAM when the estimated buffer fits under `max_ram_mb`, otherwise disk.
    #[default]
    Auto,
    Memory,
    Disk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub start_replay_on_launch: bool,
    /// Start the replay buffer automatically when a game is detected.
    pub auto_replay_on_game: bool,
    pub close_to_tray: bool,
    pub first_run_done: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            launch_at_startup: false,
            start_minimized: false,
            start_replay_on_launch: true,
            auto_replay_on_game: false,
            close_to_tray: true,
            first_run_done: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EncodingSettings {
    pub codec: CodecChoice,
    /// FFmpeg encoder id, `None` = best detected.
    pub encoder: Option<String>,
    pub profile: QualityProfile,
    pub keyframe_interval_secs: f32,
    /// Allow falling back to CPU encoding when no hardware encoder works.
    pub allow_software: bool,
}

impl Default for EncodingSettings {
    fn default() -> Self {
        Self {
            codec: CodecChoice::Auto,
            encoder: None,
            profile: QualityProfile::Balanced,
            keyframe_interval_secs: 2.0,
            allow_software: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplaySettings {
    /// Length of the rolling buffer (15 s – 10 min).
    pub buffer_secs: u32,
    pub storage: BufferStorage,
    /// Length saved by the main hotkey.
    pub default_clip_secs: u32,
    pub max_ram_mb: u32,
}

impl Default for ReplaySettings {
    fn default() -> Self {
        Self { buffer_secs: 120, storage: BufferStorage::Auto, default_clip_secs: 30, max_ram_mb: 512 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibrarySettings {
    pub clips_dir: Option<PathBuf>,
    pub organize_by_game: bool,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        Self { clips_dir: None, organize_by_game: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeySettings {
    pub save_default: Option<String>,
    pub save_30: Option<String>,
    pub save_60: Option<String>,
    pub save_120: Option<String>,
    pub save_180: Option<String>,
    pub save_300: Option<String>,
    pub toggle_replay: Option<String>,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            save_default: Some("Alt+F10".into()),
            save_30: None,
            save_60: None,
            save_120: None,
            save_180: None,
            save_300: None,
            toggle_replay: Some("Alt+F9".into()),
        }
    }
}

impl HotkeySettings {
    pub fn bindings(&self) -> Vec<Binding> {
        let mut v = Vec::new();
        let mut add = |id: &str, acc: &Option<String>, action: HotkeyAction, desc: &str| {
            if let Some(a) = acc.as_ref().filter(|a| !a.trim().is_empty()) {
                v.push(Binding { id: id.into(), accelerator: a.clone(), action, description: desc.into() });
            }
        };
        add("save_default", &self.save_default, HotkeyAction::SaveClip { secs: None }, "Save clip (default length)");
        add("save_30", &self.save_30, HotkeyAction::SaveClip { secs: Some(30) }, "Save last 30 seconds");
        add("save_60", &self.save_60, HotkeyAction::SaveClip { secs: Some(60) }, "Save last minute");
        add("save_120", &self.save_120, HotkeyAction::SaveClip { secs: Some(120) }, "Save last 2 minutes");
        add("save_180", &self.save_180, HotkeyAction::SaveClip { secs: Some(180) }, "Save last 3 minutes");
        add("save_300", &self.save_300, HotkeyAction::SaveClip { secs: Some(300) }, "Save last 5 minutes");
        add("toggle_replay", &self.toggle_replay, HotkeyAction::ToggleReplay, "Start / stop the replay buffer");
        v
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlaySettings {
    pub enabled: bool,
    pub position: Corner,
    /// Small persistent "REPLAY" dot while buffering.
    pub show_indicator: bool,
    pub show_toasts: bool,
    pub opacity: f32,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self { enabled: true, position: Corner::TopRight, show_indicator: false, show_toasts: true, opacity: 0.94 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PerformanceSettings {
    /// Caps capture at 30 fps, uses the Compact profile and slows background polling.
    pub resource_saver: bool,
    pub stats_interval_ms: u32,
    /// Destroy the UI webview when the window is hidden (frees ~60–150 MB).
    pub free_ui_when_hidden: bool,
    pub capture_priority: Priority,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            resource_saver: false,
            stats_interval_ms: 2000,
            free_ui_when_hidden: true,
            capture_priority: if cfg!(windows) { Priority::AboveNormal } else { Priority::Normal },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    pub auto_check: bool,
    pub channel: Channel,
    pub last_check: Option<i64>,
    pub skipped_version: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { auto_check: true, channel: Channel::Stable, last_check: None, skipped_version: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedSettings {
    pub ffmpeg_path: Option<PathBuf>,
    pub audio_offset_ms: i32,
    pub fragment_ms: u32,
    pub log_level: String,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self { ffmpeg_path: None, audio_offset_ms: 0, fragment_ms: 500, log_level: "info".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub general: GeneralSettings,
    pub capture: CaptureConfig,
    pub encoding: EncodingSettings,
    pub replay: ReplaySettings,
    pub library: LibrarySettings,
    pub hotkeys: HotkeySettings,
    pub overlay: OverlaySettings,
    pub performance: PerformanceSettings,
    pub updates: UpdateSettings,
    pub advanced: AdvancedSettings,
    pub custom_games: Vec<GameEntry>,
    /// Wayland portal restore token (screen chosen once).
    pub portal_restore_token: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            general: Default::default(),
            capture: Default::default(),
            encoding: Default::default(),
            replay: Default::default(),
            library: Default::default(),
            hotkeys: Default::default(),
            overlay: Default::default(),
            performance: Default::default(),
            updates: Default::default(),
            advanced: Default::default(),
            custom_games: Vec::new(),
            portal_restore_token: None,
        }
    }
}

impl Settings {
    /// Clamp values coming from the UI / an edited file into safe ranges.
    pub fn sanitize(&mut self) {
        self.version = SETTINGS_VERSION;
        self.replay.buffer_secs = self.replay.buffer_secs.clamp(MIN_BUFFER_SECS, MAX_BUFFER_SECS);
        self.replay.default_clip_secs = self.replay.default_clip_secs.clamp(5, self.replay.buffer_secs);
        self.replay.max_ram_mb = self.replay.max_ram_mb.clamp(64, 16_384);
        self.capture.fps = match self.capture.fps {
            0..=24 => 24,
            f if f > 240 => 240,
            f => f,
        };
        self.encoding.keyframe_interval_secs = self.encoding.keyframe_interval_secs.clamp(0.5, 5.0);
        self.advanced.fragment_ms = self.advanced.fragment_ms.clamp(100, 2000);
        self.advanced.audio_offset_ms = self.advanced.audio_offset_ms.clamp(-2000, 2000);
        self.performance.stats_interval_ms = self.performance.stats_interval_ms.clamp(500, 30_000);
        self.overlay.opacity = self.overlay.opacity.clamp(0.2, 1.0);
        for v in [&mut self.capture.audio.system_volume, &mut self.capture.audio.mic_volume] {
            *v = v.clamp(0.0, 4.0);
        }
    }

    pub fn effective_fps(&self) -> u32 {
        if self.performance.resource_saver {
            self.capture.fps.min(30)
        } else {
            self.capture.fps
        }
    }

    pub fn effective_profile(&self) -> QualityProfile {
        if self.performance.resource_saver {
            QualityProfile::Compact
        } else {
            self.encoding.profile
        }
    }

    /// Does switching from `old` to `self` require restarting the capture pipeline?
    pub fn capture_changed(&self, old: &Settings) -> bool {
        self.capture != old.capture
            || self.encoding != old.encoding
            || self.effective_fps() != old.effective_fps()
            || self.effective_profile() != old.effective_profile()
            || self.replay.storage != old.replay.storage
            || self.replay.max_ram_mb != old.replay.max_ram_mb
            || self.advanced.ffmpeg_path != old.advanced.ffmpeg_path
            || self.advanced.audio_offset_ms != old.advanced.audio_offset_ms
            || self.advanced.fragment_ms != old.advanced.fragment_ms
            || self.performance.capture_priority != old.performance.capture_priority
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_forward_compat() {
        let s: Settings = serde_json::from_str(r#"{"replay":{"buffer_secs":300},"unknown_future_field":1}"#).unwrap();
        assert_eq!(s.replay.buffer_secs, 300);
        assert_eq!(s.replay.default_clip_secs, 30);
        assert_eq!(s.hotkeys.save_default.as_deref(), Some("Alt+F10"));
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn sanitize_and_change_detection() {
        let mut s = Settings::default();
        s.replay.buffer_secs = 99999;
        s.replay.default_clip_secs = 5000;
        s.capture.fps = 1000;
        s.sanitize();
        assert_eq!((s.replay.buffer_secs, s.replay.default_clip_secs, s.capture.fps), (MAX_BUFFER_SECS, MAX_BUFFER_SECS, 240));
        let mut t = s.clone();
        t.replay.buffer_secs = 60;
        assert!(!t.capture_changed(&s), "buffer length is applied live");
        t.capture.fps = 30;
        assert!(t.capture_changed(&s));
        let mut r = s.clone();
        r.performance.resource_saver = true;
        assert_eq!(r.effective_fps(), 30);
        assert!(r.capture_changed(&s));
        assert_eq!(Settings::default().hotkeys.bindings().len(), 2);
    }
}
