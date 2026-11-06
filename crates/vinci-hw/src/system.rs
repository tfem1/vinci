//! Operating system, display server and CPU/RAM information.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayServer {
    Windows,
    X11,
    Wayland,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    /// `windows` / `linux`.
    pub os_family: String,
    /// Human readable, e.g. "Windows 11 Pro (build 22631)" or "Ubuntu 24.04.1 LTS".
    pub os_name: String,
    pub kernel: String,
    /// Windows build number (≥ 22000 is Windows 11). `None` on Linux.
    pub windows_build: Option<u32>,
    pub arch: String,
    pub cpu_name: String,
    pub cpu_threads: usize,
    pub ram_total_bytes: u64,
    pub display_server: DisplayServer,
    /// `XDG_CURRENT_DESKTOP` (GNOME, KDE, sway, Hyprland…) on Linux.
    pub desktop: Option<String>,
}

impl SystemInfo {
    pub fn is_windows_11(&self) -> bool {
        self.windows_build.is_some_and(|b| b >= 22000)
    }
}

/// Linux distribution family, used to print the right package-manager command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistroFamily {
    Arch,
    Debian,
    Fedora,
    Suse,
    Other,
}

/// Distribution family from `/etc/os-release` (`ID`, then `ID_LIKE`).
pub fn distro_family() -> DistroFamily {
    std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .map(|s| distro_family_from(&s))
        .unwrap_or(DistroFamily::Other)
}

pub fn distro_family_from(os_release: &str) -> DistroFamily {
    let field = |k: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix('=')))
            .map(|v| v.trim().trim_matches('"').to_lowercase())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    for id in ids.split_whitespace() {
        match id {
            "arch" | "archarm" | "manjaro" | "endeavouros" | "cachyos" | "garuda" | "artix" => return DistroFamily::Arch,
            "debian" | "ubuntu" | "linuxmint" | "pop" | "zorin" | "elementary" => return DistroFamily::Debian,
            "fedora" | "rhel" | "centos" | "nobara" | "bazzite" => return DistroFamily::Fedora,
            "suse" | "opensuse" | "opensuse-tumbleweed" | "opensuse-leap" => return DistroFamily::Suse,
            _ => {}
        }
    }
    DistroFamily::Other
}

pub fn display_server() -> DisplayServer {
    if cfg!(windows) {
        return DisplayServer::Windows;
    }
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default().to_lowercase();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || session == "wayland" {
        DisplayServer::Wayland
    } else if std::env::var_os("DISPLAY").is_some() || session == "x11" {
        DisplayServer::X11
    } else {
        DisplayServer::Unknown
    }
}

pub fn detect() -> SystemInfo {
    use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let kernel = System::kernel_version().unwrap_or_default();
    let windows_build = if cfg!(windows) { kernel.split('.').next_back().and_then(|s| s.trim().parse().ok()) } else { None };
    let mut os_name = System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_string());
    if let Some(b) = windows_build {
        // sysinfo reports "Windows 10" for Windows 11 on some builds; the build number is authoritative.
        if b >= 22000 {
            os_name = os_name.replace("Windows 10", "Windows 11");
        }
        os_name = format!("{os_name} (build {b})");
    }
    SystemInfo {
        os_family: std::env::consts::OS.to_string(),
        os_name,
        kernel,
        windows_build,
        arch: std::env::consts::ARCH.to_string(),
        cpu_name: sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        cpu_threads: sys.cpus().len().max(1),
        ram_total_bytes: sys.total_memory(),
        display_server: display_server(),
        desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
    }
}
