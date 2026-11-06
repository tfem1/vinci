//! `vinci-cli` — run the VINCI engine without the GUI.
//!
//! ```text
//! vinci-cli detect [--reprobe] [--json]        hardware, FFmpeg, working encoders
//! vinci-cli record [--backend x11] [--hotkeys] interactive replay buffer (stdin: save [secs] | status | quit)
//! vinci-cli clips [--search q] [--json]        list the library
//! vinci-cli compress <id> --size-mb 10         compress a clip in place
//! vinci-cli paths                              where settings, library and clips live
//! vinci-cli check-update                       query the update channel
//! ```

use clap::{Parser, Subcommand, ValueEnum};
use std::io::BufRead;
use std::time::{Duration, Instant};
use vinci_engine::{Engine, EngineEvent, JobKind};

#[derive(Parser)]
#[command(name = "vinci-cli", version, about = "VINCI — Capture Every Moment (headless engine)")]
struct Cli {
    /// Log level (error, warn, info, debug, trace).
    #[arg(long, global = true, default_value = "warn")]
    log: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    Auto,
    Wgc,
    Dda,
    X11,
    Portal,
    Test,
}

#[derive(Subcommand)]
enum Cmd {
    /// Detect hardware and validate encoders with real test encodes.
    Detect {
        #[arg(long)]
        reprobe: bool,
        #[arg(long)]
        json: bool,
    },
    /// Run the replay buffer. Type `save`, `save 60`, `status` or `quit` on stdin.
    Record {
        #[arg(long, value_enum)]
        backend: Option<Backend>,
        /// Buffer length in seconds.
        #[arg(long)]
        buffer: Option<u32>,
        /// Register the global hotkeys from settings (default Alt+F10).
        #[arg(long)]
        hotkeys: bool,
        /// Non-interactive: save a clip of `--save-secs` after this many seconds, then exit.
        #[arg(long)]
        save_after: Option<u64>,
        #[arg(long, default_value_t = 30)]
        save_secs: u32,
        /// Exit after this many seconds (non-interactive runs).
        #[arg(long)]
        duration: Option<u64>,
    },
    /// List clips in the library.
    Clips {
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Compress a clip in place (one physical file).
    Compress {
        id: i64,
        #[arg(long)]
        size_mb: Option<f64>,
        /// quality | balanced | compact
        #[arg(long, default_value = "compact")]
        profile: String,
        /// Keep the original and create a new file.
        #[arg(long)]
        keep: bool,
    },
    /// Show where VINCI stores its data.
    Paths,
    /// Check for a newer signed release.
    CheckUpdate,
}

fn main() {
    let cli = Cli::parse();
    let paths = match vinci_storage::AppPaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    vinci_engine::logging::init(&paths.logs_dir, &cli.log, true);
    let engine = match Engine::new(paths) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let code = run(&engine, cli.cmd);
    engine.shutdown();
    std::process::exit(code);
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1_000_000.0)
}

