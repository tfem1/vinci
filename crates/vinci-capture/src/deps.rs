//! Runtime dependencies of the Wayland capture path, and the exact command that
//! installs what is missing on the user's distribution.
//!
//! Wayland capture runs `gst-launch-1.0` with `pipewiresrc` (fed by the ScreenCast
//! portal) → H.264 encoder → Matroska. Every element of that pipeline must exist, or
//! GStreamer fails with an opaque "no element …" — so it is checked up front.

use vinci_hw::{DistroFamily, GstInfo};

/// Not an element: stands for the GStreamer command-line tools themselves.
pub const GST_LAUNCH: &str = "gst-launch-1.0";

/// Elements without which no video can be captured on Wayland. `x264enc` is the
/// encoder every configuration can fall back to (GPU encoders are optional extras).
pub const PORTAL_VIDEO: &[&str] =
    &["pipewiresrc", "videorate", "videoconvert", "videoscale", "queue", "x264enc", "h264parse", "matroskamux", "fdsink"];

/// Elements needed to record sound on Wayland. Missing ones only disable audio.
pub const PORTAL_AUDIO: &[&str] = &["pulsesrc", "audioconvert", "audioresample", "audiomixer", "volume", "avenc_aac", "aacparse"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortalDeps {
    /// Missing elements that prevent capture.
    pub video: Vec<&'static str>,
    /// Missing elements that only prevent audio.
    pub audio: Vec<&'static str>,
}

impl PortalDeps {
    pub fn check(gst: Option<&GstInfo>) -> Self {
        let Some(g) = gst else {
            let mut video = vec![GST_LAUNCH];
            video.extend_from_slice(PORTAL_VIDEO);
            return Self { video, audio: PORTAL_AUDIO.to_vec() };
        };
        let missing = |list: &[&'static str]| list.iter().copied().filter(|e| !g.has(e)).collect::<Vec<_>>();
        let mut audio = missing(PORTAL_AUDIO);
        // Any of the three AAC encoders will do.
        if ["fdkaacenc", "voaacenc"].iter().any(|e| g.has(e)) {
            audio.retain(|e| *e != "avenc_aac");
        }
        Self { video: missing(PORTAL_VIDEO), audio }
    }

    pub fn video_ok(&self) -> bool {
        self.video.is_empty()
    }

    pub fn audio_ok(&self) -> bool {
        self.audio.is_empty()
    }

    /// Everything missing (video first), for one install command.
    pub fn all(&self) -> Vec<&'static str> {
        self.video.iter().chain(self.audio.iter()).copied().collect()
    }
}

/// Package providing an element, per distribution family.
fn package(el: &str, d: DistroFamily) -> Option<&'static str> {
    use DistroFamily::*;
    // (arch, debian/ubuntu, fedora, opensuse)
    let row: [&'static str; 4] = match el {
        GST_LAUNCH | "queue" | "fdsink" => ["gstreamer", "gstreamer1.0-tools", "gstreamer1", "gstreamer-utils"],
        "pipewiresrc" => ["gst-plugin-pipewire", "gstreamer1.0-pipewire", "pipewire-gstreamer", "gstreamer-plugin-pipewire"],
        "videorate" | "videoconvert" | "videoscale" | "audioconvert" | "audioresample" | "audiomixer" | "volume" => {
            ["gst-plugins-base", "gstreamer1.0-plugins-base", "gstreamer1-plugins-base", "gstreamer-plugins-base"]
        }
        "matroskamux" | "aacparse" => {
            ["gst-plugins-good", "gstreamer1.0-plugins-good", "gstreamer1-plugins-good", "gstreamer-plugins-good"]
        }
        "pulsesrc" => ["gst-plugins-good", "gstreamer1.0-pulseaudio", "gstreamer1-plugins-good", "gstreamer-plugins-good"],
        "h264parse" | "h265parse" | "av1parse" => {
            ["gst-plugins-bad", "gstreamer1.0-plugins-bad", "gstreamer1-plugins-bad-free", "gstreamer-plugins-bad"]
        }
        "x264enc" => ["gst-plugins-ugly", "gstreamer1.0-plugins-ugly", "gstreamer1-plugins-ugly", "gstreamer-plugins-ugly"],
        "avenc_aac" => ["gst-libav", "gstreamer1.0-libav", "gstreamer1-plugin-libav", "gstreamer-plugins-libav"],
        _ => return None,
    };
    Some(match d {
        Arch => row[0],
        Debian => row[1],
        Fedora => row[2],
        Suse => row[3],
        Other => return None,
    })
}

