//! Finds font files, hashes and parses them in parallel, and writes the results to the store.

mod extract;
mod names;
mod sample;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::Result;
use rayon::prelude::*;
use ts_model::{FileHash, FontFormat};
use ts_store::{FileRecord, FileStatus, Folder, Store};
use walkdir::WalkDir;

pub use extract::extract;
pub use names::{is_junk, sanitize_family};
pub use sample::{DEFAULT_SAMPLE, pick_sample};
pub use watch::watch;

const EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "otc", "woff", "woff2"];

/// Files are parsed and committed in batches, so the store fills up while a
/// large first import runs.
const BATCH: usize = 512;

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub scanned: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    /// Font formats the indexer cannot read yet (WOFF, WOFF2).
    pub unsupported: usize,
    pub failed: usize,
    pub elapsed: Duration,
}

/// Progress callback: `(files done, files to index)`.
pub type Progress<'a> = &'a (dyn Fn(usize, usize) + Sync);

struct Candidate {
    path: PathBuf,
    size: u64,
    mtime: i64,
}

/// Indexes one folder: new and changed files are parsed, unchanged ones skipped
/// (same size and mtime), and files that disappeared are removed.
pub fn index_folder(store: &mut Store, folder: &Folder, progress: Progress) -> Result<Report> {
    let start = Instant::now();
    let root = Path::new(&folder.path);
    // A folder registered inside this one owns its own files.
    let nested: Vec<PathBuf> = store
        .folders()?
        .into_iter()
        .map(|f| PathBuf::from(f.path))
        .filter(|p| p != root && p.starts_with(root))
        .collect();

    let mut known = store.file_states(folder.id)?;
    let mut todo = Vec::new();
    let mut report = Report::default();
    let walker = WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| !is_mac_junk(e.file_name()) && !nested.iter().any(|n| n == e.path()));
    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() || !is_font_path(entry.path()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let path = entry.into_path();
        let mtime =
            meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
        report.scanned += 1;
        let key = path.to_string_lossy();
        match known.remove(key.as_ref()) {
            Some(s) if s.size == meta.len() && s.mtime == mtime => report.unchanged += 1,
            _ => todo.push(Candidate { path, size: meta.len(), mtime }),
        }
    }

    let gone: Vec<i64> = known.into_values().map(|s| s.id).collect();
    report.removed = gone.len();
    store.remove_files(&gone)?;

    let done = AtomicUsize::new(0);
    for chunk in todo.chunks(BATCH) {
        let records: Vec<FileRecord> = chunk
            .par_iter()
            .map(|c| {
                let rec = index_file(c);
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, todo.len());
                rec
            })
            .collect();
        for r in &records {
            match r.status {
                FileStatus::Ok => {}
                FileStatus::Unsupported(_) => report.unsupported += 1,
                FileStatus::Error(_) => report.failed += 1,
            }
        }
        store.write_files(folder.id, &records)?;
    }
    report.indexed = todo.len();
    report.elapsed = start.elapsed();
    Ok(report)
}

pub fn is_font_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// `__MACOSX` folders and `._*` AppleDouble files come from zips made on a Mac
/// and hold resource forks, not fonts.
fn is_mac_junk(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name == "__MACOSX" || name.starts_with("._")
}

