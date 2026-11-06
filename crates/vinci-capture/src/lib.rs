//! # vinci-capture — native capture engine
//!
//! Builds and supervises the capture → encode pipeline for each platform:
//!
//! * **Windows** — Windows.Graphics.Capture (`gfxcapture`, FFmpeg ≥ 8.1) or DXGI
//!   Desktop Duplication (`ddagrab`). Frames stay in D3D11 textures and are handed to
//!   NVENC/AMF directly (QSV through `hwmap`), audio comes from a native WASAPI
//!   loopback + microphone mixer.
//! * **Linux X11** — `x11grab` (XShm) + PulseAudio/PipeWire-Pulse monitor source, GPU
//!   colour conversion for VA-API.
//! * **Linux Wayland** — xdg-desktop-portal ScreenCast → PipeWire fd → GStreamer
//!   `pipewiresrc` → VA/NV/x264 → copy-remux to fragmented MP4.
//!
//! See [`pipeline`] for the exact process graphs.

pub mod config;
pub mod deps;
pub mod devices;
pub mod monitors;
pub mod pipeline;
#[cfg(target_os = "linux")]
pub mod portal;
pub mod session;
#[cfg(windows)]
pub mod wasapi;

pub use config::{AudioConfig, BackendKind, CaptureConfig, CaptureTarget, Resolution};
pub use monitors::{list_monitors, MonitorInfo};
pub use pipeline::{resolve_backend, PipelineSpec, Priority};
pub use session::{CaptureSession, SessionInfo};

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("no capture backend available: {0}")]
    NoBackend(String),
    #[error("could not start capture process: {0}")]
    Spawn(String),
    #[error("screen sharing portal: {0}")]
    Portal(String),
    #[error("audio capture: {0}")]
    Audio(String),
    #[error("no monitor found")]
    NoMonitor,
    /// Something must be installed first; `command` installs it on this distribution.
    #[error("{message}")]
    MissingDeps { message: String, command: Option<String> },
}
