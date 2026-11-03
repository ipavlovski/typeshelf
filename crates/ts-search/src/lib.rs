//! In-memory search over the whole library.
//!
//! A [`Snapshot`] is built once from the store. Names are matched per family
//! (nucleo fuzzy ranking, plus a trigram index for typos); filters are roaring
//! bitmaps over face ids and sorted columns for numeric ranges. A query touches no disk.

mod query;
mod text;

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};
use roaring::RoaringBitmap;
use ts_model::{ActivationMode, Block, FaceKey, FileHash};
use ts_store::{Activation, Collection, FaceRow, Folder, Store};

pub use query::{Filter, Query};
pub use text::normalize;

use text::TrigramIndex;

pub type FaceId = u32;
pub type FamilyId = u32;

pub struct Family {
    pub name: String,
    /// Faces in display order (weight, then italic).
    pub faces: Vec<FaceId>,
    haystack: Utf32String,
}

/// A numeric column sorted by value, so a range becomes a bitmap by binary search.
#[derive(Default)]
struct SortedColumn(Vec<(f32, FaceId)>);

impl SortedColumn {
    fn range(&self, min: f32, max: f32) -> RoaringBitmap {
        let lo = self.0.partition_point(|(v, _)| *v < min);
        let hi = self.0.partition_point(|(v, _)| *v <= max);
        self.0[lo..hi].iter().map(|(_, id)| *id).collect()
    }
}

pub struct Snapshot {
    pub faces: Vec<FaceRow>,
    pub families: Vec<Family>,
    face_family: Vec<FamilyId>,
    by_hash: HashMap<FileHash, Vec<FaceId>>,
    by_key: HashMap<FaceKey, FaceId>,
    trigrams: TrigramIndex,
    all: RoaringBitmap,
    scripts: HashMap<Block, RoaringBitmap>,
    features: HashMap<String, RoaringBitmap>,
    formats: HashMap<&'static str, RoaringBitmap>,
    variable: RoaringBitmap,
    italic: RoaringBitmap,
    folders: HashMap<i64, RoaringBitmap>,
    folder_paths: Vec<Folder>,
    collections: Vec<Collection>,
    collection_members: HashMap<i64, RoaringBitmap>,
    favourites: RoaringBitmap,
    /// Lowercased tag path → tag.
    tags: BTreeMap<String, Tag>,
    /// Tags on each face, as spelled.
    face_tags: HashMap<FaceId, Vec<String>>,
    active: RoaringBitmap,
    pinned: RoaringBitmap,
    columns: HashMap<String, SortedColumn>,
}

struct Tag {
    name: String,
    faces: RoaringBitmap,
}

/// A node of the tag tree: an assigned tag, or a parent implied by one.
#[derive(Clone, PartialEq, Debug)]
pub struct TagNode {
    /// Full path as spelled, e.g. `serif/slab`.
    pub path: String,
    /// Faces carrying this tag or one below it.
    pub faces: RoaringBitmap,
}

/// Lowercased tag paths are the keys of `tags`, so `#Serif` and `#serif` mean the same.
fn tag_key(tag: &str) -> String {
    tag.to_lowercase()
}

/// Matching families in rank order, each with its matching faces.
#[derive(Default, Debug)]
pub struct Results {
    pub families: Vec<(FamilyId, Vec<FaceId>)>,
    pub faces: u64,
}

impl Snapshot {
    pub fn load(store: &Store) -> Result<Snapshot> {
        let mut snap = Snapshot::build(store.faces()?, store.folders()?);
        snap.set_collections(store.collections()?, &store.collection_members()?);
        snap.set_favourites(&store.favourites()?);
        snap.set_tags(&store.face_tags()?);
        snap.set_activations(&store.activations()?);
        Ok(snap)
    }

