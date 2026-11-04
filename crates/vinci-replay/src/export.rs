//! Turning a [`Snapshot`] into a clip file — without re-encoding.
//!
//! 1. Each epoch is written as a self-contained fragmented MP4 (init + fragments)
//!    to a hidden `.part` file in the destination directory, then `fsync`ed.
//! 2. FFmpeg remuxes it (`-c copy`) into a regular MP4 with `+faststart` — timestamps
//!    rebased to 0, instant seeking, compatible with every editor/player/Discord.
//!    Multiple epochs (encoder restarted mid-buffer) are joined with the concat demuxer.
//! 3. The result is renamed into place atomically. If remuxing fails, the raw
//!    fragmented MP4 (already a valid, playable file) is kept instead, so a clip is
//!    never lost.

use crate::buffer::Snapshot;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("nothing buffered yet")]
    Empty,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExportResult {
    pub path: PathBuf,
    pub bytes: u64,
    pub duration_secs: f64,
    /// `false` when FFmpeg was unavailable/failed and the raw fragmented MP4 was kept.
    pub remuxed: bool,
}

/// Hook to customise how FFmpeg is spawned (e.g. hide the console on Windows).
pub type CommandFactory = dyn Fn(&Path) -> Command + Send + Sync;

pub fn export(snapshot: &Snapshot, dest: &Path, ffmpeg: Option<&Path>, make_cmd: &CommandFactory) -> Result<ExportResult, ExportError> {
    if snapshot.parts.is_empty() {
        return Err(ExportError::Empty);
    }
    let dir = dest.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir)?;
    let stem = dest.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip.mp4".into());

    // 1. write parts
    let mut parts = Vec::new();
    let cleanup = |parts: &[PathBuf]| {
        for p in parts {
            let _ = std::fs::remove_file(p);
        }
    };
    for (i, part) in snapshot.parts.iter().enumerate() {
        let p = dir.join(format!(".{stem}.part{i}.mp4"));
        let res = (|| -> io::Result<()> {
            let f = File::create(&p)?;
            let mut w = BufWriter::with_capacity(1 << 20, f);
            snapshot.write_part(part, &mut w)?;
            let f = w.into_inner().map_err(|e| e.into_error())?;
            f.sync_all()
        })();
        parts.push(p);
        if let Err(e) = res {
            cleanup(&parts);
            return Err(e.into());
        }
    }

    // 2. remux
    let tmp = dir.join(format!(".{stem}.vinci-tmp"));
    let remuxed = match ffmpeg {
        Some(ff) => match remux(ff, &parts, &tmp, &dir, &stem, make_cmd) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("remux failed, keeping fragmented MP4: {e}");
                let _ = std::fs::remove_file(&tmp);
                false
            }
        },
        None => false,
    };

    // 3. publish atomically
    if remuxed {
        std::fs::rename(&tmp, dest)?;
        cleanup(&parts);
    } else {
        // Keep the newest (usually only) epoch.
        let last = parts.pop().expect("at least one part");
        std::fs::rename(&last, dest)?;
        cleanup(&parts);
    }
    let bytes = std::fs::metadata(dest)?.len();
    Ok(ExportResult { path: dest.to_path_buf(), bytes, duration_secs: snapshot.duration, remuxed })
}

