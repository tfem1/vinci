//! VINCI desktop shell (Tauri 2). The engine (`vinci-engine`) does all the work; this
//! crate only wires it to windows, the tray, the overlay and OS integration.

mod commands;
mod media_server;
mod overlay;

use parking_lot::Mutex;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use vinci_engine::{Engine, EngineEvent, ReplayState};

pub struct AppState {
    pub engine: Engine,
    pub hotkeys: Arc<Mutex<Option<vinci_hotkeys::HotkeyStatus>>>,
    /// Loopback HTTP server for the in-app player (see `media_server`).
    pub media: Option<media_server::MediaServer>,
}

/// Command-line actions, usable from desktop shortcuts or compositor key bindings
/// (Wayland without the GlobalShortcuts portal): `vinci --save-clip 30`.
#[derive(Debug, PartialEq)]
enum CliAction {
    SaveClip(Option<u32>),
    ToggleReplay,
    Show,
    None,
}

fn parse_args(args: &[String]) -> (CliAction, bool) {
    let minimized = args.iter().any(|a| a == "--minimized" || a == "--hidden");
    let mut action = CliAction::None;
    let mut it = args.iter().skip(1).peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--save-clip" | "--save" => {
                let secs = it.peek().and_then(|s| s.parse::<u32>().ok());
                if secs.is_some() {
                    it.next();
                }
                action = CliAction::SaveClip(secs);
            }
            "--toggle-replay" => action = CliAction::ToggleReplay,
            "--show" => action = CliAction::Show,
            _ => {}
        }
    }
    (action, minimized)
}

fn handle_cli(app: &AppHandle, action: CliAction) {
    let engine = app.state::<AppState>().engine.clone();
    match action {
        CliAction::SaveClip(secs) => engine.save_clip(secs),
        CliAction::ToggleReplay => {
            let _ = engine.toggle_replay();
        }
        CliAction::Show => show_main(app),
        CliAction::None => show_main(app),
    }
}

/// Show (or re-create, if it was destroyed to free memory) the main window.
pub fn show_main(app: &AppHandle) {
    let win = match app.get_webview_window("main") {
        Some(w) => w,
        None => {
            let cfg = app.config().app.windows.iter().find(|w| w.label == "main").cloned();
            let built = match cfg {
                Some(c) => WebviewWindowBuilder::from_config(app, &c).and_then(|b| b.build()),
                None => WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("VINCI")
                    .inner_size(1280.0, 820.0)
                    .decorations(false)
                    .build(),
            };
            match built {
                Ok(w) => {
                    attach_close_handler(app, &w);
                    w
                }
                Err(e) => {
                    log::error!("cannot create main window: {e}");
                    return;
                }
            }
        }
    };
    let _ = win.show();
    let _ = win.unminimize();
    let _ = win.set_focus();
}

fn attach_close_handler(app: &AppHandle, win: &tauri::WebviewWindow) {
    let app = app.clone();
    let w = win.clone();
    win.on_window_event(move |ev| {
        if let WindowEvent::CloseRequested { api, .. } = ev {
            let s = app.state::<AppState>().engine.settings();
            if s.general.close_to_tray {
                api.prevent_close();
                if s.performance.free_ui_when_hidden {
                    // Destroying the webview releases its renderer process memory;
                    // the engine keeps running in the background.
                    let _ = w.destroy();
                } else {
                    let _ = w.hide();
                }
            } else {
                app.state::<AppState>().engine.shutdown();
                app.exit(0);
            }
        }
    });
}

pub fn allow_asset_dirs(app: &AppHandle) {
    let engine = app.state::<AppState>().engine.clone();
    let scope = app.asset_protocol_scope();
    let _ = scope.allow_directory(engine.clips_dir(), true);
    let _ = scope.allow_directory(&engine.paths().thumbnails_dir, false);
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open VINCI", true, None::<&str>)?;
    let save = MenuItem::with_id(app, "save", "Save clip", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Start / stop replay", true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Open clips folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit VINCI", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &sep, &save, &toggle, &folder, &sep2, &quit])?;
    let icon = app.default_window_icon().cloned();
    let mut b = TrayIconBuilder::with_id("vinci-tray").tooltip("VINCI — Capture Every Moment").menu(&menu).show_menu_on_left_click(false);
    if let Some(i) = icon {
        b = b.icon(i);
    }
    b.on_menu_event(|app, ev| {
        let engine = app.state::<AppState>().engine.clone();
        match ev.id.as_ref() {
            "open" => show_main(app),
            "save" => engine.save_clip(None),
            "toggle" => {
                std::thread::spawn(move || {
                    let _ = engine.toggle_replay();
                });
            }
            "folder" => {
                use tauri_plugin_opener::OpenerExt;
                let _ = app.opener().open_path(engine.clips_dir().display().to_string(), None::<&str>);
            }
            "quit" => {
                engine.shutdown();
                app.exit(0);
            }
            _ => {}
        }
    })
    .on_tray_icon_event(|tray, ev| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
}

