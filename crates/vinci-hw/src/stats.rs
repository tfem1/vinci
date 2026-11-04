//! Lightweight resource sampler.
//!
//! Designed to be polled every 1–5 s while the UI is visible (and not at all when it
//! is hidden). Only the VINCI process and its helper children (FFmpeg/GStreamer) are
//! refreshed, never the whole process table.

use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GpuUsage {
    pub name: String,
    /// 3D/graphics engine utilisation in percent.
    pub util_percent: Option<f32>,
    /// Hardware video encoder utilisation (NVENC / Windows VideoEncode engine).
    pub encoder_percent: Option<f32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceStats {
    /// CPU used by VINCI + its capture/encode helpers, normalised to 0–100 % of the machine.
    pub app_cpu_percent: f32,
    /// Resident memory of VINCI + helpers.
    pub app_ram_bytes: u64,
    pub sys_cpu_percent: f32,
    pub sys_ram_used_bytes: u64,
    pub sys_ram_total_bytes: u64,
    pub gpus: Vec<GpuUsage>,
}

pub struct StatsSampler {
    sys: System,
    own: Pid,
    tracked: Vec<Pid>,
    threads: f32,
    gpu: GpuMonitor,
}

impl Default for StatsSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl StatsSampler {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_cpu_usage()).with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        let threads = sys.cpus().len().max(1) as f32;
        Self { sys, own: Pid::from_u32(std::process::id()), tracked: Vec::new(), threads, gpu: GpuMonitor::new() }
    }

    /// Child processes (FFmpeg / GStreamer) whose usage is attributed to VINCI.
    pub fn set_tracked_pids(&mut self, pids: &[u32]) {
        self.tracked = pids.iter().map(|p| Pid::from_u32(*p)).collect();
    }

    pub fn sample(&mut self) -> ResourceStats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        let mut pids = vec![self.own];
        pids.extend(self.tracked.iter().copied());
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        let (mut cpu, mut ram) = (0.0f32, 0u64);
        for pid in &pids {
            if let Some(p) = self.sys.process(*pid) {
                cpu += p.cpu_usage();
                ram += p.memory();
            }
        }
        ResourceStats {
            app_cpu_percent: (cpu / self.threads).clamp(0.0, 100.0),
            app_ram_bytes: ram,
            sys_cpu_percent: self.sys.global_cpu_usage(),
            sys_ram_used_bytes: self.sys.used_memory(),
            sys_ram_total_bytes: self.sys.total_memory(),
            gpus: self.gpu.sample(),
        }
    }
}

/// GPU utilisation from the best available source:
/// NVML (NVIDIA, Windows+Linux) → amdgpu sysfs (Linux) → PDH "GPU Engine" counters (Windows).
pub struct GpuMonitor {
    #[cfg(feature = "nvml")]
    nvml: Option<nvml_wrapper::Nvml>,
    #[cfg(target_os = "linux")]
    sysfs: Vec<(String, std::path::PathBuf)>,
    #[cfg(windows)]
    pdh: Option<pdh::PdhGpu>,
}

impl Default for GpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuMonitor {
    pub fn new() -> Self {
        #[cfg(feature = "nvml")]
        let nvml = nvml_wrapper::Nvml::init().ok();
        #[cfg(target_os = "linux")]
        let sysfs = crate::gpu::list_gpus()
            .into_iter()
            .filter(|g| g.driver.as_deref() == Some("amdgpu"))
            .map(|g| (g.name, std::path::PathBuf::from(format!("/sys/class/drm/card{}/device", g.index))))
            .collect();
        #[cfg(windows)]
        let pdh = {
            #[cfg(feature = "nvml")]
            let have_nvml = nvml.is_some();
            #[cfg(not(feature = "nvml"))]
            let have_nvml = false;
            if have_nvml { None } else { pdh::PdhGpu::new() }
        };
        Self {
            #[cfg(feature = "nvml")]
            nvml,
            #[cfg(target_os = "linux")]
            sysfs,
            #[cfg(windows)]
            pdh,
        }
    }

