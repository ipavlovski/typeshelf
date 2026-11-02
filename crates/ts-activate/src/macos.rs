//! Activation through CoreText's font manager: session scope for session fonts
//! (macOS drops them at logout), persistent scope for pinned ones. CoreText
//! notifies running apps itself.

use std::collections::HashSet;
use std::ffi::c_void;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ts_model::{ActivationMode, FileHash};

use crate::Backend;

type CFTypeRef = *const c_void;

const SCOPE_PERSISTENT: u32 = 2;
const SCOPE_SESSION: u32 = 3;
/// kCTFontManagerErrorAlreadyRegistered / NotRegistered: both mean "already in the wanted state".
const ALREADY_REGISTERED: isize = 105;
const NOT_REGISTERED: isize = 201;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(alloc: CFTypeRef, buf: *const u8, len: isize, is_dir: bool)
    -> CFTypeRef;
    fn CFErrorGetCode(err: CFTypeRef) -> isize;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontManagerRegisterFontsForURL(url: CFTypeRef, scope: u32, error: *mut CFTypeRef) -> bool;
    fn CTFontManagerUnregisterFontsForURL(url: CFTypeRef, scope: u32, error: *mut CFTypeRef) -> bool;
}

fn scope(mode: ActivationMode) -> u32 {
    match mode {
        ActivationMode::Session => SCOPE_SESSION,
        ActivationMode::Pinned => SCOPE_PERSISTENT,
    }
}

/// Calls a register/unregister function, treating `ok_code` as success.
fn call(
    path: &Path,
    scope: u32,
    ok_code: isize,
    f: unsafe extern "C" fn(CFTypeRef, u32, *mut CFTypeRef) -> bool,
) -> Result<()> {
    let bytes = path.as_os_str().as_bytes();
    unsafe {
        let url =
            CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, false);
        if url.is_null() {
            bail!("invalid path {}", path.display());
        }
        let mut error: CFTypeRef = std::ptr::null();
        let ok = f(url, scope, &mut error);
        CFRelease(url);
        if ok {
            return Ok(());
        }
        let code = if error.is_null() { 0 } else { CFErrorGetCode(error) };
        if !error.is_null() {
            CFRelease(error);
        }
        if code == ok_code {
            return Ok(());
        }
        bail!("CoreText error {code}")
    }
}

pub struct MacBackend;

impl Backend for MacBackend {
    fn os_key(&self, _name: &str, hash: FileHash) -> String {
        format!("{hash}")
    }

    fn register(&self, path: &Path, _os_key: &str, mode: ActivationMode) -> Result<()> {
        // Switching modes moves the font between scopes.
        let other = if mode == ActivationMode::Session { SCOPE_PERSISTENT } else { SCOPE_SESSION };
        let _ = call(path, other, NOT_REGISTERED, CTFontManagerUnregisterFontsForURL);
        call(path, scope(mode), ALREADY_REGISTERED, CTFontManagerRegisterFontsForURL)
    }

    fn unregister(&self, path: &Path, _os_key: &str) -> Result<()> {
        for s in [SCOPE_SESSION, SCOPE_PERSISTENT] {
            call(path, s, NOT_REGISTERED, CTFontManagerUnregisterFontsForURL)?;
        }
        Ok(())
    }

    /// CoreText cannot list registrations by owner; the journal is the only record.
    fn owned(&self) -> Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }

    fn system_font_names(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }

    fn flush(&self) {}
}

const AGENT: &str = "app.typeshelf.reconcile";

fn agent_file() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(format!("Library/LaunchAgents/{AGENT}.plist")))
}

/// Runs `exe args…` at every login through a LaunchAgent.
pub fn enable_logon_reconcile(exe: &Path, args: &str) -> Result<()> {
    let file = agent_file().context("no home directory")?;
    std::fs::create_dir_all(file.parent().unwrap())?;
    let mut program = format!("<string>{}</string>", exe.display());
    for a in args.split_whitespace() {
        program += &format!("<string>{}</string>", a.trim_matches('"'));
    }
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{AGENT}</string>
<key>ProgramArguments</key><array>{program}</array>
<key>RunAtLoad</key><true/>
</dict></plist>
"#
    );
    std::fs::write(file, plist)?;
    Ok(())
}

pub fn disable_logon_reconcile() -> Result<()> {
    if let Some(file) = agent_file().filter(|f| f.exists()) {
        std::fs::remove_file(file)?;
    }
    Ok(())
}

pub fn logon_reconcile_command() -> Option<String> {
    agent_file().filter(|f| f.exists()).map(|f| f.display().to_string())
}
