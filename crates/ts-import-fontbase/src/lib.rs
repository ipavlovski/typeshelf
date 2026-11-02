//! Imports a FontBase library: its Root folder, nested collections, favourites,
//! active fonts and preview settings.
//!
//! FontBase (Electron) keeps everything in Chromium IndexedDB. Collections list
//! their members by PostScript name, which this crate matches against the index.
//! Importing is idempotent: collections that already exist are reused.

mod leveldb;
mod v8;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ts_model::{FaceKey, FileHash};
use ts_store::Store;

pub use v8::Value;

#[derive(Clone, Debug, Default)]
pub struct FontBase {
    /// Root folder as FontBase stores it (forward slashes).
    pub root: Option<String>,
    pub collections: Vec<FbCollection>,
    /// PostScript names of favourite fonts.
    pub favourites: Vec<String>,
    /// Active fonts: PostScript name → path relative to the Root.
    pub active: Vec<(String, String)>,
    pub samples: Vec<String>,
    pub font_size: Option<f64>,
    /// Download URLs of provider fonts (Google Fonts) by PostScript name.
    pub urls: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct FbCollection {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    /// Members by PostScript name.
    pub ids: Vec<String>,
}

/// `%APPDATA%\FontBase\IndexedDB\file__0.indexeddb.leveldb`, if it exists.
pub fn default_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join(r"FontBase\IndexedDB\file__0.indexeddb.leveldb");
    dir.is_dir().then_some(dir)
}

/// Reads FontBase's IndexedDB directory (`…\file__0.indexeddb.leveldb`).
pub fn read(dir: &Path) -> Result<FontBase> {
    let records = leveldb::read_dir(dir)?;
    let get = |name: &str| -> Result<Option<Value>> {
        // Object-store record key: database 1, store 1, the data index (1), a string key
        // (type 1), its length in UTF-16 units, then the UTF-16BE text.
        let mut key = vec![0, 1, 1, 1, 1, name.encode_utf16().count() as u8];
        key.extend(name.encode_utf16().flat_map(u16::to_be_bytes));
        records.get(&key).map(|v| v8::decode_record(v).with_context(|| format!("decoding {name}"))).transpose()
    };

    let mut fb = FontBase::default();
    if let Some(settings) = get("fontbase_settings")? {
        fb.root = settings.get("rootFolder").and_then(Value::str).map(str::to_owned);
        fb.font_size = settings.get("fontSize").and_then(Value::num);
        if let Some(samples) = settings.get("samples") {
            fb.samples = samples.fields().iter().filter_map(|(_, v)| v.str()).map(str::to_owned).collect();
        }
    }
    let collections = get("fontbase_collectionsAll")?.context("no collections in this FontBase profile")?;
    for (_, c) in collections.fields() {
        let ids: Vec<String> = c
            .get("query")
            .and_then(|q| q.get("ids"))
            .map(|ids| ids.items().iter().filter_map(Value::str).map(str::to_owned).collect())
            .unwrap_or_default();
        match c.get("type").and_then(Value::str).unwrap_or_default() {
            // Built-in smart lists: All, Active, Inactive, Recent.
            "default" => {}
            "favorites" => fb.favourites = ids,
            _ => fb.collections.push(FbCollection {
                id: c.get("id").and_then(Value::str).unwrap_or_default().to_owned(),
                name: c.get("name").and_then(Value::str).unwrap_or("Untitled").to_owned(),
                parent: c.get("parent").and_then(Value::str).map(str::to_owned),
                ids,
            }),
        }
    }
    if let Some(status) = get("fontbase_fontsStatus")? {
        fb.active = status.fields().iter().filter_map(|(ps, p)| Some((ps.clone(), p.str()?.to_owned()))).collect();
    }
    fb.urls = provider_urls(dir);
    Ok(fb)
}

/// Large IndexedDB values live in sibling blob files. The file records (path →
/// {id, url, …}) are recognised by shape, since mapping blob numbers to keys
/// needs Chromium's blob journal.
fn provider_urls(leveldb_dir: &Path) -> HashMap<String, String> {
    let mut urls = HashMap::new();
    let Some(name) = leveldb_dir.file_name().and_then(|n| n.to_str()) else { return urls };
    let blob_dir = leveldb_dir.with_file_name(name.replace(".leveldb", ".blob"));
    let mut stack = vec![blob_dir];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(value) = std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| v8::decode_blink(&b)) else {
                continue;
            };
            for (key, rec) in value.fields() {
                if !key.starts_with('/') {
                    break;
                }
                if let (Some(id), Some(url)) = (rec.get("id").and_then(Value::str), rec.get("url").and_then(Value::str))
                {
                    urls.insert(id.to_owned(), url.to_owned());
                }
            }
        }
    }
    urls
}

#[derive(Clone, Debug)]
pub struct PlannedCollection {
    pub name: String,
    /// Index into [`Plan::collections`]; parents always come before children.
    pub parent: Option<usize>,
    pub depth: usize,
    pub faces: Vec<FaceKey>,
    /// Members not in the index, with a download URL when FontBase knew one.
    pub unmatched: Vec<(String, Option<String>)>,
}

#[derive(Clone, Debug)]
pub struct ActiveFont {
    pub hash: FileHash,
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub collections: Vec<PlannedCollection>,
    pub favourites: Vec<FaceKey>,
    pub active: Vec<ActiveFont>,
    /// PostScript names (favourites, active) not found in the index.
    pub unmatched: Vec<String>,
}