    /// `faces` must be in display order (family, weight, italic), as [`Store::faces`] returns them.
    pub fn build(faces: Vec<FaceRow>, folders: Vec<Folder>) -> Snapshot {
        let mut snap = Snapshot {
            families: Vec::new(),
            face_family: Vec::with_capacity(faces.len()),
            by_hash: HashMap::new(),
            by_key: HashMap::with_capacity(faces.len()),
            trigrams: TrigramIndex::default(),
            all: (0..faces.len() as u32).collect(),
            scripts: HashMap::new(),
            features: HashMap::new(),
            formats: HashMap::new(),
            variable: RoaringBitmap::new(),
            italic: RoaringBitmap::new(),
            folders: HashMap::new(),
            folder_paths: folders,
            collections: Vec::new(),
            collection_members: HashMap::new(),
            favourites: RoaringBitmap::new(),
            tags: BTreeMap::new(),
            face_tags: HashMap::new(),
            active: RoaringBitmap::new(),
            pinned: RoaringBitmap::new(),
            columns: HashMap::new(),
            faces: Vec::new(),
        };
        let mut family_ids: HashMap<String, FamilyId> = HashMap::new();
        let mut styles: Vec<Vec<String>> = Vec::new();
        let mut cols: HashMap<&str, Vec<(f32, FaceId)>> = HashMap::new();
        for (i, f) in faces.iter().enumerate() {
            let id = i as FaceId;
            let info = &f.info;
            let fam = *family_ids.entry(info.family.to_lowercase()).or_insert_with(|| {
                snap.families.push(Family { name: info.family.clone(), faces: Vec::new(), haystack: "".into() });
                styles.push(Vec::new());
                snap.families.len() as FamilyId - 1
            });
            snap.families[fam as usize].faces.push(id);
            styles[fam as usize].push(normalize(&info.subfamily));
            snap.face_family.push(fam);
            snap.by_hash.entry(f.key.hash).or_default().push(id);
            snap.by_key.insert(f.key, id);
            for b in info.coverage.blocks() {
                snap.scripts.entry(b).or_default().insert(id);
            }
            for t in &info.features {
                snap.features.entry(t.clone()).or_default().insert(id);
            }
            snap.formats.entry(info.format.as_str()).or_default().insert(id);
            if info.is_variable() {
                snap.variable.insert(id);
            }
            if info.italic {
                snap.italic.insert(id);
            }
            snap.folders.entry(f.folder_id).or_default().insert(id);
            cols.entry("wght").or_default().push((info.weight as f32, id));
            cols.entry("wdth").or_default().push((info.width as f32, id));
            cols.entry("glyphs").or_default().push((info.glyph_count as f32, id));
            let m = &info.metrics;
            for (key, v) in [("xheight", m.x_height), ("stem", m.weight), ("width", m.width), ("contrast", m.contrast)]
            {
                if let Some(v) = v {
                    cols.entry(key).or_default().push((v, id));
                }
            }
        }
        // Haystack = family name + its style words, so "garamond bold italic" finds the family.
        for (fam, styles) in snap.families.iter_mut().zip(styles) {
            let mut words: Vec<String> = Vec::new();
            for w in styles.iter().flat_map(|s| s.split(' ')) {
                if !w.is_empty() && !words.iter().any(|x| x == w) {
                    words.push(w.to_owned());
                }
            }
            fam.haystack = format!("{} {}", normalize(&fam.name), words.join(" ")).into();
        }
        for (i, fam) in snap.families.iter().enumerate() {
            snap.trigrams.insert(i as u32, &fam.haystack.to_string());
        }
        for (key, mut v) in cols {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            snap.columns.insert(key.to_owned(), SortedColumn(v));
        }
        snap.faces = faces;
        snap
    }

    pub fn set_collections(&mut self, collections: Vec<Collection>, members: &[(i64, FaceKey)]) {
        self.collection_members.clear();
        for (cid, key) in members {
            let ids = self.faces_with_key(*key);
            self.collection_members.entry(*cid).or_default().extend(ids);
        }
        self.collections = collections;
    }

    pub fn set_favourites(&mut self, keys: &[FaceKey]) {
        self.favourites = keys.iter().flat_map(|&k| self.faces_with_key(k)).collect();
    }

    pub fn set_tags(&mut self, tags: &[(String, FaceKey)]) {
        self.tags.clear();
        self.face_tags.clear();
        for (name, key) in tags {
            for id in self.faces_with_key(*key) {
                let tag = self
                    .tags
                    .entry(tag_key(name))
                    .or_insert_with(|| Tag { name: name.clone(), faces: RoaringBitmap::new() });
                tag.faces.insert(id);
                let on_face = self.face_tags.entry(id).or_default();
                if !on_face.contains(&tag.name) {
                    on_face.push(tag.name.clone());
                }
            }
        }
    }

