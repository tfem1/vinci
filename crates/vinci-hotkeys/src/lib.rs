//! # vinci-hotkeys — hotkey manager
//!
//! A self-contained service (own thread, own Win32 message loop on Windows) so global
//! hotkeys work identically in the GUI and the headless CLI.
//!
//! * Windows / X11: `global-hotkey` (RegisterHotKey / XGrabKey) — works while a game
//!   has focus, no keyboard hook, no injection.
//! * Wayland: the XDG **GlobalShortcuts** portal (KDE Plasma, GNOME 48+, Hyprland…).
//!   Where the portal is missing, users bind `vinci --save-clip` in their desktop's
//!   keyboard settings (documented in the README).

use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HotkeyAction {
    /// Save the last `secs` seconds (`None` = configured default length).
    SaveClip { secs: Option<u32> },
    ToggleReplay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub accelerator: String,
    pub action: HotkeyAction,
    pub description: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HotkeyError {
    #[error("empty shortcut")]
    Empty,
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("a shortcut needs a non-modifier key")]
    NoKey,
    #[error("`{0}` alone is too easy to press by accident — add Ctrl, Alt or Shift (F-keys are allowed alone)")]
    NeedsModifier(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accelerator {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    /// Normalised key name: `A`…`Z`, `0`…`9`, `F1`…`F24`, `Space`, `PrintScreen`…
    pub key: String,
}

const NAMED: &[(&str, &[&str])] = &[
    ("Space", &["space"]),
    ("Enter", &["enter", "return"]),
    ("Tab", &["tab"]),
    ("Backspace", &["backspace"]),
    ("Delete", &["delete", "del"]),
    ("Insert", &["insert", "ins"]),
    ("Home", &["home"]),
    ("End", &["end"]),
    ("PageUp", &["pageup", "pgup"]),
    ("PageDown", &["pagedown", "pgdn"]),
    ("Up", &["up", "arrowup"]),
    ("Down", &["down", "arrowdown"]),
    ("Left", &["left", "arrowleft"]),
    ("Right", &["right", "arrowright"]),
    ("PrintScreen", &["printscreen", "print", "prtsc"]),
    ("ScrollLock", &["scrolllock"]),
    ("Pause", &["pause"]),
    ("Escape", &["escape", "esc"]),
    ("Backquote", &["backquote", "`"]),
    ("Minus", &["minus", "-"]),
    ("Equal", &["equal", "="]),
    ("NumpadAdd", &["numpadadd"]),
    ("NumpadSubtract", &["numpadsubtract"]),
];

impl FromStr for Accelerator {
    type Err = HotkeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(HotkeyError::Empty);
        }
        let mut acc = Accelerator { ctrl: false, alt: false, shift: false, meta: false, key: String::new() };
        let parts: Vec<&str> = s.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
        for part in parts {
            let l = part.to_lowercase();
            match l.as_str() {
                "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => acc.ctrl = true,
                "alt" | "option" => acc.alt = true,
                "shift" => acc.shift = true,
                "super" | "meta" | "win" | "cmd" | "command" | "logo" => acc.meta = true,
                _ => {
                    if !acc.key.is_empty() {
                        return Err(HotkeyError::UnknownKey(part.into()));
                    }
                    acc.key = normalise_key(&l).ok_or_else(|| HotkeyError::UnknownKey(part.into()))?;
                }
            }
        }
        if acc.key.is_empty() {
            return Err(HotkeyError::NoKey);
        }
        let is_fkey = acc.key.starts_with('F') && acc.key.len() > 1 && acc.key[1..].parse::<u8>().is_ok();
        let special = matches!(acc.key.as_str(), "PrintScreen" | "Pause" | "ScrollLock");
        if !(acc.ctrl || acc.alt || acc.shift || acc.meta) && !is_fkey && !special {
            return Err(HotkeyError::NeedsModifier(acc.key.clone()));
        }
        Ok(acc)
    }
}

fn normalise_key(l: &str) -> Option<String> {
    if l.len() == 1 {
        let c = l.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
    }
    if let Some(n) = l.strip_prefix('f') {
        if let Ok(n) = n.parse::<u8>() {
            if (1..=24).contains(&n) {
                return Some(format!("F{n}"));
            }
        }
    }
    if let Some(k) = l.strip_prefix("key") {
        if k.len() == 1 {
            return normalise_key(k);
        }
    }
    if let Some(d) = l.strip_prefix("digit") {
        if d.len() == 1 {
            return normalise_key(d);
        }
    }
    NAMED.iter().find(|(_, al)| al.contains(&l)).map(|(n, _)| n.to_string())
}

