//! Engine status and events (serialisable for the UI / CLI).

use serde::Serialize;
use vinci_capture::SessionInfo;
use vinci_clips::Clip;
use vinci_games::DetectedGame;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReplayState {
    #[default]
    Stopped,
    Starting,
    Running,
    /// The pipeline stopped unexpectedly and is being restarted.
    Restarting,
    Error,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct EngineStatus {
    pub state: ReplayState,
    pub message: Option<String>,
    pub buffered_secs: f64,
    pub buffer_target_secs: u32,
    pub buffer_bytes: u64,
    /// `memory` or `disk`.
    pub buffer_storage: String,
    pub session: Option<SessionInfo>,
    /// Epoch ms when the current session started.
    pub since_ms: Option<i64>,
    pub restarts: u32,
    pub game: Option<DetectedGame>,
    pub saving: bool,
    /// Set when VINCI had to use another codec/encoder than requested.
    pub fallback_note: Option<String>,
    pub last_clip_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    Status(EngineStatus),
    ClipSaved { clip: Clip, remuxed: bool, elapsed_ms: u64 },
    ClipFailed { message: String },
    GameChanged { game: Option<DetectedGame> },
    JobProgress { job_id: u64, clip_id: i64, kind: String, progress: f32 },
    JobDone { job_id: u64, clip_id: i64, kind: String, clip: Option<Clip>, error: Option<String> },
    LibraryChanged,
    HotkeyPressed { action: vinci_hotkeys::HotkeyAction },
}
