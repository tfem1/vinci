//! How VINCI was installed, and how to apply an update for each install type.

use crate::UpdateError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstallKind {
    /// Per-user NSIS installer (`%LOCALAPPDATA%\VINCI`) — no admin rights needed.
    WindowsNsis {
        exe: PathBuf,
    },
    /// MSI (per-machine).
    WindowsMsi {
        exe: PathBuf,
    },
    AppImage {
        path: PathBuf,
    },
    Deb,
    Rpm,
    /// Portable archive installed by `install.sh` (~/.local/bin) or `install.ps1`
    /// (%LOCALAPPDATA%\vinci). Updated by re-running the installer.
    Archive {
        exe: PathBuf,
    },
    /// Built from source / unknown.
    Source,
}

impl InstallKind {
    pub fn asset_suffix(&self) -> &'static str {
        match self {
            InstallKind::WindowsNsis { .. } => "nsis",
            InstallKind::WindowsMsi { .. } => "msi",
            InstallKind::AppImage { .. } => "appimage",
            InstallKind::Deb => "deb",
            InstallKind::Rpm => "rpm",
            InstallKind::Archive { .. } => "archive",
            InstallKind::Source => "source",
        }
    }
    pub fn can_self_update(&self) -> bool {
        !matches!(self, InstallKind::Source | InstallKind::Archive { .. })
    }
}

pub fn detect_install_kind() -> InstallKind {
    if let Some(p) = std::env::var_os("APPIMAGE") {
        return InstallKind::AppImage { path: PathBuf::from(p) };
    }
    let Ok(exe) = std::env::current_exe() else { return InstallKind::Source };
    if cfg!(windows) {
        let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        if dir.join("uninstall.exe").exists() {
            return InstallKind::WindowsNsis { exe };
        }
        let s = exe.to_string_lossy().to_lowercase();
        if s.contains("program files") {
            return InstallKind::WindowsMsi { exe };
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            if dir.to_string_lossy().eq_ignore_ascii_case(&PathBuf::from(local).join("vinci").to_string_lossy()) {
                return InstallKind::Archive { exe };
            }
        }
        return InstallKind::Source;
    }
    if exe.parent().is_some_and(|d| d.ends_with(".local/bin")) {
        return InstallKind::Archive { exe };
    }
    if exe.starts_with("/usr") {
        if Path::new("/var/lib/dpkg/info/vinci.list").exists() {
            return InstallKind::Deb;
        }
        if let Some(rpm) = vinci_hw::process::which("rpm") {
            let ok = vinci_hw::process::command(rpm)
                .args(["-q", "vinci"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return InstallKind::Rpm;
            }
        }
    }
    InstallKind::Source
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApplyOutcome {
    /// Installed; restart VINCI (the app relaunches `exec`).
    Restart { exec: PathBuf },
    /// The installer was started and will relaunch VINCI; the app must exit now.
    ExitNow,
    /// Cannot install automatically: show this command to the user.
    Manual { command: String },
}

/// The one-line installer that updates a portable (archive) installation in place.
pub fn installer_command(repo: &str) -> String {
    if cfg!(windows) {
        format!("irm https://github.com/{repo}/releases/latest/download/install.ps1 | iex")
    } else {
        format!("curl -fsSL https://github.com/{repo}/releases/latest/download/install.sh | bash")
    }
}

pub fn apply(kind: &InstallKind, file: &Path) -> Result<ApplyOutcome, UpdateError> {
    match kind {
        InstallKind::AppImage { path } => {
            // Write next to the target, then atomically replace it.
            let new = path.with_extension("AppImage.new");
            std::fs::copy(file, &new)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
            }
            std::fs::OpenOptions::new().write(true).open(&new)?.sync_all()?;
            std::fs::rename(&new, path)?;
            let _ = std::fs::remove_file(file);
            Ok(ApplyOutcome::Restart { exec: path.clone() })
        }
        InstallKind::WindowsNsis { exe } | InstallKind::WindowsMsi { exe } => {
            let inst = file.display().to_string().replace('\'', "''");
            let app = exe.display().to_string().replace('\'', "''");
            let run = if kind.asset_suffix() == "msi" {
                format!("Start-Process -FilePath 'msiexec.exe' -ArgumentList '/i','\"{inst}\"','/passive','/norestart' -Wait")
            } else {
                format!("Start-Process -FilePath '{inst}' -ArgumentList '/S' -Wait")
            };
            let script = format!("Start-Sleep -Seconds 2; {run}; Start-Process -FilePath '{app}'");
            vinci_hw::process::command("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", &script])
                .spawn()?;
            Ok(ApplyOutcome::ExitNow)
        }
        InstallKind::Deb | InstallKind::Rpm => {
            let f = file.display().to_string();
            let (manual, args): (String, Vec<String>) = if *kind == InstallKind::Deb {
                (format!("sudo apt install '{f}'"), vec!["apt-get".into(), "install".into(), "-y".into(), f.clone()])
            } else if vinci_hw::process::which("dnf").is_some() {
                (format!("sudo dnf install '{f}'"), vec!["dnf".into(), "install".into(), "-y".into(), f.clone()])
            } else {
                (format!("sudo zypper install '{f}'"), vec!["zypper".into(), "--non-interactive".into(), "install".into(), f.clone()])
            };
            match vinci_hw::process::which("pkexec") {
                Some(pk) => {
                    let ok = vinci_hw::process::command(pk).args(&args).status().map(|s| s.success()).unwrap_or(false);
                    if ok {
                        Ok(ApplyOutcome::Restart { exec: PathBuf::from("/usr/bin/vinci") })
                    } else {
                        Ok(ApplyOutcome::Manual { command: manual })
                    }
                }
                None => Ok(ApplyOutcome::Manual { command: manual }),
            }
        }
        InstallKind::Archive { .. } => Ok(ApplyOutcome::Manual { command: installer_command("tfem1/vinci") }),
        InstallKind::Source => Ok(ApplyOutcome::Manual { command: "git pull && ./scripts/install-from-source.sh".into() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn appimage_is_replaced_atomically() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("VINCI.AppImage");
        std::fs::write(&target, b"old").unwrap();
        let dl = d.path().join("download.bin");
        std::fs::write(&dl, b"new").unwrap();
        let out = apply(&InstallKind::AppImage { path: target.clone() }, &dl).unwrap();
        assert_eq!(out, ApplyOutcome::Restart { exec: target.clone() });
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
    }

    #[test]
    fn archive_install_is_updated_by_the_installer() {
        let k = InstallKind::Archive { exe: PathBuf::from("/home/u/.local/bin/vinci") };
        assert!(!k.can_self_update());
        match apply(&k, Path::new("unused")).unwrap() {
            ApplyOutcome::Manual { command } => assert!(command.contains("releases/latest/download/install."), "{command}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn source_build_is_manual() {
        assert!(matches!(apply(&InstallKind::Source, Path::new("x")).unwrap(), ApplyOutcome::Manual { .. }));
    }
}
