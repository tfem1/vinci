//! FFmpeg discovery and capability inspection.
//!
//! VINCI drives FFmpeg as an out-of-process media engine: it gives us the native
//! capture sources (`gfxcapture` = Windows.Graphics.Capture, `ddagrab` = DXGI Desktop
//! Duplication, `x11grab` = XShm) and every hardware encoder (NVENC, AMF, QSV, VAAPI)
//! with zero-copy GPU paths, while keeping the VINCI process itself tiny.

use crate::process::{command, is_executable, run_capture, which};
use crate::HwError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FfmpegSource {
    /// `VINCI_FFMPEG` environment variable.
    Env,
    /// Path configured by the user in settings.
    Settings,
    /// Shipped next to the VINCI executable (Windows installer, AppImage).
    Bundled,
    /// Found on `PATH` (Linux distribution package).
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FfmpegInfo {
    pub path: PathBuf,
    pub ffprobe: Option<PathBuf>,
    pub source: FfmpegSource,
    /// Raw version token, e.g. `6.1.1-3ubuntu5`, `n8.1`, `N-121000-g1234`.
    pub version: String,
    /// Parsed `(major, minor)` when the version is a release number, else `None`
    /// (git builds) — use [`FfmpegInfo::lavfi`] for feature checks in that case.
    pub release: Option<(u32, u32)>,
    /// libavfilter version `(major, minor, micro)`.
    pub lavfi: (u32, u32, u32),
    pub encoders: BTreeSet<String>,
    pub filters: BTreeSet<String>,
    pub input_devices: BTreeSet<String>,
}

impl FfmpegInfo {
    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders.contains(name)
    }
    pub fn has_filter(&self, name: &str) -> bool {
        self.filters.contains(name)
    }
    pub fn has_input_device(&self, name: &str) -> bool {
        self.input_devices.contains(name)
    }
    /// Windows.Graphics.Capture source (FFmpeg ≥ 8.1).
    pub fn supports_wgc(&self) -> bool {
        self.has_filter("gfxcapture")
    }
    /// DXGI Desktop Duplication source (FFmpeg ≥ 6.0 on Windows).
    pub fn supports_dda(&self) -> bool {
        self.has_filter("ddagrab")
    }
}

const EXE: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
const PROBE_EXE: &str = if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" };

/// Ordered list of candidate FFmpeg binaries.
///
/// * Windows: bundled build first (it is a known-good FFmpeg 8.x with every hardware
///   encoder and `gfxcapture`), then `PATH`.
/// * Linux: distribution FFmpeg first (it matches the system VAAPI/Mesa stack), then
///   the bundled copy shipped inside the AppImage.
pub fn candidates(explicit: Option<&Path>) -> Vec<(PathBuf, FfmpegSource)> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("VINCI_FFMPEG") {
        out.push((PathBuf::from(p), FfmpegSource::Env));
    }
    if let Some(p) = explicit {
        out.push((p.to_path_buf(), FfmpegSource::Settings));
    }
    let mut bundled = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bundled.push(dir.join(EXE));
            bundled.push(dir.join("ffmpeg").join(EXE));
            // AppImage / Linux package layout: <prefix>/bin/vinci -> <prefix>/lib/vinci/ffmpeg
            bundled.push(dir.join("../lib/vinci").join(EXE));
            bundled.push(dir.join("../lib/VINCI").join(EXE));
        }
    }
    let system = which("ffmpeg");
    if cfg!(windows) {
        out.extend(bundled.into_iter().map(|p| (p, FfmpegSource::Bundled)));
        out.extend(system.map(|p| (p, FfmpegSource::System)));
    } else {
        out.extend(system.map(|p| (p, FfmpegSource::System)));
        out.extend(bundled.into_iter().map(|p| (p, FfmpegSource::Bundled)));
    }
    out
}

/// Find the first working FFmpeg among [`candidates`].
pub fn locate(explicit: Option<&Path>) -> Result<FfmpegInfo, HwError> {
    let cands = candidates(explicit);
    let mut errors = Vec::new();
    for (path, source) in &cands {
        if !is_executable(path) {
            continue;
        }
        match inspect(path, *source) {
            Ok(info) => return Ok(info),
            Err(e) => {
                log::warn!("ffmpeg candidate {} rejected: {e}", path.display());
                errors.push(format!("{} ({e})", path.display()));
            }
        }
    }
    let searched: Vec<String> = cands.iter().map(|(p, _)| p.display().to_string()).collect();
    if errors.is_empty() {
        Err(HwError::FfmpegNotFound(if searched.is_empty() { "PATH".into() } else { searched.join(", ") }))
    } else {
        Err(HwError::FfmpegBroken { path: errors.join("; "), reason: "no usable candidate".into() })
    }
}

/// Run the binary and read its version and capability lists.
pub fn inspect(path: &Path, source: FfmpegSource) -> Result<FfmpegInfo, HwError> {
    let broken = |reason: String| HwError::FfmpegBroken { path: path.display().to_string(), reason };
    let (ok, out, err) = run_capture(command(path).arg("-hide_banner").arg("-version")).map_err(|e| broken(e.to_string()))?;
    if !ok {
        return Err(broken(err.lines().last().unwrap_or("non-zero exit").to_string()));
    }
    let (version, release) = parse_version(&out);
    let lavfi = parse_lib_version(&out, "libavfilter").unwrap_or((0, 0, 0));

    let list = |flag: &str| -> String { run_capture(command(path).args(["-hide_banner", flag])).map(|(_, o, _)| o).unwrap_or_default() };
    let encoders = parse_encoders(&list("-encoders"));
    let filters = parse_filters(&list("-filters"));
    let input_devices = parse_devices(&list("-devices"));
    if encoders.is_empty() {
        return Err(broken("`ffmpeg -encoders` returned nothing".into()));
    }

    let ffprobe = path.parent().map(|d| d.join(PROBE_EXE)).filter(|p| is_executable(p)).or_else(|| which("ffprobe"));

    Ok(FfmpegInfo { path: path.to_path_buf(), ffprobe, source, version, release, lavfi, encoders, filters, input_devices })
}

