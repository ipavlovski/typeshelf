//! Crash-safe font activation.
//!
//! Every OS change is bracketed by journal writes: `activating` before the change,
//! `active` after, and `deactivating` before a removal. [`reconcile`] runs at
//! launch and at logon, finishes or rolls back anything half-done, clears session
//! fonts after a logoff or crash, re-points fonts whose file moved, and removes registry entries we own that the
//! journal no longer knows about.

mod memory;
#[cfg(windows)]
pub mod windows;

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use ts_model::{ActivationMode, FileHash};
use ts_store::{Activation, JournalState, Store};

pub use memory::MemoryBackend;

/// The OS side of activation. Implementations must be idempotent: registering an
/// already registered font, or unregistering a missing one, succeeds.
pub trait Backend {
    /// Key under which the OS knows this file (e.g. a registry value name).
    /// Must be unique per file and recognisable by [`Backend::owned`].
    fn os_key(&self, name: &str, hash: FileHash) -> String;
    fn register(&self, path: &Path, os_key: &str, mode: ActivationMode) -> Result<()>;
    fn unregister(&self, path: &Path, os_key: &str) -> Result<()>;
    /// OS entries created by typeshelf, as `(os_key, path)`.
    fn owned(&self) -> Result<Vec<(String, String)>>;
    /// Lowercased full names of fonts installed system-wide, which deactivation cannot hide.
    fn system_font_names(&self) -> Result<HashSet<String>>;
    /// Finishes deferred work for the batch and tells running apps the font list
    /// changed. Called once per batch that changed anything.
    fn flush(&self);
}

/// The backend for the current OS, if one exists yet.
pub fn platform() -> Option<Box<dyn Backend>> {
    #[cfg(windows)]
    return Some(Box::new(windows::WindowsBackend::default()));
    #[cfg(not(windows))]
    None
}

/// One font file to activate.
#[derive(Clone, Debug)]
pub struct Request {
    pub hash: FileHash,
    pub path: String,
    /// Display name of the file's first face, used for the OS key.
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// Files whose OS state changed.
    pub changed: Vec<FileHash>,
    /// Files that were already active and only gained a holder or a mode.
    pub held: Vec<FileHash>,
    /// Files whose name matches a system-wide install.
    pub shadowed: Vec<FileHash>,
    pub failed: Vec<(FileHash, String)>,
}

pub fn activate(store: &mut Store, backend: &dyn Backend, reqs: &[Request], mode: ActivationMode) -> Result<Outcome> {
    let mut out = Outcome::default();
    let mut fresh = Vec::new();
    let mut updated = Vec::new();
    for r in reqs {
        match store.activation(r.hash)? {
            Some(a) if a.state == JournalState::Active => {
                let mode = stronger(a.mode, mode);
                if mode != a.mode {
                    // Scope can differ per mode (macOS), so re-register with the new one.
                    if let Err(e) = backend.register(Path::new(&a.path), &a.os_key, mode) {
                        out.failed.push((r.hash, format!("{e:#}")));
                        continue;
                    }
                }
                updated.push(Activation { mode, refcount: a.refcount + 1, ..a });
                out.held.push(r.hash);
            }
            // Missing, or left half-done by a crash: (re)do the OS change.
            prev => fresh.push(Activation {
                hash: r.hash,
                path: r.path.clone(),
                os_key: backend.os_key(&r.name, r.hash),
                mode: prev.as_ref().map_or(mode, |p| stronger(p.mode, mode)),
                refcount: prev.map_or(1, |p| p.refcount.max(1)),
                state: JournalState::Activating,
            }),
        }
    }
    store.put_activations(&updated)?;
    store.put_activations(&fresh)?;

    let mut failed = Vec::new();
    for a in &mut fresh {
        match backend.register(Path::new(&a.path), &a.os_key, a.mode) {
            Ok(()) => {
                a.state = JournalState::Active;
                out.changed.push(a.hash);
            }
            Err(e) => {
                let _ = backend.unregister(Path::new(&a.path), &a.os_key);
                failed.push(a.hash);
                out.failed.push((a.hash, format!("{e:#}")));
            }
        }
    }
    fresh.retain(|a| a.state == JournalState::Active);
    store.put_activations(&fresh)?;
    store.delete_activations(&failed)?;
    if !out.changed.is_empty() {
        backend.flush();
    }

    if let Ok(system) = backend.system_font_names() {
        out.shadowed = reqs.iter().filter(|r| system.contains(&r.name.to_lowercase())).map(|r| r.hash).collect();
    }
    Ok(out)
}

