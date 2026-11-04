//! Physical GPU enumeration.
//!
//! * Linux: `/sys/class/drm/cardN/device` (PCI vendor/device ids, kernel driver,
//!   DRM render node used by VAAPI, VRAM for amdgpu).
//! * Windows: DXGI adapters (`IDXGIFactory1::EnumAdapters1`), software adapters skipped.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

impl GpuVendor {
    pub fn from_pci_id(id: u32) -> Self {
        match id {
            0x10de => GpuVendor::Nvidia,
            0x1002 | 0x1022 => GpuVendor::Amd,
            0x8086 => GpuVendor::Intel,
            _ => GpuVendor::Other,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            GpuVendor::Nvidia => "NVIDIA",
            GpuVendor::Amd => "AMD",
            GpuVendor::Intel => "Intel",
            GpuVendor::Other => "GPU",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    /// Adapter index (DXGI index on Windows, DRM card number on Linux).
    pub index: u32,
    pub vendor: GpuVendor,
    pub vendor_id: u32,
    pub device_id: u32,
    pub name: String,
    pub vram_bytes: Option<u64>,
    /// Kernel driver (`nvidia`, `amdgpu`, `i915`, `xe`, …) — Linux only.
    pub driver: Option<String>,
    /// DRM render node (`/dev/dri/renderD128`) used for VAAPI — Linux only.
    pub render_node: Option<PathBuf>,
}

pub fn list_gpus() -> Vec<GpuInfo> {
    #[cfg(target_os = "linux")]
    {
        linux::list()
    }
    #[cfg(windows)]
    {
        windows_impl::list()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn read_hex(p: &Path) -> Option<u32> {
        let s = fs::read_to_string(p).ok()?;
        u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
    }

    pub fn list() -> Vec<GpuInfo> {
        list_from(Path::new("/sys/class/drm"))
    }

    pub(crate) fn list_from(root: &Path) -> Vec<GpuInfo> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let Ok(entries) = fs::read_dir(root) else { return out };
        let mut names: Vec<String> = entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in names {
            let Some(num) = name.strip_prefix("card") else { continue };
            let Ok(index) = num.parse::<u32>() else { continue }; // skips card0-HDMI-A-1 connectors
            let dev = root.join(&name).join("device");
            let Some(vendor_id) = read_hex(&dev.join("vendor")) else { continue };
            let device_id = read_hex(&dev.join("device")).unwrap_or(0);
            let pci = fs::canonicalize(&dev).ok().and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()));
            if let Some(p) = &pci {
                if !seen.insert(p.clone()) {
                    continue;
                }
            }
            let driver = fs::read_link(dev.join("driver")).ok().and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()));
            let render_node = fs::read_dir(dev.join("drm")).ok().and_then(|it| {
                it.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .find(|n| n.starts_with("renderD"))
                    .map(|n| PathBuf::from("/dev/dri").join(n))
            });
            let vram_bytes = fs::read_to_string(dev.join("mem_info_vram_total")).ok().and_then(|s| s.trim().parse().ok());
            let vendor = GpuVendor::from_pci_id(vendor_id);
            let name = nvidia_model(pci.as_deref())
                .or_else(|| pci_ids_name(vendor_id, device_id))
                .unwrap_or_else(|| format!("{} GPU ({:04x})", vendor.label(), device_id));
            out.push(GpuInfo { index, vendor, vendor_id, device_id, name, vram_bytes, driver, render_node });
        }
        out
    }

    fn nvidia_model(pci: Option<&str>) -> Option<String> {
        let pci = pci?;
        let info = fs::read_to_string(format!("/proc/driver/nvidia/gpus/{pci}/information")).ok()?;
        info.lines().find_map(|l| l.strip_prefix("Model:").map(|m| m.trim().to_string()))
    }

    /// Look the device up in the system `pci.ids` database (hwdata / pciutils).
    fn pci_ids_name(vendor: u32, device: u32) -> Option<String> {
        for p in ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"] {
            if let Ok(data) = fs::read_to_string(p) {
                return parse_pci_ids(&data, vendor, device);
            }
        }
        None
    }

    pub(crate) fn parse_pci_ids(data: &str, vendor: u32, device: u32) -> Option<String> {
        let v = format!("{vendor:04x}");
        let d = format!("{device:04x}");
        let mut in_vendor = false;
        let mut vendor_name = String::new();
        for line in data.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if !line.starts_with('\t') {
                if in_vendor {
                    break;
                }
                if line.starts_with(&v) {
                    in_vendor = true;
                    vendor_name = line[4..].trim().to_string();
                }
                continue;
            }
            if in_vendor && !line.starts_with("\t\t") && line[1..].starts_with(&d) {
                let dev = line[5..].trim();
                // "GA104 [GeForce RTX 3070]" → "GeForce RTX 3070"
                let pretty = match (dev.find('['), dev.rfind(']')) {
                    (Some(a), Some(b)) if b > a => dev[a + 1..b].to_string(),
                    _ => dev.to_string(),
                };
                let short_vendor = vendor_name.split_whitespace().next().unwrap_or("").replace(',', "");
                return Some(format!("{short_vendor} {pretty}").trim().to_string());
            }
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn pci_ids() {
            let db = "# c\n10de  NVIDIA Corporation\n\t2484  GA104 [GeForce RTX 3070]\n\t\t1458 4001  sub\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX]\n";
            assert_eq!(parse_pci_ids(db, 0x10de, 0x2484).as_deref(), Some("NVIDIA GeForce RTX 3070"));
            assert_eq!(parse_pci_ids(db, 0x1002, 0x744c).as_deref(), Some("Advanced Radeon RX 7900 XT/7900 XTX"));
            assert_eq!(parse_pci_ids(db, 0x10de, 0xffff), None);
        }

        #[test]
        fn fake_sysfs() {
            let tmp = tempfile::tempdir().unwrap();
            let dev = tmp.path().join("card1/device");
            fs::create_dir_all(dev.join("drm/renderD129")).unwrap();
            fs::write(dev.join("vendor"), "0x1002\n").unwrap();
            fs::write(dev.join("device"), "0x744c\n").unwrap();
            fs::write(dev.join("mem_info_vram_total"), "25753026560\n").unwrap();
            fs::create_dir_all(tmp.path().join("card1-DP-1")).unwrap();
            let gpus = list_from(tmp.path());
            assert_eq!(gpus.len(), 1);
            assert_eq!(gpus[0].vendor, GpuVendor::Amd);
            assert_eq!(gpus[0].index, 1);
            assert_eq!(gpus[0].render_node.as_deref(), Some(Path::new("/dev/dri/renderD129")));
            assert_eq!(gpus[0].vram_bytes, Some(25753026560));
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};

    pub fn list() -> Vec<GpuInfo> {
        let mut out = Vec::new();
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
            let mut i = 0u32;
            while let Ok(adapter) = factory.EnumAdapters1(i) {
                if let Ok(desc) = adapter.GetDesc1() {
                    let software = (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0;
                    if !software {
                        let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
                        let name = String::from_utf16_lossy(&desc.Description[..len]).trim().to_string();
                        out.push(GpuInfo {
                            index: i,
                            vendor: GpuVendor::from_pci_id(desc.VendorId),
                            vendor_id: desc.VendorId,
                            device_id: desc.DeviceId,
                            name,
                            vram_bytes: Some(desc.DedicatedVideoMemory as u64),
                            driver: None,
                            render_node: None,
                        });
                    }
                }
                i += 1;
            }
        }
        out
    }
}
