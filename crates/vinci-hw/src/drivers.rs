//! Graphics drivers and video players: what is installed, whether the GPU video
//! encoder is usable, and — when something is missing — the exact command or download
//! page that fixes it.
//!
//! The encoder probes live in `vinci-encoding`; callers pass their results as
//! [`ProbeOutcome`] so this module stays independent of it.

use crate::gpu::{GpuInfo, GpuVendor};
use crate::system::DistroFamily;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Result of one FFmpeg encoder probe (`h264_nvenc`, `hevc_vaapi`, …).
#[derive(Debug, Clone)]
pub struct ProbeOutcome {
    pub id: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DriverCheck {
    pub gpu: String,
    pub vendor: GpuVendor,
    /// Human description: "nvidia 580.95.05", "i915 · VA-API driver iHD", "NVIDIA 560.94 (32.0.15.6094)".
    pub driver: String,
    /// Video encoding API VINCI uses on this GPU: NVENC, VA-API, AMF or Quick Sync.
    pub encoder_api: &'static str,
    /// The GPU encoder works (validated by a real test encode).
    pub encoder_ok: bool,
    /// What is wrong, in plain words (None when everything works).
    pub problem: Option<String>,
    /// Shell command that fixes it.
    pub fix: Option<String>,
    /// Download page when there is no command (Windows drivers).
    pub link: Option<String>,
    /// Not needed because another GPU already encodes (hybrid laptops).
    pub optional: bool,
}

fn any_ok(probes: &[ProbeOutcome], suffix: &str) -> bool {
    probes.iter().any(|p| p.ok && p.id.ends_with(suffix))
}

fn first_error<'a>(probes: &'a [ProbeOutcome], suffix: &str) -> Option<&'a str> {
    probes.iter().filter(|p| !p.ok && p.id.ends_with(suffix)).find_map(|p| p.error.as_deref())
}

fn pkg_cmd(d: DistroFamily, pkgs: [&str; 4]) -> Option<String> {
    Some(match d {
        DistroFamily::Arch => format!("sudo pacman -S --needed {}", pkgs[0]),
        DistroFamily::Debian => format!("sudo apt install {}", pkgs[1]),
        DistroFamily::Fedora => format!("sudo dnf install {}", pkgs[2]),
        DistroFamily::Suse => format!("sudo zypper install {}", pkgs[3]),
        DistroFamily::Other => return None,
    })
}

/// NVENC refuses a driver older than the NVENC SDK FFmpeg was built with.
fn nvenc_driver_too_old(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("nvenc api version") || e.contains("minimum required nvidia driver") || e.contains("driver does not support the required")
}