/// Releases one holder of each file; the font leaves the OS when none are left.
/// `force` removes it regardless of holders.
pub fn deactivate(store: &mut Store, backend: &dyn Backend, hashes: &[FileHash], force: bool) -> Result<Outcome> {
    let mut out = Outcome::default();
    let mut released = Vec::new();
    let mut removing = Vec::new();
    for &h in hashes {
        let Some(a) = store.activation(h)? else { continue };
        if !force && a.refcount > 1 && a.state == JournalState::Active {
            released.push(Activation { refcount: a.refcount - 1, ..a });
            out.held.push(h);
        } else {
            removing.push(Activation { state: JournalState::Deactivating, ..a });
        }
    }
    store.put_activations(&released)?;
    store.put_activations(&removing)?;

    let mut done = Vec::new();
    for a in &removing {
        match backend.unregister(Path::new(&a.path), &a.os_key) {
            // A failed removal stays `deactivating`; the next reconcile retries it.
            Err(e) => out.failed.push((a.hash, format!("{e:#}"))),
            Ok(()) => done.push(a.hash),
        }
    }
    store.delete_activations(&done)?;
    if !done.is_empty() {
        backend.flush();
    }
    out.changed = done;
    Ok(out)
}

/// Removes every session font, e.g. when the app quits.
pub fn deactivate_session(store: &mut Store, backend: &dyn Backend) -> Result<Outcome> {
    let hashes: Vec<_> =
        store.activations()?.into_iter().filter(|a| a.mode == ActivationMode::Session).map(|a| a.hash).collect();
    deactivate(store, backend, &hashes, true)
}

/// Changes the mode of active fonts without changing holders.
pub fn set_mode(
    store: &mut Store,
    backend: &dyn Backend,
    hashes: &[FileHash],
    mode: ActivationMode,
) -> Result<Outcome> {
    let mut out = Outcome::default();
    let mut rows = Vec::new();
    for &h in hashes {
        let Some(a) = store.activation(h)? else { continue };
        if a.mode == mode || a.state != JournalState::Active {
            continue;
        }
        match backend.register(Path::new(&a.path), &a.os_key, mode) {
            Ok(()) => {
                out.changed.push(h);
                rows.push(Activation { mode, ..a });
            }
            Err(e) => out.failed.push((h, format!("{e:#}"))),
        }
    }
    store.put_activations(&rows)?;
    Ok(out)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Activations interrupted before they finished.
    pub rolled_back: usize,
    /// Deactivations interrupted before they finished.
    pub finished: usize,
    /// Session fonts cleared at logon.
    pub session_cleared: usize,
    /// Activations whose file moved and were re-pointed.
    pub repaired: usize,
    /// Activations whose file is gone.
    pub missing: usize,
    /// OS entries we own that the journal did not know.
    pub orphans: usize,
}

/// Brings the OS in line with the journal. `logon` = a new user session has started,
/// so session fonts from the previous one are removed. Pinned fonts need nothing:
/// every backend's registration survives a logon.
pub fn reconcile(store: &mut Store, backend: &dyn Backend, logon: bool) -> Result<ReconcileReport> {
    let mut rep = ReconcileReport::default();
    let mut drop = Vec::new();
    let mut update = Vec::new();
    for a in store.activations()? {
        let path = Path::new(&a.path);
        let counter = match a.state {
            JournalState::Activating => Some(&mut rep.rolled_back),
            JournalState::Deactivating => Some(&mut rep.finished),
            JournalState::Active if logon && a.mode == ActivationMode::Session => Some(&mut rep.session_cleared),
            JournalState::Active => None,
        };
        if let Some(counter) = counter {
            let _ = backend.unregister(path, &a.os_key);
            *counter += 1;
            drop.push(a.hash);
        } else if !path.exists() {
            let moved = store.paths_for_hash(a.hash)?.into_iter().find(|p| Path::new(p).exists());
            let _ = backend.unregister(path, &a.os_key);
            match moved {
                Some(new) if backend.register(Path::new(&new), &a.os_key, a.mode).is_ok() => {
                    rep.repaired += 1;
                    update.push(Activation { path: new, ..a });
                }
                _ => {
                    rep.missing += 1;
                    drop.push(a.hash);
                }
            }
        }
    }
    store.put_activations(&update)?;
    store.delete_activations(&drop)?;

    let known: HashSet<String> = store.activations()?.into_iter().map(|a| a.os_key).collect();
    for (key, path) in backend.owned()? {
        if !known.contains(&key) {
            let _ = backend.unregister(Path::new(&path), &key);
            rep.orphans += 1;
        }
    }
    if rep != ReconcileReport::default() {
        backend.flush();
    }
    Ok(rep)
}

fn stronger(a: ActivationMode, b: ActivationMode) -> ActivationMode {
    if a == ActivationMode::Pinned || b == ActivationMode::Pinned {
        ActivationMode::Pinned
    } else {
        ActivationMode::Session
    }
}

#[cfg(test)]
mod tests;