impl fmt::Display for Accelerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.meta {
            parts.push("Super");
        }
        parts.push(&self.key);
        write!(f, "{}", parts.join("+"))
    }
}

impl Accelerator {
    /// String understood by `global-hotkey` (`HotKey::from_str`).
    pub fn to_global_hotkey(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("control+");
        }
        if self.alt {
            s.push_str("alt+");
        }
        if self.shift {
            s.push_str("shift+");
        }
        if self.meta {
            s.push_str("super+");
        }
        let k = match self.key.as_str() {
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => format!("Digit{k}"),
            k if k.len() == 1 => format!("Key{k}"),
            k => k.to_string(),
        };
        s.push_str(&k);
        s
    }

    /// XDG "shortcuts" trigger syntax for the GlobalShortcuts portal (`CTRL+ALT+F10`).
    pub fn to_xdg_trigger(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("CTRL".into());
        }
        if self.alt {
            parts.push("ALT".into());
        }
        if self.shift {
            parts.push("SHIFT".into());
        }
        if self.meta {
            parts.push("LOGO".into());
        }
        let key = match self.key.as_str() {
            "PrintScreen" => "Print".to_string(),
            "PageUp" => "Prior".to_string(),
            "PageDown" => "Next".to_string(),
            "Enter" => "Return".to_string(),
            k if k.len() == 1 => k.to_lowercase(),
            k => k.to_string(),
        };
        parts.push(key);
        parts.join("+")
    }
}

/// Validate and normalise a user-entered shortcut (`"ctrl + shift + s"` → `"Ctrl+Shift+S"`).
pub fn normalise(s: &str) -> Result<String, HotkeyError> {
    Ok(s.parse::<Accelerator>()?.to_string())
}