    /// Every face row with this content: the same file in several folders is one key but
    /// several rows, and favourites, collections and tags belong to all of them.
    fn faces_with_key(&self, key: FaceKey) -> Vec<FaceId> {
        let rows = self.by_hash.get(&key.hash).map_or(&[][..], Vec::as_slice);
        rows.iter().copied().filter(|&id| self.faces[id as usize].key.index == key.index).collect()
    }

    pub fn collections(&self) -> &[Collection] {
        &self.collections
    }

    /// Faces tagged `tag` or anything below it (`serif` includes `serif/slab`).
    pub fn tag_faces(&self, tag: &str) -> RoaringBitmap {
        let key = tag_key(tag);
        let mut out = RoaringBitmap::new();
        // Keys sharing the prefix are contiguous, but `serif-x` sorts between `serif` and `serif/`.
        for (k, t) in self.tags.range(key.clone()..).take_while(|(k, _)| k.starts_with(&key)) {
            if k.len() == key.len() || k.as_bytes()[key.len()] == b'/' {
                out |= &t.faces;
            }
        }
        out
    }

    /// Every tag and implied parent tag, depth-first (each tag followed by its sub-tags),
    /// with the faces below each.
    pub fn tag_tree(&self) -> Vec<TagNode> {
        // Keyed by segments so `serif/x` sorts right after `serif`, before `serif-ish`.
        let mut nodes: BTreeMap<Vec<String>, TagNode> = BTreeMap::new();
        for t in self.tags.values() {
            let mut end = 0;
            loop {
                end = t.name[end..].find('/').map_or(t.name.len(), |i| end + i);
                let path = &t.name[..end];
                nodes
                    .entry(tag_key(path).split('/').map(str::to_owned).collect())
                    .or_insert_with(|| TagNode { path: path.to_owned(), faces: RoaringBitmap::new() })
                    .faces |= &t.faces;
                if end == t.name.len() {
                    break;
                }
                end += 1;
            }
        }
        nodes.into_values().collect()
    }

