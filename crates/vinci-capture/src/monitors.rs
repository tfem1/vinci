//! Monitor enumeration.
//!
//! * Windows: DXGI outputs — gives the adapter/output indices `ddagrab` needs and the
//!   `HMONITOR` handle `gfxcapture` needs.
//! * Linux/X11: RandR 1.5 monitors (falls back to the root window size).
//! * Linux/Wayland: the compositor decides through the portal dialog.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub index: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    /// DXGI adapter index (Windows).
    pub adapter: u32,
    /// DXGI output index on that adapter (Windows, `ddagrab=output_idx`).
    pub output: u32,
    /// `HMONITOR` (Windows, `gfxcapture=hmonitor`).
    pub handle: u64,
}

impl MonitorInfo {
    pub fn synthetic(width: u32, height: u32) -> Self {
        Self { index: 0, name: "Virtual display".into(), x: 0, y: 0, width, height, primary: true, adapter: 0, output: 0, handle: 0 }
    }
}

pub fn list_monitors() -> Vec<MonitorInfo> {
    #[cfg(windows)]
    {
        windows_impl::list()
    }
    #[cfg(target_os = "linux")]
    {
        if vinci_hw::system::display_server() == vinci_hw::DisplayServer::Wayland {
            return Vec::new();
        }
        x11_impl::list().unwrap_or_default()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Vec::new()
    }
}

/// Can a transparent always-on-top overlay window be shown and positioned?
/// * Windows: always (DWM composition).
/// * X11: only when a compositing manager owns `_NET_WM_CM_S<n>` (otherwise the
///   "transparent" window would be an opaque black box).
/// * Wayland: no — clients cannot position windows or stay above fullscreen apps.
pub fn overlay_supported() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        if vinci_hw::system::display_server() != vinci_hw::DisplayServer::X11 {
            return false;
        }
        x11_impl::compositor_running().unwrap_or(false)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// Monitor for a requested index (falls back to primary, then first).
pub fn pick(monitors: &[MonitorInfo], index: u32) -> Option<MonitorInfo> {
    monitors
        .iter()
        .find(|m| m.index == index)
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
        .cloned()
}

#[cfg(target_os = "linux")]
mod x11_impl {
    use super::MonitorInfo;
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as _;
    use x11rb::protocol::xproto::ConnectionExt as _;

    pub fn compositor_running() -> Option<bool> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let name = format!("_NET_WM_CM_S{screen_num}");
        let atom = conn.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom;
        let owner = conn.get_selection_owner(atom).ok()?.reply().ok()?.owner;
        Some(owner != x11rb::NONE)
    }

    pub fn list() -> Option<Vec<MonitorInfo>> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let screen = conn.setup().roots.get(screen_num)?.clone();
        let mut out = Vec::new();
        if let Ok(reply) = conn.randr_get_monitors(screen.root, true).ok()?.reply() {
            for (i, m) in reply.monitors.iter().enumerate() {
                let name = conn
                    .get_atom_name(m.name)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                    .unwrap_or_else(|| format!("Monitor {}", i + 1));
                out.push(MonitorInfo {
                    index: i as u32,
                    name,
                    x: m.x as i32,
                    y: m.y as i32,
                    width: m.width as u32,
                    height: m.height as u32,
                    primary: m.primary,
                    adapter: 0,
                    output: i as u32,
                    handle: 0,
                });
            }
        }
        if out.is_empty() {
            out.push(MonitorInfo {
                index: 0,
                name: "Screen".into(),
                x: 0,
                y: 0,
                width: screen.width_in_pixels as u32,
                height: screen.height_in_pixels as u32,
                primary: true,
                adapter: 0,
                output: 0,
                handle: 0,
            });
        }
        if !out.iter().any(|m| m.primary) {
            out[0].primary = true;
        }
        Some(out)
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::MonitorInfo;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};

    const MONITORINFOF_PRIMARY: u32 = 1;

    pub fn list() -> Vec<MonitorInfo> {
        let mut out = Vec::new();
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
            let mut a = 0u32;
            while let Ok(adapter) = factory.EnumAdapters1(a) {
                let mut o = 0u32;
                while let Ok(output) = adapter.EnumOutputs(o) {
                    if let Ok(desc) = output.GetDesc() {
                        if desc.AttachedToDesktop.as_bool() {
                            let r = desc.DesktopCoordinates;
                            let len = desc.DeviceName.iter().position(|&c| c == 0).unwrap_or(desc.DeviceName.len());
                            let dev = String::from_utf16_lossy(&desc.DeviceName[..len]);
                            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                            let primary = GetMonitorInfoW(desc.Monitor, &mut mi).as_bool() && (mi.dwFlags & MONITORINFOF_PRIMARY) != 0;
                            out.push(MonitorInfo {
                                index: out.len() as u32,
                                name: format!("Display {}", dev.trim_start_matches(r"\\.\").trim_start_matches("DISPLAY")),
                                x: r.left,
                                y: r.top,
                                width: (r.right - r.left).max(0) as u32,
                                height: (r.bottom - r.top).max(0) as u32,
                                primary,
                                adapter: a,
                                output: o,
                                handle: desc.Monitor.0 as usize as u64,
                            });
                        }
                    }
                    o += 1;
                }
                a += 1;
            }
        }
        out
    }
}