/// Linux: driver state of every GPU. `dri_dirs` are searched for VA-API drivers,
/// `lib_dirs` for NVIDIA's NVENC library.
pub fn check_linux(
    gpus: &[GpuInfo],
    probes: &[ProbeOutcome],
    distro: DistroFamily,
    dri_dirs: &[PathBuf],
    lib_dirs: &[PathBuf],
) -> Vec<DriverCheck> {
    let has_file = |dirs: &[PathBuf], name: &str| dirs.iter().any(|d| d.join(name).exists());
    let nvenc_ok = any_ok(probes, "_nvenc");
    let vaapi_ok = any_ok(probes, "_vaapi");
    let mut out = Vec::new();
    for g in gpus {
        let kdrv = g.driver.clone().unwrap_or_else(|| "no driver".into());
        let mut c = DriverCheck {
            gpu: g.name.clone(),
            vendor: g.vendor,
            driver: kdrv.clone(),
            encoder_api: "",
            encoder_ok: false,
            problem: None,
            fix: None,
            link: None,
            optional: false,
        };
        match g.vendor {
            GpuVendor::Nvidia => {
                c.encoder_api = "NVENC";
                c.encoder_ok = nvenc_ok;
                if let Some(v) = &g.driver_version {
                    c.driver = format!("{kdrv} {v}");
                }
                if g.driver.as_deref() != Some("nvidia") {
                    c.problem = Some(format!(
                        "The {kdrv} driver cannot use the NVIDIA video encoder (NVENC). Install NVIDIA's driver, then restart the PC."
                    ));
                    c.fix = pkg_cmd(distro, ["nvidia-open nvidia-utils", "nvidia-driver", "akmod-nvidia", "nvidia-video-G06"]);
                    c.link = Some("https://www.nvidia.com/Download/index.aspx".into());
                } else if !nvenc_ok {
                    let err = first_error(probes, "_nvenc").unwrap_or("");
                    if nvenc_driver_too_old(err) {
                        c.problem = Some("Your NVIDIA driver is too old for this FFmpeg's NVENC. Update it, then restart the PC.".into());
                        c.fix = match distro {
                            DistroFamily::Arch => Some("sudo pacman -Syu".into()),
                            _ => None,
                        };
                        c.link = Some("https://www.nvidia.com/Download/index.aspx".into());
                    } else if !has_file(lib_dirs, "libnvidia-encode.so.1") {
                        c.problem = Some("NVIDIA's encoding library (libnvidia-encode) is missing.".into());
                        c.fix =
                            pkg_cmd(distro, ["nvidia-utils", "libnvidia-encode1", "xorg-x11-drv-nvidia-cuda-libs", "nvidia-compute-G06"]);
                    } else {
                        c.problem = Some(format!("NVENC test encode failed: {}", short(err)));
                    }
                }
            }
            GpuVendor::Intel | GpuVendor::Amd => {
                c.encoder_api = "VA-API";
                c.encoder_ok = vaapi_ok;
                let (drivers, pkgs): (&[&str], [&str; 4]) = if g.vendor == GpuVendor::Intel {
                    (
                        &["iHD_drv_video.so", "i965_drv_video.so"],
                        ["intel-media-driver", "intel-media-va-driver-non-free", "intel-media-driver", "intel-media-driver"],
                    )
                } else {
                    (&["radeonsi_drv_video.so"], ["libva-mesa-driver", "mesa-va-drivers", "mesa-va-drivers-freeworld", "Mesa-libva"])
                };
                let found: Vec<&str> = drivers.iter().copied().filter(|d| has_file(dri_dirs, d)).collect();
                if let Some(f) = found.first() {
                    c.driver = format!("{kdrv} · VA-API driver {}", f.trim_end_matches("_drv_video.so"));
                }
                if g.vendor == GpuVendor::Amd && g.driver.as_deref() == Some("radeon") {
                    c.problem =
                        Some("This older AMD GPU (radeon driver) has no VA-API video encoder; VINCI will encode on the CPU.".into());
                } else if found.is_empty() {
                    c.problem = Some(format!(
                        "The VA-API video driver for this {} GPU is not installed, so it cannot encode video.",
                        g.vendor.label()
                    ));
                    c.fix = pkg_cmd(distro, pkgs);
                } else if !vaapi_ok {
                    c.problem = first_error(probes, "_vaapi").map(|e| format!("VA-API test encode failed: {}", short(e)));
                }
                // On a hybrid laptop the NVIDIA GPU already encodes: this one is only a spare.
                c.optional = nvenc_ok;
            }
            GpuVendor::Other => {
                c.encoder_api = "—";
                c.optional = true;
            }
        }
        out.push(c);
    }
    out
}

