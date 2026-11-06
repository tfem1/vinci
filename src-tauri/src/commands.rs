//! Tauri commands — thin wrappers over `vinci_engine::Engine`.
//! Anything that may block (FFmpeg, disk, network) runs on the blocking pool.

use crate::AppState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;
use vinci_clips::{Clip, ClipQuery, LibraryStats};
use vinci_engine::{EngineStatus, Estimate, JobKind, Settings};

type CmdResult<T> = Result<T, String>;

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(e)?
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> EngineStatus {
    state.engine.status()
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.engine.settings()
}

#[tauri::command]
pub async fn update_settings(app: AppHandle, settings: Settings) -> CmdResult<Settings> {
    let engine = app.state::<AppState>().engine.clone();
    let saved = blocking(move || engine.update_settings(settings).map_err(e)).await?;
    crate::allow_asset_dirs(&app);
    crate::overlay::sync_indicator(&app);
    Ok(saved)
}

#[tauri::command]
pub fn start_replay(state: State<AppState>) -> CmdResult<()> {
    state.engine.start_replay().map_err(e)
}

#[tauri::command]
pub async fn stop_replay(app: AppHandle) -> CmdResult<()> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || {
        engine.stop_replay();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn toggle_replay(app: AppHandle) -> CmdResult<bool> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.toggle_replay().map_err(e)).await
}

#[tauri::command]
pub fn clear_buffer(state: State<AppState>) {
    state.engine.clear_buffer();
}

#[tauri::command]
pub fn save_clip(state: State<AppState>, secs: Option<u32>) {
    state.engine.save_clip(secs);
}

#[tauri::command]
pub async fn get_hardware(app: AppHandle, force: bool) -> CmdResult<serde_json::Value> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || serde_json::to_value(&*engine.hardware(force)).map_err(e)).await
}

#[tauri::command]
pub async fn get_monitors(app: AppHandle) -> CmdResult<Vec<vinci_capture::MonitorInfo>> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || Ok(engine.monitors())).await
}

#[tauri::command]
pub async fn get_audio_devices(app: AppHandle) -> CmdResult<Vec<vinci_capture::devices::AudioDevice>> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || Ok(engine.audio_devices())).await
}

#[tauri::command]
pub async fn get_estimate(app: AppHandle, settings: Settings) -> CmdResult<Estimate> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || Ok(engine.estimate(&settings))).await
}

#[tauri::command]
pub fn get_stats(state: State<AppState>) -> vinci_engine::ResourceStats {
    state.engine.stats()
}

#[tauri::command]
pub async fn list_clips(app: AppHandle, query: ClipQuery) -> CmdResult<Vec<Clip>> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.list_clips(&query).map_err(e)).await
}

#[tauri::command]
pub async fn library_stats(app: AppHandle) -> CmdResult<LibraryStats> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.library_stats().map_err(e)).await
}

#[tauri::command]
pub async fn rename_clip(app: AppHandle, id: i64, title: String) -> CmdResult<Clip> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.rename_clip(id, &title).map_err(e)).await
}

#[tauri::command]
pub async fn set_favorite(app: AppHandle, id: i64, favorite: bool) -> CmdResult<Clip> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.set_favorite(id, favorite).map_err(e)).await
}

#[tauri::command]
pub async fn delete_clip(app: AppHandle, id: i64) -> CmdResult<()> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.delete_clip(id).map_err(e)).await
}

#[tauri::command]
pub async fn move_clip(app: AppHandle, id: i64, dest: String) -> CmdResult<Clip> {
    let engine = app.state::<AppState>().engine.clone();
    let clip = blocking(move || engine.move_clip(id, &PathBuf::from(dest)).map_err(e)).await?;
    if let Some(dir) = clip.path.parent() {
        let _ = app.asset_protocol_scope().allow_directory(dir, false);
    }
    Ok(clip)
}

#[tauri::command]
pub async fn clip_thumbnail(app: AppHandle, id: i64) -> CmdResult<Option<String>> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.thumbnail(id).map(|p| p.map(|p| p.display().to_string())).map_err(e)).await
}

