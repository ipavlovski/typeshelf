use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use ts_activate::{Backend, Outcome, Request};
use ts_model::{ActivationMode, FileHash};
use ts_store::Store;

fn backend() -> Result<Box<dyn Backend>> {
    ts_activate::platform().context("font activation is not supported on this OS yet")
}

/// Files whose faces match `query`: exact PostScript, full or family name first,
/// then family substring. Case-insensitive.
fn resolve(store: &Store, query: &str) -> Result<Vec<Request>> {
    let q = query.to_lowercase();
    let faces = store.faces()?;
    let exact: Vec<_> = faces
        .iter()
        .filter(|f| [&f.info.ps_name, &f.info.full_name, &f.info.family].iter().any(|n| n.to_lowercase() == q))
        .collect();
    let matched = if exact.is_empty() {
        faces.iter().filter(|f| f.info.family.to_lowercase().contains(&q)).collect()
    } else {
        exact
    };
    // One request per file; a collection's faces share a file.
    let mut files: BTreeMap<FileHash, Request> = BTreeMap::new();
    for f in matched {
        files.entry(f.key.hash).or_insert_with(|| Request {
            hash: f.key.hash,
            path: f.path.clone(),
            name: f.info.full_name.clone(),
        });
    }
    if files.is_empty() {
        bail!("no fonts match {query:?}");
    }
    Ok(files.into_values().collect())
}

fn print_outcome(verb: &str, out: &Outcome, store: &Store, started: Instant) {
    println!("{verb} {} files, {} unchanged in {:.2?}", out.changed.len(), out.held.len(), started.elapsed());
    for (hash, err) in &out.failed {
        let path = store.paths_for_hash(*hash).ok().and_then(|p| p.into_iter().next()).unwrap_or_default();
        eprintln!("  failed {path}: {err}");
    }
    if !out.shadowed.is_empty() {
        eprintln!(
            "  warning: {} fonts are also installed system-wide; deactivating them here won't hide them",
            out.shadowed.len()
        );
    }
}

pub fn activate(store: &mut Store, query: &str, pin: bool) -> Result<()> {
    let backend = backend()?;
    let reqs = resolve(store, query)?;
    let mode = if pin { ActivationMode::Pinned } else { ActivationMode::Session };
    let started = Instant::now();
    let out = ts_activate::activate(store, backend.as_ref(), &reqs, mode)?;
    print_outcome("activated", &out, store, started);
    Ok(())
}

pub fn deactivate(store: &mut Store, query: Option<&str>, force: bool, all_session: bool) -> Result<()> {
    let backend = backend()?;
    let started = Instant::now();
    let out = match query {
        _ if all_session => ts_activate::deactivate_session(store, backend.as_ref())?,
        Some(q) => {
            let hashes: Vec<_> = resolve(store, q)?.into_iter().map(|r| r.hash).collect();
            ts_activate::deactivate(store, backend.as_ref(), &hashes, force)?
        }
        None => bail!("give a query or --all-session"),
    };
    print_outcome("deactivated", &out, store, started);
    Ok(())
}

pub fn set_mode(store: &mut Store, query: &str, mode: ActivationMode) -> Result<()> {
    let backend = backend()?;
    let hashes: Vec<_> = resolve(store, query)?.into_iter().map(|r| r.hash).collect();
    let started = Instant::now();
    let out = ts_activate::set_mode(store, backend.as_ref(), &hashes, mode)?;
    print_outcome(if mode == ActivationMode::Pinned { "pinned" } else { "unpinned" }, &out, store, started);
    Ok(())
}

pub fn list(store: &Store) -> Result<()> {
    for a in store.activations()? {
        println!("{:<8} x{} {:?} {}", a.mode.as_str(), a.refcount, a.state, a.path);
    }
    Ok(())
}

pub fn reconcile(store: &mut Store, logon: bool) -> Result<()> {
    let backend = backend()?;
    let rep = ts_activate::reconcile(store, backend.as_ref(), logon)?;
    println!("{rep:?}");
    Ok(())
}

#[cfg(windows)]
pub fn autostart(db: &Path, state: &str) -> Result<()> {
    use ts_activate::windows as win;
    match state {
        "on" => {
            let exe = std::env::current_exe()?;
            win::enable_logon_reconcile(&exe, &format!("--db \"{}\" reconcile --logon", db.display()))?;
        }
        "off" => win::disable_logon_reconcile()?,
        _ => {}
    }
    println!("{}", win::logon_reconcile_command().unwrap_or_else(|| "off".into()));
    Ok(())
}

#[cfg(not(windows))]
pub fn autostart(_db: &Path, _state: &str) -> Result<()> {
    bail!("logon reconcile is not supported on this OS yet")
}