/// `ffmpeg version 6.1.1-3ubuntu5 Copyright …` → (`6.1.1-3ubuntu5`, Some((6,1)))
/// `ffmpeg version n8.1-7-gabc …`          → (`n8.1-7-gabc`, Some((8,1)))
/// `ffmpeg version N-121000-g1234 …`       → (`N-121000-g1234`, None)
pub fn parse_version(out: &str) -> (String, Option<(u32, u32)>) {
    let first = out.lines().find(|l| l.starts_with("ffmpeg version")).unwrap_or("");
    let token = first.split_whitespace().nth(2).unwrap_or("unknown").to_string();
    let t = token.trim_start_matches('n');
    let mut parts = t.split(|c: char| !c.is_ascii_digit());
    let major = parts.next().and_then(|s| s.parse::<u32>().ok());
    let minor = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    // Gyan builds use dates ("2025-10-01-git-…"): treat as unknown release.
    let release = match major {
        Some(m) if (2..1000).contains(&m) && !t.starts_with("N-") => Some((m, minor)),
        _ => None,
    };
    (token, release)
}

/// `libavfilter    10.  1.100 / 10.  1.100` → (10, 1, 100)
pub fn parse_lib_version(out: &str, lib: &str) -> Option<(u32, u32, u32)> {
    let line = out.lines().find(|l| l.trim_start().starts_with(lib))?;
    let rest = line.trim_start().trim_start_matches(lib);
    let first = rest.split('/').next()?;
    let nums: Vec<u32> = first.split('.').filter_map(|s| s.trim().parse().ok()).collect();
    if nums.len() == 3 {
        Some((nums[0], nums[1], nums[2]))
    } else {
        None
    }
}

pub fn parse_encoders(out: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let mut started = false;
    for line in out.lines() {
        if line.trim_start().starts_with("------") {
            started = true;
            continue;
        }
        if !started {
            continue;
        }
        let mut it = line.split_whitespace();
        if let (Some(flags), Some(name)) = (it.next(), it.next()) {
            if flags.len() == 6 && matches!(flags.as_bytes()[0], b'V' | b'A' | b'S') {
                set.insert(name.to_string());
            }
        }
    }
    set
}

pub fn parse_filters(out: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for line in out.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() >= 3 && toks[2].contains("->") && toks[0].len() <= 3 {
            set.insert(toks[1].to_string());
        }
    }
    set
}

pub fn parse_devices(out: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let mut started = false;
    for line in out.lines() {
        let t = line.trim();
        // FFmpeg 6.x prints " --", newer builds "---".
        if t.len() >= 2 && t.chars().all(|c| c == '-') {
            started = true;
            continue;
        }
        if !started {
            continue;
        }
        let mut it = line.split_whitespace();
        if let (Some(flags), Some(name)) = (it.next(), it.next()) {
            if flags.contains('D') && flags.len() <= 2 {
                // "dshow" / "x11grab" / "pulse" / "lavfi"; some entries are "name1,name2"
                for n in name.split(',') {
                    set.insert(n.to_string());
                }
            }
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("ffmpeg version 6.1.1-3ubuntu5 Copyright (c)").1, Some((6, 1)));
        assert_eq!(parse_version("ffmpeg version n8.1-7-gabc Copyright").1, Some((8, 1)));
        assert_eq!(parse_version("ffmpeg version N-121000-g1234 Copyright").1, None);
        assert_eq!(parse_version("ffmpeg version 2025-10-01-git-abc-full_build-www.gyan.dev").1, None);
        assert_eq!(parse_lib_version("  libavfilter    10.  1.100 / 10.  1.100\n", "libavfilter"), Some((10, 1, 100)));
    }

    #[test]
    fn lists() {
        let enc = "Encoders:\n V..... = Video\n ------\n V....D h264_nvenc           NVIDIA NVENC\n A....D aac  AAC\n";
        let e = parse_encoders(enc);
        assert!(e.contains("h264_nvenc") && e.contains("aac") && !e.contains("="));
        let fil = "Filters:\n  T.. = Timeline support\n  | = Source or sink filter\n ... gfxcapture        |->V       Capture\n TSC scale V->V Scale\n";
        let f = parse_filters(fil);
        assert!(f.contains("gfxcapture") && f.contains("scale"));
        assert_eq!(f.len(), 2);
        let dev = "Devices:\n D. = Demuxing supported\n ---\n D  x11grab  X11\n DE alsa  ALSA\n  E sdl,sdl2 SDL\n";
        let d = parse_devices(dev);
        assert!(d.contains("x11grab") && d.contains("alsa") && !d.contains("sdl"));
        let dev61 =
            "Devices:\n D. = Demuxing supported\n .E = Muxing supported\n --\n DE alsa  ALSA\n D  lavfi  Libavfilter\n D  x11grab X11\n";
        let d = parse_devices(dev61);
        assert!(d.contains("lavfi") && d.contains("x11grab"), "FFmpeg 6.1 separator");
    }

    #[test]
    fn real_ffmpeg_if_present() {
        if let Ok(info) = locate(None) {
            assert!(info.has_encoder("aac"));
            assert!(info.lavfi.0 > 0);
            assert!(info.has_input_device("lavfi"), "devices parsed from the real binary");
        }
    }
}
