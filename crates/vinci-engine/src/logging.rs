//! Minimal file + stderr logger with size-based rotation (no extra dependency).

use log::{Level, LevelFilter, Log, Metadata, Record};
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 5 * 1024 * 1024;

struct FileLogger {
    level: LevelFilter,
    path: PathBuf,
    file: Mutex<Option<File>>,
    stderr: bool,
}

impl FileLogger {
    fn rotate_if_needed(&self, f: &mut Option<File>) {
        let big = f.as_ref().and_then(|x| x.metadata().ok()).is_some_and(|m| m.len() > MAX_BYTES);
        if big {
            *f = None;
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
            *f = OpenOptions::new().create(true).append(true).open(&self.path).ok();
        }
    }
}

impl Log for FileLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.level && (m.target().starts_with("vinci") || m.level() <= Level::Warn)
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:<5} [{}] {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), r.level(), r.target(), r.args());
        if self.stderr {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        let mut f = self.file.lock();
        self.rotate_if_needed(&mut f);
        if let Some(file) = f.as_mut() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().as_mut() {
            let _ = f.flush();
        }
    }
}

/// Install the logger. `level`: error|warn|info|debug|trace.
pub fn init(logs_dir: &Path, level: &str, stderr: bool) {
    let level = level.parse().unwrap_or(LevelFilter::Info);
    let _ = std::fs::create_dir_all(logs_dir);
    let path = logs_dir.join("vinci.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    let logger = FileLogger { level, path, file: Mutex::new(file), stderr };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
}