    pub fn sample(&mut self) -> Vec<GpuUsage> {
        #[allow(unused_mut)]
        let mut out = Vec::new();
        #[cfg(feature = "nvml")]
        if let Some(nvml) = &self.nvml {
            let count = nvml.device_count().unwrap_or(0);
            for i in 0..count {
                if let Ok(d) = nvml.device_by_index(i) {
                    let mem = d.memory_info().ok();
                    out.push(GpuUsage {
                        name: d.name().unwrap_or_else(|_| "NVIDIA GPU".into()),
                        util_percent: d.utilization_rates().ok().map(|u| u.gpu as f32),
                        encoder_percent: d.encoder_utilization().ok().map(|u| u.utilization as f32),
                        vram_used_bytes: mem.as_ref().map(|m| m.used),
                        vram_total_bytes: mem.as_ref().map(|m| m.total),
                    });
                }
            }
        }
        #[cfg(target_os = "linux")]
        for (name, dev) in &self.sysfs {
            let read = |f: &str| std::fs::read_to_string(dev.join(f)).ok().and_then(|s| s.trim().parse::<u64>().ok());
            out.push(GpuUsage {
                name: name.clone(),
                util_percent: read("gpu_busy_percent").map(|v| v as f32),
                encoder_percent: None,
                vram_used_bytes: read("mem_info_vram_used"),
                vram_total_bytes: read("mem_info_vram_total"),
            });
        }
        #[cfg(windows)]
        if let Some(p) = &mut self.pdh {
            if let Some(u) = p.sample() {
                out.push(u);
            }
        }
        out
    }
}

#[cfg(windows)]
mod pdh {
    //! Windows Performance Data Helper: `\GPU Engine(*engtype_3D)\Utilization Percentage`
    //! and `…engtype_VideoEncode…` — the same counters Task Manager uses.
    use super::GpuUsage;
    use windows::core::PCWSTR;
    use windows::Win32::System::Performance::*;

    pub struct PdhGpu {
        query: PDH_HQUERY,
        three_d: PDH_HCOUNTER,
        encode: PDH_HCOUNTER,
        name: String,
    }

    // PDH handles are plain opaque handles that are only used from the sampling thread.
    unsafe impl Send for PdhGpu {}

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    impl PdhGpu {
        pub fn new() -> Option<Self> {
            unsafe {
                let mut query = PDH_HQUERY::default();
                if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                    return None;
                }
                let mut three_d = PDH_HCOUNTER::default();
                let mut encode = PDH_HCOUNTER::default();
                let p1 = wide(r"\GPU Engine(*engtype_3D)\Utilization Percentage");
                let p2 = wide(r"\GPU Engine(*engtype_VideoEncode)\Utilization Percentage");
                if PdhAddEnglishCounterW(query, PCWSTR(p1.as_ptr()), 0, &mut three_d) != 0 {
                    let _ = PdhCloseQuery(query);
                    return None;
                }
                let _ = PdhAddEnglishCounterW(query, PCWSTR(p2.as_ptr()), 0, &mut encode);
                let _ = PdhCollectQueryData(query);
                let name = crate::gpu::list_gpus().first().map(|g| g.name.clone()).unwrap_or_else(|| "GPU".into());
                Some(Self { query, three_d, encode, name })
            }
        }

        fn sum(&self, counter: PDH_HCOUNTER) -> Option<f32> {
            if counter.is_invalid() {
                return None;
            }
            unsafe {
                let mut size = 0u32;
                let mut count = 0u32;
                let _ = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
                if size == 0 {
                    return None;
                }
                let mut buf = vec![0u8; size as usize];
                let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
                if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
                    return None;
                }
                let slice = std::slice::from_raw_parts(items, count as usize);
                let total: f64 = slice.iter().map(|i| i.FmtValue.Anonymous.doubleValue).sum();
                Some(total.clamp(0.0, 100.0) as f32)
            }
        }

        pub fn sample(&mut self) -> Option<GpuUsage> {
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
            }
            Some(GpuUsage {
                name: self.name.clone(),
                util_percent: self.sum(self.three_d),
                encoder_percent: self.sum(self.encode),
                vram_used_bytes: None,
                vram_total_bytes: None,
            })
        }
    }

    impl Drop for PdhGpu {
        fn drop(&mut self) {
            unsafe {
                let _ = PdhCloseQuery(self.query);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sampler_reports_own_process() {
        let mut s = super::StatsSampler::new();
        let _ = s.sample();
        std::thread::sleep(std::time::Duration::from_millis(250));
        let st = s.sample();
        assert!(st.app_ram_bytes > 0, "own RSS should be visible");
        assert!(st.sys_ram_total_bytes >= st.sys_ram_used_bytes);
    }
}
