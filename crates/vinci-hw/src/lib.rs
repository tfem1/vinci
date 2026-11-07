//! # vinci-hw — hardware & capability detection
//!
//! Everything VINCI needs to know about the machine before it starts capturing:
//!
//! * [`gpu`] — physical GPUs (Linux sysfs / Windows DXGI), vendor, VRAM, render node.
//! * [`ffmpeg`] — locating FFmpeg / FFprobe and reading their real capabilities
//!   (encoders, filters such as `gfxcapture`/`ddagrab`, input devices).
//! * [`gstreamer`] — GStreamer element availability (Wayland/PipeWire path on Linux).
//! * [`system`] — OS, Windows build, display server (X11 / Wayland), CPU, RAM.
//! * [`stats`] — a cheap resource sampler (CPU, RAM, GPU utilisation incl. NVENC load).
//!
//! The crate never guesses: capabilities come from the binaries that will actually be
//! executed, and hardware encoders are validated with a real test encode in
//! `vinci-encoding`.

pub mod drivers;
pub mod ffmpeg;
pub mod gpu;
pub mod gstreamer;
pub mod process;
pub mod stats;
pub mod system;

use serde::{Deserialize, Serialize};
use std::path::Path;

pub use ffmpeg::{FfmpegInfo, FfmpegSource};
pub use gpu::{GpuInfo, GpuVendor};
pub use gstreamer::GstInfo;
pub use stats::{GpuUsage, ResourceStats, StatsSampler};
pub use system::{DisplayServer, DistroFamily, SystemInfo};

#[derive(Debug, thiserror::Error)]
pub enum HwError {
    #[error("FFmpeg was not found. Install FFmpeg or set its path in Settings → Advanced. Searched: {0}")]
    FfmpegNotFound(String),
    #[error("FFmpeg at {path} could not be executed: {reason}")]
    FfmpegBroken { path: String, reason: String },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Full snapshot of the machine capabilities, serialisable so it can be cached and
/// shown in the UI ("Settings → Diagnostics").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareReport {
    pub system: SystemInfo,
    pub gpus: Vec<GpuInfo>,
    pub ffmpeg: Option<FfmpegInfo>,
    pub ffmpeg_error: Option<String>,
    pub gstreamer: Option<GstInfo>,
}

impl HardwareReport {
    /// A short fingerprint used to invalidate cached encoder probes when the machine
    /// changes (new GPU, new FFmpeg build, driver update visible through the name).
    pub fn fingerprint(&self) -> String {
        let mut s = String::new();
        for g in &self.gpus {
            s.push_str(&format!("{:04x}:{:04x}:{};", g.vendor_id, g.device_id, g.driver.clone().unwrap_or_default()));
        }
        if let Some(f) = &self.ffmpeg {
            s.push_str(&format!("{}|{}|{}", f.path.display(), f.version, f.encoders.len()));
        }
        s
    }
}

/// Run the complete (non-probing) detection. Takes ~50–200 ms, mostly spent running
/// `ffmpeg -encoders/-filters/-devices`.
pub fn detect(explicit_ffmpeg: Option<&Path>) -> HardwareReport {
    let system = system::detect();
    let gpus = gpu::list_gpus();
    let (ffmpeg, ffmpeg_error) = match ffmpeg::locate(explicit_ffmpeg) {
        Ok(f) => (Some(f), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let gstreamer = if cfg!(target_os = "linux") { gstreamer::detect() } else { None };
    HardwareReport { system, gpus, ffmpeg, ffmpeg_error, gstreamer }
}
