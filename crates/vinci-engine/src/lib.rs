//! # vinci-engine — the VINCI engine
//!
//! Owns every moving part and runs without any GUI (the Tauri app and `vinci-cli` are
//! thin front-ends over [`Engine`]):
//!
//! * **capture supervisor** — builds the native pipeline, feeds the replay buffer,
//!   restarts it on crashes (with backoff), falls back to the next working encoder
//!   when one keeps failing, and kills a stalled pipeline (watchdog);
//! * **save worker** — serialised, never blocks capture; waits for the freshest
//!   fragment so the clip includes the moment the hotkey was pressed;
//! * **job worker** — compression / trimming with progress and cancellation;
//! * **game detector**, **hotkeys**, **resource stats**, **updates**.

pub mod events;
pub mod hwstate;
pub mod logging;
pub mod settings;

pub use events::{EngineEvent, EngineStatus, ReplayState};
pub use hwstate::HwState;
pub use settings::*;
pub use vinci_hw::ResourceStats;

use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vinci_capture::{BackendKind, CaptureError, CaptureSession, CaptureTarget, MonitorInfo, PipelineSpec};
use vinci_clips::{Clip, ClipQuery, Library, LibraryStats, NewClip, ScanReport};
use vinci_encoding::{CompressOptions, MediaInfo, QualityProfile, VideoParams};
use vinci_games::{DetectedGame, GameDetector};
use vinci_hotkeys::{HotkeyAction, HotkeyService, HotkeyStatus};
use vinci_hw::StatsSampler;
use vinci_replay::{Fmp4Demuxer, Fmp4Event, ReplayBuffer, ReplayConfig, Storage};
use vinci_storage::AppPaths;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// GitHub `owner/repo` used for updates — set `VINCI_REPO` at build time.
pub const REPO: &str = match option_env!("VINCI_REPO") {
    Some(r) => r,
    None => "USERNAME/vinci",
};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Library(#[from] vinci_clips::LibraryError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<String> for EngineError {
    fn from(s: String) -> Self {
        EngineError::Msg(s)
    }
}
impl From<&str> for EngineError {
    fn from(s: &str) -> Self {
        EngineError::Msg(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Default)]
struct StatusInner {
    state: ReplayState,
    message: Option<String>,
    session: Option<vinci_capture::SessionInfo>,
    since_ms: Option<i64>,
    restarts: u32,
    fallback_note: Option<String>,
    saving: bool,
    last_clip_ms: Option<i64>,
}

struct CaptureHandle {
    stop: Arc<AtomicBool>,
    session: Arc<Mutex<Option<CaptureSession>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct SaveRequest {
    secs: Option<u32>,
    requested: Instant,
    reply: Option<Sender<std::result::Result<Clip, String>>>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobKind {
    Compress { options: CompressOptions, replace: bool },
    Trim { start: f64, end: f64, replace: bool },
}

struct JobRequest {
    id: u64,
    clip_id: i64,
    kind: JobKind,
    cancel: Arc<AtomicBool>,
}

/// UI-facing estimate of what the current settings cost.
#[derive(Debug, Clone, Serialize)]
pub struct Estimate {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: vinci_encoding::Codec,
    pub mb_per_minute: f64,
    pub buffer_mb: f64,
    pub buffer_storage: String,
    pub clip_mb: Vec<(u32, f64)>,
}

pub struct Inner {
    paths: AppPaths,
    settings: RwLock<Settings>,
    hw: RwLock<Option<Arc<HwState>>>,
    hw_detecting: Mutex<()>,
    buffer: Mutex<Option<ReplayBuffer>>,
    status: Mutex<StatusInner>,
    capture: Mutex<Option<CaptureHandle>>,
    subscribers: Mutex<Vec<Sender<EngineEvent>>>,
    library: Mutex<Library>,
    save_tx: Sender<SaveRequest>,
    job_tx: Sender<JobRequest>,
    jobs: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next_job: AtomicU64,
    game: RwLock<Option<DetectedGame>>,
    sampler: Mutex<StatsSampler>,
    hotkeys: Mutex<Option<HotkeyService>>,
    shutdown: AtomicBool,
    auto_started: AtomicBool,
}

/// Cheap to clone handle to the engine.
#[derive(Clone)]
pub struct Engine(Arc<Inner>);

impl Engine {
    /// Create the engine: load settings, open the library, start background workers.
    /// Hardware detection runs in the background; [`Engine::hardware`] waits for it.
    pub fn new(paths: AppPaths) -> Result<Self> {
        paths.ensure()?;
        let (mut settings, outcome) = vinci_storage::load_json::<Settings>(&paths.settings_file);
        settings.sanitize();
        if outcome != vinci_storage::LoadOutcome::Loaded {
            let _ = vinci_storage::save_json(&paths.settings_file, &settings);
        }
        let clips_dir = settings.library.clips_dir.clone().unwrap_or_else(|| paths.default_clips_dir.clone());
        std::fs::create_dir_all(&clips_dir)?;
        let removed = vinci_storage::cleanup_leftovers(&paths, &clips_dir, Duration::from_secs(5));
        if removed > 0 {
            log::info!("removed {removed} leftover temporary files from a previous session");
        }
        let library = Library::open(&paths.database_file, &paths.thumbnails_dir)?;
        let (save_tx, save_rx) = unbounded();
        let (job_tx, job_rx) = unbounded();
        let inner = Arc::new(Inner {
            paths,
            settings: RwLock::new(settings),
            hw: RwLock::new(None),
            hw_detecting: Mutex::new(()),
            buffer: Mutex::new(None),
            status: Mutex::new(StatusInner::default()),
            capture: Mutex::new(None),
            subscribers: Mutex::new(Vec::new()),
            library: Mutex::new(library),
            save_tx,
            job_tx,
            jobs: Mutex::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            game: RwLock::new(None),
            sampler: Mutex::new(StatsSampler::new()),
            hotkeys: Mutex::new(None),
            shutdown: AtomicBool::new(false),
            auto_started: AtomicBool::new(false),
        });
        let e = Engine(inner);
        {
            let e2 = e.clone();
            std::thread::Builder::new().name("vinci-save".into()).spawn(move || e2.save_worker(save_rx))?;
            let e3 = e.clone();
            std::thread::Builder::new().name("vinci-jobs".into()).spawn(move || e3.job_worker(job_rx))?;
            let e4 = e.clone();
            std::thread::Builder::new().name("vinci-games".into()).spawn(move || e4.game_loop())?;
            let e5 = e.clone();
            std::thread::Builder::new().name("vinci-init".into()).spawn(move || {
                let _ = e5.hardware(false);
                if let Ok(r) = e5.scan_library() {
                    if r.added + r.removed + r.updated > 0 {
                        e5.emit(EngineEvent::LibraryChanged);
                    }
                }
            })?;
        }
        Ok(e)
    }

    pub fn paths(&self) -> &AppPaths {
        &self.0.paths
    }

    // ------------------------------------------------------------------ events

    pub fn subscribe(&self) -> Receiver<EngineEvent> {
        let (tx, rx) = unbounded();
        self.0.subscribers.lock().push(tx);
        rx
    }

    fn emit(&self, ev: EngineEvent) {
        self.0.subscribers.lock().retain(|s| s.send(ev.clone()).is_ok());
    }

    fn emit_status(&self) {
        let st = self.status();
        self.emit(EngineEvent::Status(st));
    }

    fn set_state(&self, state: ReplayState, message: Option<String>) {
        {
            let mut s = self.0.status.lock();
            s.state = state;
            s.message = message;
            if matches!(state, ReplayState::Stopped | ReplayState::Error) {
                s.session = None;
                s.since_ms = None;
            }
        }
        self.emit_status();
    }

    // ---------------------------------------------------------------- settings

    pub fn settings(&self) -> Settings {
        self.0.settings.read().clone()
    }

    /// Apply new settings: persisted atomically; the capture pipeline restarts only if
    /// a capture-relevant field changed (buffer length is applied live, keeping footage).
    pub fn update_settings(&self, mut new: Settings) -> Result<Settings> {
        new.sanitize();
        let old = self.settings();
        // The portal token is owned by the engine.
        if new.portal_restore_token.is_none() {
            new.portal_restore_token = old.portal_restore_token.clone();
        }
        vinci_storage::save_json(&self.0.paths.settings_file, &new)?;
        *self.0.settings.write() = new.clone();
        if new.advanced.ffmpeg_path != old.advanced.ffmpeg_path {
            *self.0.hw.write() = None;
        }
        if new.replay.buffer_secs != old.replay.buffer_secs {
            if let Some(b) = self.0.buffer.lock().as_mut() {
                b.set_max_duration(new.replay.buffer_secs as f64 + new.encoding.keyframe_interval_secs as f64);
            }
        }
        if let Some(dir) = &new.library.clips_dir {
            std::fs::create_dir_all(dir)?;
        }
        if new.hotkeys != old.hotkeys {
            self.rebind_hotkeys();
        }
        if new.capture_changed(&old) && self.is_capturing() {
            log::info!("capture settings changed: restarting the replay buffer");
            self.stop_replay();
            *self.0.buffer.lock() = None;
            self.start_replay()?;
        }
        self.emit_status();
        Ok(new)
    }

    pub fn clips_dir(&self) -> PathBuf {
        let s = self.0.settings.read();
        s.library.clips_dir.clone().unwrap_or_else(|| self.0.paths.default_clips_dir.clone())
    }

    // ---------------------------------------------------------------- hardware

    /// Hardware report + validated encoders (cached; `force` re-probes everything).
    pub fn hardware(&self, force: bool) -> Arc<HwState> {
        if !force {
            if let Some(h) = self.0.hw.read().clone() {
                return h;
            }
        }
        let _guard = self.0.hw_detecting.lock();
        if !force {
            if let Some(h) = self.0.hw.read().clone() {
                return h;
            }
        }
        let ffmpeg = self.settings().advanced.ffmpeg_path;
        let st = Arc::new(hwstate::load_or_detect(&self.0.paths.hardware_cache_file, ffmpeg.as_deref(), force));
        *self.0.hw.write() = Some(st.clone());
        st
    }

    pub fn monitors(&self) -> Vec<MonitorInfo> {
        vinci_capture::list_monitors()
    }

    pub fn audio_devices(&self) -> Vec<vinci_capture::devices::AudioDevice> {
        vinci_capture::devices::list_audio_devices()
    }

    // ------------------------------------------------------------------ status

    pub fn status(&self) -> EngineStatus {
        let s = self.0.status.lock();
        let buf = self.0.buffer.lock();
        let stats = buf.as_ref().map(|b| b.stats()).unwrap_or_default();
        let settings = self.0.settings.read();
        EngineStatus {
            state: s.state,
            message: s.message.clone(),
            buffered_secs: stats.buffered_secs,
            buffer_target_secs: settings.replay.buffer_secs,
            buffer_bytes: stats.bytes,
            buffer_storage: if stats.disk { "disk".into() } else { "memory".into() },
            session: s.session.clone(),
            since_ms: s.since_ms,
            restarts: s.restarts,
            game: self.0.game.read().clone(),
            saving: s.saving,
            fallback_note: s.fallback_note.clone(),
            last_clip_ms: s.last_clip_ms,
        }
    }

    pub fn is_capturing(&self) -> bool {
        self.0.capture.lock().is_some()
    }

    pub fn stats(&self) -> ResourceStats {
        self.0.sampler.lock().sample()
    }

    // ----------------------------------------------------------------- capture

    pub fn start_replay(&self) -> Result<()> {
        let mut cap = self.0.capture.lock();
        if cap.is_some() {
            return Ok(());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let slot = Arc::new(Mutex::new(None));
        let (e, st, sl) = (self.clone(), stop.clone(), slot.clone());
        {
            let mut s = self.0.status.lock();
            s.state = ReplayState::Starting;
            s.message = None;
            s.restarts = 0;
        }
        let thread = std::thread::Builder::new().name("vinci-capture".into()).spawn(move || e.capture_loop(st, sl))?;
        *cap = Some(CaptureHandle { stop, session: slot, thread: Some(thread) });
        drop(cap);
        self.emit_status();
        Ok(())
    }

    pub fn stop_replay(&self) {
        let handle = self.0.capture.lock().take();
        if let Some(mut h) = handle {
            h.stop.store(true, Ordering::SeqCst);
            if let Some(mut s) = h.session.lock().take() {
                s.stop();
            }
            if let Some(t) = h.thread.take() {
                let _ = t.join();
            }
        }
        self.0.auto_started.store(false, Ordering::SeqCst);
        self.0.sampler.lock().set_tracked_pids(&[]);
        self.set_state(ReplayState::Stopped, None);
    }

    pub fn toggle_replay(&self) -> Result<bool> {
        if self.is_capturing() {
            self.stop_replay();
            Ok(false)
        } else {
            self.start_replay()?;
            Ok(true)
        }
    }

    /// Discard the buffered footage (e.g. before sharing the screen).
    pub fn clear_buffer(&self) {
        if let Some(b) = self.0.buffer.lock().as_mut() {
            b.clear();
        }
        self.emit_status();
    }

    fn build_spec(&self, hw: &HwState, blacklist: &HashSet<String>) -> std::result::Result<(PipelineSpec, vinci_hw::FfmpegInfo, Option<String>), String> {
        let s = self.settings();
        let ff = hw.report.ffmpeg.clone().ok_or_else(|| hw.report.ffmpeg_error.clone().unwrap_or_else(|| "FFmpeg not found".into()))?;
        let display = hw.report.system.display_server;
        let backend = vinci_capture::resolve_backend(s.capture.backend, &ff, hw.report.gstreamer.as_ref(), display).map_err(|e| e.to_string())?;
        let monitor = match backend {
            BackendKind::Test | BackendKind::Portal => MonitorInfo::synthetic(1920, 1080),
            _ => {
                let mons = vinci_capture::list_monitors();
                let idx = match &s.capture.target {
                    CaptureTarget::Monitor { index } => *index,
                    _ => mons.iter().find(|m| m.primary).map(|m| m.index).unwrap_or(0),
                };
                vinci_capture::monitors::pick(&mons, idx).ok_or_else(|| CaptureError::NoMonitor.to_string())?
            }
        };
        let window_exe = match (&s.capture.target, backend) {
            (CaptureTarget::Window { exe }, BackendKind::Wgc) if !exe.trim().is_empty() => Some(exe.clone()),
            (CaptureTarget::FollowGame, BackendKind::Wgc) => self.0.game.read().as_ref().map(|g| g.exe.clone()).filter(|e| !e.is_empty()),
            _ => None,
        };
        let profile = s.effective_profile();
        let codec = s.encoding.codec.resolve(profile);
        let probes: Vec<_> = hw.probes.iter().filter(|p| !blacklist.contains(&p.id)).cloned().collect();
        let (enc, fell_back) = vinci_encoding::select(&probes, codec, s.encoding.encoder.as_deref(), s.encoding.allow_software).ok_or_else(|| {
            if hw.probes.iter().any(|p| p.ok) {
                "every working encoder failed repeatedly — see Settings → Diagnostics".to_string()
            } else {
                "no working video encoder found (no hardware encoder, and CPU encoding is disabled or missing from FFmpeg)".to_string()
            }
        })?;
        let fps = s.effective_fps();
        let out = if backend == BackendKind::Test {
            s.capture.resolution.output_size(1920, 1080)
        } else {
            s.capture.resolution.output_size(monitor.width, monitor.height)
        };
        let video = VideoParams { codec: enc.codec, profile, width: out.0, height: out.1, fps, keyint_secs: s.encoding.keyframe_interval_secs };
        let note = fell_back.then(|| format!("Using {} — the requested {} encoder is not available on this PC", enc.label, codec.label()));
        let spec = PipelineSpec {
            backend,
            monitor,
            window_exe,
            out_size: out,
            fps,
            cursor: s.capture.cursor,
            audio: s.capture.audio.clone(),
            encoder: *enc,
            video,
            hw: hw.hw.clone(),
            audio_kbps: vinci_encoding::profile::audio_kbps(profile),
            frag_ms: s.advanced.fragment_ms,
            audio_offset_ms: s.advanced.audio_offset_ms,
            priority: s.performance.capture_priority,
            portal_restore_token: s.portal_restore_token.clone(),
            x11_display: None,
        };
        Ok((spec, ff, note))
    }

    /// Replay buffer sized for the current settings (RAM or disk ring).
    fn make_buffer(&self, s: &Settings, video: &VideoParams) -> std::io::Result<ReplayBuffer> {
        let keyint = s.encoding.keyframe_interval_secs as f64;
        let dur = s.replay.buffer_secs as f64 + keyint;
        let peak_kbps = vinci_encoding::profile::max_kbps(video.codec, video.profile, video.width, video.height, video.fps) + vinci_encoding::profile::audio_kbps(video.profile);
        let est_bytes = (peak_kbps as f64 * 1000.0 / 8.0 * (dur + keyint)) as u64;
        let ram_cap = s.replay.max_ram_mb as u64 * 1024 * 1024;
        let use_disk = match s.replay.storage {
            BufferStorage::Memory => false,
            BufferStorage::Disk => true,
            BufferStorage::Auto => est_bytes > ram_cap,
        };
        let storage = if use_disk {
            let capacity = (est_bytes as f64 * 1.5) as u64;
            let capacity = capacity.max(64 << 20);
            let free = vinci_storage::free_space(&self.0.paths.ring_dir).unwrap_or(u64::MAX);
            if free < capacity + (512 << 20) {
                log::warn!("not enough disk space for a {capacity}-byte replay ring; using RAM");
                Storage::Memory
            } else {
                Storage::Disk { path: self.0.paths.ring_dir.join(format!("replay-{}.ring", std::process::id())), capacity }
            }
        } else {
            Storage::Memory
        };
        let max_bytes = match storage {
            Storage::Memory => ((est_bytes as f64 * 1.6) as u64).clamp(32 << 20, ram_cap.max(est_bytes * 2)),
            Storage::Disk { .. } => u64::MAX,
        };
        log::info!("replay buffer: {:.0} s, storage {:?}, estimated {:.0} MB", dur, storage, est_bytes as f64 / 1e6);
        ReplayBuffer::new(ReplayConfig { max_duration_secs: dur, max_bytes, storage })
    }

    fn capture_loop(&self, stop: Arc<AtomicBool>, slot: Arc<Mutex<Option<CaptureSession>>>) {
        let mut blacklist: HashSet<String> = HashSet::new();
        let mut consecutive = 0u32;
        let mut backoff = Duration::from_millis(500);
        let sleep_interruptible = |d: Duration| {
            let end = Instant::now() + d;
            while Instant::now() < end && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        while !stop.load(Ordering::SeqCst) {
            let hw = self.hardware(false);
            let (spec, ff, note) = match self.build_spec(&hw, &blacklist) {
                Ok(x) => x,
                Err(e) => {
                    log::error!("cannot start capture: {e}");
                    self.set_state(ReplayState::Error, Some(e));
                    break;
                }
            };
            if self.0.buffer.lock().is_none() {
                match self.make_buffer(&self.settings(), &spec.video) {
                    Ok(b) => *self.0.buffer.lock() = Some(b),
                    Err(e) => {
                        self.set_state(ReplayState::Error, Some(format!("cannot allocate the replay buffer: {e}")));
                        break;
                    }
                }
            }
            let started = Instant::now();
            let mut session = match CaptureSession::start(&spec, &ff, hw.report.gstreamer.as_ref()) {
                Ok(s) => s,
                Err(e) => {
                    let fatal = matches!(e, CaptureError::NoBackend(_) | CaptureError::Portal(_));
                    log::error!("capture start failed: {e}");
                    if fatal {
                        self.set_state(ReplayState::Error, Some(e.to_string()));
                        break;
                    }
                    consecutive += 1;
                    if consecutive >= 2 {
                        blacklist.insert(spec.encoder.id.to_string());
                    }
                    self.set_state(ReplayState::Restarting, Some(e.to_string()));
                    sleep_interruptible(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(10));
                    continue;
                }
            };
            if let Some(tok) = session.info.restore_token.clone() {
                let mut s = self.0.settings.write();
                if s.portal_restore_token.as_deref() != Some(tok.as_str()) {
                    s.portal_restore_token = Some(tok);
                    let _ = vinci_storage::save_json(&self.0.paths.settings_file, &*s);
                }
            }
            self.0.sampler.lock().set_tracked_pids(&session.pids());
            let stdout = session.take_stdout();
            {
                let mut st = self.0.status.lock();
                st.session = Some(session.info.clone());
                st.fallback_note = note.clone();
            }
            *slot.lock() = Some(session);
            if stop.load(Ordering::SeqCst) {
                break;
            }

            // Watchdog: kill a pipeline that stops producing data (driver hang, minimised
            // window with WGC, portal stream paused…). The supervisor then restarts it.
            let last_data = Arc::new(AtomicU64::new(0));
            let done = Arc::new(AtomicBool::new(false));
            let watchdog = {
                let (last, done, slot, stop) = (last_data.clone(), done.clone(), slot.clone(), stop.clone());
                std::thread::spawn(move || {
                    while !done.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(500));
                        let el = started.elapsed().as_millis() as u64;
                        let last = last.load(Ordering::SeqCst);
                        let limit = if last == 0 { 25_000 } else { 12_000 };
                        if el.saturating_sub(last) > limit {
                            log::warn!("capture pipeline stalled for {} ms, restarting", el - last);
                            if let Some(s) = slot.lock().as_mut() {
                                s.stop();
                            }
                            break;
                        }
                    }
                })
            };

            let mut got_fragment = false;
            if let Some(mut out) = stdout {
                let mut demux = Fmp4Demuxer::new();
                let mut buf = vec![0u8; 256 * 1024];
                let mut events = Vec::new();
                let mut last_status = Instant::now();
                loop {
                    let n = match out.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    last_data.store(started.elapsed().as_millis() as u64, Ordering::SeqCst);
                    if let Err(e) = demux.push(&buf[..n], &mut events) {
                        log::error!("invalid stream from encoder: {e}");
                        break;
                    }
                    let mut first_fragment = false;
                    {
                        // Lock order everywhere: status → buffer. Never take status while
                        // holding the buffer lock.
                        let mut guard = self.0.buffer.lock();
                        if let Some(b) = guard.as_mut() {
                            for ev in events.drain(..) {
                                match ev {
                                    Fmp4Event::Init { data, info } => b.begin_epoch(data, info),
                                    Fmp4Event::Fragment(f) => {
                                        if let Err(e) = b.push(f) {
                                            log::error!("replay buffer write failed: {e}");
                                        }
                                        if !got_fragment {
                                            got_fragment = true;
                                            first_fragment = true;
                                        }
                                    }
                                }
                            }
                        } else {
                            events.clear();
                        }
                    }
                    if first_fragment {
                        {
                            let mut st = self.0.status.lock();
                            st.state = ReplayState::Running;
                            st.message = None;
                            st.since_ms = Some(now_ms());
                        }
                        self.emit_status();
                    }
                    if got_fragment && last_status.elapsed() > Duration::from_millis(1000) {
                        last_status = Instant::now();
                        self.emit_status();
                    }
                }
            }
            done.store(true, Ordering::SeqCst);
            let _ = watchdog.join();
            let (tail, exit) = match slot.lock().take() {
                Some(mut s) => {
                    let exit = s.exited();
                    let tail = s.stderr_tail();
                    s.stop();
                    (tail, exit)
                }
                None => (Vec::new(), None),
            };
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let ran = started.elapsed();
            if !got_fragment || ran < Duration::from_secs(10) {
                consecutive += 1;
            } else {
                consecutive = 0;
                backoff = Duration::from_millis(500);
            }
            if consecutive >= 3 {
                log::warn!("encoder {} failed {consecutive} times in a row, trying another one", spec.encoder.id);
                blacklist.insert(spec.encoder.id.to_string());
                consecutive = 0;
            }
            let reason = tail.last().cloned().or(exit).unwrap_or_else(|| "capture pipeline stopped".into());
            log::warn!("capture pipeline ended after {:.1} s: {reason}", ran.as_secs_f64());
            {
                let mut st = self.0.status.lock();
                st.restarts += 1;
            }
            self.set_state(ReplayState::Restarting, Some(reason));
            sleep_interruptible(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(10));
        }
        if !stop.load(Ordering::SeqCst) {
            // Ended on error: release the handle so the user can retry.
            let me = self.clone();
            std::thread::spawn(move || {
                let h = me.0.capture.lock().take();
                if let Some(mut h) = h {
                    h.thread.take(); // this very thread — do not join
                }
            });
        }
    }

    // -------------------------------------------------------------------- save

    /// Queue a save of the last `secs` seconds (default length when `None`).
    /// Returns immediately; the result arrives as [`EngineEvent::ClipSaved`] / `ClipFailed`.
    pub fn save_clip(&self, secs: Option<u32>) {
        let _ = self.0.save_tx.send(SaveRequest { secs, requested: Instant::now(), reply: None });
    }

    /// Save and wait for the result (CLI, tests).
    pub fn save_clip_blocking(&self, secs: Option<u32>) -> std::result::Result<Clip, String> {
        let (tx, rx) = unbounded();
        self.0.save_tx.send(SaveRequest { secs, requested: Instant::now(), reply: Some(tx) }).map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }

    fn save_worker(&self, rx: Receiver<SaveRequest>) {
        for req in rx {
            if self.0.shutdown.load(Ordering::SeqCst) {
                break;
            }
            self.0.status.lock().saving = true;
            self.emit_status();
            let t0 = Instant::now();
            let res = self.do_save(&req);
            {
                let mut st = self.0.status.lock();
                st.saving = false;
                if res.is_ok() {
                    st.last_clip_ms = Some(now_ms());
                }
            }
            match &res {
                Ok((clip, remuxed)) => {
                    log::info!("clip saved: {} ({:.1} s, {} bytes) in {} ms", clip.path.display(), clip.duration, clip.size, t0.elapsed().as_millis());
                    self.emit(EngineEvent::ClipSaved { clip: clip.clone(), remuxed: *remuxed, elapsed_ms: t0.elapsed().as_millis() as u64 });
                    self.emit(EngineEvent::LibraryChanged);
                }
                Err(e) => {
                    log::error!("clip save failed: {e}");
                    self.emit(EngineEvent::ClipFailed { message: e.clone() });
                }
            }
            self.emit_status();
            if let Some(tx) = req.reply {
                let _ = tx.send(res.map(|(c, _)| c));
            }
        }
    }

    fn do_save(&self, req: &SaveRequest) -> std::result::Result<(Clip, bool), String> {
        let s = self.settings();
        let secs = req.secs.unwrap_or(s.replay.default_clip_secs).clamp(5, MAX_BUFFER_SECS);
        // Wait (briefly) for the fragment covering the moment of the request.
        let deadline = req.requested + Duration::from_millis(s.advanced.fragment_ms as u64 + 500);
        loop {
            let fresh = self.0.buffer.lock().as_ref().and_then(|b| b.last_push()).is_some_and(|t| t >= req.requested);
            let running = self.0.status.lock().state == ReplayState::Running;
            if fresh || !running || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let snap = self.0.buffer.lock().as_ref().and_then(|b| b.snapshot(secs as f64)).ok_or("Nothing to save yet — start the replay buffer first")?;
        let hw = self.hardware(false);
        let ff = hw.report.ffmpeg.clone().ok_or("FFmpeg not found")?;
        let game = self.0.game.read().as_ref().map(|g| g.name.clone());
        let now = chrono::Local::now();
        let root = self.clips_dir();
        let dir = match (&game, s.library.organize_by_game) {
            (Some(g), true) => root.join(vinci_storage::sanitize_filename(g)),
            _ => root,
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        if let Some(free) = vinci_storage::free_space(&dir) {
            if free < snap.bytes * 2 + (50 << 20) {
                return Err(format!("not enough free space in {} ({} MB left)", dir.display(), free / 1_000_000));
            }
        }
        let path = vinci_storage::unique_path(&dir, &vinci_clips::default_file_stem(game.as_deref(), now), "mp4");
        let res = vinci_replay::export(&snap, &path, Some(&ff.path), &|p: &Path| vinci_hw::process::command(p)).map_err(|e| e.to_string())?;
        let media = ff
            .ffprobe
            .as_ref()
            .and_then(|p| vinci_encoding::transcode::probe_media(p, &path).ok())
            .unwrap_or(MediaInfo { duration_secs: snap.duration, size_bytes: res.bytes, ..Default::default() });
        let encoder = self.0.status.lock().session.as_ref().map(|s| s.encoder.clone());
        let lib = self.0.library.lock();
        let clip = lib
            .insert(NewClip { path, title: vinci_clips::default_title(game.as_deref(), now), game, created_at: now.timestamp_millis(), encoder, media })
            .map_err(|e| e.to_string())?;
        let _ = lib.ensure_thumbnail(clip.id, &ff.path);
        let clip = lib.get(clip.id).map_err(|e| e.to_string())?;
        Ok((clip, res.remuxed))
    }

    // ----------------------------------------------------------------- library

    pub fn list_clips(&self, q: &ClipQuery) -> Result<Vec<Clip>> {
        Ok(self.0.library.lock().list(q)?)
    }
    pub fn clip(&self, id: i64) -> Result<Clip> {
        Ok(self.0.library.lock().get(id)?)
    }
    pub fn library_stats(&self) -> Result<LibraryStats> {
        Ok(self.0.library.lock().stats()?)
    }
    pub fn rename_clip(&self, id: i64, title: &str) -> Result<Clip> {
        let c = self.0.library.lock().rename(id, title)?;
        self.emit(EngineEvent::LibraryChanged);
        Ok(c)
    }
    pub fn set_favorite(&self, id: i64, fav: bool) -> Result<Clip> {
        Ok(self.0.library.lock().set_favorite(id, fav)?)
    }
    pub fn set_clip_game(&self, id: i64, game: Option<&str>) -> Result<Clip> {
        Ok(self.0.library.lock().set_game(id, game)?)
    }
    pub fn delete_clip(&self, id: i64) -> Result<()> {
        self.0.library.lock().delete(id, true)?;
        self.emit(EngineEvent::LibraryChanged);
        Ok(())
    }
    pub fn move_clip(&self, id: i64, dest: &Path) -> Result<Clip> {
        let c = self.0.library.lock().move_to(id, dest)?;
        self.emit(EngineEvent::LibraryChanged);
        Ok(c)
    }
    pub fn thumbnail(&self, id: i64) -> Result<Option<PathBuf>> {
        let ff = self.hardware(false).report.ffmpeg.clone().ok_or("FFmpeg not found")?;
        Ok(self.0.library.lock().ensure_thumbnail(id, &ff.path)?)
    }
    pub fn scan_library(&self) -> Result<ScanReport> {
        let dir = self.clips_dir();
        let ffprobe = self.hardware(false).report.ffmpeg.as_ref().and_then(|f| f.ffprobe.clone());
        let probe = move |p: &Path| ffprobe.as_ref().and_then(|fp| vinci_encoding::transcode::probe_media(fp, p).ok());
        Ok(self.0.library.lock().scan(&dir, &probe)?)
    }

    // -------------------------------------------------------------------- jobs

    /// Queue a compression/trim job. Progress: `JobProgress`, result: `JobDone`.
    pub fn start_job(&self, clip_id: i64, kind: JobKind) -> u64 {
        let id = self.0.next_job.fetch_add(1, Ordering::SeqCst);
        let cancel = Arc::new(AtomicBool::new(false));
        self.0.jobs.lock().insert(id, cancel.clone());
        let _ = self.0.job_tx.send(JobRequest { id, clip_id, kind, cancel });
        id
    }

    pub fn cancel_job(&self, id: u64) {
        if let Some(c) = self.0.jobs.lock().get(&id) {
            c.store(true, Ordering::SeqCst);
        }
    }

    fn job_worker(&self, rx: Receiver<JobRequest>) {
        for job in rx {
            let kind_name = match job.kind {
                JobKind::Compress { .. } => "compress",
                JobKind::Trim { .. } => "trim",
            };
            let res = self.run_job(&job, kind_name);
            self.0.jobs.lock().remove(&job.id);
            let (clip, error) = match res {
                Ok(c) => (Some(c), None),
                Err(e) => (None, Some(e)),
            };
            self.emit(EngineEvent::JobDone { job_id: job.id, clip_id: job.clip_id, kind: kind_name.into(), clip, error });
            self.emit(EngineEvent::LibraryChanged);
        }
    }

    fn run_job(&self, job: &JobRequest, kind_name: &str) -> std::result::Result<Clip, String> {
        let clip = self.clip(job.clip_id).map_err(|e| e.to_string())?;
        let hw = self.hardware(false);
        let ff = hw.report.ffmpeg.clone().ok_or("FFmpeg not found")?;
        let dir = clip.path.parent().map(Path::to_path_buf).unwrap_or_else(|| self.clips_dir());
        let stem = clip.path.file_stem().and_then(|s| s.to_str()).unwrap_or("clip").to_string();
        let replace = match &job.kind {
            JobKind::Compress { replace, .. } | JobKind::Trim { replace, .. } => *replace,
        };
        // Intermediate output (hidden) — renamed over the original on success when replacing.
        let out = if replace { dir.join(format!(".{stem}.vinci-job.mp4")) } else { vinci_storage::unique_path(&dir, &format!("{stem} ({})", if kind_name == "trim" { "trimmed" } else { "compressed" }), "mp4") };
        let mut last = Instant::now() - Duration::from_secs(1);
        let mut used_encoder: Option<String> = None;
        match &job.kind {
            JobKind::Compress { options, .. } => {
                let enc = vinci_encoding::select_offline(&hw.probes, options.codec).ok_or("no working encoder")?;
                used_encoder = Some(enc.id.to_string());
                let j = vinci_encoding::transcode::Job { ffmpeg: &ff, encoder: enc, hw: &hw.hw, cancel: &job.cancel };
                vinci_encoding::transcode::compress(&j, &clip.path, &out, options, |p| {
                    if last.elapsed() > Duration::from_millis(250) {
                        last = Instant::now();
                        self.emit(EngineEvent::JobProgress { job_id: job.id, clip_id: clip.id, kind: kind_name.into(), progress: p });
                    }
                })
                .map_err(|e| e.to_string())?;
            }
            JobKind::Trim { start, end, .. } => {
                vinci_encoding::transcode::trim(&ff, &clip.path, &out, *start, *end, &job.cancel).map_err(|e| e.to_string())?;
            }
        }
        let media = ff.ffprobe.as_ref().and_then(|p| vinci_encoding::transcode::probe_media(p, &out).ok()).unwrap_or_default();
        let lib = self.0.library.lock();
        let result = if replace {
            let final_path = clip.path.with_extension("mp4");
            std::fs::rename(&out, &final_path).map_err(|e| e.to_string())?;
            if final_path != clip.path {
                let _ = std::fs::remove_file(&clip.path);
                lib.set_path(clip.id, &final_path).map_err(|e| e.to_string())?;
            }
            if let Some(t) = &clip.thumbnail {
                let _ = std::fs::remove_file(t);
            }
            lib.refresh_media(clip.id, &media, used_encoder.as_deref()).map_err(|e| e.to_string())?
        } else {
            let suffix = if kind_name == "trim" { "trimmed" } else { "compressed" };
            lib.insert(NewClip { path: out, title: format!("{} ({suffix})", clip.title), game: clip.game.clone(), created_at: now_ms(), encoder: used_encoder, media }).map_err(|e| e.to_string())?
        };
        let _ = lib.ensure_thumbnail(result.id, &ff.path);
        lib.get(result.id).map_err(|e| e.to_string())
    }

    // ------------------------------------------------------------------- games

    fn game_loop(&self) {
        let mut det = GameDetector::new();
        let mut custom_len = usize::MAX;
        while !self.0.shutdown.load(Ordering::SeqCst) {
            let s = self.settings();
            if s.custom_games.len() != custom_len {
                custom_len = s.custom_games.len();
                det.set_custom_games(s.custom_games.clone());
            }
            let found = det.detect();
            let changed = {
                let cur = self.0.game.read();
                cur.as_ref().map(|g| (&g.name, g.pid)) != found.as_ref().map(|g| (&g.name, g.pid))
            };
            if changed {
                let previous = self.0.game.read().as_ref().map(|g| g.name.clone());
                *self.0.game.write() = found.clone();
                if previous.as_deref() != found.as_ref().map(|g| g.name.as_str()) {
                    log::info!("game: {:?}", found.as_ref().map(|g| &g.name));
                    self.emit(EngineEvent::GameChanged { game: found.clone() });
                    if s.general.auto_replay_on_game {
                        if found.is_some() && !self.is_capturing() {
                            if self.start_replay().is_ok() {
                                self.0.auto_started.store(true, Ordering::SeqCst);
                            }
                        } else if found.is_none() && self.0.auto_started.load(Ordering::SeqCst) {
                            self.stop_replay();
                        }
                    }
                    if s.capture.target == CaptureTarget::FollowGame && self.is_capturing() && self.status().session.is_some_and(|x| x.backend == BackendKind::Wgc) {
                        self.stop_replay();
                        *self.0.buffer.lock() = None;
                        let _ = self.start_replay();
                    }
                }
            }
            let interval = if s.performance.resource_saver { 10 } else { 4 };
            for _ in 0..interval * 10 {
                if self.0.shutdown.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    pub fn current_game(&self) -> Option<DetectedGame> {
        self.0.game.read().clone()
    }

    // ----------------------------------------------------------------- hotkeys

    /// Start the global hotkey service and dispatch its actions to the engine.
    pub fn start_hotkeys(&self) -> HotkeyStatus {
        let bindings = self.settings().hotkeys.bindings();
        let (svc, status) = HotkeyService::start(bindings);
        let rx = svc.events();
        *self.0.hotkeys.lock() = Some(svc);
        let e = self.clone();
        let _ = std::thread::Builder::new().name("vinci-hotkey-dispatch".into()).spawn(move || {
            for a in rx {
                e.handle_action(a);
            }
        });
        log::info!("hotkeys: backend {} ({} registered)", status.backend, status.registrations.iter().filter(|r| r.ok).count());
        status
    }

    pub fn rebind_hotkeys(&self) -> Option<HotkeyStatus> {
        let b = self.settings().hotkeys.bindings();
        self.0.hotkeys.lock().as_ref().and_then(|s| s.rebind(b))
    }

    pub fn handle_action(&self, a: HotkeyAction) {
        self.emit(EngineEvent::HotkeyPressed { action: a });
        match a {
            HotkeyAction::SaveClip { secs } => self.save_clip(secs),
            HotkeyAction::ToggleReplay => {
                let _ = self.toggle_replay();
            }
        }
    }

    // ----------------------------------------------------------------- updates

    fn updater(&self) -> vinci_update::Updater {
        vinci_update::Updater::new(REPO, VERSION, self.settings().updates.channel)
    }

    pub fn check_update(&self) -> std::result::Result<Option<vinci_update::AvailableUpdate>, String> {
        let r = self.updater().check().map_err(|e| e.to_string());
        let mut s = self.settings();
        s.updates.last_check = Some(now_ms());
        let _ = vinci_storage::save_json(&self.0.paths.settings_file, &s);
        *self.0.settings.write() = s;
        r
    }

    /// Download, verify and apply an update. The caller restarts/exits per the outcome.
    pub fn install_update(&self, upd: &vinci_update::AvailableUpdate, progress: impl FnMut(u64, u64)) -> std::result::Result<vinci_update::kind::ApplyOutcome, String> {
        let u = self.updater();
        let file = u.download(upd, &self.0.paths.cache_dir.join("updates"), progress).map_err(|e| e.to_string())?;
        u.apply(&file).map_err(|e| e.to_string())
    }

    pub fn install_kind(&self) -> vinci_update::InstallKind {
        vinci_update::detect_install_kind()
    }

    // ---------------------------------------------------------------- estimate

    pub fn estimate(&self, s: &Settings) -> Estimate {
        let mons = vinci_capture::list_monitors();
        let (mw, mh) = match &s.capture.target {
            CaptureTarget::Monitor { index } => vinci_capture::monitors::pick(&mons, *index).map(|m| (m.width, m.height)),
            _ => mons.iter().find(|m| m.primary).map(|m| (m.width, m.height)),
        }
        .unwrap_or((1920, 1080));
        let (w, h) = s.capture.resolution.output_size(mw, mh);
        let profile = s.effective_profile();
        let codec = self
            .0
            .hw
            .read()
            .as_ref()
            .and_then(|hw| vinci_encoding::select(&hw.probes, s.encoding.codec.resolve(profile), s.encoding.encoder.as_deref(), s.encoding.allow_software).map(|(e, _)| e.codec))
            .unwrap_or_else(|| s.encoding.codec.resolve(profile));
        let fps = s.effective_fps();
        let mbm = vinci_encoding::profile::estimate_mb_per_minute(codec, profile, w, h, fps);
        let buffer_mb = mbm * s.replay.buffer_secs as f64 / 60.0;
        let peak_mb = buffer_mb * vinci_encoding::profile::max_kbps(codec, profile, w, h, fps) as f64 / vinci_encoding::profile::target_kbps(codec, profile, w, h, fps) as f64;
        let storage = match s.replay.storage {
            BufferStorage::Memory => "memory",
            BufferStorage::Disk => "disk",
            BufferStorage::Auto => {
                if peak_mb * 1.0486 > s.replay.max_ram_mb as f64 {
                    "disk"
                } else {
                    "memory"
                }
            }
        };
        Estimate {
            width: w,
            height: h,
            fps,
            codec,
            mb_per_minute: mbm,
            buffer_mb,
            buffer_storage: storage.into(),
            clip_mb: CLIP_PRESETS.iter().map(|secs| (*secs, mbm * *secs as f64 / 60.0)).collect(),
        }
    }

    /// Stop everything (capture, workers) — call before exiting.
    pub fn shutdown(&self) {
        self.0.shutdown.store(true, Ordering::SeqCst);
        self.stop_replay();
        *self.0.hotkeys.lock() = None;
        for c in self.0.jobs.lock().values() {
            c.store(true, Ordering::SeqCst);
        }
        *self.0.buffer.lock() = None; // removes the disk ring
    }
}

/// Quality profile labels for UIs.
pub fn profile_label(p: QualityProfile) -> &'static str {
    match p {
        QualityProfile::Quality => "Quality",
        QualityProfile::Balanced => "Balanced",
        QualityProfile::Compact => "Compact",
    }
}

#[cfg(test)]
mod tests;
