//! A running capture pipeline (one or two helper processes + optional audio pump /
//! portal session), exposing the fragmented-MP4 stdout stream.

use crate::config::BackendKind;
use crate::pipeline::{ffmpeg_command, PipelineSpec, Priority};
#[cfg(target_os = "linux")]
use crate::pipeline::{gst_portal_args, remux_mkv_to_fmp4_args};
use crate::CaptureError;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use vinci_hw::{FfmpegInfo, GstInfo};

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub backend: BackendKind,
    pub encoder: String,
    pub codec: vinci_encoding::Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub monitor: String,
    /// New portal restore token to persist (Wayland).
    pub restore_token: Option<String>,
}

pub struct CaptureSession {
    children: Vec<(String, Child)>,
    stdout: Option<ChildStdout>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    #[cfg(windows)]
    audio: Option<crate::wasapi::AudioPump>,
    #[cfg(target_os = "linux")]
    _wayland: Option<WaylandSource>,
    pub info: SessionInfo,
}

const TAIL: usize = 60;

/// What feeds `pipewiresrc` on Wayland; kept alive for as long as the capture runs.
#[cfg(target_os = "linux")]
#[allow(dead_code)] // held for its Drop (closes the KWin stream / portal session)
enum WaylandSource {
    /// KWin direct capture: no dialog, no "screen shared" indicator.
    Kde(crate::kde::KdeStream),
    /// xdg-desktop-portal ScreenCast (GNOME, Hyprland, … or when KWin refuses).
    Portal(crate::portal::PortalSession),
}

fn apply_priority(cmd: &mut Command, p: Priority) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const BELOW_NORMAL: u32 = 0x0000_4000;
        const ABOVE_NORMAL: u32 = 0x0000_8000;
        let class = match p {
            Priority::BelowNormal => BELOW_NORMAL,
            Priority::AboveNormal => ABOVE_NORMAL,
            Priority::Normal => 0,
        };
        cmd.creation_flags(vinci_hw::process::CREATE_NO_WINDOW | class);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let nice: i32 = match p {
            Priority::BelowNormal => 5,
            _ => 0,
        };
        if nice != 0 {
            // SAFETY: `nice` is async-signal-safe and only affects the child.
            unsafe {
                cmd.pre_exec(move || {
                    libc::nice(nice);
                    Ok(())
                });
            }
        }
    }
}

fn spawn_stderr_reader(name: &str, child: &mut Child, tail: Arc<Mutex<VecDeque<String>>>) {
    let Some(stderr) = child.stderr.take() else { return };
    let name = name.to_string();
    let _ = std::thread::Builder::new().name(format!("vinci-{name}-stderr")).spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            log::warn!("[{name}] {line}");
            let mut t = tail.lock();
            if t.len() >= TAIL {
                t.pop_front();
            }
            t.push_back(format!("[{name}] {line}"));
        }
    });
}

impl CaptureSession {
    pub fn start(spec: &PipelineSpec, ffmpeg: &FfmpegInfo, gst: Option<&GstInfo>) -> Result<Self, CaptureError> {
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let info = SessionInfo {
            backend: spec.backend,
            encoder: spec.encoder.id.to_string(),
            codec: spec.encoder.codec,
            width: spec.out_size.0,
            height: spec.out_size.1,
            fps: spec.fps,
            monitor: spec.monitor.name.clone(),
            restore_token: None,
        };
        match spec.backend {
            BackendKind::Portal => Self::start_portal(spec, ffmpeg, gst, tail, info),
            _ => Self::start_ffmpeg(spec, ffmpeg, tail, info),
        }
    }

    fn start_ffmpeg(
        spec: &PipelineSpec,
        ffmpeg: &FfmpegInfo,
        tail: Arc<Mutex<VecDeque<String>>>,
        info: SessionInfo,
    ) -> Result<Self, CaptureError> {
        let cmdline = ffmpeg_command(spec, ffmpeg)?;
        log::info!("capture: {} {}", ffmpeg.path.display(), cmdline.args.join(" "));
        let mut cmd = vinci_hw::process::command(&ffmpeg.path);
        cmd.args(&cmdline.args).stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.stdin(if cmdline.stdin_audio { Stdio::piped() } else { Stdio::null() });
        apply_priority(&mut cmd, spec.priority);
        let mut child = cmd.spawn().map_err(|e| CaptureError::Spawn(format!("{}: {e}", ffmpeg.path.display())))?;
        spawn_stderr_reader("ffmpeg", &mut child, tail.clone());
        let stdout = child.stdout.take();
        #[cfg(windows)]
        let audio = if cmdline.stdin_audio {
            let stdin = child.stdin.take().ok_or_else(|| CaptureError::Spawn("no stdin for audio".into()))?;
            match crate::wasapi::AudioPump::start(stdin, &spec.audio) {
                Ok(p) => Some(p),
                Err(e) => {
                    let _ = child.kill();
                    return Err(CaptureError::Audio(e.to_string()));
                }
            }
        } else {
            None
        };
        Ok(Self {
            children: vec![("ffmpeg".into(), child)],
            stdout,
            stderr_tail: tail,
            #[cfg(windows)]
            audio,
            #[cfg(target_os = "linux")]
            _wayland: None,
            info,
        })
    }