#[tauri::command]
pub async fn scan_library(app: AppHandle) -> CmdResult<serde_json::Value> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.scan_library().map(|r| serde_json::to_value(r).unwrap_or_default()).map_err(e)).await
}

#[tauri::command]
pub fn reveal_clip(app: AppHandle, state: State<AppState>, id: i64) -> CmdResult<()> {
    let clip = state.engine.clip(id).map_err(e)?;
    app.opener().reveal_item_in_dir(&clip.path).map_err(e)
}

#[tauri::command]
pub fn open_clip(app: AppHandle, state: State<AppState>, id: i64) -> CmdResult<()> {
    let clip = state.engine.clip(id).map_err(e)?;
    app.opener().open_path(clip.path.display().to_string(), None::<&str>).map_err(e)
}

#[tauri::command]
pub fn open_clips_folder(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    let dir = state.engine.clips_dir();
    std::fs::create_dir_all(&dir).map_err(e)?;
    app.opener().open_path(dir.display().to_string(), None::<&str>).map_err(e)
}

#[tauri::command]
pub fn open_logs_folder(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    app.opener().open_path(state.engine.paths().logs_dir.display().to_string(), None::<&str>).map_err(e)
}

#[tauri::command]
pub fn start_compress(state: State<AppState>, id: i64, options: vinci_encoding::CompressOptions, replace: bool) -> u64 {
    state.engine.start_job(id, JobKind::Compress { options, replace })
}

#[tauri::command]
pub fn start_trim(state: State<AppState>, id: i64, start: f64, end: f64, replace: bool) -> u64 {
    state.engine.start_job(id, JobKind::Trim { start, end, replace })
}

#[tauri::command]
pub fn cancel_job(state: State<AppState>, job_id: u64) {
    state.engine.cancel_job(job_id);
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> CmdResult<Option<vinci_update::AvailableUpdate>> {
    let engine = app.state::<AppState>().engine.clone();
    blocking(move || engine.check_update()).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle, update: vinci_update::AvailableUpdate) -> CmdResult<String> {
    let engine = app.state::<AppState>().engine.clone();
    let outcome = blocking(move || engine.install_update(&update, |_, _| {})).await?;
    use vinci_update::kind::ApplyOutcome;
    match outcome {
        ApplyOutcome::ExitNow => {
            app.state::<AppState>().engine.shutdown();
            app.exit(0);
            Ok("Installing…".into())
        }
        ApplyOutcome::Restart { .. } => {
            app.state::<AppState>().engine.shutdown();
            app.restart();
        }
        ApplyOutcome::Manual { command } => Ok(format!("Run this command to finish the update: {command}")),
    }
}

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    repo: &'static str,
    clips_dir: String,
    logs_dir: String,
    config_dir: String,
    install_kind: String,
    platform: &'static str,
}

#[tauri::command]
pub fn get_app_info(state: State<AppState>) -> AppInfo {
    let p = state.engine.paths();
    AppInfo {
        version: vinci_engine::VERSION,
        repo: vinci_engine::REPO,
        clips_dir: state.engine.clips_dir().display().to_string(),
        logs_dir: p.logs_dir.display().to_string(),
        config_dir: p.config_dir.display().to_string(),
        install_kind: state.engine.install_kind().asset_suffix().to_string(),
        platform: std::env::consts::OS,
    }
}

#[tauri::command]
pub fn hotkey_status(state: State<AppState>) -> Option<vinci_hotkeys::HotkeyStatus> {
    state.hotkeys.lock().clone()
}

#[tauri::command]
pub fn validate_hotkey(accelerator: String) -> CmdResult<String> {
    vinci_hotkeys::normalise(&accelerator).map_err(e)
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<()> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if enabled {
        al.enable().map_err(e)
    } else {
        al.disable().map_err(e)
    }
}

#[tauri::command]
pub fn preview_overlay(app: AppHandle) {
    crate::overlay::show(&app, crate::overlay::Msg::Saved { title: "Preview — this is how clips are announced".into(), seconds: 30.0 });
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle, title: String) -> CmdResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().set_title(title).pick_folder(move |p| {
        let _ = tx.send(p.map(|p| p.to_string()));
    });
    blocking(move || rx.recv().map_err(e)).await
}
