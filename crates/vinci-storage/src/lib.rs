//! # vinci-storage — storage manager
//!
//! * [`AppPaths`] — per-OS locations (config, data, cache, logs, clips). User data
//!   lives outside the install directory, so installers and updates never touch it.
//! * [`load_json`] / [`save_json`] — atomic, crash-safe persistence (temp + fsync +
//!   rename, corrupted files are backed up instead of silently overwritten).
//! * Disk space, unique file names, cross-volume moves and startup cleanup of
//!   leftovers from an interrupted session.

use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub thumbnails_dir: PathBuf,
    pub ring_dir: PathBuf,
    pub settings_file: PathBuf,
    pub database_file: PathBuf,
    pub hardware_cache_file: PathBuf,
    pub default_clips_dir: PathBuf,
}

impl AppPaths {
    /// Resolve standard locations. `VINCI_HOME` overrides everything (portable mode, tests).
    pub fn resolve() -> io::Result<Self> {
        if let Some(home) = std::env::var_os("VINCI_HOME") {
            return Ok(Self::rooted(Path::new(&home)));
        }
        let pd = directories::ProjectDirs::from("", "", "VINCI").ok_or_else(|| io::Error::other("cannot determine the home directory"))?;
        let config_dir = pd.config_dir().to_path_buf();
        let data_dir = pd.data_local_dir().to_path_buf();
        let cache_dir = pd.cache_dir().to_path_buf();
        let videos = dirs::video_dir().or_else(|| dirs::home_dir().map(|h| h.join("Videos"))).unwrap_or_else(|| PathBuf::from("."));
        Ok(Self::from_parts(config_dir, data_dir, cache_dir, videos.join("VINCI")))
    }

    pub fn rooted(root: &Path) -> Self {
        Self::from_parts(root.join("config"), root.join("data"), root.join("cache"), root.join("Clips"))
    }

    fn from_parts(config_dir: PathBuf, data_dir: PathBuf, cache_dir: PathBuf, clips: PathBuf) -> Self {
        Self {
            settings_file: config_dir.join("settings.json"),
            database_file: data_dir.join("library.db"),
            hardware_cache_file: cache_dir.join("hardware.json"),
            logs_dir: data_dir.join("logs"),
            thumbnails_dir: cache_dir.join("thumbnails"),
            ring_dir: cache_dir.join("replay"),
            default_clips_dir: clips,
            config_dir,
            data_dir,
            cache_dir,
        }
    }

    pub fn ensure(&self) -> io::Result<()> {
        for d in [&self.config_dir, &self.data_dir, &self.cache_dir, &self.logs_dir, &self.thumbnails_dir, &self.ring_dir] {
            fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOutcome {
    Loaded,
    Missing,
    /// The file was unreadable; it was renamed to `*.corrupt-<ts>` and defaults used.
    RecoveredFromCorruption,
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> (T, LoadOutcome) {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<T>(&bytes) {
            Ok(v) => (v, LoadOutcome::Loaded),
            Err(e) => {
                let ts = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let backup = path.with_extension(format!("corrupt-{ts}.json"));
                log::error!("{} is corrupted ({e}); backed up to {}", path.display(), backup.display());
                let _ = fs::rename(path, &backup);
                (T::default(), LoadOutcome::RecoveredFromCorruption)
            }
        },
        Err(_) => (T::default(), LoadOutcome::Missing),
    }
}

/// Atomically replace `path` with the JSON serialisation of `value`.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    atomic_write(path, &bytes)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Free space available to the current user on the volume holding `path`.
pub fn free_space(path: &Path) -> Option<u64> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let w: Vec<u16> = p.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let mut avail = 0u64;
        unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut avail), None, None).ok()? };
        Some(avail)
    }
}

/// Total size of regular files below `dir` (non-recursive into symlinks).
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// Make a string safe as a file/folder name on Windows and Linux.
pub fn sanitize_filename(name: &str) -> String {
    const RESERVED: [&str; 22] = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    let mut s: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() {
        s = "Untitled".into();
    }
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(s.split('.').next().unwrap_or(""))) {
        s.insert(0, '_');
    }
    s.chars().take(120).collect()
}

