//! Folder watching. Bursts of events (a 300-font zip being extracted) are
//! debounced into one callback per quiet period.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use notify::{RecursiveMode, Watcher};

use crate::is_font_path;

/// Watches `folders` recursively and calls `on_change` with the font paths that
/// changed once events have been quiet for `quiet`. Blocks until the watcher fails.
pub fn watch(folders: &[PathBuf], quiet: Duration, mut on_change: impl FnMut(Vec<PathBuf>)) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    for f in folders {
        watcher.watch(f, RecursiveMode::Recursive)?;
    }
    let mut pending = BTreeSet::new();
    loop {
        let event = if pending.is_empty() { rx.recv().ok() } else { rx.recv_timeout(quiet).ok() };
        match event {
            Some(Ok(ev)) => pending.extend(ev.paths.into_iter().filter(|p| is_font_path(p))),
            Some(Err(e)) => return Err(e.into()),
            None if pending.is_empty() => return Ok(()),
            None => on_change(std::mem::take(&mut pending).into_iter().collect()),
        }
    }
}