/// The FontBase Root as a local path (`C:/Users/x/FontBase` → `C:\Users\x\FontBase` on Windows).
pub fn root_path(fb: &FontBase) -> Option<PathBuf> {
    fb.root.as_ref().map(|r| PathBuf::from(if cfg!(windows) { r.replace('/', "\\") } else { r.clone() }))
}

fn norm_path(p: &str) -> String {
    p.replace('\\', "/").to_lowercase()
}

/// Matches FontBase's members against the index. The Root should be indexed first.
pub fn plan(fb: &FontBase, store: &Store) -> Result<Plan> {
    let faces = store.faces()?;
    // FontBase ids are PostScript names; fall back to the full name without spaces.
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        by_name.entry(f.info.ps_name.clone()).or_default().push(i);
        let compact: String = f.info.full_name.split_whitespace().collect();
        if compact != f.info.ps_name {
            by_name.entry(compact).or_default().push(i);
        }
    }
    let lookup = |id: &str| by_name.get(id).map(|v| v.iter().map(|&i| faces[i].key).collect::<Vec<_>>());

    let mut plan = Plan::default();
    // Parents before children, siblings by name (FontBase numbers them "1A.", "1B.", …).
    let mut order: Vec<&FbCollection> = fb.collections.iter().collect();
    order.sort_by(|a, b| a.name.cmp(&b.name));
    let mut placed: HashMap<&str, usize> = HashMap::new();
    let mut remaining = order;
    while !remaining.is_empty() {
        let before = remaining.len();
        remaining.retain(|c| {
            let parent = match &c.parent {
                Some(p) if fb.collections.iter().any(|x| &x.id == p) => match placed.get(p.as_str()) {
                    Some(&i) => Some(i),
                    None => return true,
                },
                _ => None,
            };
            let mut keys = Vec::new();
            let mut unmatched = Vec::new();
            for id in &c.ids {
                match lookup(id) {
                    Some(k) => keys.extend(k),
                    None => unmatched.push((id.clone(), fb.urls.get(id).cloned())),
                }
            }
            keys.sort();
            keys.dedup();
            let depth = parent.map_or(0, |p| plan.collections[p].depth + 1);
            plan.collections.push(PlannedCollection { name: c.name.clone(), parent, depth, faces: keys, unmatched });
            placed.insert(&c.id, plan.collections.len() - 1);
            false
        });
        // A parent cycle would loop forever; attach what is left at the top level.
        if remaining.len() == before {
            for c in remaining.drain(..) {
                let unmatched = c.ids.iter().map(|id| (id.clone(), fb.urls.get(id).cloned())).collect();
                plan.collections.push(PlannedCollection {
                    name: c.name.clone(),
                    parent: None,
                    depth: 0,
                    faces: vec![],
                    unmatched,
                });
            }
        }
    }

    plan.collections = depth_first(std::mem::take(&mut plan.collections));

    for id in &fb.favourites {
        match lookup(id) {
            Some(k) => plan.favourites.extend(k),
            None => plan.unmatched.push(id.clone()),
        }
    }
    for (ps, rel) in &fb.active {
        // Prefer the file at FontBase's path; duplicates elsewhere share the PostScript name.
        let rel = norm_path(rel);
        let candidates: Vec<&ts_store::FaceRow> = by_name.get(ps).into_iter().flatten().map(|&i| &faces[i]).collect();
        let pick = candidates.iter().find(|f| norm_path(&f.path).ends_with(&rel)).or(candidates.first());
        match pick {
            Some(f) => {
                plan.active.push(ActiveFont { hash: f.key.hash, path: f.path.clone(), name: f.info.full_name.clone() })
            }
            None => plan.unmatched.push(ps.clone()),
        }
    }
    Ok(plan)
}

/// Reorders collections so each is followed by its subtree, keeping sibling order.
fn depth_first(cols: Vec<PlannedCollection>) -> Vec<PlannedCollection> {
    fn visit(i: usize, cols: &[PlannedCollection], order: &mut Vec<usize>) {
        order.push(i);
        for (j, c) in cols.iter().enumerate() {
            if c.parent == Some(i) {
                visit(j, cols, order);
            }
        }
    }
    let mut order = Vec::with_capacity(cols.len());
    for (i, c) in cols.iter().enumerate() {
        if c.parent.is_none() {
            visit(i, &cols, &mut order);
        }
    }
    let new_index: HashMap<usize, usize> = order.iter().enumerate().map(|(new, &old)| (old, new)).collect();
    order
        .iter()
        .map(|&old| {
            let mut c = cols[old].clone();
            c.parent = c.parent.map(|p| new_index[&p]);
            c
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub created: usize,
    pub reused: usize,
    pub memberships: usize,
    /// Members kept by PostScript name until their font is indexed.
    pub pending: usize,
    pub favourites: usize,
}

/// Creates the collections (reusing same-named ones under the same parent) and favourites.
pub fn apply(plan: &Plan, store: &mut Store) -> Result<Applied> {
    let mut out = Applied::default();
    let mut ids: Vec<i64> = Vec::with_capacity(plan.collections.len());
    for c in &plan.collections {
        let parent = c.parent.map(|p| ids[p]);
        let existing = store.collections()?.into_iter().find(|x| x.parent_id == parent && x.name == c.name);
        let id = match existing {
            Some(x) => {
                out.reused += 1;
                x.id
            }
            None => {
                out.created += 1;
                store.create_collection(parent, &c.name)?
            }
        };
        store.add_to_collection(id, &c.faces)?;
        store.add_pending(id, &c.unmatched)?;
        out.memberships += c.faces.len();
        out.pending += c.unmatched.len();
        ids.push(id);
    }
    for &f in &plan.favourites {
        store.set_favourite(f, true)?;
        out.favourites += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
