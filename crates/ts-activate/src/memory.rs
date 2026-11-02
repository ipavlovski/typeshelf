use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Result, bail};
use ts_model::{ActivationMode, FileHash};

use crate::Backend;

/// In-memory backend for tests and for running on platforms without a backend yet.
#[derive(Default)]
pub struct MemoryBackend {
    /// os_key → (path, mode)
    pub entries: Mutex<BTreeMap<String, (String, ActivationMode)>>,
    pub system: Mutex<HashSet<String>>,
    /// Paths whose registration fails.
    pub fail: Mutex<HashSet<String>>,
    pub notifications: AtomicUsize,
}

impl Backend for MemoryBackend {
    fn os_key(&self, name: &str, hash: FileHash) -> String {
        format!("{name} [typeshelf {hash}]")
    }

    fn register(&self, path: &Path, os_key: &str, mode: ActivationMode) -> Result<()> {
        let path = path.to_string_lossy().into_owned();
        if self.fail.lock().unwrap().contains(&path) {
            bail!("registration refused");
        }
        self.entries.lock().unwrap().insert(os_key.to_owned(), (path, mode));
        Ok(())
    }

    fn unregister(&self, _path: &Path, os_key: &str) -> Result<()> {
        self.entries.lock().unwrap().remove(os_key);
        Ok(())
    }

    fn owned(&self) -> Result<Vec<(String, String)>> {
        Ok(self.entries.lock().unwrap().iter().map(|(k, (p, _))| (k.clone(), p.clone())).collect())
    }

    fn system_font_names(&self) -> Result<HashSet<String>> {
        Ok(self.system.lock().unwrap().clone())
    }

    fn flush(&self) {
        self.notifications.fetch_add(1, Ordering::Relaxed);
    }
}