/// The command that installs the packages providing `elements`, e.g.
/// `sudo pacman -S --needed gst-plugin-pipewire gst-plugins-bad`.
/// `None` when nothing is missing or the distribution is unknown.
pub fn install_command(elements: &[&str], d: DistroFamily) -> Option<String> {
    let mut pkgs: Vec<&str> = Vec::new();
    for e in elements {
        if let Some(p) = package(e, d) {
            if !pkgs.contains(&p) {
                pkgs.push(p);
            }
        }
    }
    if pkgs.is_empty() {
        return None;
    }
    let cmd = match d {
        DistroFamily::Arch => "sudo pacman -S --needed",
        DistroFamily::Debian => "sudo apt install",
        DistroFamily::Fedora => "sudo dnf install",
        DistroFamily::Suse => "sudo zypper install",
        DistroFamily::Other => return None,
    };
    Some(format!("{cmd} {}", pkgs.join(" ")))
}

/// Human description of what is missing, naming the packages when the distribution
/// is unknown (no command can be given then).
pub fn describe_missing(elements: &[&str]) -> String {
    let names: Vec<&str> = elements.to_vec();
    format!("missing GStreamer pieces: {}", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn gst(els: &[&str]) -> GstInfo {
        GstInfo { launch: PathBuf::from("/usr/bin/gst-launch-1.0"), elements: els.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>() }
    }

    #[test]
    fn arch_kde_without_pipewire_plugin() {
        // Typical Arch + Plasma: GStreamer base/good come with WebKitGTK, nothing else.
        let g = gst(&[
            "videorate",
            "videoconvert",
            "videoscale",
            "queue",
            "fdsink",
            "matroskamux",
            "pulsesrc",
            "audioconvert",
            "audioresample",
            "audiomixer",
            "volume",
            "aacparse",
        ]);
        let d = PortalDeps::check(Some(&g));
        assert_eq!(d.video, vec!["pipewiresrc", "x264enc", "h264parse"]);
        assert_eq!(d.audio, vec!["avenc_aac"]);
        assert_eq!(
            install_command(&d.all(), DistroFamily::Arch).unwrap(),
            "sudo pacman -S --needed gst-plugin-pipewire gst-plugins-ugly gst-plugins-bad gst-libav"
        );
        assert_eq!(
            install_command(&d.all(), DistroFamily::Debian).unwrap(),
            "sudo apt install gstreamer1.0-pipewire gstreamer1.0-plugins-ugly gstreamer1.0-plugins-bad gstreamer1.0-libav"
        );
    }

    #[test]
    fn no_gstreamer_at_all() {
        let d = PortalDeps::check(None);
        assert!(!d.video_ok() && d.video[0] == GST_LAUNCH);
        let cmd = install_command(&d.all(), DistroFamily::Arch).unwrap();
        for p in
            ["gstreamer", "gst-plugin-pipewire", "gst-plugins-base", "gst-plugins-good", "gst-plugins-bad", "gst-plugins-ugly", "gst-libav"]
        {
            assert!(cmd.split_whitespace().any(|w| w == p), "{p} missing from {cmd}");
        }
    }

    #[test]
    fn complete_install_and_other_aac_encoder() {
        let mut all: Vec<&str> = PORTAL_VIDEO.to_vec();
        all.extend(PORTAL_AUDIO.iter().filter(|e| **e != "avenc_aac"));
        all.push("fdkaacenc");
        let d = PortalDeps::check(Some(&gst(&all)));
        assert!(d.video_ok() && d.audio_ok());
        assert_eq!(install_command(&d.all(), DistroFamily::Arch), None);
        assert_eq!(install_command(&["pipewiresrc"], DistroFamily::Other), None);
    }

    #[test]
    fn distro_detection() {
        use vinci_hw::system::distro_family_from as f;
        assert_eq!(f("NAME=\"Arch Linux\"\nID=arch\n"), DistroFamily::Arch);
        assert_eq!(f("ID=cachyos\nID_LIKE=arch\n"), DistroFamily::Arch);
        assert_eq!(f("ID=endeavouros\nID_LIKE=\"arch\"\n"), DistroFamily::Arch);
        assert_eq!(f("ID=ubuntu\nID_LIKE=debian\n"), DistroFamily::Debian);
        assert_eq!(f("ID=fedora\n"), DistroFamily::Fedora);
        assert_eq!(f("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"), DistroFamily::Suse);
        assert_eq!(f("ID=nixos\n"), DistroFamily::Other);
    }
}
