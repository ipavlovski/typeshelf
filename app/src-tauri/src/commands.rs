//! Tauri commands. All are `async` so they run off the UI thread.

use std::sync::atomic::Ordering;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use ts_search::{FaceId, Filter, Query, Snapshot};

use crate::state::AppState;

type Res<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Which sidebar entry the list shows.
#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub kind: String,
    pub id: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyRow {
    pub id: u32,
    pub name: String,
    pub faces: u32,
    pub active: u32,
    pub pinned: u32,
    pub favourites: u32,
    /// Face shown as the family's preview: the one closest to Regular.
    pub preview: FaceId,
    /// Every matching face, so activation and collection actions apply to exactly what is shown.
    pub face_ids: Vec<FaceId>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub families: Vec<FamilyRow>,
    pub faces: u64,
    pub micros: u64,
}

fn scope_filter(scope: &Scope) -> Option<Filter> {
    Some(match (scope.kind.as_str(), scope.id) {
        ("active", _) => Filter::Active,
        ("inactive", _) => Filter::Not(Box::new(Filter::Active)),
        ("pinned", _) => Filter::Pinned,
        ("favourites", _) => Filter::Favourite,
        ("collection", Some(id)) => Filter::Collection(id),
        ("folder", Some(id)) => Filter::Folder(id),
        _ => return None,
    })
}

fn family_row(snap: &Snapshot, fam: u32, faces: Vec<FaceId>) -> FamilyRow {
    let preview = faces
        .iter()
        .copied()
        .min_by_key(|&id| {
            let f = &snap.faces[id as usize].info;
            (f.italic, (f.weight as i32 - 400).abs(), f.width.abs_diff(5))
        })
        .unwrap_or(0);
    FamilyRow {
        id: fam,
        name: snap.families[fam as usize].name.clone(),
        faces: faces.len() as u32,
        active: faces.iter().filter(|&&f| snap.is_active(f)).count() as u32,
        pinned: faces.iter().filter(|&&f| snap.is_pinned(f)).count() as u32,
        favourites: faces.iter().filter(|&&f| snap.is_favourite(f)).count() as u32,
        preview,
        face_ids: faces,
    }
}