    /// Tags on any of a family's faces, sorted case-insensitively.
    pub fn family_tags(&self, fam: FamilyId) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in &self.families[fam as usize].faces {
            for t in self.face_tags.get(id).into_iter().flatten() {
                if !out.contains(t) {
                    out.push(t.clone());
                }
            }
        }
        out.sort_by_key(|t| tag_key(t));
        out
    }

    /// Activation is per file, so every face of an active file is active.
    pub fn set_activations(&mut self, acts: &[Activation]) {
        self.active.clear();
        self.pinned.clear();
        for a in acts {
            for &id in self.by_hash.get(&a.hash).into_iter().flatten() {
                self.active.insert(id);
                if a.mode == ActivationMode::Pinned {
                    self.pinned.insert(id);
                }
            }
        }
    }

    /// Adds or replaces a numeric column (e.g. SuperSearch metrics) usable as `key:min..max`.
    pub fn set_column(&mut self, key: &str, values: impl IntoIterator<Item = (FaceId, f32)>) {
        let mut v: Vec<(f32, FaceId)> = values.into_iter().map(|(id, x)| (x, id)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.columns.insert(key.to_owned(), SortedColumn(v));
    }

    /// Value counts per bin over `[min, max]` of a numeric column, for range-slider histograms.
    pub fn histogram(&self, key: &str, bins: usize) -> Option<(f32, f32, Vec<u32>)> {
        let col = &self.columns.get(key)?.0;
        // Trim outliers so one broken font does not squash the chart.
        let lo = col.get(col.len() / 100)?.0;
        let hi = col.get(col.len() - 1 - col.len() / 100)?.0;
        let span = (hi - lo).max(f32::EPSILON);
        let mut counts = vec![0u32; bins];
        for &(v, _) in col {
            let b = (((v - lo) / span) * bins as f32).floor().clamp(0.0, bins as f32 - 1.0) as usize;
            counts[b] += 1;
        }
        Some((lo, hi, counts))
    }

    pub fn family_of(&self, face: FaceId) -> FamilyId {
        self.face_family[face as usize]
    }

    pub fn is_active(&self, face: FaceId) -> bool {
        self.active.contains(face)
    }

    pub fn is_pinned(&self, face: FaceId) -> bool {
        self.pinned.contains(face)
    }

    pub fn is_favourite(&self, face: FaceId) -> bool {
        self.favourites.contains(face)
    }

    pub fn face_by_key(&self, key: FaceKey) -> Option<FaceId> {
        self.by_key.get(&key).copied()
    }

    /// Faces in a collection and all its sub-collections.
    pub fn collection_faces(&self, id: i64) -> RoaringBitmap {
        let mut out = self.collection_members.get(&id).cloned().unwrap_or_default();
        for child in self.collections.iter().filter(|c| c.parent_id == Some(id)) {
            out |= self.collection_faces(child.id);
        }
        out
    }

    fn filter_bitmap(&self, f: &Filter) -> RoaringBitmap {
        let get = |m: Option<&RoaringBitmap>| m.cloned().unwrap_or_default();
        match f {
            Filter::Script(b) => get(self.scripts.get(b)),
            Filter::Feature(t) => get(self.features.get(t)),
            Filter::Format(fmt) => get(self.formats.get(fmt.as_str())),
            Filter::Active => self.active.clone(),
            Filter::Pinned => self.pinned.clone(),
            Filter::Variable => self.variable.clone(),
            Filter::Italic => self.italic.clone(),
            Filter::Favourite => self.favourites.clone(),
            Filter::Tag(t) => self.tag_faces(t),
            Filter::Tagged => self.tags.values().fold(RoaringBitmap::new(), |acc, t| acc | &t.faces),
            Filter::Collection(id) => self.collection_faces(*id),
            Filter::CollectionName(name) => self
                .collections
                .iter()
                .filter(|c| c.name.to_lowercase() == *name)
                .fold(RoaringBitmap::new(), |acc, c| acc | self.collection_faces(c.id)),
            Filter::Folder(id) => get(self.folders.get(id)),
            Filter::FolderPath(p) => self
                .folder_paths
                .iter()
                .filter(|f| f.path.to_lowercase().contains(p.as_str()))
                .fold(RoaringBitmap::new(), |acc, f| acc | get(self.folders.get(&f.id))),
            Filter::Range { key, min, max } => self.columns.get(key).map(|c| c.range(*min, *max)).unwrap_or_default(),
            Filter::Not(inner) => &self.all - self.filter_bitmap(inner),
        }
    }

    /// Runs a query. `cancelled` is polled while ranking; when it returns true the
    /// search stops and returns `None` (a newer keystroke superseded it).
    pub fn search(&self, q: &Query, cancelled: &dyn Fn() -> bool) -> Option<Results> {
        let mut faces = self.all.clone();
        for f in &q.filters {
            faces &= self.filter_bitmap(f);
            if faces.is_empty() {
                return Some(Results::default());
            }
        }
        let text = normalize(&q.text);
        let mut ranked: Vec<(u32, FamilyId)> = Vec::new();
        if text.is_empty() {
            let mut seen = RoaringBitmap::new();
            for id in &faces {
                if seen.insert(self.family_of(id)) {
                    ranked.push((0, self.family_of(id)));
                }
            }
        } else {
            // nucleo scans every family that passed the filters; trigrams only add
            // typo matches it rejects ("helvteica"), and only for atoms long enough to judge.
            let mut typos: Option<RoaringBitmap> = None;
            if text.split(' ').any(|a| a.chars().count() >= text::FUZZY_ATOM) {
                let mut counts = Vec::new();
                for atom in text.split(' ') {
                    if let Some(c) = self.trigrams.candidates(atom, &mut counts) {
                        typos = Some(match typos {
                            Some(prev) => prev & c,
                            None => c,
                        });
                    }
                }
            }
            let families: RoaringBitmap = faces.iter().map(|id| self.family_of(id)).collect();
            let pattern = Pattern::parse(&text, CaseMatching::Ignore, Normalization::Smart);
            let mut matcher = Matcher::new(Config::DEFAULT);
            for (n, fam) in families.iter().enumerate() {
                if n % 512 == 0 && cancelled() {
                    return None;
                }
                let hay = self.families[fam as usize].haystack.slice(..);
                match pattern.score(hay, &mut matcher) {
                    Some(score) => ranked.push((score + 1, fam)),
                    // Typo matches rank below every fuzzy match.
                    None if typos.as_ref().is_some_and(|t| t.contains(fam)) => ranked.push((0, fam)),
                    None => {}
                }
            }
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        let mut out = Results::default();
        for (_, fam) in ranked {
            let members: Vec<FaceId> =
                self.families[fam as usize].faces.iter().copied().filter(|id| faces.contains(*id)).collect();
            out.faces += members.len() as u64;
            out.families.push((fam, members));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests;