/// `dir/name.ext` or `dir/name (2).ext`, `dir/name (3).ext`… whichever does not exist.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..10_000).map(|i| dir.join(format!("{stem} ({i}).{ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Move a file, falling back to copy + fsync + delete across volumes.
pub fn move_file(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(d) = dst.parent() {
        fs::create_dir_all(d)?;
    }
    if dst.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", dst.display())));
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            let tmp = dst.with_file_name(format!(".{}.vinci-move", dst.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
            fs::copy(src, &tmp)?;
            File::open(&tmp)?.sync_all()?;
            fs::rename(&tmp, dst)?;
            fs::remove_file(src)
        }
    }
}

/// Remove leftovers of an interrupted session: ring buffers, half-written clips/parts
/// older than `min_age`. Returns the number of files removed.
pub fn cleanup_leftovers(paths: &AppPaths, clips_dir: &Path, min_age: Duration) -> usize {
    let mut n = 0;
    let now = SystemTime::now();
    let old_enough = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).map(|t| now.duration_since(t).unwrap_or_default() >= min_age).unwrap_or(false);
    if let Ok(rd) = fs::read_dir(&paths.ring_dir) {
        for e in rd.flatten() {
            if old_enough(&e.path()) && fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
    }
    let mut stack = vec![(clips_dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() && depth < 2 {
                stack.push((p, depth + 1));
            } else if name.starts_with('.') && (name.ends_with(".vinci-tmp") || name.contains(".part") || name.ends_with(".vinci-move") || name.ends_with(".concat.txt")) && old_enough(&p) && fs::remove_file(&p).is_ok() {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, Default, PartialEq, Debug)]
    struct S {
        a: u32,
    }

    #[test]
    fn json_roundtrip_and_corruption_recovery() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cfg/settings.json");
        assert_eq!(load_json::<S>(&p).1, LoadOutcome::Missing);
        save_json(&p, &S { a: 7 }).unwrap();
        assert_eq!(load_json::<S>(&p), (S { a: 7 }, LoadOutcome::Loaded));
        fs::write(&p, b"{not json").unwrap();
        let (v, o) = load_json::<S>(&p);
        assert_eq!((v, o), (S::default(), LoadOutcome::RecoveredFromCorruption));
        assert!(fs::read_dir(p.parent().unwrap()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("corrupt")));
    }

    #[test]
    fn names() {
        assert_eq!(sanitize_filename("Counter-Strike: 2 <ace>?"), "Counter-Strike_ 2 _ace__");
        assert_eq!(sanitize_filename("  ..  "), "Untitled");
        assert_eq!(sanitize_filename("con"), "_con");
        let d = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(d.path(), "clip", "mp4"), d.path().join("clip.mp4"));
        fs::write(d.path().join("clip.mp4"), b"x").unwrap();
        assert_eq!(unique_path(d.path(), "clip", "mp4"), d.path().join("clip (2).mp4"));
    }

    #[test]
    fn move_and_space_and_cleanup() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.mp4");
        fs::write(&a, b"data").unwrap();
        let b = d.path().join("sub/b.mp4");
        move_file(&a, &b).unwrap();
        assert!(!a.exists() && b.exists());
        assert!(move_file(&b, &b).is_err());
        assert!(free_space(d.path()).unwrap() > 0);
        assert_eq!(dir_size(d.path()), 4);
        let paths = AppPaths::rooted(&d.path().join("home"));
        paths.ensure().unwrap();
        fs::write(paths.ring_dir.join("replay-1.ring"), b"x").unwrap();
        fs::write(d.path().join("sub/.x.mp4.part0.mp4"), b"x").unwrap();
        fs::write(d.path().join("sub/keep.mp4"), b"x").unwrap();
        assert_eq!(cleanup_leftovers(&paths, d.path(), Duration::ZERO), 2);
        assert!(d.path().join("sub/keep.mp4").exists());
    }
}
