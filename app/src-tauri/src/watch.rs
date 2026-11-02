//! Re-indexes watched folders when files change on disk.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Watcher threads exit when they see a newer generation (folders changed).
static GENERATION: AtomicU64 = AtomicU64::new(0);
static LOCK: Mutex<()> = Mutex::new(());

pub fn restart(app: &AppHandle) {
    let _guard = LOCK.lock().unwrap();
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let state = app.state::<AppState>();
    let Ok(folders) = state.store.lock().unwrap().folders() else { return };
    if folders.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let roots: Vec<PathBuf> = folders.iter().map(|f| PathBuf::from(&f.path)).collect();
        let _ = ts_index::watch(&roots, Duration::from_millis(800), |changed| {
            if GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            let state = app.state::<AppState>();
            for folder in folders.iter().filter(|f| changed.iter().any(|p| p.starts_with(&f.path))) {
                let _ = crate::commands::reindex(&app, &state, folder);
            }
        });
    });
}
