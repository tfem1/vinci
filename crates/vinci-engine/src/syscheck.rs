//! "System check": graphics drivers, GPU encoder, video players and codec
//! compatibility, turned into a short list of issues with the command or link that
//! fixes each one.

use serde::Serialize;
use vinci_encoding::Codec;
use vinci_hw::drivers::{DriverCheck, PlayerCheck};
use vinci_hw::DistroFamily;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Something does not work or costs performance.
    Warning,
    /// Good to know; nothing is broken.
    Info,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub fix: Option<String>,
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemCheck {
    pub drivers: Vec<DriverCheck>,
    pub players: PlayerCheck,
    /// Codec new clips are recorded with.
    pub codec: Codec,
    /// A GPU video encoder works (otherwise VINCI encodes on the CPU).
    pub gpu_encoder: bool,
    pub issues: Vec<Issue>,
}

fn vlc_command(windows: bool, d: DistroFamily) -> Option<String> {
    if windows {
        return Some("winget install VideoLAN.VLC".into());
    }
    Some(
        match d {
            DistroFamily::Arch => "sudo pacman -S --needed vlc vlc-plugins-all",
            DistroFamily::Debian => "sudo apt install vlc",
            DistroFamily::Fedora => "sudo dnf install vlc",
            DistroFamily::Suse => "sudo zypper install vlc",
            DistroFamily::Other => return None,
        }
        .into(),
    )
}

pub fn issues(
    drivers: &[DriverCheck],
    players: &PlayerCheck,
    codec: Codec,
    gpu_encoder: bool,
    windows: bool,
    distro: DistroFamily,
) -> Vec<Issue> {
    let mut out = Vec::new();
    if !gpu_encoder {
        out.push(Issue {
            severity: Severity::Warning,
            title: "No graphics-card video encoder works".into(),
            detail: if drivers.is_empty() {
                "No graphics card was detected, so VINCI encodes on the processor (more CPU use while you play).".into()
            } else {
                "VINCI encodes on the processor, which costs game performance. Fix the driver problem below to encode on the GPU.".into()
            },
            fix: None,
            link: None,
        });
    }
    for d in drivers {
        if let Some(p) = &d.problem {
            out.push(Issue {
                severity: if d.optional { Severity::Info } else { Severity::Warning },
                title: format!("{} — {}", d.gpu, if d.optional { "optional driver" } else { "driver problem" }),
                detail: if d.optional { format!("{p} (Not needed: another GPU already encodes.)") } else { p.clone() },
                fix: d.fix.clone(),
                link: d.link.clone(),
            });
        }
    }
    let vlc_link = Some("https://www.videolan.org/vlc/".to_string());
    if windows {
        let needs = match codec {
            Codec::Hevc if players.hevc_extension != Some(true) => Some(("HEVC", "https://apps.microsoft.com/detail/9n4wgh0z6vhq")),
            Codec::Av1 if players.av1_extension != Some(true) => Some(("AV1", "https://apps.microsoft.com/detail/9mvzqvxjbq9v")),
            _ => None,
        };
        if let (Some((name, ext)), false) = (needs, players.plays_everything) {
            out.push(Issue {
                severity: Severity::Warning,
                title: format!("Your {name} clips won’t open in Windows’ video player"),
                detail: format!(
                    "Install VLC (free, plays every clip), or the {name} Video Extension from the Microsoft Store — or set Capture → Codec to Auto (H.264)."
                ),
                fix: vlc_command(true, distro),
                link: Some(ext.into()),
            });
        } else if !players.plays_everything {
            out.push(Issue {
                severity: Severity::Info,
                title: "Tip: install VLC".into(),
                detail: "Windows’ player opens H.264 clips. VLC also plays HEVC and AV1 clips and every format you may get from friends."
                    .into(),
                fix: vlc_command(true, distro),
                link: vlc_link,
            });
        }
    } else if !players.plays_everything && players.default_handler.is_none() {
        out.push(Issue {
            severity: Severity::Warning,
            title: "No video player installed".into(),
            detail:
                "Your clips can be watched inside VINCI, but nothing on this PC opens .mp4 files. Install VLC (or mpv, Haruna, Celluloid…)."
                    .into(),
            fix: vlc_command(false, distro),
            link: vlc_link,
        });
    }
    if matches!(codec, Codec::Hevc | Codec::Av1) {
        out.push(Issue {
            severity: Severity::Info,
            title: format!("Recording in {}", if codec == Codec::Hevc { "HEVC" } else { "AV1" }),
            detail: "Smaller files, but Discord and some phones can’t play them. Capture → Codec → Auto (H.264) plays everywhere.".into(),
            fix: None,
            link: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vinci_hw::GpuVendor;

    fn drv(problem: Option<&str>, optional: bool) -> DriverCheck {
        DriverCheck {
            gpu: "Intel UHD Graphics".into(),
            vendor: GpuVendor::Intel,
            driver: "i915".into(),
            encoder_api: "VA-API",
            encoder_ok: false,
            problem: problem.map(String::from),
            fix: Some("sudo pacman -S --needed intel-media-driver".into()),
            link: None,
            optional,
        }
    }

    #[test]
    fn arch_laptop_without_player() {
        let players = PlayerCheck::default();
        let i = issues(&[drv(Some("VA-API driver missing"), true)], &players, Codec::H264, true, false, DistroFamily::Arch);
        assert_eq!(i.len(), 2, "{i:?}");
        assert_eq!(i[0].severity, Severity::Info, "spare iGPU is not a warning");
        assert_eq!(i[1].title, "No video player installed");
        assert_eq!(i[1].fix.as_deref(), Some("sudo pacman -S --needed vlc vlc-plugins-all"));
    }

    #[test]
    fn windows_hevc_without_extension_or_vlc() {
        let players =
            PlayerCheck { players: vec!["Windows Media Player (H.264)".into()], hevc_extension: Some(false), ..Default::default() };
        let i = issues(&[], &players, Codec::Hevc, true, true, DistroFamily::Other);
        assert_eq!(i[0].severity, Severity::Warning);
        assert!(i[0].title.contains("HEVC"));
        assert_eq!(i[0].fix.as_deref(), Some("winget install VideoLAN.VLC"));
        // H.264 (the Auto default) plays in the built-in player: only a tip.
        let i = issues(&[], &players, Codec::H264, true, true, DistroFamily::Other);
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].severity, Severity::Info);
        // With VLC installed there is nothing to say about H.264.
        let vlc = PlayerCheck { plays_everything: true, players: vec!["VLC".into()], ..Default::default() };
        assert!(issues(&[], &vlc, Codec::H264, true, true, DistroFamily::Other).is_empty());
    }

    #[test]
    fn cpu_encoding_is_a_warning() {
        let vlc = PlayerCheck { plays_everything: true, ..Default::default() };
        let i = issues(&[drv(Some("no driver"), false)], &vlc, Codec::H264, false, false, DistroFamily::Arch);
        assert_eq!(i[0].title, "No graphics-card video encoder works");
        assert!(i.iter().all(|x| x.severity == Severity::Warning));
    }
}
