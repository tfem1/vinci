//! # vinci-encoding — encoding & compression engine
//!
//! * [`catalog`] — codecs (H.264 / HEVC / AV1) and encoders (NVENC, AMF, Quick Sync,
//!   VA-API, CPU) with preference ranking.
//! * [`profile`] — Quality / Balanced / Compact profiles and the bitrate model.
//! * [`args`] — FFmpeg arguments that keep frames on the GPU from capture to encoder.
//! * [`probe`] — real test-encode validation of every encoder + best-encoder selection.
//! * [`transcode`] — compression to a profile or to a target size, lossless trimming.

pub mod args;
pub mod catalog;
pub mod probe;
pub mod profile;
pub mod transcode;

pub use args::{audio_args, plan_video, EncodePlan, FilterCaps, FrameSource, HwContext, RateControl};
pub use catalog::{Codec, EncoderFamily, EncoderInfo, CATALOG};
pub use probe::{probe_all, probe_encoder, select, select_offline, ProbeResult};
pub use profile::{QualityProfile, VideoParams};
pub use transcode::{CompressOptions, CompressTarget, MediaInfo};

/// Choose the VA-API render node: prefer AMD, then Intel; NVIDIA uses NVENC instead.
pub fn default_hw_context(gpus: &[vinci_hw::GpuInfo]) -> HwContext {
    use vinci_hw::GpuVendor;
    let mut ranked: Vec<&vinci_hw::GpuInfo> = gpus.iter().filter(|g| g.render_node.is_some() && g.vendor != GpuVendor::Nvidia).collect();
    ranked.sort_by_key(|g| match g.vendor {
        GpuVendor::Amd => 0,
        GpuVendor::Intel => 1,
        _ => 2,
    });
    HwContext { vaapi_device: ranked.first().and_then(|g| g.render_node.clone()), d3d11_adapter: gpus.first().map(|g| g.index) }
}