/// Windows: driver state of every DXGI adapter.
pub fn check_windows(gpus: &[GpuInfo], probes: &[ProbeOutcome]) -> Vec<DriverCheck> {
    let nvenc_ok = any_ok(probes, "_nvenc");
    let amf_ok = any_ok(probes, "_amf");
    let qsv_ok = any_ok(probes, "_qsv");
    let any_gpu_encoder = nvenc_ok || amf_ok || qsv_ok;
    gpus.iter()
        .map(|g| {
            let ver = g.driver_version.clone().unwrap_or_else(|| "unknown version".into());
            let mut c = DriverCheck {
                gpu: g.name.clone(),
                vendor: g.vendor,
                driver: ver.clone(),
                encoder_api: "",
                encoder_ok: false,
                problem: None,
                fix: None,
                link: None,
                optional: false,
            };
            if g.vendor_id == 0x1414 {
                // "Microsoft Basic Display Adapter": no vendor driver at all.
                c.driver = "Microsoft Basic Display Adapter".into();
                c.problem = Some("No graphics driver is installed. Install your GPU maker's driver (Windows Update → Optional updates, or the links below).".into());
                c.link = Some("https://www.nvidia.com/Download/index.aspx".into());
                return c;
            }
            match g.vendor {
                GpuVendor::Nvidia => {
                    c.encoder_api = "NVENC";
                    c.encoder_ok = nvenc_ok;
                    if let Some(r) = g.driver_version.as_deref().and_then(crate::gpu::nvidia_release_from_windows) {
                        c.driver = format!("NVIDIA {r} ({ver})");
                    }
                    c.link = Some("https://www.nvidia.com/Download/index.aspx".into());
                    if !nvenc_ok {
                        let err = first_error(probes, "_nvenc").unwrap_or("");
                        c.problem = Some(if nvenc_driver_too_old(err) {
                            "Your NVIDIA driver is too old for VINCI's video encoder (NVENC). Update it (NVIDIA App or the link below), then restart VINCI.".into()
                        } else {
                            format!("NVENC test encode failed: {}", short(err))
                        });
                    }
                }
                GpuVendor::Amd => {
                    c.encoder_api = "AMF";
                    c.encoder_ok = amf_ok;
                    c.link = Some("https://www.amd.com/en/support/download/drivers.html".into());
                    if !amf_ok {
                        c.problem = Some("The AMD video encoder (AMF) does not work: install or update AMD Software: Adrenalin Edition.".into());
                    }
                }
                GpuVendor::Intel => {
                    c.encoder_api = "Quick Sync";
                    c.encoder_ok = qsv_ok;
                    c.link = Some("https://www.intel.com/content/www/us/en/download-center/home.html".into());
                    if !qsv_ok {
                        c.problem = Some("The Intel video encoder (Quick Sync) does not work: update the Intel graphics driver.".into());
                    }
                }
                GpuVendor::Other => c.encoder_api = "—",
            }
            // A GPU whose encoder is not needed (another one works) is not a problem.
            c.optional = !c.encoder_ok && any_gpu_encoder;
            c
        })
        .collect()
}

/// Default search paths for the current OS.
pub fn check(gpus: &[GpuInfo], probes: &[ProbeOutcome]) -> Vec<DriverCheck> {
    if cfg!(windows) {
        return check_windows(gpus, probes);
    }
    let mut dri: Vec<PathBuf> = std::env::var_os("LIBVA_DRIVERS_PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let mut libs = Vec::new();
    for base in ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu", "/run/opengl-driver/lib"] {
        dri.push(Path::new(base).join("dri"));
        libs.push(PathBuf::from(base));
    }
    check_linux(gpus, probes, crate::system::distro_family(), &dri, &libs)
}

fn short(s: &str) -> String {
    let line = s.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(s).trim();
    let mut t: String = line.chars().take(180).collect();
    if line.chars().count() > 180 {
        t.push('…');
    }
    t
}

// ------------------------------------------------------------------- players

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct PlayerCheck {
    /// Video players found (VLC, mpv, Haruna, Windows Media Player…).
    pub players: Vec<String>,
    /// Desktop file / app opening .mp4 by default (Linux).
    pub default_handler: Option<String>,
    /// A player that plays H.264, HEVC and AV1 without extra codecs is installed.
    pub plays_everything: bool,
    /// Windows: HEVC / AV1 Video Extensions installed (the built-in player needs them).
    pub hevc_extension: Option<bool>,
    pub av1_extension: Option<bool>,
}