fn run(engine: &Engine, cmd: Cmd) -> i32 {
    match cmd {
        Cmd::Detect { reprobe, json } => {
            let hw = engine.hardware(reprobe);
            if json {
                println!("{}", serde_json::to_string_pretty(&*hw).unwrap());
                return 0;
            }
            let r = &hw.report;
            println!("VINCI {}  —  {}", vinci_engine::VERSION, r.system.os_name);
            println!("CPU      {} ({} threads), RAM {}", r.system.cpu_name, r.system.cpu_threads, mb(r.system.ram_total_bytes));
            println!("Display  {:?}{}", r.system.display_server, r.system.desktop.as_ref().map(|d| format!(" ({d})")).unwrap_or_default());
            for g in &r.gpus {
                println!(
                    "GPU {}    {} [{}]{}",
                    g.index,
                    g.name,
                    g.driver.clone().unwrap_or_default(),
                    g.render_node.as_ref().map(|n| format!(" {}", n.display())).unwrap_or_default()
                );
            }
            match &r.ffmpeg {
                Some(f) => println!(
                    "FFmpeg   {} ({}) — WGC: {}, DDA: {}, x11grab: {}",
                    f.version,
                    f.path.display(),
                    yn(f.supports_wgc()),
                    yn(f.supports_dda()),
                    yn(f.has_input_device("x11grab"))
                ),
                None => println!("FFmpeg   NOT FOUND — {}", r.ffmpeg_error.clone().unwrap_or_default()),
            }
            if cfg!(target_os = "linux") {
                let deps = vinci_capture::deps::PortalDeps::check(r.gstreamer.as_ref());
                match &r.gstreamer {
                    Some(g) => println!("GStreamer {} — pipewiresrc: {}", g.launch.display(), yn(g.has("pipewiresrc"))),
                    None => println!("GStreamer NOT FOUND (gst-launch-1.0 / gst-inspect-1.0 not in PATH)"),
                }
                if let Some(g) = &r.gstreamer {
                    let nv = hw.probes.iter().any(|p| p.ok && p.id.ends_with("_nvenc"));
                    let va = hw.probes.iter().any(|p| p.ok && p.id.ends_with("_vaapi"));
                    let enc = if nv && g.has("nvh264enc") {
                        if g.cuda_nvenc {
                            "NVENC, colour conversion on the GPU (CUDA)"
                        } else {
                            "NVENC, colour conversion on the CPU"
                        }
                    } else if va && g.has("vah264enc") {
                        "VA-API"
                    } else {
                        "x264 on the CPU (no GPU encoder usable from GStreamer)"
                    };
                    println!("Wayland encoder: {enc}");
                }
                if deps.video_ok() && deps.audio_ok() {
                    println!("Wayland capture: ready");
                } else {
                    println!("Wayland capture: missing {}", deps.all().join(", "));
                    if let Some(cmd) = vinci_capture::deps::install_command(&deps.all(), vinci_hw::system::distro_family()) {
                        println!("  fix: {cmd}");
                    }
                }
            }
            println!("\nEncoders (validated with a real test encode):");
            for p in &hw.probes {
                let label = vinci_encoding::catalog::find(&p.id).map(|e| e.label).unwrap_or("");
                println!("  {} {:<12} {:<34} {}", if p.ok { "✔" } else { "✘" }, p.id, label, p.error.clone().unwrap_or_default());
            }
            println!("\nMonitors:");
            for m in engine.monitors() {
                println!(
                    "  #{} {} {}×{} at {},{}{}",
                    m.index,
                    m.name,
                    m.width,
                    m.height,
                    m.x,
                    m.y,
                    if m.primary { " (primary)" } else { "" }
                );
            }
            0
        }
        Cmd::Record { backend, buffer, hotkeys, save_after, save_secs, duration } => {
            let mut s = engine.settings();
            if let Some(b) = backend {
                s.capture.backend = match b {
                    Backend::Auto => vinci_capture::BackendKind::Auto,
                    Backend::Wgc => vinci_capture::BackendKind::Wgc,
                    Backend::Dda => vinci_capture::BackendKind::Dda,
                    Backend::X11 => vinci_capture::BackendKind::X11,
                    Backend::Portal => vinci_capture::BackendKind::Portal,
                    Backend::Test => vinci_capture::BackendKind::Test,
                };
            }
            if let Some(b) = buffer {
                s.replay.buffer_secs = b;
            }
            if let Err(e) = engine.update_settings(s) {
                eprintln!("error: {e}");
                return 1;
            }
            let events = engine.subscribe();
            if hotkeys {
                let st = engine.start_hotkeys();
                for r in &st.registrations {
                    println!(
                        "hotkey {:<14} {:<10} {}",
                        r.id,
                        r.accelerator,
                        if r.ok { "registered".to_string() } else { r.error.clone().unwrap_or_default() }
                    );
                }
                if let Some(n) = st.note {
                    println!("note: {n}");
                }
            }
            if let Err(e) = engine.start_replay() {
                eprintln!("error: {e}");
                return 1;
            }
            let start = Instant::now();
            let interactive = save_after.is_none() && duration.is_none();
            let (line_tx, line_rx) = crossbeam_channel::unbounded::<String>();
            if interactive {
                println!("Replay buffer starting… commands: save [secs] | status | quit");
                std::thread::spawn(move || {
                    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                        if line_tx.send(line).is_err() {
                            break;
                        }
                    }
                });
            }
            let mut saved_once = false;
            let mut exit_code = 0;
            loop {
                for ev in events.try_iter() {
                    match ev {
                        EngineEvent::ClipSaved { clip, elapsed_ms, remuxed } => {
                            println!(
                                "SAVED {} ({:.1} s, {}, {} ms{})",
                                clip.path.display(),
                                clip.duration,
                                mb(clip.size),
                                elapsed_ms,
                                if remuxed { "" } else { ", raw fMP4" }
                            )
                        }
                        EngineEvent::ClipFailed { message } => {
                            println!("SAVE FAILED: {message}");
                            exit_code = 2;
                        }
                        EngineEvent::HotkeyPressed { action } => println!("hotkey: {action:?}"),
                        EngineEvent::GameChanged { game } => println!("game: {}", game.map(|g| g.name).unwrap_or_else(|| "none".into())),
                        _ => {}
                    }
                }
                if let Ok(line) = line_rx.try_recv() {
                    let mut it = line.split_whitespace();
                    match it.next() {
                        Some("save") => engine.save_clip(it.next().and_then(|s| s.parse().ok())),
                        Some("status") => println!("{}", serde_json::to_string_pretty(&engine.status()).unwrap()),
                        Some("stats") => println!("{}", serde_json::to_string_pretty(&engine.stats()).unwrap()),
                        Some("quit") | Some("exit") | Some("q") => break,
                        Some(other) => println!("unknown command `{other}`"),
                        None => {}
                    }
                }
                let st = engine.status();
                if st.state == vinci_engine::ReplayState::Error {
                    eprintln!("error: {}", st.message.unwrap_or_default());
                    if let Some(fix) = st.fix_command {
                        eprintln!("fix:   {fix}");
                    }
                    return 1;
                }
                if let Some(after) = save_after {
                    if !saved_once && start.elapsed() >= Duration::from_secs(after) {
                        saved_once = true;
                        // The ClipSaved event prints the result.
                        match engine.save_clip_blocking(Some(save_secs)) {
                            Ok(_) => {}
                            Err(e) => {
                                eprintln!("save failed: {e}");
                                return 2;
                            }
                        }
                        if duration.is_none() {
                            break;
                        }
                    }
                }
                if let Some(d) = duration {
                    if start.elapsed() >= Duration::from_secs(d) {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            // Drain late events (e.g. a hotkey save finishing).
            std::thread::sleep(Duration::from_millis(300));
            for ev in events.try_iter() {
                if let EngineEvent::ClipSaved { clip, .. } = ev {
                    println!("SAVED {} ({:.1} s, {})", clip.path.display(), clip.duration, mb(clip.size));
                }
            }
            exit_code
        }
        Cmd::Clips { search, json } => {
            let clips = match engine.list_clips(&vinci_clips::ClipQuery { search, ..Default::default() }) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&clips).unwrap());
            } else {
                for c in clips {
                    println!(
                        "{:>5}  {:<40} {:>6.1}s {:>9}  {}x{}  {}",
                        c.id,
                        c.title,
                        c.duration,
                        mb(c.size),
                        c.width,
                        c.height,
                        c.path.display()
                    );
                }
            }
            0
        }
        Cmd::Compress { id, size_mb, profile, keep } => {
            let target = match size_mb {
                Some(m) => vinci_encoding::CompressTarget::SizeMb(m),
                None => vinci_encoding::CompressTarget::Profile(match profile.as_str() {
                    "quality" => vinci_encoding::QualityProfile::Quality,
                    "balanced" => vinci_encoding::QualityProfile::Balanced,
                    _ => vinci_encoding::QualityProfile::Compact,
                }),
            };
            let events = engine.subscribe();
            let before = engine.clip(id).map(|c| c.size).unwrap_or(0);
            let job = engine.start_job(
                id,
                JobKind::Compress {
                    options: vinci_encoding::CompressOptions {
                        codec: vinci_encoding::Codec::Hevc,
                        target,
                        max_height: None,
                        max_fps: None,
                    },
                    replace: !keep,
                },
            );
            for ev in events {
                match ev {
                    EngineEvent::JobProgress { job_id, progress, .. } if job_id == job => {
                        eprint!("\rcompressing… {:>3.0}%", progress * 100.0)
                    }
                    EngineEvent::JobDone { job_id, clip, error, .. } if job_id == job => {
                        eprintln!();
                        return match (clip, error) {
                            (Some(c), _) => {
                                println!("OK {} → {} ({})", mb(before), mb(c.size), c.path.display());
                                0
                            }
                            (_, e) => {
                                eprintln!("error: {}", e.unwrap_or_default());
                                1
                            }
                        };
                    }
                    _ => {}
                }
            }
            1
        }
        Cmd::Paths => {
            println!("{}", serde_json::to_string_pretty(engine.paths()).unwrap());
            println!("clips: {}", engine.clips_dir().display());
            0
        }
        Cmd::CheckUpdate => match engine.check_update() {
            Ok(Some(u)) => {
                println!("Update available: {} → {} ({:?})", u.current, u.version, u.channel);
                0
            }
            Ok(None) => {
                println!("VINCI {} is up to date.", vinci_engine::VERSION);
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
    }
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}