fn index_file(c: &Candidate) -> FileRecord {
    let mut rec = FileRecord {
        path: c.path.to_string_lossy().into_owned(),
        size: c.size,
        mtime: c.mtime,
        hash: None,
        status: FileStatus::Ok,
        faces: Vec::new(),
    };
    let bytes = match std::fs::read(&c.path) {
        Ok(b) => b,
        Err(e) => {
            rec.status = FileStatus::Error(e.to_string());
            return rec;
        }
    };
    rec.hash = Some(FileHash(xxhash_rust::xxh3::xxh3_64(&bytes)));
    if let Some(f @ (FontFormat::Woff | FontFormat::Woff2)) = FontFormat::sniff(&bytes) {
        rec.status = FileStatus::Unsupported(format!("{} is not supported yet", f.as_str()));
        return rec;
    }
    let stem = c.path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
    // A malformed font must not take the whole import down with it.
    match std::panic::catch_unwind(|| extract(&bytes, &stem)) {
        Ok(Ok(faces)) => rec.faces = faces,
        Ok(Err(e)) => rec.status = FileStatus::Error(format!("{e:#}")),
        Err(_) => rec.status = FileStatus::Error("parser panicked".into()),
    }
    rec
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_model::Block;
    use ts_store::FolderKind;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").canonicalize().unwrap()
    }

    fn indexed() -> (Store, Folder, Report) {
        let mut store = Store::open_in_memory().unwrap();
        let path = fixtures().to_string_lossy().into_owned();
        let id = store.add_folder(&path, FolderKind::Watched).unwrap();
        let folder = Folder { id, path, kind: FolderKind::Watched };
        let report = index_folder(&mut store, &folder, &|_, _| {}).unwrap();
        (store, folder, report)
    }

    fn by_ps(store: &Store, ps: &str) -> ts_model::FaceInfo {
        store.faces().unwrap().into_iter().find(|f| f.info.ps_name == ps).unwrap_or_else(|| panic!("{ps}")).info
    }

    #[test]
    fn indexes_every_fixture() {
        let (store, _, report) = indexed();
        assert_eq!(report.scanned, 9);
        assert_eq!(report.failed, 1, "not-a-font.ttf");
        let stats = store.stats().unwrap();
        assert_eq!(stats.faces, 9);
        assert_eq!(stats.errors, 1);
    }

    #[test]
    fn second_run_skips_unchanged_files() {
        let (mut store, folder, _) = indexed();
        let report = index_folder(&mut store, &folder, &|_, _| {}).unwrap();
        assert_eq!((report.indexed, report.unchanged), (0, 9));
    }

    #[test]
    fn prefers_typographic_names() {
        let (store, _, _) = indexed();
        let f = by_ps(&store, "FixtureSans-Light");
        assert_eq!((f.family.as_str(), f.subfamily.as_str(), f.weight), ("Fixture Sans", "Light", 300));
        assert_eq!(f.format, FontFormat::Ttf);
        assert_eq!(f.features, ["liga", "ss01"]);
        assert_eq!(f.x_height, Some(500));
    }

    #[test]
    fn reads_cff_italic() {
        let (store, _, _) = indexed();
        let f = by_ps(&store, "FixtureSans-Italic");
        assert_eq!(f.format, FontFormat::Otf);
        assert!(f.italic);
        assert_eq!(f.family, "Fixture Sans");
    }

    #[test]
    fn sanitises_junk_family() {
        let (store, _, _) = indexed();
        let f = store.faces().unwrap().into_iter().find(|f| f.info.family_raw == "-").unwrap().info;
        assert_eq!(f.family, "Junky");
    }

    #[test]
    fn coverage_aware_samples() {
        let (store, _, _) = indexed();
        assert_eq!(by_ps(&store, "FixtureCaps-Regular").sample, DEFAULT_SAMPLE.to_uppercase());
        let cyr = by_ps(&store, "FixtureCyr-Regular");
        assert!(cyr.coverage.has(Block::Cyrillic) && !cyr.coverage.has(Block::BasicLatin));
        assert!(cyr.sample.starts_with("Съешь"));
        let dingbats = by_ps(&store, "FixtureDingbats-Regular");
        assert!(dingbats.coverage.has(Block::Dingbats));
        assert!(dingbats.sample.starts_with('\u{2701}'));
    }

    #[test]
    fn variable_axes() {
        let (store, _, _) = indexed();
        let f = by_ps(&store, "FixtureVar-Regular");
        assert_eq!(f.axes.len(), 1);
        assert_eq!((f.axes[0].tag.as_str(), f.axes[0].min, f.axes[0].max), ("wght", 100.0, 900.0));
    }

    #[test]
    fn collection_faces() {
        let (store, _, _) = indexed();
        let duo: Vec<_> = store.faces().unwrap().into_iter().filter(|f| f.info.family == "Fixture Duo").collect();
        assert_eq!(duo.len(), 2);
        assert_eq!(duo[0].key.hash, duo[1].key.hash);
        assert_eq!(duo.iter().map(|f| f.key.index).collect::<Vec<_>>(), [0, 1]);
        assert!(duo.iter().all(|f| f.info.format == FontFormat::Ttc));
    }

    #[test]
    fn removes_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(fixtures().join("FixtureCyr-Regular.ttf"), dir.path().join("a.ttf")).unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        let id = store.add_folder(&path, FolderKind::Watched).unwrap();
        let folder = Folder { id, path, kind: FolderKind::Watched };
        index_folder(&mut store, &folder, &|_, _| {}).unwrap();
        std::fs::remove_file(dir.path().join("a.ttf")).unwrap();
        let report = index_folder(&mut store, &folder, &|_, _| {}).unwrap();
        assert_eq!(report.removed, 1);
        assert_eq!(store.stats().unwrap().files, 0);
    }
}