/// Linux players: (binary, display name). All of them decode H.264/HEVC/AV1.
const LINUX_PLAYERS: &[(&str, &str)] = &[
    ("vlc", "VLC"),
    ("mpv", "mpv"),
    ("celluloid", "Celluloid"),
    ("haruna", "Haruna"),
    ("dragon", "Dragon Player"),
    ("totem", "Videos (Totem)"),
    ("showtime", "Showtime"),
    ("clapper", "Clapper"),
    ("smplayer", "SMPlayer"),
    ("parole", "Parole"),
    ("mplayer", "MPlayer"),
];

pub fn players_linux(path_dirs: &[PathBuf], default_handler: Option<String>) -> PlayerCheck {
    let players: Vec<String> = LINUX_PLAYERS
        .iter()
        .filter(|(bin, _)| path_dirs.iter().any(|d| crate::process::is_executable(&d.join(bin))))
        .map(|(_, name)| name.to_string())
        .collect();
    let plays_everything = !players.is_empty();
    PlayerCheck { players, default_handler, plays_everything, hevc_extension: None, av1_extension: None }
}

pub fn players() -> PlayerCheck {
    #[cfg(windows)]
    {
        windows_players()
    }
    #[cfg(not(windows))]
    {
        let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let default_handler = crate::process::which("xdg-mime").and_then(|x| {
            let mut c = crate::process::command(x);
            c.args(["query", "default", "video/mp4"]);
            crate::process::run_capture(&mut c).ok().map(|(_, out, _)| out.trim().to_string()).filter(|s| !s.is_empty())
        });
        players_linux(&dirs, default_handler)
    }
}

