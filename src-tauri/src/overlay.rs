//! In-game overlay: a tiny transparent, click-through, always-on-top window.
//!
//! To stay light it only exists while it has something to show: it is created for a
//! notification and destroyed a few seconds later — unless the persistent "REPLAY"
//! indicator is enabled. Works over borderless/windowed games (exclusive-fullscreen
//! games are covered by the OS; VINCI never injects into game processes).

use crate::AppState;
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use vinci_engine::Corner;

const LABEL: &str = "overlay";
const W: u32 = 440;
const H: u32 = 120;

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Msg {
    Saved { title: String, seconds: f64 },
    Error { title: String },
    Info { title: String },
    Indicator { active: bool },
}

#[derive(Clone, Serialize)]
struct Payload {
    #[serde(flatten)]
    msg: Msg,
    position: Corner,
    opacity: f32,
}

pub struct OverlayState {
    /// Transparent overlay windows work here (see `vinci_capture::monitors::overlay_supported`).
    supported: bool,
    ready: Mutex<bool>,
    pending: Mutex<Vec<Payload>>,
    generation: AtomicU64,
}

fn place(app: &AppHandle, win: &tauri::WebviewWindow, corner: Corner) {
    let mon = win.current_monitor().ok().flatten().or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = mon {
        let scale = m.scale_factor();
        let (w, h) = ((W as f64 * scale) as i32, (H as f64 * scale) as i32);
        let (mx, my) = (m.position().x, m.position().y);
        let (mw, mh) = (m.size().width as i32, m.size().height as i32);
        let (x, y) = match corner {
            Corner::TopLeft => (mx, my),
            Corner::TopRight => (mx + mw - w, my),
            Corner::BottomLeft => (mx, my + mh - h),
            Corner::BottomRight => (mx + mw - w, my + mh - h),
        };
        let _ = win.set_size(PhysicalSize::new(w as u32, h as u32));
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
}

fn ensure_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Some(w);
    }
    *app.state::<OverlayState>().ready.lock() = false;
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("VINCI overlay")
        .inner_size(W as f64, H as f64)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|e| log::warn!("overlay window: {e}"))
        .ok()?;
    Some(win)
}

/// Show the overlay and make it click-through. Must happen *after* `show()`: on
/// Linux/GTK the native window does not exist before it is realised (tao panics).
fn reveal(win: &tauri::WebviewWindow) {
    let _ = win.show();
    let _ = win.set_ignore_cursor_events(true);
}

fn send(app: &AppHandle, payload: Payload) {
    let st = app.state::<OverlayState>();
    if *st.ready.lock() {
        let _ = app.emit_to(LABEL, "vinci://overlay", payload);
    } else {
        st.pending.lock().push(payload);
    }
}

/// Called once at startup: flush queued messages when the overlay page is ready.
pub fn init(app: &AppHandle) {
    let supported = vinci_capture::monitors::overlay_supported();
    log::info!("in-game overlay: {}", if supported { "transparent window" } else { "desktop notifications (no compositor / Wayland)" });
    app.manage(OverlayState { supported, ready: Mutex::new(false), pending: Mutex::new(Vec::new()), generation: AtomicU64::new(0) });
    let handle = app.clone();
    app.listen("vinci://overlay-ready", move |_| {
        let st = handle.state::<OverlayState>();
        *st.ready.lock() = true;
        let pending: Vec<Payload> = st.pending.lock().drain(..).collect();
        for p in pending {
            let _ = handle.emit_to(LABEL, "vinci://overlay", p);
        }
    });
}

/// Show a notification in the overlay (no-op when disabled).
pub fn show(app: &AppHandle, msg: Msg) {
    let s = app.state::<AppState>().engine.settings();
    if !s.overlay.enabled || (!s.overlay.show_toasts && !matches!(msg, Msg::Indicator { .. })) {
        return;
    }
    if !app.state::<OverlayState>().supported {
        notify(app, &msg);
        return;
    }
    let Some(win) = ensure_window(app) else { return };
    place(app, &win, s.overlay.position);
    reveal(&win);
    send(app, Payload { msg, position: s.overlay.position, opacity: s.overlay.opacity });
    schedule_cleanup(app);
}

/// Fallback when no transparent overlay is possible: a native desktop notification.
fn notify(app: &AppHandle, msg: &Msg) {
    use tauri_plugin_notification::NotificationExt;
    let (title, body) = match msg {
        Msg::Saved { title, seconds } => (format!("Clip saved · {}s", seconds.round()), title.clone()),
        Msg::Error { title } => ("Clip not saved".to_string(), title.clone()),
        Msg::Info { .. } | Msg::Indicator { .. } => return,
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::debug!("desktop notification failed: {e}");
    }
}

/// Destroy the overlay window when idle (and no persistent indicator is wanted).
fn schedule_cleanup(app: &AppHandle) {
    let st = app.state::<OverlayState>();
    let gen = st.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let st = app.state::<OverlayState>();
        if st.generation.load(Ordering::SeqCst) != gen {
            return;
        }
        let s = app.state::<AppState>().engine.settings();
        let indicator = s.overlay.enabled && s.overlay.show_indicator && app.state::<AppState>().engine.is_capturing();
        if !indicator {
            if let Some(w) = app.get_webview_window(LABEL) {
                *st.ready.lock() = false;
                let _ = w.destroy();
            }
        }
    });
}

/// Keep the persistent indicator in sync with settings / replay state.
pub fn sync_indicator(app: &AppHandle) {
    if !app.state::<OverlayState>().supported {
        return;
    }
    let engine = app.state::<AppState>().engine.clone();
    let s = engine.settings();
    let want = s.overlay.enabled && s.overlay.show_indicator && engine.is_capturing();
    if want {
        if let Some(win) = ensure_window(app) {
            place(app, &win, s.overlay.position);
            reveal(&win);
            send(app, Payload { msg: Msg::Indicator { active: true }, position: s.overlay.position, opacity: s.overlay.opacity });
        }
    } else if let Some(w) = app.get_webview_window(LABEL) {
        send(app, Payload { msg: Msg::Indicator { active: false }, position: s.overlay.position, opacity: s.overlay.opacity });
        schedule_cleanup(app);
        let _ = w;
    }
}
