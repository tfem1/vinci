//! Helpers to spawn helper processes (FFmpeg, GStreamer) without flashing console
//! windows on Windows and with predictable behaviour on every platform.

use std::ffi::OsStr;
use std::process::Command;

/// `CREATE_NO_WINDOW` — prevents a console window from popping up for every
/// FFmpeg invocation when VINCI runs as a GUI application on Windows.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Build a [`Command`] that never opens a console window.
pub fn command<S: AsRef<OsStr>>(program: S) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Search `PATH` for an executable (adds `.exe` on Windows).
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) && !name.ends_with(".exe") { format!("{name}.exe") } else { name.to_string() };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(&exe);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub fn is_executable(p: &std::path::Path) -> bool {
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        _ => false,
    }
}

/// Run a command and capture stdout+stderr as UTF-8 (lossy). Returns `(success, stdout, stderr)`.
pub fn run_capture(cmd: &mut Command) -> std::io::Result<(bool, String, String)> {
    use std::process::Stdio;
    let out = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned()))
}

/// Run to completion with a deadline (the child is killed when it is exceeded).
/// `Ok(true)` when it exited successfully in time.
pub fn run_with_timeout(cmd: &mut Command, timeout: std::time::Duration) -> std::io::Result<bool> {
    use std::process::Stdio;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let end = std::time::Instant::now() + timeout;
    loop {
        if let Some(st) = child.try_wait()? {
            return Ok(st.success());
        }
        if std::time::Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
