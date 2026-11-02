//! Per-user activation through fontconfig: a symlink to the font file in
//! `~/.local/share/fonts/typeshelf/{session,pinned}/`, then `fc-cache` on that folder.
//! Session links are cleared by the logon reconcile (an XDG autostart entry).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ts_model::{ActivationMode, FileHash};

use crate::Backend;

const MODES: [ActivationMode; 2] = [ActivationMode::Session, ActivationMode::Pinned];

pub struct LinuxBackend {
    base: PathBuf,
    dirty: AtomicBool,
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

impl LinuxBackend {
    pub fn new(base: PathBuf) -> Self {
        Self { base, dirty: AtomicBool::new(false) }
    }

    pub fn default_location(app: &str) -> Option<Self> {
        Some(Self::new(data_home()?.join("fonts").join(app)))
    }

    fn dir(&self, mode: ActivationMode) -> PathBuf {
        self.base.join(mode.as_str())
    }
}

impl Backend for LinuxBackend {
    fn os_key(&self, _name: &str, hash: FileHash) -> String {
        format!("{hash}")
    }

    fn register(&self, path: &Path, os_key: &str, mode: ActivationMode) -> Result<()> {
        // The link keeps the extension so fontconfig recognises the format.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("ttf");
        let name = format!("{os_key}.{ext}");
        for m in MODES {
            let _ = std::fs::remove_file(self.dir(m).join(&name));
        }
        let dir = self.dir(mode);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        std::os::unix::fs::symlink(path, dir.join(&name)).with_context(|| format!("linking {}", path.display()))?;
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn unregister(&self, _path: &Path, os_key: &str) -> Result<()> {
        for m in MODES {
            let Ok(entries) = std::fs::read_dir(self.dir(m)) else { continue };
            for e in entries.flatten() {
                if e.path().file_stem().and_then(|s| s.to_str()) == Some(os_key) {
                    std::fs::remove_file(e.path())?;
                    self.dirty.store(true, Ordering::Relaxed);
                }
            }
        }
        Ok(())
    }

    fn owned(&self) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for m in MODES {
            let Ok(entries) = std::fs::read_dir(self.dir(m)) else { continue };
            for e in entries.flatten() {
                let (Some(key), Ok(target)) =
                    (e.path().file_stem().and_then(|s| s.to_str()).map(str::to_owned), std::fs::read_link(e.path()))
                else {
                    continue;
                };
                out.push((key, target.to_string_lossy().into_owned()));
            }
        }
        Ok(out)
    }

    fn system_font_names(&self) -> Result<HashSet<String>> {
        let out = Command::new("fc-list").args([":", "fullname", "file"]).output().context("running fc-list")?;
        let mut names = HashSet::new();
        // Lines look like "/usr/share/fonts/x.ttf: :fullname=Name,Other Name".
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let Some((file, rest)) = line.split_once(": ") else { continue };
            if Path::new(file).starts_with(&self.base) {
                continue;
            }
            if let Some(full) = rest.split(":fullname=").nth(1) {
                names.extend(full.split(',').map(|n| n.trim().to_lowercase()));
            }
        }
        Ok(names)
    }

    fn flush(&self) {
        if self.dirty.swap(false, Ordering::Relaxed) {
            // fontconfig trusts a folder's cache while its mtime (in whole seconds) is
            // unchanged, so a link added and removed within one second would be missed.
            // Moving the mtime forward avoids `fc-cache -f`, which sleeps for 2 s.
            let stamp = self.base.join(".mtime");
            let last = std::fs::read_to_string(&stamp).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            // Strictly later than every earlier bump, so the value fontconfig cached never repeats.
            let next = now.max(last + 1);
            for m in MODES {
                set_mtime(&self.dir(m), next);
            }
            let _ = std::fs::write(&stamp, next.to_string());
            let _ = Command::new("fc-cache").arg(&self.base).status();
        }
    }
}

fn set_mtime(dir: &Path, secs: u64) {
    if let Ok(file) = std::fs::File::open(dir) {
        let _ = file.set_modified(UNIX_EPOCH + Duration::from_secs(secs));
    }
}

fn autostart_file(app: &str) -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart").join(format!("{app}-reconcile.desktop")))
}

/// Runs `args` at every login through an XDG autostart entry.
pub fn enable_logon_reconcile(app: &str, exe: &Path, args: &str) -> Result<()> {
    let file = autostart_file(app).context("no home directory")?;
    std::fs::create_dir_all(file.parent().unwrap())?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={app} font cleanup\nExec=\"{}\" {args}\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    std::fs::write(&file, entry)?;
    Ok(())
}

pub fn disable_logon_reconcile(app: &str) -> Result<()> {
    if let Some(file) = autostart_file(app).filter(|f| f.exists()) {
        std::fs::remove_file(file)?;
    }
    Ok(())
}

pub fn logon_reconcile_command(app: &str) -> Option<String> {
    let text = std::fs::read_to_string(autostart_file(app)?).ok()?;
    text.lines().find_map(|l| l.strip_prefix("Exec=")).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_moves_between_modes() {
        let dir = tempfile::tempdir().unwrap();
        let font = dir.path().join("Font.otf");
        std::fs::write(&font, b"x").unwrap();
        let b = LinuxBackend::new(dir.path().join("fonts"));
        let key = b.os_key("Font", FileHash(0xab));

        b.register(&font, &key, ActivationMode::Session).unwrap();
        assert!(dir.path().join("fonts/session/00000000000000ab.otf").is_symlink());
        b.register(&font, &key, ActivationMode::Pinned).unwrap();
        assert!(!dir.path().join("fonts/session/00000000000000ab.otf").exists());
        assert_eq!(b.owned().unwrap(), [(key.clone(), font.to_string_lossy().into_owned())]);

        b.unregister(&font, &key).unwrap();
        assert!(b.owned().unwrap().is_empty());
    }
}