fn remux(ffmpeg: &Path, parts: &[PathBuf], tmp: &Path, dir: &Path, stem: &str, make_cmd: &CommandFactory) -> io::Result<()> {
    let mut cmd = make_cmd(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error"]);
    let list = dir.join(format!(".{stem}.concat.txt"));
    if parts.len() == 1 {
        cmd.arg("-i").arg(&parts[0]);
    } else {
        let mut f = File::create(&list)?;
        for p in parts {
            // concat demuxer quoting: wrap in single quotes, escape embedded quotes.
            let s = p.display().to_string().replace('\'', r"'\''");
            writeln!(f, "file '{s}'")?;
        }
        f.sync_all()?;
        cmd.args(["-f", "concat", "-safe", "0", "-i"]).arg(&list);
    }
    cmd.args(["-map", "0", "-c", "copy", "-avoid_negative_ts", "make_zero", "-movflags", "+faststart", "-f", "mp4"]).arg(tmp);
    let out = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output();
    let _ = std::fs::remove_file(&list);
    let out = out?;
    if !out.status.success() {
        return Err(io::Error::other(String::from_utf8_lossy(&out.stderr).trim().chars().take(400).collect::<String>()));
    }
    // fsync the result before publishing it
    File::open(tmp)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{ReplayBuffer, ReplayConfig, Storage};
    use crate::stream::{Fmp4Demuxer, Fmp4Event};
    use std::io::Read;

    fn ffmpeg() -> Option<PathBuf> {
        let p = std::env::var_os("PATH")?;
        std::env::split_paths(&p).map(|d| d.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" })).find(|p| p.exists())
    }
    fn ffprobe_duration(file: &Path) -> (f64, usize) {
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type", "-of", "default=nw=1"])
            .arg(file)
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        let dur = s.lines().find_map(|l| l.strip_prefix("duration=")).and_then(|d| d.parse().ok()).unwrap_or(0.0);
        let streams = s.lines().filter(|l| l.starts_with("codec_type=")).count();
        (dur, streams)
    }

    /// Run a real FFmpeg encoder producing fragmented MP4 on stdout (like the capture
    /// engine does), feed it through demuxer + buffer, export, and validate with ffprobe.
    fn encode_into(buf: &mut ReplayBuffer, ff: &Path, seconds: u32) {
        let mut child = Command::new(ff)
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"])
            .args(["-t", &seconds.to_string(), "-c:v", "libx264", "-preset", "ultrafast", "-g", "60", "-c:a", "aac", "-b:a", "96k"])
            .args(["-f", "mp4", "-movflags", "+frag_keyframe+empty_moov+default_base_moof", "-frag_duration", "500000", "pipe:1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = child.stdout.take().unwrap();
        let mut d = Fmp4Demuxer::new();
        let mut chunk = vec![0u8; 37_000]; // odd size on purpose
        let mut ev = Vec::new();
        loop {
            let n = out.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            d.push(&chunk[..n], &mut ev).unwrap();
            for e in ev.drain(..) {
                match e {
                    Fmp4Event::Init { data, info } => buf.begin_epoch(data, info),
                    Fmp4Event::Fragment(f) => buf.push(f).unwrap(),
                }
            }
        }
        assert!(child.wait().unwrap().success());
    }

    fn cmd(p: &Path) -> Command {
        Command::new(p)
    }

    #[test]
    fn real_export_memory_and_disk() {
        let Some(ff) = ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        for disk in [false, true] {
            let storage = if disk { Storage::Disk { path: dir.path().join("ring.bin"), capacity: 8 << 20 } } else { Storage::Memory };
            let mut buf = ReplayBuffer::new(ReplayConfig { max_duration_secs: 12.0, max_bytes: u64::MAX, storage }).unwrap();
            encode_into(&mut buf, &ff, 20);
            let st = buf.stats();
            assert!(st.buffered_secs >= 12.0 && st.buffered_secs <= 14.5, "{st:?}");
            let snap = buf.snapshot(8.0).unwrap();
            let dest = dir.path().join(format!("clip-{disk}.mp4"));
            let r = export(&snap, &dest, Some(&ff), &cmd).unwrap();
            assert!(r.remuxed);
            let (dur, streams) = ffprobe_duration(&dest);
            assert!((8.0..=10.2).contains(&dur), "duration {dur}");
            assert_eq!(streams, 2, "video + audio");
            let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with('.')).collect();
            assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
        }
    }

    #[test]
    fn real_export_across_encoder_restart() {
        let Some(ff) = ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let mut buf = ReplayBuffer::new(ReplayConfig { max_duration_secs: 30.0, max_bytes: u64::MAX, storage: Storage::Memory }).unwrap();
        encode_into(&mut buf, &ff, 6);
        encode_into(&mut buf, &ff, 6);
        let snap = buf.snapshot(10.0).unwrap();
        assert_eq!(snap.parts.len(), 2);
        let dest = dir.path().join("joined.mp4");
        let r = export(&snap, &dest, Some(&ff), &cmd).unwrap();
        assert!(r.remuxed);
        let (dur, _) = ffprobe_duration(&dest);
        assert!((10.0..=12.5).contains(&dur), "duration {dur}");
    }

    #[test]
    fn export_without_ffmpeg_keeps_playable_fmp4() {
        let Some(ff) = ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let mut buf = ReplayBuffer::new(ReplayConfig { max_duration_secs: 10.0, max_bytes: u64::MAX, storage: Storage::Memory }).unwrap();
        encode_into(&mut buf, &ff, 6);
        let snap = buf.snapshot(4.0).unwrap();
        let dest = dir.path().join("raw.mp4");
        let r = export(&snap, &dest, None, &cmd).unwrap();
        assert!(!r.remuxed);
        let (_, streams) = ffprobe_duration(&dest);
        assert_eq!(streams, 2, "raw fragmented MP4 is a valid file");
    }
}