    #[allow(unused_variables, unused_mut)]
    fn start_portal(
        spec: &PipelineSpec,
        ffmpeg: &FfmpegInfo,
        gst: Option<&GstInfo>,
        tail: Arc<Mutex<VecDeque<String>>>,
        mut info: SessionInfo,
    ) -> Result<Self, CaptureError> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::process::CommandExt;
            let gst = gst.ok_or_else(|| CaptureError::NoBackend("GStreamer not found".into()))?;
            // KDE Plasma: ask KWin directly (no screen-sharing dialog or indicator); any
            // other desktop, or KWin refusing, goes through the portal.
            let kde = if std::env::var_os("VINCI_NO_KWIN_DIRECT").is_some() {
                None
            } else {
                match crate::kde::KdeStream::start(spec.monitor.index, spec.cursor) {
                    Ok(k) => k,
                    Err(e) => {
                        log::warn!("{e}; using the screen-sharing portal instead");
                        None
                    }
                }
            };
            let (node_id, size, pw_fd, source) = match kde {
                Some(k) => {
                    log::info!("capture source: KWin direct screencast of {} (node {})", k.output_name, k.node);
                    info.monitor = match k.size {
                        Some((w, h)) => format!("{} {w}×{h} · KWin direct", k.output_name),
                        None => format!("{} · KWin direct", k.output_name),
                    };
                    (k.node, k.size, None, WaylandSource::Kde(k))
                }
                None => {
                    let portal = crate::portal::PortalSession::start(spec.portal_restore_token.clone(), spec.cursor)?;
                    info.restore_token = portal.restore_token.clone();
                    if let Some((w, h)) = portal.size {
                        info.monitor = format!("Portal stream {w}×{h}");
                    }
                    let fd = portal.fd.as_raw_fd();
                    (portal.node_id, portal.size, Some(fd), WaylandSource::Portal(portal))
                }
            };
            let mut spec = spec.clone();
            if let Some((w, h)) = size {
                let res = vinci_hw_size_fit(spec.out_size, (w.max(2) as u32, h.max(2) as u32));
                spec.out_size = res;
                spec.video.width = res.0;
                spec.video.height = res.1;
            }
            let (args, actual_codec) = gst_portal_args(&spec, gst, pw_fd.map(|_| 3), node_id);
            info.codec = actual_codec;
            log::info!("capture: {} {}", gst.launch.display(), args.join(" "));
            let mut g = vinci_hw::process::command(&gst.launch);
            g.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
            // SAFETY: dup2 is async-signal-safe; it places the portal's PipeWire remote on
            // fd 3 (without CLOEXEC) for `pipewiresrc fd=3`.
            if let Some(fd) = pw_fd {
                unsafe {
                    g.pre_exec(move || {
                        if fd == 3 {
                            // dup2(3, 3) is a no-op that keeps FD_CLOEXEC: clear it explicitly.
                            let flags = libc::fcntl(3, libc::F_GETFD);
                            if flags < 0 || libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                        } else if libc::dup2(fd, 3) < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
            }
            apply_priority(&mut g, spec.priority);
            let mut gchild = g.spawn().map_err(|e| CaptureError::Spawn(format!("gst-launch-1.0: {e}")))?;
            spawn_stderr_reader("gstreamer", &mut gchild, tail.clone());
            let gout = gchild.stdout.take().ok_or_else(|| CaptureError::Spawn("gst stdout".into()))?;
            let mut f = vinci_hw::process::command(&ffmpeg.path);
            f.args(remux_mkv_to_fmp4_args(spec.frag_ms, actual_codec == vinci_encoding::Codec::Hevc))
                .stdin(Stdio::from(gout))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut fchild = match f.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = gchild.kill();
                    return Err(CaptureError::Spawn(format!("ffmpeg remux: {e}")));
                }
            };
            spawn_stderr_reader("ffmpeg-remux", &mut fchild, tail.clone());
            let stdout = fchild.stdout.take();
            info.width = spec.out_size.0;
            info.height = spec.out_size.1;
            Ok(Self {
                children: vec![("gstreamer".into(), gchild), ("ffmpeg-remux".into(), fchild)],
                stdout,
                stderr_tail: tail,
                _wayland: Some(source),
                info,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(CaptureError::NoBackend("the PipeWire portal backend is Linux-only".into()))
        }
    }

    /// The fragmented-MP4 stream. Can be taken once.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.stdout.take()
    }

    pub fn pids(&self) -> Vec<u32> {
        self.children.iter().map(|(_, c)| c.id()).collect()
    }

    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail.lock().iter().cloned().collect()
    }

    /// `Some(description)` if any helper process has exited.
    pub fn exited(&mut self) -> Option<String> {
        for (name, c) in &mut self.children {
            if let Ok(Some(st)) = c.try_wait() {
                return Some(format!("{name} exited ({st})"));
            }
        }
        None
    }

    /// Stop everything. Fragments already received are complete (fragmented MP4 needs
    /// no trailer), so a hard kill loses nothing.
    pub fn stop(&mut self) {
        #[cfg(windows)]
        if let Some(mut a) = self.audio.take() {
            a.stop();
        }
        for (_, c) in &mut self.children {
            let _ = c.kill();
        }
        for (_, c) in &mut self.children {
            let _ = c.wait();
        }
        self.children.clear();
    }
}

#[cfg(target_os = "linux")]
fn vinci_hw_size_fit(requested: (u32, u32), source: (u32, u32)) -> (u32, u32) {
    // Keep the requested height cap but follow the portal stream's aspect ratio.
    let (_, rh) = requested;
    let (sw, sh) = source;
    let even = |v: u32| (v / 2 * 2).max(2);
    if sh <= rh {
        (even(sw), even(sh))
    } else {
        (even((sw as u64 * rh as u64 / sh as u64) as u32), even(rh))
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.stop();
    }
}
