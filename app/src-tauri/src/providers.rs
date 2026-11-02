//! FontBase import and the Google Fonts provider.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use ts_store::{Folder, FolderKind};

use crate::commands::{IndexProgress, reindex};
use crate::state::AppState;

type Res<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// The Root folder, where downloads go. Created under the data folder if none is registered.
pub fn root_folder(state: &AppState) -> anyhow::Result<Folder> {
    if let Some(f) = state.store.lock().unwrap().folders()?.into_iter().find(|f| f.kind == FolderKind::Root) {
        return Ok(f);
    }
    let dir = state.data_dir.join("Library");
    std::fs::create_dir_all(&dir)?;
    state.add_folder(&dir, true)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCollection {
    pub name: String,
    pub depth: usize,
    pub fonts: usize,
    pub to_download: usize,
    pub missing: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub root: Option<String>,
    pub root_indexed: bool,
    pub collections: Vec<ImportCollection>,
    pub favourites: usize,
    pub active: usize,
}

/// `None` when no FontBase profile is found.
#[tauri::command]
pub async fn fontbase_preview(state: State<'_, AppState>) -> Res<Option<ImportPreview>> {
    let Some(dir) = ts_import_fontbase::default_dir() else { return Ok(None) };
    let fb = ts_import_fontbase::read(&dir).map_err(err)?;
    let root = ts_import_fontbase::root_path(&fb);
    let store = state.store.lock().unwrap();
    let root_indexed =
        root.as_ref().is_some_and(|r| store.folders().is_ok_and(|fs| fs.iter().any(|f| Path::new(&f.path) == r)));
    let plan = ts_import_fontbase::plan(&fb, &store).map_err(err)?;
    Ok(Some(ImportPreview {
        root: root.map(|r| r.to_string_lossy().into_owned()),
        root_indexed,
        collections: plan
            .collections
            .iter()
            .map(|c| ImportCollection {
                name: c.name.clone(),
                depth: c.depth,
                fonts: c.faces.len(),
                to_download: c.unmatched.iter().filter(|(_, u)| u.is_some()).count(),
                missing: c.unmatched.iter().filter(|(_, u)| u.is_none()).count(),
            })
            .collect(),
        favourites: plan.favourites.len(),
        active: plan.active.len(),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    pub download: bool,
    /// "none", "session" or "pinned".
    pub activate: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub created: usize,
    pub reused: usize,
    pub members: usize,
    pub resolved: usize,
    pub pending: u64,
    pub activated: usize,
    pub failed_downloads: usize,
}

/// Imports FontBase: indexes its Root, creates collections, optionally downloads
/// missing Google fonts and re-activates FontBase's active fonts.
#[tauri::command]
pub async fn fontbase_import(app: AppHandle, state: State<'_, AppState>, options: ImportOptions) -> Res<ImportResult> {
    let dir = ts_import_fontbase::default_dir().ok_or("FontBase data not found")?;
    let fb = ts_import_fontbase::read(&dir).map_err(err)?;
    if let Some(root) = ts_import_fontbase::root_path(&fb).filter(|r| r.is_dir()) {
        let known =
            state.store.lock().unwrap().folders().map_err(err)?.into_iter().find(|f| Path::new(&f.path) == root);
        let folder = match known {
            Some(f) => f,
            None => state.add_folder(&root, true).map_err(err)?,
        };
        reindex(&app, &state, &folder).map_err(err)?;
    }
    let (plan, applied) = {
        let mut store = state.store.lock().unwrap();
        let plan = ts_import_fontbase::plan(&fb, &store).map_err(err)?;
        let applied = ts_import_fontbase::apply(&plan, &mut store).map_err(err)?;
        if let (Some(sample), None) = (fb.samples.first(), store.setting("ui").map_err(err)?) {
            let ui = serde_json::json!({ "sample": sample, "size": fb.font_size.unwrap_or(42.0) });
            store.set_setting("ui", &ui.to_string()).map_err(err)?;
        }
        (plan, applied)
    };
    let (resolved, failed_downloads) =
        if options.download { download_pending(&app, &state).map_err(err)? } else { (0, 0) };

    let mut activated = 0;
    if options.activate != "none" && !plan.active.is_empty() {
        let faces: Vec<u32> = {
            let snap = state.snap.read().unwrap();
            let hashes: Vec<_> = plan.active.iter().map(|a| a.hash).collect();
            (0..snap.faces.len() as u32).filter(|&i| hashes.contains(&snap.faces[i as usize].key.hash)).collect()
        };
        activated = state.set_active(&faces, true, options.activate == "pinned").map_err(err)?.changed.len();
    }
    state.reload_snapshot().map_err(err)?;
    crate::watch::restart(&app);
    let _ = app.emit("library-changed", ());
    Ok(ImportResult {
        created: applied.created,
        reused: applied.reused,
        members: applied.memberships,
        resolved,
        pending: state.store.lock().unwrap().pending_count().map_err(err)?,
        activated,
        failed_downloads,
    })
}

/// Downloads pending collection members into the Root and resolves them. Returns (resolved, failed).
fn download_pending(app: &AppHandle, state: &AppState) -> anyhow::Result<(usize, usize)> {
    let root = root_folder(state)?;
    let dest = ts_providers::google_dir(Path::new(&root.path));
    let items: Vec<(String, PathBuf)> = state
        .store
        .lock()
        .unwrap()
        .pending_downloads()?
        .into_iter()
        .map(|(ps, url)| (url, dest.join(format!("{ps}.ttf"))))
        .collect();
    let failed = ts_providers::download_many(&items, &|done, total| {
        if done == total || done.is_multiple_of(10) {
            let _ =
                app.emit("index-progress", IndexProgress { folder: "Downloading Google Fonts".into(), done, total });
        }
    });
    reindex(app, state, &root)?;
    let resolved = state.store.lock().unwrap().resolve_pending()?;
    state.reload_snapshot()?;
    Ok((resolved, failed.len()))
}

#[tauri::command]
pub async fn download_pending_fonts(app: AppHandle, state: State<'_, AppState>) -> Res<usize> {
    download_pending(&app, &state).map(|(n, _)| n).map_err(err)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GoogleFamily {
    pub family: String,
    pub category: String,
    pub styles: usize,
    pub variable: bool,
    pub popularity: u32,
    pub installed: bool,
}

/// The catalogue is parsed once per run (2.7 MB of JSON).
static CATALOGUE: Mutex<Option<Vec<ts_providers::Family>>> = Mutex::new(None);

#[tauri::command]
pub async fn google_catalogue(state: State<'_, AppState>) -> Res<Vec<GoogleFamily>> {
    let mut cat = CATALOGUE.lock().unwrap();
    if cat.is_none() {
        *cat = Some(ts_providers::catalogue(&state.data_dir.join("google-fonts.json")).map_err(err)?);
    }
    let snap = state.snap.read().unwrap();
    let installed: std::collections::HashSet<String> = snap.families.iter().map(|f| f.name.to_lowercase()).collect();
    Ok(cat
        .as_ref()
        .unwrap()
        .iter()
        .map(|f| GoogleFamily {
            family: f.family.clone(),
            category: f.category.clone(),
            styles: f.styles(),
            variable: !f.axes.is_empty(),
            popularity: f.popularity,
            installed: installed.contains(&f.family.to_lowercase()),
        })
        .collect())
}

/// Downloads a Google family into the Root, indexes it, and returns its face ids.
#[tauri::command]
pub async fn google_install(app: AppHandle, state: State<'_, AppState>, family: String) -> Res<Vec<u32>> {
    let root = root_folder(&state).map_err(err)?;
    let files =
        ts_providers::download_family(&family, &ts_providers::google_dir(Path::new(&root.path))).map_err(err)?;
    reindex(&app, &state, &root).map_err(err)?;
    let _ = state.store.lock().unwrap().resolve_pending();
    state.reload_snapshot().map_err(err)?;
    let snap = state.snap.read().unwrap();
    let paths: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    Ok((0..snap.faces.len() as u32).filter(|&i| paths.contains(&snap.faces[i as usize].path)).collect())
}