/// Forward engine events to the UI and drive the overlay / tray tooltip.
fn forward_events(app: &AppHandle) {
    let engine = app.state::<AppState>().engine.clone();
    let rx = engine.subscribe();
    let app = app.clone();
    std::thread::Builder::new()
        .name("vinci-ui-events".into())
        .spawn(move || {
            let mut last_state = ReplayState::Stopped;
            for ev in rx {
                match &ev {
                    EngineEvent::ClipSaved { clip, .. } => {
                        overlay::show(&app, overlay::Msg::Saved { title: clip.title.clone(), seconds: clip.duration })
                    }
                    EngineEvent::ClipFailed { message } => overlay::show(&app, overlay::Msg::Error { title: message.clone() }),
                    EngineEvent::Status(st) if st.state != last_state => {
                        last_state = st.state;
                        if let Some(t) = app.tray_by_id("vinci-tray") {
                            let label = match st.state {
                                ReplayState::Running => "VINCI — replay on",
                                ReplayState::Error => "VINCI — capture error",
                                _ => "VINCI — replay off",
                            };
                            let _ = t.set_tooltip(Some(label));
                        }
                        overlay::sync_indicator(&app);
                        match st.state {
                            ReplayState::Running => overlay::show(&app, overlay::Msg::Info { title: "Instant replay is on".into() }),
                            ReplayState::Stopped => overlay::show(&app, overlay::Msg::Info { title: "Instant replay is off".into() }),
                            _ => {}
                        }
                    }
                    _ => {}
                }
                // Only emit to the UI if a window exists (no work while hidden).
                if app.get_webview_window("main").is_some() {
                    let _ = app.emit("vinci://event", &ev);
                }
            }
        })
        .expect("spawn event thread");
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let (startup_action, start_minimized_flag) = parse_args(&args);

    let paths = match vinci_storage::AppPaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("VINCI cannot determine its data directories: {e}");
            std::process::exit(1);
        }
    };
    let _ = paths.ensure();
    let (settings_peek, _) = vinci_storage::load_json::<vinci_engine::Settings>(&paths.settings_file);
    vinci_engine::logging::init(&paths.logs_dir, &settings_peek.advanced.log_level, cfg!(debug_assertions));
    log::info!("VINCI {} starting ({} {})", vinci_engine::VERSION, std::env::consts::OS, std::env::consts::ARCH);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let (action, _) = parse_args(&argv);
            handle_cli(app, action);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .setup(move |app| {
            let engine = Engine::new(paths.clone()).map_err(|e| Box::new(std::io::Error::other(e.to_string())))?;
            let hotkeys = Arc::new(Mutex::new(None));
            let resolver_engine = engine.clone();
            let media = media_server::start(move |id| resolver_engine.clip(id).ok().map(|c| c.path))
                .map_err(|e| log::error!("media server unavailable: {e}"))
                .ok();
            app.manage(AppState { engine: engine.clone(), hotkeys: hotkeys.clone(), media });
            overlay::init(app.handle());
            allow_asset_dirs(app.handle());
            build_tray(app.handle())?;
            forward_events(app.handle());

            // Hotkeys run on their own thread (own message loop on Windows).
            let st = engine.start_hotkeys();
            *hotkeys.lock() = Some(st);

            let s = engine.settings();
            if s.general.start_replay_on_launch && !s.general.auto_replay_on_game {
                if let Err(e) = engine.start_replay() {
                    log::error!("replay buffer did not start: {e}");
                }
            }
            if let Some(main) = app.get_webview_window("main") {
                attach_close_handler(app.handle(), &main);
                if !(start_minimized_flag || s.general.start_minimized) {
                    let _ = main.show();
                    let _ = main.set_focus();
                } else if s.performance.free_ui_when_hidden {
                    let _ = main.destroy();
                }
            }
            if let CliAction::SaveClip(_) | CliAction::ToggleReplay = startup_action {
                handle_cli(app.handle(), startup_action);
            }

            // Daily signed update check, in the background.
            if s.updates.auto_check {
                let due = s.updates.last_check.is_none_or(|t| chrono_now_ms() - t > 24 * 3600 * 1000);
                if due {
                    let handle = app.handle().clone();
                    let engine = engine.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(20));
                        match engine.check_update() {
                            Ok(Some(u)) if engine.settings().updates.skipped_version.as_deref() != Some(&u.version) => {
                                let _ = handle.emit("vinci://update-available", &u);
                            }
                            Ok(_) => {}
                            Err(e) => log::info!("update check: {e}"),
                        }
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_settings,
            commands::update_settings,
            commands::start_replay,
            commands::stop_replay,
            commands::toggle_replay,
            commands::clear_buffer,
            commands::save_clip,
            commands::get_hardware,
            commands::get_monitors,
            commands::get_audio_devices,
            commands::get_estimate,
            commands::get_stats,
            commands::list_clips,
            commands::library_stats,
            commands::rename_clip,
            commands::set_favorite,
            commands::delete_clip,
            commands::move_clip,
            commands::clip_thumbnail,
            commands::scan_library,
            commands::reveal_clip,
            commands::open_clip,
            commands::open_clips_folder,
            commands::open_logs_folder,
            commands::start_compress,
            commands::start_trim,
            commands::cancel_job,
            commands::check_update,
            commands::install_update,
            commands::get_app_info,
            commands::hotkey_status,
            commands::validate_hotkey,
            commands::set_autostart,
            commands::preview_overlay,
            commands::pick_folder,
            commands::media_url,
        ])
        .build(tauri::generate_context!())
        .expect("error while building VINCI");

    app.run(|app, ev| match ev {
        // Last window closed/destroyed: keep running in the tray.
        RunEvent::ExitRequested { code: None, api, .. } if app.state::<AppState>().engine.settings().general.close_to_tray => {
            api.prevent_exit()
        }
        RunEvent::Exit => app.state::<AppState>().engine.shutdown(),
        _ => {}
    });
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(v: &[&str]) -> Vec<String> {
        std::iter::once("vinci").chain(v.iter().copied()).map(String::from).collect()
    }
    #[test]
    fn cli_args() {
        assert_eq!(parse_args(&a(&["--save-clip", "60"])), (CliAction::SaveClip(Some(60)), false));
        assert_eq!(parse_args(&a(&["--save-clip"])), (CliAction::SaveClip(None), false));
        assert_eq!(parse_args(&a(&["--minimized"])), (CliAction::None, true));
        assert_eq!(parse_args(&a(&["--toggle-replay"])).0, CliAction::ToggleReplay);
    }
}
