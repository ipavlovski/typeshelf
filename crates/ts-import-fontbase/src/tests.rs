use ts_model::{Coverage, FaceInfo, FontFormat};
use ts_store::{FileRecord, FileStatus, FolderKind};

use super::*;

fn store_with(fonts: &[(&str, &str, u64)]) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let folder = store.add_folder("C:/FontBase", FolderKind::Root).unwrap();
    let records: Vec<FileRecord> = fonts
        .iter()
        .map(|&(ps, path, hash)| FileRecord {
            path: path.into(),
            size: 1,
            mtime: 0,
            hash: Some(FileHash(hash)),
            status: FileStatus::Ok,
            faces: vec![FaceInfo {
                index: 0,
                family: ps.into(),
                family_raw: ps.into(),
                subfamily: "Regular".into(),
                full_name: ps.replace('-', " "),
                ps_name: ps.into(),
                weight: 400,
                width: 5,
                italic: false,
                format: FontFormat::Otf,
                glyph_count: 1,
                units_per_em: 1000,
                x_height: None,
                cap_height: None,
                axes: vec![],
                features: vec![],
                coverage: Coverage(0),
                vendor: String::new(),
                license: String::new(),
                sample: String::new(),
                metrics: Default::default(),
            }],
        })
        .collect();
    store.write_files(folder, &records).unwrap();
    store
}

fn col(id: &str, name: &str, parent: Option<&str>, ids: &[&str]) -> FbCollection {
    FbCollection {
        id: id.into(),
        name: name.into(),
        parent: parent.map(Into::into),
        ids: ids.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn fontbase() -> FontBase {
    FontBase {
        root: Some("C:/FontBase".into()),
        // Children listed before their parent, as FontBase's object order allows.
        collections: vec![
            col("b", "1B. Old Style", Some("p"), &["Garamond-Regular", "Missing-Font"]),
            col("a", "1A. Humanist", Some("p"), &["Lato-Black"]),
            col("p", "1. SERIF", None, &[]),
        ],
        favourites: vec!["Lato-Black".into()],
        active: vec![("Lato-Black".into(), "/Sans/Lato-Black.ttf".into())],
        samples: vec![],
        font_size: Some(42.0),
        urls: [("Missing-Font".to_owned(), "https://x/missing.ttf".to_owned())].into(),
    }
}

#[test]
fn plans_tree_and_matches_postscript_names() {
    let store = store_with(&[
        ("Garamond-Regular", "C:/FontBase/Serif/Garamond.otf", 1),
        ("Lato-Black", "C:/Other/Lato-Black.ttf", 2),
        ("Lato-Black", "C:/FontBase/Sans/Lato-Black.ttf", 3),
    ]);
    let plan = plan(&fontbase(), &store).unwrap();
    let names: Vec<_> = plan.collections.iter().map(|c| (c.name.as_str(), c.depth, c.faces.len())).collect();
    assert_eq!(names, [("1. SERIF", 0, 0), ("1A. Humanist", 1, 2), ("1B. Old Style", 1, 1)]);
    assert_eq!(plan.collections[2].unmatched, [("Missing-Font".to_owned(), Some("https://x/missing.ttf".to_owned()))]);
    assert_eq!(plan.favourites.len(), 2, "duplicates of a favourite are all favourites");
    // The active font resolves to the copy at FontBase's own path.
    assert_eq!(plan.active.len(), 1);
    assert_eq!(plan.active[0].hash, FileHash(3));
}

#[test]
fn apply_is_idempotent() {
    let mut store = store_with(&[("Garamond-Regular", "C:/FontBase/g.otf", 1), ("Lato-Black", "C:/FontBase/l.ttf", 2)]);
    let p = plan(&fontbase(), &store).unwrap();
    let first = apply(&p, &mut store).unwrap();
    assert_eq!((first.created, first.reused, first.memberships, first.pending, first.favourites), (3, 0, 2, 1, 1));
    assert_eq!(store.pending_downloads().unwrap(), [("Missing-Font".to_owned(), "https://x/missing.ttf".to_owned())]);
    let second = apply(&p, &mut store).unwrap();
    assert_eq!((second.created, second.reused), (0, 3));
    assert_eq!(store.collections().unwrap().len(), 3);
    assert_eq!(store.collection_members().unwrap().len(), 2);
    let tree = store.collections().unwrap();
    let serif = tree.iter().find(|c| c.name == "1. SERIF").unwrap();
    assert!(tree.iter().filter(|c| c.parent_id == Some(serif.id)).count() == 2);
}

#[test]
fn reads_a_leveldb_log() {
    // One FULL record holding a batch with a put and a later delete of another key.
    let mut batch = Vec::new();
    batch.extend(7u64.to_le_bytes());
    batch.extend(2u32.to_le_bytes());
    batch.extend([1, 3, b'k', b'e', b'y', 2, b'v', b'1']);
    batch.extend([0, 4, b'g', b'o', b'n', b'e']);
    let mut log = vec![0, 0, 0, 0];
    log.extend((batch.len() as u16).to_le_bytes());
    log.push(1);
    log.extend(&batch);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("000001.log"), &log).unwrap();
    let records = leveldb::read_dir(dir.path()).unwrap();
    assert_eq!(records.get(b"key".as_slice()), Some(&b"v1".to_vec()));
    assert!(!records.contains_key(b"gone".as_slice()));
}

/// Reads a real FontBase profile when `TS_FONTBASE_DIR` points at its IndexedDB folder.
#[test]
fn reads_real_profile_if_available() {
    let Some(dir) = std::env::var_os("TS_FONTBASE_DIR") else { return };
    let fb = read(Path::new(&dir)).unwrap();
    eprintln!(
        "root {:?}, {} collections, {} favourites, {} active, {} provider urls, samples {:?}",
        fb.root,
        fb.collections.len(),
        fb.favourites.len(),
        fb.active.len(),
        fb.urls.len(),
        fb.samples.first()
    );
    assert!(!fb.collections.is_empty());
}