#[cfg(windows)]
fn windows_players() -> PlayerCheck {
    let pf: Vec<PathBuf> =
        ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"].iter().filter_map(|v| std::env::var_os(v).map(PathBuf::from)).collect();
    let known: &[(&str, &str)] = &[
        ("VideoLAN\\VLC\\vlc.exe", "VLC"),
        ("mpv\\mpv.exe", "mpv"),
        ("DAUM\\PotPlayer\\PotPlayerMini64.exe", "PotPlayer"),
        ("MPC-HC\\mpc-hc64.exe", "MPC-HC"),
        ("MPC-BE x64\\mpc-be64.exe", "MPC-BE"),
        ("Programs\\mpv\\mpv.exe", "mpv"),
    ];
    let mut players: Vec<String> =
        known.iter().filter(|(rel, _)| pf.iter().any(|d| d.join(rel).exists())).map(|(_, n)| n.to_string()).collect();
    players.dedup();
    let plays_everything = !players.is_empty();
    // Store-app packages are registered per user under AppModel\\Repository\\Packages.
    let has_package = |needle: &str| {
        let mut c = crate::process::command("reg");
        c.args([
            "query",
            r"HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
            "/k",
            "/f",
            needle,
        ]);
        crate::process::run_capture(&mut c).map(|(ok, out, _)| ok && out.contains(needle)).ok()
    };
    players.push("Windows Media Player (H.264)".into());
    PlayerCheck {
        players,
        default_handler: None,
        plays_everything,
        hevc_extension: has_package("HEVCVideoExtension"),
        av1_extension: has_package("AV1VideoExtension"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(vendor: GpuVendor, vendor_id: u32, driver: Option<&str>, version: Option<&str>) -> GpuInfo {
        GpuInfo {
            index: 0,
            vendor,
            vendor_id,
            device_id: 0,
            name: format!("{} GPU", vendor.label()),
            vram_bytes: None,
            driver: driver.map(String::from),
            render_node: None,
            driver_version: version.map(String::from),
        }
    }
    fn probe(id: &str, ok: bool, err: Option<&str>) -> ProbeOutcome {
        ProbeOutcome { id: id.into(), ok, error: err.map(String::from) }
    }

    #[test]
    fn hybrid_laptop_like_the_rtx_4050_report() {
        // NVENC works; the Intel iGPU has no VA-API driver → reported, but optional.
        let gpus = [gpu(GpuVendor::Nvidia, 0x10de, Some("nvidia"), Some("580.95.05")), gpu(GpuVendor::Intel, 0x8086, Some("i915"), None)];
        let probes = [
            probe("h264_nvenc", true, None),
            probe("h264_vaapi", false, Some("Failed to initialise VAAPI connection: -1 (unknown libva error).")),
        ];
        let tmp = tempfile::tempdir().unwrap();
        let c = check_linux(&gpus, &probes, DistroFamily::Arch, &[tmp.path().to_path_buf()], &[tmp.path().to_path_buf()]);
        assert_eq!(c[0].driver, "nvidia 580.95.05");
        assert!(c[0].encoder_ok && c[0].problem.is_none());
        assert_eq!(c[1].fix.as_deref(), Some("sudo pacman -S --needed intel-media-driver"));
        assert!(c[1].optional, "NVENC already encodes");
    }

    #[test]
    fn nouveau_and_old_nvidia_driver() {
        let tmp = tempfile::tempdir().unwrap();
        let d = [tmp.path().to_path_buf()];
        let c = check_linux(&[gpu(GpuVendor::Nvidia, 0x10de, Some("nouveau"), None)], &[], DistroFamily::Arch, &d, &d);
        assert!(c[0].problem.as_deref().unwrap().contains("nouveau"));
        assert_eq!(c[0].fix.as_deref(), Some("sudo pacman -S --needed nvidia-open nvidia-utils"));
        let probes =
            [probe("h264_nvenc", false, Some("Driver does not support the required nvenc API version. Required: 13.0 Found: 12.1"))];
        let c = check_linux(&[gpu(GpuVendor::Nvidia, 0x10de, Some("nvidia"), Some("535.0"))], &probes, DistroFamily::Debian, &d, &d);
        assert!(c[0].problem.as_deref().unwrap().contains("too old"), "{:?}", c[0].problem);
    }

    #[test]
    fn amd_vaapi_driver_found() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("radeonsi_drv_video.so"), b"").unwrap();
        let c = check_linux(
            &[gpu(GpuVendor::Amd, 0x1002, Some("amdgpu"), None)],
            &[probe("hevc_vaapi", true, None)],
            DistroFamily::Fedora,
            &[tmp.path().to_path_buf()],
            &[],
        );
        assert_eq!(c[0].driver, "amdgpu · VA-API driver radeonsi");
        assert!(c[0].encoder_ok && c[0].problem.is_none());
    }

    #[test]
    fn windows_drivers() {
        assert_eq!(crate::gpu::nvidia_release_from_windows("32.0.15.6094").as_deref(), Some("560.94"));
        assert_eq!(crate::gpu::nvidia_release_from_windows("31.0.15.3623").as_deref(), Some("536.23"));
        let c = check_windows(&[gpu(GpuVendor::Nvidia, 0x10de, None, Some("32.0.15.6094"))], &[probe("h264_nvenc", true, None)]);
        assert_eq!(c[0].driver, "NVIDIA 560.94 (32.0.15.6094)");
        assert!(c[0].problem.is_none());
        let c = check_windows(&[gpu(GpuVendor::Other, 0x1414, None, None)], &[]);
        assert!(c[0].problem.as_deref().unwrap().contains("No graphics driver"));
        let c = check_windows(
            &[gpu(GpuVendor::Amd, 0x1002, None, Some("31.0.24027.1012"))],
            &[probe("h264_amf", false, Some("DLL amfrt64.dll failed to open"))],
        );
        assert!(c[0].problem.as_deref().unwrap().contains("Adrenalin"));
    }

    #[test]
    fn linux_players() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!players_linux(&[tmp.path().to_path_buf()], None).plays_everything);
        let vlc = tmp.path().join("vlc");
        std::fs::write(&vlc, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&vlc, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let p = players_linux(&[tmp.path().to_path_buf()], Some("vlc.desktop".into()));
        assert_eq!(p.players, vec!["VLC"]);
        assert!(p.plays_everything);
    }
}
