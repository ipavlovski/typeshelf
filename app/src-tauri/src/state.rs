//! Everything the app holds in memory, and the library operations that keep the
//! store, the search snapshot and the OS activation state in step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result, bail};
use ts_activate::{Backend, Outcome, Request};
use ts_model::{ActivationMode, FileHash};
use ts_render::{Pool, Renderer};
use ts_search::{FaceId, Snapshot};
use ts_store::Store;

pub struct AppState {
    pub store: Mutex<Store>,
    pub snap: RwLock<Snapshot>,
    pub renderer: Arc<Renderer>,
    pub pool: Pool,
    /// `None` on platforms without an activation backend yet.
    pub backend: Option<Box<dyn Backend>>,
    /// Sequence number of the newest thumbnail request (see `thumbs`).
    pub thumb_seq: Arc<AtomicU64>,
    /// Bumped per search; a search that sees a newer value stops early.
    pub search_gen: AtomicU64,
    pub data_dir: PathBuf,
}

pub fn data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", crate::channel::APP).context("no home directory")?;
    Ok(dirs.data_local_dir().to_owned())
}

impl AppState {
    pub fn open(data_dir: PathBuf) -> Result<AppState> {
        Self::open_with(data_dir, ts_activate::platform_for(crate::channel::APP))
    }

    /// Opens with a given activation backend (tests use an in-memory one).
    pub fn open_with(data_dir: PathBuf, backend: Option<Box<dyn Backend>>) -> Result<AppState> {
        let mut store = Store::open(&data_dir.join("library.db"))?;
        if let Some(b) = &backend {
            // Finish anything a crash interrupted; session fonts from this logon stay.
            ts_activate::reconcile(&mut store, b.as_ref(), false)?;
        }
        let snap = Snapshot::load(&store)?;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8);
        Ok(AppState {
            store: Mutex::new(store),
            snap: RwLock::new(snap),
            renderer: Arc::new(Renderer::new(data_dir.join("thumbs"))?),
            pool: Pool::new(threads),
            backend,
            thumb_seq: Arc::new(AtomicU64::new(0)),
            search_gen: AtomicU64::new(0),
            data_dir,
        })
    }

    pub fn reload_snapshot(&self) -> Result<()> {
        let snap = Snapshot::load(&self.store.lock().unwrap())?;
        *self.snap.write().unwrap() = snap;
        Ok(())
    }

    fn backend(&self) -> Result<&dyn Backend> {
        match &self.backend {
            Some(b) => Ok(b.as_ref()),
            None => bail!("font activation is not supported on this OS yet"),
        }
    }

    /// One activation request per font file behind `faces`.
    fn requests(&self, faces: &[FaceId]) -> Vec<Request> {
        let snap = self.snap.read().unwrap();
        let mut files: BTreeMap<FileHash, Request> = BTreeMap::new();
        for &id in faces {
            let Some(row) = snap.faces.get(id as usize) else { continue };
            files.entry(row.key.hash).or_insert_with(|| Request {
                hash: row.key.hash,
                path: row.path.clone(),
                name: row.info.full_name.clone(),
            });
        }
        files.into_values().collect()
    }

    fn sync_activations(&self, store: &Store) -> Result<()> {
        self.snap.write().unwrap().set_activations(&store.activations()?);
        Ok(())
    }

    /// Clicking a dot is explicit, so deactivation ignores other holders.
    pub fn set_active(&self, faces: &[FaceId], on: bool, pin: bool) -> Result<Outcome> {
        let backend = self.backend()?;
        let reqs = self.requests(faces);
        let mut store = self.store.lock().unwrap();
        let mode = if pin { ActivationMode::Pinned } else { ActivationMode::Session };
        let out = if on {
            ts_activate::activate(&mut store, backend, &reqs, mode)?
        } else {
            let hashes: Vec<_> = reqs.iter().map(|r| r.hash).collect();
            ts_activate::deactivate(&mut store, backend, &hashes, true)?
        };
        self.sync_activations(&store)?;
        Ok(out)
    }

    pub fn set_pinned(&self, faces: &[FaceId], pinned: bool) -> Result<Outcome> {
        let backend = self.backend()?;
        let hashes: Vec<_> = self.requests(faces).into_iter().map(|r| r.hash).collect();
        let mut store = self.store.lock().unwrap();
        let mode = if pinned { ActivationMode::Pinned } else { ActivationMode::Session };
        let out = ts_activate::set_mode(&mut store, backend, &hashes, mode)?;
        self.sync_activations(&store)?;
        Ok(out)
    }

    pub fn deactivate_session(&self) -> Result<()> {
        let Ok(backend) = self.backend() else { return Ok(()) };
        let mut store = self.store.lock().unwrap();
        ts_activate::deactivate_session(&mut store, backend)?;
        self.sync_activations(&store)
    }

    pub fn set_favourite(&self, faces: &[FaceId], on: bool) -> Result<()> {
        let store = self.store.lock().unwrap();
        let mut snap = self.snap.write().unwrap();
        for &id in faces {
            if let Some(row) = snap.faces.get(id as usize) {
                store.set_favourite(row.key, on)?;
            }
        }
        snap.set_favourites(&store.favourites()?);
        Ok(())
    }

    pub fn add_to_collection(&self, collection: i64, faces: &[FaceId], add: bool) -> Result<()> {
        let keys: Vec<_> = {
            let snap = self.snap.read().unwrap();
            faces.iter().filter_map(|&id| snap.faces.get(id as usize).map(|r| r.key)).collect()
        };
        let mut store = self.store.lock().unwrap();
        if add {
            store.add_to_collection(collection, &keys)?;
        } else {
            store.remove_from_collection(collection, &keys)?;
        }
        self.refresh_collections(&store)
    }

    pub fn refresh_collections(&self, store: &Store) -> Result<()> {
        self.snap.write().unwrap().set_collections(store.collections()?, &store.collection_members()?);
        Ok(())
    }

    /// Indexes one folder; `progress` gets `(done, total)`.
    pub fn index_folder(&self, folder: &ts_store::Folder, progress: ts_index::Progress) -> Result<ts_index::Report> {
        let report = ts_index::index_folder(&mut self.store.lock().unwrap(), folder, progress)?;
        self.reload_snapshot()?;
        Ok(report)
    }

    pub fn add_folder(&self, path: &Path, root: bool) -> Result<ts_store::Folder> {
        if !path.is_dir() {
            bail!("{} is not a folder", path.display());
        }
        let path = path.to_string_lossy().trim_end_matches(['/', '\\']).to_owned();
        let kind = if root { ts_store::FolderKind::Root } else { ts_store::FolderKind::Watched };
        let id = self.store.lock().unwrap().add_folder(&path, kind)?;
        Ok(ts_store::Folder { id, path, kind })
    }
}
