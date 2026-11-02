//! Per-user activation on Windows 10 1809+. No admin rights, nothing under `C:\Windows\Fonts`.
//!
//! Activating writes an HKCU font registry value and broadcasts `WM_FONTCHANGE`;
//! Windows' font cache picks the value up and running GDI and DirectWrite apps see
//! the font within ~200 ms. `AddFontResourceExW` is not needed and costs ~200 ms per font.
//!
//! Deactivating also needs `RemoveFontResourceExW`: once loaded, a font stays in
//! the session's GDI table after its registry value is gone. Those removals are
//! queued and run in parallel in [`Backend::flush`]; they take ~1-3 ms per font an
//! app has loaded, so callers with a UI should flush off the UI thread.

use std::collections::HashSet;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result};
use ts_model::{ActivationMode, FileHash};
use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
use windows_sys::Win32::Graphics::Gdi::RemoveFontResourceExW;
use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SendNotifyMessageW, WM_FONTCHANGE};

use crate::Backend;

const FONTS_KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "typeshelf-reconcile";
/// Every value we write carries this, so reconcile can find our entries and never touches others.
const MARKER: &str = " [typeshelf ";
/// GDI reference-counts loads (another app may have loaded the file too).
const MAX_REMOVALS: usize = 16;

/// Windows' font cache picks up registry changes within ~50-200 ms, so an app
/// that re-enumerates the instant it gets the first broadcast can miss new fonts.
/// A second broadcast after this delay catches them.
const SETTLE: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct WindowsBackend {
    /// NUL-terminated paths waiting for `RemoveFontResourceExW`.
    pending_removals: Mutex<Vec<Vec<u16>>>,
    registered: AtomicBool,
    settle: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for WindowsBackend {
    /// Lets a short-lived process (the CLI) send the settle broadcast before exiting.
    fn drop(&mut self) {
        if let Some(t) = self.settle.get_mut().unwrap().take() {
            let _ = t.join();
        }
    }
}

impl Backend for WindowsBackend {
    fn os_key(&self, name: &str, hash: FileHash) -> String {
        format!("{name}{MARKER}{hash}]")
    }

    fn register(&self, path: &Path, os_key: &str, _mode: ActivationMode) -> Result<()> {
        // Both modes use the same registry value; reconcile removes session fonts at logon.
        let path = path.to_str().context("font path is not valid Unicode")?;
        CURRENT_USER.create(FONTS_KEY)?.set_string(os_key, path)?;
        self.registered.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn unregister(&self, path: &Path, os_key: &str) -> Result<()> {
        let key = CURRENT_USER.create(FONTS_KEY)?;
        if key.get_string(os_key).is_ok() {
            key.remove_value(os_key)?;
        }
        self.pending_removals.lock().unwrap().push(wide(path));
        Ok(())
    }

    fn owned(&self) -> Result<Vec<(String, String)>> {
        let key = CURRENT_USER.create(FONTS_KEY)?;
        Ok(key
            .values()?
            .filter(|(name, _)| name.contains(MARKER))
            .filter_map(|(name, value)| Some((name, String::try_from(value).ok()?)))
            .collect())
    }

    fn system_font_names(&self) -> Result<HashSet<String>> {
        let key = LOCAL_MACHINE.open(FONTS_KEY)?;
        let mut names = HashSet::new();
        // Values look like "Arial Bold (TrueType)" or "Cambria & Cambria Math (TrueType)".
        for (name, _) in key.values()? {
            let base = name.rsplit_once(" (").map_or(name.as_str(), |(b, _)| b);
            names.extend(base.split(" & ").map(|n| n.trim().to_lowercase()));
        }
        Ok(names)
    }

    fn flush(&self) {
        let paths = std::mem::take(&mut *self.pending_removals.lock().unwrap());
        // Removals contend inside win32k; more than ~8 threads gets slower, not faster.
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
        std::thread::scope(|s| {
            for chunk in paths.chunks(paths.len().div_ceil(threads).max(1)) {
                s.spawn(move || {
                    for p in chunk {
                        for _ in 0..MAX_REMOVALS {
                            if unsafe { RemoveFontResourceExW(p.as_ptr(), 0, std::ptr::null()) } == 0 {
                                break;
                            }
                        }
                    }
                });
            }
        });
        broadcast();
        if self.registered.swap(false, Ordering::Relaxed) {
            let mut settle = self.settle.lock().unwrap();
            if settle.as_ref().is_none_or(|t| t.is_finished()) {
                *settle = Some(std::thread::spawn(|| {
                    std::thread::sleep(SETTLE);
                    broadcast();
                }));
            }
        }
    }
}

/// Queued to every top-level window without waiting for each app to re-enumerate
/// its fonts (a blocking broadcast costs ~200 ms).
fn broadcast() {
    unsafe {
        SendNotifyMessageW(HWND_BROADCAST, WM_FONTCHANGE, 0, 0);
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Registers `args` to run at every logon (HKCU Run), so session fonts left by a
/// crash or power-off are cleared even if the app is never started.
pub fn enable_logon_reconcile(exe: &Path, args: &str) -> Result<()> {
    let cmd = format!("\"{}\" {args}", exe.display());
    CURRENT_USER.create(RUN_KEY)?.set_string(RUN_VALUE, &cmd)?;
    Ok(())
}

pub fn disable_logon_reconcile() -> Result<()> {
    let key = CURRENT_USER.create(RUN_KEY)?;
    if key.get_string(RUN_VALUE).is_ok() {
        key.remove_value(RUN_VALUE)?;
    }
    Ok(())
}

pub fn logon_reconcile_command() -> Option<String> {
    CURRENT_USER.open(RUN_KEY).ok()?.get_string(RUN_VALUE).ok()
}