#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String, scope: Scope) -> Res<Option<SearchResult>> {
    let started = Instant::now();
    let generation = state.search_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let mut q = Query::parse(&query);
    if let Some(f) = scope_filter(&scope) {
        q = q.with(f);
    }
    let snap = state.snap.read().unwrap();
    let cancelled = || state.search_gen.load(Ordering::Relaxed) != generation;
    let Some(res) = snap.search(&q, &cancelled) else { return Ok(None) };
    let families = res.families.into_iter().map(|(fam, faces)| family_row(&snap, fam, faces)).collect();
    Ok(Some(SearchResult { families, faces: res.faces, micros: started.elapsed().as_micros() as u64 }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceDto {
    pub id: FaceId,
    pub style: String,
    pub full_name: String,
    pub ps_name: String,
    pub weight: u16,
    pub width: u16,
    pub italic: bool,
    pub format: String,
    pub glyphs: u16,
    pub path: String,
    pub active: bool,
    pub pinned: bool,
    pub favourite: bool,
    pub axes: Vec<AxisDto>,
    pub features: Vec<String>,
    pub scripts: Vec<String>,
    pub vendor: String,
    pub license: String,
    pub hash: String,
    pub index: u32,
}

#[derive(Serialize)]
pub struct AxisDto {
    pub tag: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

fn face_dto(snap: &Snapshot, id: FaceId) -> FaceDto {
    let row = &snap.faces[id as usize];
    let f = &row.info;
    FaceDto {
        id,
        style: f.subfamily.clone(),
        full_name: f.full_name.clone(),
        ps_name: f.ps_name.clone(),
        weight: f.weight,
        width: f.width,
        italic: f.italic,
        format: f.format.as_str().to_owned(),
        glyphs: f.glyph_count,
        path: row.path.clone(),
        active: snap.is_active(id),
        pinned: snap.is_pinned(id),
        favourite: snap.is_favourite(id),
        axes: f
            .axes
            .iter()
            .map(|a| AxisDto { tag: a.tag.clone(), min: a.min, default: a.default, max: a.max })
            .collect(),
        features: f.features.clone(),
        scripts: f.coverage.blocks().map(|b| b.slug().to_owned()).collect(),
        vendor: f.vendor.clone(),
        license: f.license.clone(),
        hash: row.key.hash.to_string(),
        index: row.key.index,
    }
}

/// All faces of a family, for the expanded row and the detail page.
#[tauri::command]
pub async fn family_faces(state: State<'_, AppState>, family: u32) -> Res<Vec<FaceDto>> {
    let snap = state.snap.read().unwrap();
    let fam = snap.families.get(family as usize).ok_or("unknown family")?;
    Ok(fam.faces.iter().map(|&id| face_dto(&snap, id)).collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationResult {
    pub changed: usize,
    pub failed: Vec<String>,
    /// Fonts also installed system-wide, which deactivation cannot hide.
    pub shadowed: usize,
    /// Running apps known to cache font lists (they need a restart to see changes).
    pub restart_apps: Vec<String>,
}

fn activation_result(out: ts_activate::Outcome) -> ActivationResult {
    ActivationResult {
        changed: out.changed.len(),
        failed: out.failed.into_iter().map(|(_, e)| e).collect(),
        shadowed: out.shadowed.len(),
        restart_apps: if out.changed.is_empty() { vec![] } else { crate::platform::font_caching_apps() },
    }
}

#[tauri::command]
pub async fn set_active(state: State<'_, AppState>, faces: Vec<FaceId>, on: bool, pin: bool) -> Res<ActivationResult> {
    state.set_active(&faces, on, pin).map(activation_result).map_err(err)
}

#[tauri::command]
pub async fn set_pinned(state: State<'_, AppState>, faces: Vec<FaceId>, pinned: bool) -> Res<ActivationResult> {
    state.set_pinned(&faces, pinned).map(activation_result).map_err(err)
}

/// Activates or deactivates everything in a collection, including sub-collections.
#[tauri::command]
pub async fn set_collection_active(state: State<'_, AppState>, id: i64, on: bool) -> Res<ActivationResult> {
    let faces: Vec<FaceId> = state.snap.read().unwrap().collection_faces(id).iter().collect();
    state.set_active(&faces, on, false).map(activation_result).map_err(err)
}

#[tauri::command]
pub async fn deactivate_all(state: State<'_, AppState>) -> Res<()> {
    state.deactivate_session().map_err(err)
}

#[tauri::command]
pub async fn set_favourite(state: State<'_, AppState>, faces: Vec<FaceId>, on: bool) -> Res<()> {
    state.set_favourite(&faces, on).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDto {
    pub id: i64,
    pub path: String,
    pub root: bool,
    pub faces: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDto {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub faces: u64,
    pub active: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub folders: Vec<FolderDto>,
    pub collections: Vec<CollectionDto>,
    pub all: u64,
    pub active: u64,
    pub pinned: u64,
    pub favourites: u64,
    pub can_activate: bool,
}

#[tauri::command]
pub async fn library(state: State<'_, AppState>) -> Res<Library> {
    let store = state.store.lock().unwrap();
    let snap = state.snap.read().unwrap();
    let count = |f: Filter| snap.search(&Query::default().with(f), &|| false).map_or(0, |r| r.faces);
    let folders = store
        .folders()
        .map_err(err)?
        .into_iter()
        .map(|f| FolderDto {
            faces: count(Filter::Folder(f.id)),
            id: f.id,
            path: f.path,
            root: f.kind == ts_store::FolderKind::Root,
        })
        .collect();
    let collections = store
        .collections()
        .map_err(err)?
        .into_iter()
        .map(|c| {
            let faces = snap.collection_faces(c.id);
            let active = faces.iter().filter(|&f| snap.is_active(f)).count() as u64;
            CollectionDto { id: c.id, parent_id: c.parent_id, name: c.name, faces: faces.len(), active }
        })
        .collect();
    Ok(Library {
        folders,
        collections,
        all: snap.faces.len() as u64,
        active: count(Filter::Active),
        pinned: count(Filter::Pinned),
        favourites: count(Filter::Favourite),
        can_activate: state.backend.is_some(),
    })
}

#[derive(Serialize, Clone)]
pub struct IndexProgress {
    pub folder: String,
    pub done: usize,
    pub total: usize,
}

/// Registers a folder and indexes it, emitting `index-progress` events.
#[tauri::command]
pub async fn add_folder(app: AppHandle, state: State<'_, AppState>, path: String, root: bool) -> Res<()> {
    let folder = state.add_folder(path.as_ref(), root).map_err(err)?;
    reindex(&app, &state, &folder).map_err(err)?;
    crate::watch::restart(&app);
    Ok(())
}

pub fn reindex(app: &AppHandle, state: &AppState, folder: &ts_store::Folder) -> anyhow::Result<()> {
    let name = folder.path.clone();
    let progress = |done: usize, total: usize| {
        if done == total || done.is_multiple_of(200) {
            let _ = app.emit("index-progress", IndexProgress { folder: name.clone(), done, total });
        }
    };
    state.index_folder(folder, &progress)?;
    let _ = app.emit("library-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn remove_folder(app: AppHandle, state: State<'_, AppState>, id: i64) -> Res<()> {
    let path = {
        let store = state.store.lock().unwrap();
        let folders = store.folders().map_err(err)?;
        folders.into_iter().find(|f| f.id == id).ok_or("unknown folder")?.path
    };
    state.store.lock().unwrap().remove_folder(&path).map_err(err)?;
    state.reload_snapshot().map_err(err)?;
    crate::watch::restart(&app);
    Ok(())
}

#[tauri::command]
pub async fn collection_create(state: State<'_, AppState>, parent: Option<i64>, name: String) -> Res<i64> {
    let store = state.store.lock().unwrap();
    let id = store.create_collection(parent, &name).map_err(err)?;
    state.refresh_collections(&store).map_err(err)?;
    Ok(id)
}

#[tauri::command]
pub async fn collection_rename(state: State<'_, AppState>, id: i64, name: String) -> Res<()> {
    state.store.lock().unwrap().rename_collection(id, &name).map_err(err)
}

#[tauri::command]
pub async fn collection_delete(state: State<'_, AppState>, id: i64) -> Res<()> {
    let store = state.store.lock().unwrap();
    store.delete_collection(id).map_err(err)?;
    state.refresh_collections(&store).map_err(err)
}

#[tauri::command]
pub async fn collection_add(state: State<'_, AppState>, id: i64, faces: Vec<FaceId>, add: bool) -> Res<()> {
    state.add_to_collection(id, &faces, add).map_err(err)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Res<serde_json::Value> {
    let store = state.store.lock().unwrap();
    let raw = store.setting("ui").map_err(err)?.unwrap_or_else(|| "{}".into());
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_settings(state: State<'_, AppState>, settings: serde_json::Value) -> Res<()> {
    state.store.lock().unwrap().set_setting("ui", &settings.to_string()).map_err(err)
}

/// Raw font bytes for the detail page, which loads one font with the FontFace API.
#[tauri::command]
pub async fn font_bytes(state: State<'_, AppState>, face: FaceId) -> Res<tauri::ipc::Response> {
    let path = state.snap.read().unwrap().faces.get(face as usize).ok_or("unknown face")?.path.clone();
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Code points the face maps, for the glyph grid.
#[tauri::command]
pub async fn face_codepoints(state: State<'_, AppState>, face: FaceId) -> Res<Vec<u32>> {
    let (key, path) = {
        let snap = state.snap.read().unwrap();
        let row = snap.faces.get(face as usize).ok_or("unknown face")?;
        (row.key, row.path.clone())
    };
    let font = state.renderer.font(key, path.as_ref()).map_err(err)?;
    Ok(font.codepoints())
}

#[tauri::command]
pub async fn reveal(state: State<'_, AppState>, face: FaceId) -> Res<()> {
    let path = state.snap.read().unwrap().faces.get(face as usize).ok_or("unknown face")?.path.clone();
    crate::platform::reveal(&path).map_err(err)
}
