//! Hardware report + encoder probes, cached on disk so startup stays instant.

use serde::{Deserialize, Serialize};
use std::path::Path;
use vinci_encoding::{HwContext, ProbeResult};
use vinci_hw::HardwareReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwState {
    pub report: HardwareReport,
    pub probes: Vec<ProbeResult>,
    pub fingerprint: String,
    #[serde(skip)]
    pub hw: HwContext,
}

impl HwState {
    pub fn working_encoders(&self) -> Vec<&ProbeResult> {
        self.probes.iter().filter(|p| p.ok).collect()
    }
}

/// Detect, reusing cached probe results when the machine fingerprint is unchanged.
pub fn load_or_detect(cache: &Path, explicit_ffmpeg: Option<&Path>, force: bool) -> HwState {
    let report = vinci_hw::detect(explicit_ffmpeg);
    // Bump when the probe / live encoder arguments change, so cached results are
    // re-validated with the new options.
    const PROBE_REVISION: u32 = 2;
    let fingerprint = format!("{}#probe{PROBE_REVISION}", report.fingerprint());
    let hw = vinci_encoding::default_hw_context(&report.gpus);
    if !force {
        if let Ok(bytes) = std::fs::read(cache) {
            if let Ok(mut cached) = serde_json::from_slice::<HwState>(&bytes) {
                if cached.fingerprint == fingerprint && !cached.probes.is_empty() {
                    log::info!("using cached encoder probes ({} working)", cached.working_encoders().len());
                    cached.report = report;
                    cached.hw = hw;
                    return cached;
                }
            }
        }
    }
    let probes = match &report.ffmpeg {
        Some(ff) => vinci_encoding::probe_all(ff, &hw),
        None => Vec::new(),
    };
    let state = HwState { report, probes, fingerprint, hw };
    if state.report.ffmpeg.is_some() {
        if let Err(e) = vinci_storage::save_json(cache, &state) {
            log::warn!("cannot cache hardware report: {e}");
        }
    }
    state
}