/// Ids of bindings that share the same accelerator.
pub fn conflicts(bindings: &[Binding]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, a) in bindings.iter().enumerate() {
        for b in &bindings[i + 1..] {
            if let (Ok(x), Ok(y)) = (a.accelerator.parse::<Accelerator>(), b.accelerator.parse::<Accelerator>()) {
                if x == y {
                    out.push((a.id.clone(), b.id.clone()));
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Registration {
    pub id: String,
    pub accelerator: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HotkeyStatus {
    /// `global-hotkey`, `xdg-portal` or `unavailable`.
    pub backend: String,
    pub registrations: Vec<Registration>,
    pub note: Option<String>,
}

enum Cmd {
    Rebind(Vec<Binding>, Sender<HotkeyStatus>),
    Stop,
}

pub struct HotkeyService {
    cmd: Sender<Cmd>,
    events: Receiver<HotkeyAction>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HotkeyService {
    /// Start the service and register `bindings`. Returns the initial status.
    pub fn start(bindings: Vec<Binding>) -> (Self, HotkeyStatus) {
        let (cmd_tx, cmd_rx) = unbounded::<Cmd>();
        let (ev_tx, ev_rx) = unbounded::<HotkeyAction>();
        let (st_tx, st_rx) = unbounded::<HotkeyStatus>();
        let use_portal = cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("VINCI_FORCE_X11_HOTKEYS").is_none();
        let thread = std::thread::Builder::new()
            .name("vinci-hotkeys".into())
            .spawn(move || {
                #[cfg(target_os = "linux")]
                if use_portal {
                    match portal::run(bindings.clone(), cmd_rx.clone(), ev_tx.clone(), st_tx.clone()) {
                        Ok(()) => return,
                        Err(e) => log::warn!("GlobalShortcuts portal unavailable ({e}); falling back to X11 grabs"),
                    }
                }
                let _ = use_portal;
                native::run(bindings, cmd_rx, ev_tx, st_tx);
            })
            .expect("spawn hotkey thread");
        let status = st_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap_or(HotkeyStatus {
            backend: "unavailable".into(),
            registrations: vec![],
            note: Some("hotkey service did not start".into()),
        });
        (Self { cmd: cmd_tx, events: ev_rx, thread: Some(thread) }, status)
    }

    pub fn events(&self) -> Receiver<HotkeyAction> {
        self.events.clone()
    }

    pub fn rebind(&self, bindings: Vec<Binding>) -> Option<HotkeyStatus> {
        let (tx, rx) = unbounded();
        self.cmd.send(Cmd::Rebind(bindings, tx)).ok()?;
        rx.recv_timeout(std::time::Duration::from_secs(30)).ok()
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

mod native {
    use super::*;
    use global_hotkey::hotkey::HotKey;
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use std::collections::HashMap;
    use std::time::Duration;

    fn register(mgr: &GlobalHotKeyManager, bindings: &[Binding], current: &mut Vec<HotKey>, map: &mut HashMap<u32, HotkeyAction>) -> Vec<Registration> {
        for hk in current.drain(..) {
            let _ = mgr.unregister(hk);
        }
        map.clear();
        let mut regs = Vec::new();
        for b in bindings {
            let res = b
                .accelerator
                .parse::<Accelerator>()
                .map_err(|e| e.to_string())
                .and_then(|a| HotKey::from_str(&a.to_global_hotkey()).map_err(|e| e.to_string()))
                .and_then(|hk| mgr.register(hk).map(|_| hk).map_err(|e| format!("already used by another application? ({e})")));
            match res {
                Ok(hk) => {
                    map.insert(hk.id(), b.action);
                    current.push(hk);
                    regs.push(Registration { id: b.id.clone(), accelerator: b.accelerator.clone(), ok: true, error: None });
                }
                Err(e) => regs.push(Registration { id: b.id.clone(), accelerator: b.accelerator.clone(), ok: false, error: Some(e) }),
            }
        }
        regs
    }

    pub fn run(bindings: Vec<Binding>, cmd: Receiver<Cmd>, ev: Sender<HotkeyAction>, st: Sender<HotkeyStatus>) {
        let mgr = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => {
                let _ = st.send(HotkeyStatus { backend: "unavailable".into(), registrations: vec![], note: Some(e.to_string()) });
                // Keep answering commands so callers never block.
                while let Ok(c) = cmd.recv() {
                    match c {
                        Cmd::Rebind(_, tx) => {
                            let _ = tx.send(HotkeyStatus { backend: "unavailable".into(), registrations: vec![], note: Some(e.to_string()) });
                        }
                        Cmd::Stop => return,
                    }
                }
                return;
            }
        };
        let mut current = Vec::new();
        let mut map = HashMap::new();
        let regs = register(&mgr, &bindings, &mut current, &mut map);
        let note = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            Some("Wayland session: shortcuts only fire while an X11/XWayland window is focused. Bind `vinci --save-clip` in your desktop keyboard settings for full coverage.".into())
        } else {
            None
        };
        let _ = st.send(HotkeyStatus { backend: "global-hotkey".into(), registrations: regs, note: note.clone() });
        let hk_rx = GlobalHotKeyEvent::receiver();
        loop {
            #[cfg(windows)]
            pump_messages();
            while let Ok(e) = hk_rx.try_recv() {
                if e.state == HotKeyState::Pressed {
                    if let Some(a) = map.get(&e.id) {
                        let _ = ev.send(*a);
                    }
                }
            }
            match cmd.recv_timeout(Duration::from_millis(15)) {
                Ok(Cmd::Rebind(b, tx)) => {
                    let regs = register(&mgr, &b, &mut current, &mut map);
                    let _ = tx.send(HotkeyStatus { backend: "global-hotkey".into(), registrations: regs, note: note.clone() });
                }
                Ok(Cmd::Stop) => break,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
        }
        for hk in current {
            let _ = mgr.unregister(hk);
        }
    }

    #[cfg(windows)]
    fn pump_messages() {
        use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod portal {
    use super::*;
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    use std::time::Duration;

    pub fn run(bindings: Vec<Binding>, cmd: Receiver<Cmd>, ev: Sender<HotkeyAction>, st: Sender<HotkeyStatus>) -> Result<(), String> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
        rt.block_on(async move {
            let proxy = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
            let session = proxy.create_session().await.map_err(|e| e.to_string())?;
            let bind = |b: &[Binding]| -> Vec<NewShortcut> {
                b.iter()
                    .map(|x| {
                        let trig = x.accelerator.parse::<Accelerator>().map(|a| a.to_xdg_trigger()).ok();
                        NewShortcut::new(x.id.clone(), x.description.clone()).preferred_trigger(trig.as_deref())
                    })
                    .collect()
            };
            let do_bind = |b: Vec<Binding>| {
                let shortcuts = bind(&b);
                let proxy = &proxy;
                let session = &session;
                async move {
                    let res = proxy.bind_shortcuts(session, &shortcuts, None).await.map_err(|e| e.to_string())?;
                    let resp = res.response().map_err(|e| e.to_string())?;
                    let bound: Vec<String> = resp.shortcuts().iter().map(|s| s.id().to_string()).collect();
                    let regs = b
                        .iter()
                        .map(|x| Registration {
                            id: x.id.clone(),
                            accelerator: resp.shortcuts().iter().find(|s| s.id() == x.id).map(|s| s.trigger_description().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| x.accelerator.clone()),
                            ok: bound.contains(&x.id),
                            error: (!bound.contains(&x.id)).then(|| "not confirmed by the desktop".to_string()),
                        })
                        .collect::<Vec<_>>();
                    Ok::<_, String>((regs, b))
                }
            };
            let (regs, mut current) = do_bind(bindings).await?;
            let _ = st.send(HotkeyStatus { backend: "xdg-portal".into(), registrations: regs, note: Some("Shortcuts are managed by your desktop; you can change them in its keyboard settings.".into()) });
            let mut activated = proxy.receive_activated().await.map_err(|e| e.to_string())?;
            loop {
                tokio::select! {
                    Some(a) = activated.next() => {
                        if let Some(b) = current.iter().find(|b| b.id == a.shortcut_id()) {
                            let _ = ev.send(b.action);
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        match cmd.try_recv() {
                            Ok(Cmd::Rebind(b, tx)) => {
                                let status = match do_bind(b).await {
                                    Ok((regs, nb)) => { current = nb; HotkeyStatus { backend: "xdg-portal".into(), registrations: regs, note: None } }
                                    Err(e) => HotkeyStatus { backend: "xdg-portal".into(), registrations: vec![], note: Some(e) },
                                };
                                let _ = tx.send(status);
                            }
                            Ok(Cmd::Stop) | Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                            Err(_) => {}
                        }
                    }
                }
            }
            let _ = session.close().await;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_and_normalisation() {
        assert_eq!(normalise("alt+f10").unwrap(), "Alt+F10");
        assert_eq!(normalise(" ctrl + shift + s ").unwrap(), "Ctrl+Shift+S");
        assert_eq!(normalise("F9").unwrap(), "F9");
        assert_eq!(normalise("PrintScreen").unwrap(), "PrintScreen");
        assert_eq!(normalise("Super+KeyQ").unwrap(), "Super+Q");
        assert_eq!(normalise("s"), Err(HotkeyError::NeedsModifier("S".into())));
        assert_eq!(normalise("ctrl+shift"), Err(HotkeyError::NoKey));
        assert!(matches!(normalise("ctrl+foo"), Err(HotkeyError::UnknownKey(_))));
        assert_eq!(normalise(""), Err(HotkeyError::Empty));
    }

    #[test]
    fn backend_strings() {
        let a: Accelerator = "Ctrl+Alt+1".parse().unwrap();
        assert_eq!(a.to_global_hotkey(), "control+alt+Digit1");
        assert!(global_hotkey::hotkey::HotKey::from_str(&a.to_global_hotkey()).is_ok());
        let b: Accelerator = "Alt+F10".parse().unwrap();
        assert!(global_hotkey::hotkey::HotKey::from_str(&b.to_global_hotkey()).is_ok());
        assert_eq!(b.to_xdg_trigger(), "ALT+F10");
        assert_eq!("Shift+Super+S".parse::<Accelerator>().unwrap().to_xdg_trigger(), "SHIFT+LOGO+s");
    }

    #[test]
    fn conflict_detection() {
        let b = |id: &str, acc: &str| Binding { id: id.into(), accelerator: acc.into(), action: HotkeyAction::ToggleReplay, description: String::new() };
        let c = conflicts(&[b("a", "Alt+F10"), b("b", "alt + f10"), b("c", "Alt+F9")]);
        assert_eq!(c, vec![("a".to_string(), "b".to_string())]);
    }
}
