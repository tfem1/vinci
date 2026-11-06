//! # vinci-games — game detection
//!
//! Identifies the game being played so clips are named, tagged and filed per game,
//! and so the replay buffer can start automatically when a game launches.
//!
//! Sources, in order of confidence:
//! 1. user-defined games (custom executable names),
//! 2. the embedded database of popular games (`games.json`),
//! 3. store-library heuristics: executables under `steamapps/common/<Game>`,
//!    `Epic Games/<Game>`, `Riot Games/<Game>`, `GOG Galaxy/Games/<Game>`…
//!
//! The foreground window's process wins over background processes. Proton/Wine games
//! are recognised through their Windows executable name in the command line.
//! No injection, no hooks: anti-cheat safe.

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameEntry {
    pub name: String,
    /// Lower-case executable file names.
    pub exes: Vec<String>,
    /// Optional substrings of the command line (e.g. Java games).
    #[serde(default)]
    pub cmd_contains: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DetectionSource {
    Custom,
    Database,
    StoreLibrary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectedGame {
    pub name: String,
    pub exe: String,
    pub pid: u32,
    pub foreground: bool,
    pub source: DetectionSource,
}

pub fn builtin_database() -> Vec<GameEntry> {
    serde_json::from_str(include_str!("games.json")).expect("embedded games.json is valid")
}

/// Executables that live inside game folders but are never the game itself.
const NOT_GAMES: &[&str] = &[
    "unitycrashhandler64.exe",
    "unitycrashhandler32.exe",
    "crashreportclient.exe",
    "crashpad_handler.exe",
    "easyanticheat.exe",
    "easyanticheat_eos_setup.exe",
    "beservice.exe",
    "battleye.exe",
    "vc_redist.x64.exe",
    "vc_redist.x86.exe",
    "dxsetup.exe",
    "dotnetfx.exe",
    "steamwebhelper.exe",
    "steam.exe",
    "steamservice.exe",
    "epicgameslauncher.exe",
    "epicwebhelper.exe",
    "riotclientservices.exe",
    "riotclientux.exe",
    "riotclientcrashhandler.exe",
    "battle.net.exe",
    "agent.exe",
    "galaxyclient.exe",
    "ubisoftconnect.exe",
    "upc.exe",
    "eadesktop.exe",
    "origin.exe",
    "uninstall.exe",
    "unins000.exe",
    "launcher.exe",
    "redprelauncher.exe",
    "start_protected_game.exe",
    "steamerrorreporter.exe",
    "steamerrorreporter64.exe",
    "reaper",
    "pressure-vessel-wrap",
    "wineserver",
    "wine64-preloader",
    "wine-preloader",
    "srt-bwrap",
    "proton",
    "pv-adverb",
    "python3",
    "bash",
    "sh",
];

const STORE_MARKERS: &[&str] = &[
    "steamapps/common/",
    "epic games/",
    "riot games/",
    "gog galaxy/games/",
    "xboxgames/",
    "ubisoft game launcher/games/",
    "ea games/",
    "heroic/",
];

fn file_name_lower(s: &str) -> String {
    s.rsplit(['/', '\\']).next().unwrap_or(s).trim_matches('"').to_lowercase()
}

/// Game folder name from a path inside a store library.
pub fn store_game_name(path: &str) -> Option<String> {
    let norm = path.replace('\\', "/");
    let lower = norm.to_lowercase();
    for m in STORE_MARKERS {
        if let Some(i) = lower.find(m) {
            let rest = &norm[i + m.len()..];
            let name = rest.split('/').next()?.trim();
            if !name.is_empty() && rest.contains('/') {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub struct GameDetector {
    sys: System,
    db: Vec<GameEntry>,
    custom: Vec<GameEntry>,
}

impl Default for GameDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// What we know about one process (decoupled from sysinfo for testing).
#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub exe_path: String,
    pub cmd: Vec<String>,
}

impl GameDetector {
    pub fn new() -> Self {
        Self { sys: System::new(), db: builtin_database(), custom: Vec::new() }
    }

    pub fn set_custom_games(&mut self, games: Vec<GameEntry>) {
        self.custom = games
            .into_iter()
            .map(|mut g| {
                g.exes = g.exes.iter().map(|e| e.to_lowercase()).collect();
                g
            })
            .collect();
    }

    /// Classify a single process.
    pub fn classify(&self, p: &ProcInfo) -> Option<(String, String, DetectionSource)> {
        // Candidate executable names: real exe, process name, and for Wine/Proton the
        // Windows executable found in the command line.
        let mut names = vec![file_name_lower(&p.exe_path), p.name.to_lowercase()];
        for arg in p.cmd.iter().take(3) {
            if arg.to_lowercase().ends_with(".exe") {
                names.push(file_name_lower(arg));
            }
        }
        names.retain(|n| !n.is_empty());
        let cmdline = p.cmd.join(" ").to_lowercase();
        for (list, src) in [(&self.custom, DetectionSource::Custom), (&self.db, DetectionSource::Database)] {
            for g in list.iter() {
                if let Some(n) = names.iter().find(|n| g.exes.iter().any(|e| e == *n)) {
                    return Some((g.name.clone(), n.clone(), src));
                }
                if !g.cmd_contains.is_empty() && g.cmd_contains.iter().any(|c| cmdline.contains(&c.to_lowercase())) {
                    return Some((g.name.clone(), names.first().cloned().unwrap_or_default(), src));
                }
            }
        }
        if names.iter().any(|n| NOT_GAMES.contains(&n.as_str())) {
            // Still allow the Windows exe inside a Wine command line.
            let wine_exe = p.cmd.iter().take(3).find(|a| a.to_lowercase().ends_with(".exe"));
            if let Some(exe) = wine_exe {
                let n = file_name_lower(exe);
                if !NOT_GAMES.contains(&n.as_str()) {
                    if let Some(game) = store_game_name(exe) {
                        return Some((game, n, DetectionSource::StoreLibrary));
                    }
                }
            }
            return None;
        }
        for path in std::iter::once(p.exe_path.as_str()).chain(p.cmd.iter().take(1).map(String::as_str)) {
            if let Some(game) = store_game_name(path) {
                return Some((game, names.first().cloned().unwrap_or_default(), DetectionSource::StoreLibrary));
            }
        }
        None
    }

    fn snapshot(&mut self) -> Vec<ProcInfo> {
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet).with_cmd(UpdateKind::OnlyIfNotSet),
        );
        self.sys
            .processes()
            .iter()
            .map(|(pid, p)| ProcInfo {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                exe_path: p.exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default(),
                cmd: p.cmd().iter().take(4).map(|c| c.to_string_lossy().into_owned()).collect(),
            })
            .collect()
    }

    /// Scan running processes (≈ a few ms). Prefers the foreground process.
    pub fn detect(&mut self) -> Option<DetectedGame> {
        let procs = self.snapshot();
        let fg = foreground_pid();
        self.pick(&procs, fg)
    }

    pub fn pick(&self, procs: &[ProcInfo], fg: Option<u32>) -> Option<DetectedGame> {
        let mut best: Option<DetectedGame> = None;
        for p in procs {
            if let Some((name, exe, source)) = self.classify(p) {
                let foreground = fg == Some(p.pid);
                let cand = DetectedGame { name, exe, pid: p.pid, foreground, source };
                let better = match &best {
                    None => true,
                    Some(b) => {
                        (cand.foreground && !b.foreground)
                            || (cand.foreground == b.foreground && source_rank(cand.source) < source_rank(b.source))
                    }
                };
                if better {
                    best = Some(cand);
                }
            }
        }
        best
    }
}

fn source_rank(s: DetectionSource) -> u8 {
    match s {
        DetectionSource::Custom => 0,
        DetectionSource::Database => 1,
        DetectionSource::StoreLibrary => 2,
    }
}

/// PID owning the focused window.
pub fn foreground_pid() -> Option<u32> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
    #[cfg(target_os = "linux")]
    {
        x11_active_pid()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn x11_active_pid() -> Option<u32> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    std::env::var_os("DISPLAY")?;
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atom = |n: &str| conn.intern_atom(false, n.as_bytes()).ok()?.reply().ok().map(|r| r.atom);
    let active = atom("_NET_ACTIVE_WINDOW")?;
    let wm_pid = atom("_NET_WM_PID")?;
    let w = conn.get_property(false, root, active, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?.value32()?.next()?;
    if w == 0 {
        return None;
    }
    let reply = conn.get_property(false, w, wm_pid, AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
    let pid = reply.value32()?.next();
    pid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, name: &str, exe: &str, cmd: &[&str]) -> ProcInfo {
        ProcInfo { pid, name: name.into(), exe_path: exe.into(), cmd: cmd.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn database_is_sane() {
        let db = builtin_database();
        assert!(db.len() > 70);
        for g in &db {
            assert!(!g.name.is_empty());
            assert!(g.exes.iter().all(|e| e == &e.to_lowercase()), "{} has non-lowercase exe", g.name);
        }
    }

    #[test]
    fn classify_sources() {
        let d = GameDetector::new();
        let valo = p(10, "VALORANT-Win64-", r"C:\Riot Games\VALORANT\live\ShooterGame\Binaries\Win64\VALORANT-Win64-Shipping.exe", &[]);
        assert_eq!(d.classify(&valo).unwrap().0, "Valorant");
        // Proton: process is wine-preloader, game exe is in argv.
        let proton = p(
            11,
            "wine64-preloader",
            "/usr/bin/wine64-preloader",
            &[r"Z:\home\me\.steam\steam\steamapps\common\Elden Ring\Game\eldenring.exe"],
        );
        assert_eq!(d.classify(&proton).unwrap().0, "Elden Ring");
        // Unknown game in a Steam library → folder name.
        let indie = p(12, "MyIndie", "/home/me/.local/share/Steam/steamapps/common/Cool Indie Game/bin/game", &[]);
        let (name, _, src) = d.classify(&indie).unwrap();
        assert_eq!((name.as_str(), src), ("Cool Indie Game", DetectionSource::StoreLibrary));
        let proton_unknown = p(14, "wine64-preloader", "/usr/bin/wine64-preloader", &[r"Z:\games\steamapps\common\Some Game\Some.exe"]);
        assert_eq!(d.classify(&proton_unknown).unwrap().0, "Some Game");
        // Helpers inside game folders are ignored.
        assert!(d
            .classify(&p(13, "UnityCrashHandler64.exe", r"D:\SteamLibrary\steamapps\common\X\UnityCrashHandler64.exe", &[]))
            .is_none());
        assert!(d.classify(&p(1, "firefox", "/usr/lib/firefox/firefox", &[])).is_none());
        // Java game via command line.
        assert_eq!(d.classify(&p(2, "java", "/usr/bin/java", &["java", "-cp", "net.minecraft.client.main.Main"])).unwrap().0, "Minecraft");
    }

    #[test]
    fn foreground_and_custom_win() {
        let mut d = GameDetector::new();
        let procs = vec![p(1, "cs2", "/games/cs2/game/bin/linuxsteamrt64/cs2", &[]), p(2, "dota2", "/x/dota2", &[])];
        assert_eq!(d.pick(&procs, Some(2)).unwrap().name, "Dota 2");
        assert_eq!(d.pick(&procs, None).unwrap().name, "Counter-Strike 2");
        d.set_custom_games(vec![GameEntry { name: "My Mod".into(), exes: vec!["CS2".into()], cmd_contains: vec![] }]);
        assert_eq!(d.pick(&procs, None).unwrap().name, "My Mod");
    }

    #[test]
    fn live_scan_does_not_crash() {
        let mut d = GameDetector::new();
        let _ = d.detect();
    }
}
