//! GStreamer detection (Linux Wayland capture path).
//!
//! On Wayland, screen capture must go through `xdg-desktop-portal` + PipeWire. FFmpeg
//! has no mainline PipeWire source, so VINCI uses GStreamer's `pipewiresrc` there.

use crate::process::{command, run_capture, which};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GstInfo {
    pub launch: PathBuf,
    pub elements: BTreeSet<String>,
}

/// Elements VINCI cares about — used to keep the cached list small.
pub const INTERESTING: &[&str] = &[
    "pipewiresrc", "pulsesrc", "videoconvert", "videorate", "videoscale", "queue", "mpegtsmux",
    "h264parse", "h265parse", "av1parse", "aacparse",
    "vah264enc", "vah265enc", "vaav1enc", "vah264lpenc", "vah265lpenc",
    "vaapih264enc", "vaapih265enc", "nvh264enc", "nvh265enc", "nvav1enc", "qsvh264enc", "qsvh265enc",
    "x264enc", "x265enc", "svtav1enc", "avenc_aac", "fdkaacenc", "voaacenc", "vapostproc", "cudaupload",
    "cudaconvert", "fdsink", "matroskamux", "vaapipostproc", "audioconvert", "audioresample", "audiomixer", "volume",
];

impl GstInfo {
    pub fn has(&self, element: &str) -> bool {
        self.elements.contains(element)
    }
}

pub fn detect() -> Option<GstInfo> {
    let launch = which("gst-launch-1.0")?;
    let inspect = which("gst-inspect-1.0")?;
    let (_, out, _) = run_capture(&mut command(inspect)).ok()?;
    Some(GstInfo { launch, elements: parse_inspect(&out) })
}

/// Parse `gst-inspect-1.0` (no args) output: `plugin:  element: Description`.
pub fn parse_inspect(out: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for line in out.lines() {
        let mut parts = line.splitn(3, ':');
        let (Some(_plugin), Some(el), Some(_)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let el = el.trim();
        if INTERESTING.contains(&el) {
            set.insert(el.to_string());
        }
    }
    set
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse() {
        let s = "pipewire:  pipewiresrc: PipeWire source\nva:  vah264enc: VA-API H.264 Encoder in Intel(R)\nx264:  x264enc: x264 H.264 Encoder\nfoo:  bar: baz\n\nTotal count: 3 plugins\n";
        let set = super::parse_inspect(s);
        assert!(set.contains("pipewiresrc") && set.contains("vah264enc") && set.contains("x264enc"));
        assert!(!set.contains("bar"));
    }
}
