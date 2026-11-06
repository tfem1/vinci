//! GStreamer detection (Linux Wayland capture path).
//!
//! On Wayland, screen capture must go through `xdg-desktop-portal` + PipeWire. FFmpeg
//! has no mainline PipeWire source, so VINCI uses GStreamer's `pipewiresrc` there.

use crate::process::{command, run_capture, which};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GstInfo {
    pub launch: PathBuf,
    pub elements: BTreeSet<String>,
    /// NVIDIA: a real test encode through `cudaupload ! cudaconvertscale ! nvh264enc`
    /// succeeded, so colour conversion and scaling can run on the GPU.
    #[serde(default)]
    pub cuda_nvenc: bool,
}

/// Elements VINCI cares about — used to keep the cached list small.
pub const INTERESTING: &[&str] = &[
    "pipewiresrc",
    "pulsesrc",
    "videoconvert",
    "videorate",
    "videoscale",
    "queue",
    "mpegtsmux",
    "h264parse",
    "h265parse",
    "av1parse",
    "aacparse",
    "vah264enc",
    "vah265enc",
    "vaav1enc",
    "vah264lpenc",
    "vah265lpenc",
    "vaapih264enc",
    "vaapih265enc",
    "nvh264enc",
    "nvh265enc",
    "nvav1enc",
    "qsvh264enc",
    "qsvh265enc",
    "x264enc",
    "x265enc",
    "svtav1enc",
    "avenc_aac",
    "fdkaacenc",
    "voaacenc",
    "vapostproc",
    "cudaupload",
    "cudaconvert",
    "cudaconvertscale",
    "fdsink",
    "matroskamux",
    "vaapipostproc",
    "audioconvert",
    "audioresample",
    "audiomixer",
    "volume",
];

impl GstInfo {
    pub fn has(&self, element: &str) -> bool {
        self.elements.contains(element)
    }
}

/// Elements the Wayland capture pipeline cannot work without (plus audio). When the
/// listing does not show one of them, it is checked individually with `--exists`, which
/// does not depend on `gst-inspect`'s output format.
pub const VERIFY: &[&str] = &[
    "pipewiresrc",
    "videorate",
    "videoconvert",
    "videoscale",
    "queue",
    "x264enc",
    "h264parse",
    "matroskamux",
    "fdsink",
    "pulsesrc",
    "audioconvert",
    "audioresample",
    "audiomixer",
    "volume",
    "avenc_aac",
    "aacparse",
];

pub fn detect() -> Option<GstInfo> {
    let launch = which("gst-launch-1.0")?;
    let inspect = which("gst-inspect-1.0")?;
    let mut list = command(&inspect);
    // Plain output whatever the user's environment says (colors, pager).
    list.arg("--no-colors").env("GST_INSPECT_NO_COLORS", "1").env("NO_COLOR", "1").env("PAGER", "cat").env("GST_PAGER", "cat");
    let mut elements = match run_capture(&mut list) {
        Ok((_, out, _)) => parse_inspect(&out),
        Err(e) => {
            log::warn!("gst-inspect-1.0 failed: {e}");
            BTreeSet::new()
        }
    };
    let listed = elements.len();
    for el in VERIFY {
        if !elements.contains(*el) && exists(&inspect, el) {
            elements.insert(el.to_string());
        }
    }
    if elements.len() != listed {
        log::warn!("gst-inspect-1.0 listing missed {} element(s) that --exists found", elements.len() - listed);
    }
    let missing: Vec<&str> = VERIFY.iter().copied().filter(|e| !elements.contains(*e)).collect();
    log::info!(
        "GStreamer: {} ({} elements of interest; missing: {})",
        launch.display(),
        elements.len(),
        if missing.is_empty() { "none".into() } else { missing.join(", ") }
    );
    let cuda_nvenc = probe_cuda_nvenc(&launch, &elements);
    Some(GstInfo { launch, elements, cuda_nvenc })
}

/// Real test encode through the NVIDIA GPU path (CUDA upload + conversion → NVENC).
/// Memoised per process: it takes ~0.5–1 s.
fn probe_cuda_nvenc(launch: &std::path::Path, elements: &BTreeSet<String>) -> bool {
    use std::sync::Mutex;
    const NEED: [&str; 3] = ["cudaupload", "cudaconvertscale", "nvh264enc"];
    if !NEED.iter().all(|e| elements.contains(*e)) {
        return false;
    }
    static CACHE: Mutex<Option<(PathBuf, bool)>> = Mutex::new(None);
    if let Some((p, ok)) = CACHE.lock().ok().and_then(|c| c.clone()) {
        if p == launch {
            return ok;
        }
    }
    let mut c = command(launch);
    c.args([
        "-q",
        "videotestsrc",
        "num-buffers=30",
        "!",
        "video/x-raw,format=BGRx,width=640,height=360,framerate=30/1",
        "!",
        "cudaupload",
        "!",
        "cudaconvertscale",
        "!",
        "video/x-raw(memory:CUDAMemory),format=NV12,width=640,height=360",
        "!",
        "nvh264enc",
        "!",
        "h264parse",
        "!",
        "fakesink",
    ]);
    let t0 = std::time::Instant::now();
    let ok = crate::process::run_with_timeout(&mut c, std::time::Duration::from_secs(15)).unwrap_or(false);
    log::info!("GStreamer CUDA -> NVENC probe: {} ({} ms)", if ok { "OK" } else { "unavailable" }, t0.elapsed().as_millis());
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((launch.to_path_buf(), ok));
    }
    ok
}

/// `gst-inspect-1.0 --exists <element>`: exit status 0 when the element is available.
fn exists(inspect: &std::path::Path, element: &str) -> bool {
    let mut c = command(inspect);
    c.args(["--exists", element]).env("GST_INSPECT_NO_COLORS", "1");
    run_capture(&mut c).map(|(ok, _, _)| ok).unwrap_or(false)
}

/// Parse `gst-inspect-1.0` (no args) output: `plugin:  element: Description`.
/// ANSI color sequences are ignored.
pub fn parse_inspect(out: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for line in out.lines() {
        let plain = strip_ansi(line);
        let line = plain.as_str();
        let mut parts = line.splitn(3, ':');
        let (Some(_plugin), Some(el), Some(_)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let el = el.trim();
        if INTERESTING.contains(&el) {
            set.insert(el.to_string());
        }
    }
    set
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI: parameters/intermediates until a final byte in @..~
                for d in chars.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
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

    #[test]
    fn parse_colored() {
        // `gst-inspect-1.0 --color` style output.
        let s = "\u{1b}[32mpipewire\u{1b}[00m:  \u{1b}[1;36mpipewiresrc\u{1b}[00m: \u{1b}[37mPipeWire source\u{1b}[00m\n";
        assert!(super::parse_inspect(s).contains("pipewiresrc"));
    }

    #[test]
    fn detect_finds_installed_elements() {
        // Real gst-inspect on this machine: every element the listing shows must also
        // pass `--exists`, and the result must agree with a plain listing.
        let Some(g) = super::detect() else { return };
        if g.has("pipewiresrc") {
            assert!(super::exists(&super::which("gst-inspect-1.0").unwrap(), "pipewiresrc"));
        }
        assert!(!super::exists(&super::which("gst-inspect-1.0").unwrap(), "vinci-no-such-element"));
    }
}
