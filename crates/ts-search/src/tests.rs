use ts_model::{ActivationMode, Axis, Coverage, FaceInfo, FaceKey, FileHash, FontFormat};
use ts_store::{Activation, Collection, FaceRow, Folder, FolderKind, JournalState};

use super::*;

fn face(n: u64, family: &str, sub: &str, weight: u16) -> FaceRow {
    let mut cov = vec![0x41u32, 0x61];
    cov.extend(0x20..0x7F);
    if family.contains("Cyr") {
        cov.extend(0x410..0x450);
    }
    FaceRow {
        key: FaceKey { hash: FileHash(n), index: 0 },
        path: format!("/fonts/{family}-{sub}.otf"),
        folder_id: if family.starts_with('E') { 2 } else { 1 },
        info: FaceInfo {
            index: 0,
            family: family.into(),
            family_raw: family.into(),
            subfamily: sub.into(),
            full_name: format!("{family} {sub}"),
            ps_name: format!("{family}-{sub}").replace(' ', ""),
            weight,
            width: 5,
            italic: sub.contains("Italic"),
            format: FontFormat::Otf,
            glyph_count: 300,
            units_per_em: 1000,
            x_height: None,
            cap_height: None,
            axes: if family.contains("Var") {
                vec![Axis { tag: "wght".into(), min: 100.0, default: 400.0, max: 900.0 }]
            } else {
                vec![]
            },
            features: if weight == 700 { vec!["ss01".into()] } else { vec![] },
            coverage: Coverage::from_codepoints(cov),
            vendor: String::new(),
            license: String::new(),
            sample: String::new(),
            metrics: Default::default(),
        },
    }
}

fn snapshot() -> Snapshot {
    let faces = vec![
        face(1, "Adobe Garamond", "Regular", 400),
        face(2, "Adobe Garamond", "Bold", 700),
        face(3, "Adobe Garamond", "Italic", 400),
        face(4, "EB Garamond", "Regular", 400),
        face(5, "Helvetica Neue", "Light", 300),
        face(6, "Helvetica Neue", "Bold", 700),
        face(7, "PT Sans Cyr", "Regular", 400),
        face(8, "Inter Var", "Regular", 400),
    ];
    let folders = vec![
        Folder { id: 1, path: "/fonts".into(), kind: FolderKind::Root },
        Folder { id: 2, path: "/archive/eb".into(), kind: FolderKind::Watched },
    ];
    let mut s = Snapshot::build(faces, folders);
    let cols = vec![
        Collection { id: 10, parent_id: None, name: "1. SERIF".into(), sort: 0 },
        Collection { id: 11, parent_id: Some(10), name: "Old style".into(), sort: 0 },
    ];
    let key = |n| FaceKey { hash: FileHash(n), index: 0 };
    s.set_collections(cols, &[(10, key(4)), (11, key(1))]);
    s.set_favourites(&[key(5)]);
    s.set_activations(&[Activation {
        hash: FileHash(6),
        path: String::new(),
        os_key: String::new(),
        mode: ActivationMode::Pinned,
        refcount: 1,
        state: JournalState::Active,
    }]);
    s
}

fn names(s: &Snapshot, q: &str) -> Vec<String> {
    let r = s.search(&Query::parse(q), &|| false).unwrap();
    r.families.iter().map(|(f, faces)| format!("{}:{}", s.families[*f as usize].name, faces.len())).collect()
}

#[test]
fn empty_query_lists_everything_by_family() {
    let s = snapshot();
    assert_eq!(
        names(&s, ""),
        ["Adobe Garamond:3", "EB Garamond:1", "Helvetica Neue:2", "PT Sans Cyr:1", "Inter Var:1"]
    );
}

#[test]
fn substring_fuzzy_and_typo() {
    let s = snapshot();
    assert_eq!(names(&s, "garamond"), ["Adobe Garamond:3", "EB Garamond:1"]);
    assert_eq!(names(&s, "neue"), ["Helvetica Neue:2"]);
    assert_eq!(names(&s, "hlvtca"), ["Helvetica Neue:2"], "subsequence");
    assert_eq!(names(&s, "helvteica")[0], "Helvetica Neue:2", "transposition typo");
    assert_eq!(names(&s, "GARAMOND"), names(&s, "garamond"));
}

#[test]
fn style_words_find_families() {
    let s = snapshot();
    assert_eq!(names(&s, "helvetica light"), ["Helvetica Neue:2"]);
}

#[test]
fn facets() {
    let s = snapshot();
    assert_eq!(names(&s, "script:cyrillic"), ["PT Sans Cyr:1"]);
    assert_eq!(names(&s, "feat:ss01"), ["Adobe Garamond:1", "Helvetica Neue:1"]);
    assert_eq!(names(&s, "is:variable"), ["Inter Var:1"]);
    assert_eq!(names(&s, "garamond is:italic"), ["Adobe Garamond:1"]);
    assert_eq!(names(&s, "garamond -is:italic"), ["Adobe Garamond:2", "EB Garamond:1"]);
    assert_eq!(names(&s, "is:active"), ["Helvetica Neue:1"]);
    assert_eq!(names(&s, "is:pinned"), ["Helvetica Neue:1"]);
    assert_eq!(names(&s, "is:fav"), ["Helvetica Neue:1"]);
    assert_eq!(names(&s, "folder:archive"), ["EB Garamond:1"]);
}

#[test]
fn ranges() {
    let s = snapshot();
    assert_eq!(names(&s, "wght:>600"), ["Adobe Garamond:1", "Helvetica Neue:1"]);
    assert_eq!(names(&s, "wght:..300"), ["Helvetica Neue:1"]);
    assert_eq!(names(&s, "wght:400 garamond"), ["Adobe Garamond:2", "EB Garamond:1"]);
}

#[test]
fn collections_include_children() {
    let s = snapshot();
    assert_eq!(names(&s, r#"col:"1. serif""#), ["Adobe Garamond:1", "EB Garamond:1"]);
    assert_eq!(names(&s, r#"col:"old style""#), ["Adobe Garamond:1"]);
    let q = Query::default().with(Filter::Collection(10));
    assert_eq!(s.search(&q, &|| false).unwrap().faces, 2);
}

#[test]
fn cancellation() {
    let s = snapshot();
    assert!(s.search(&Query::parse("garamond"), &|| true).is_none());
}

#[test]
fn activation_updates() {
    let mut s = snapshot();
    s.set_activations(&[]);
    assert!(names(&s, "is:active").is_empty());
}

#[test]
fn tags_match_their_subtree() {
    let mut s = snapshot();
    let key = |n| FaceKey { hash: FileHash(n), index: 0 };
    s.set_tags(&[
        ("Serif".into(), key(4)),
        ("Serif/Old style".into(), key(1)),
        ("Serif/Old style".into(), key(2)),
        ("serif-ish".into(), key(8)),
        ("sans/grotesque".into(), key(5)),
    ]);
    assert_eq!(names(&s, "#serif"), ["Adobe Garamond:2", "EB Garamond:1"]);
    assert_eq!(names(&s, r#"#"serif/old style""#), ["Adobe Garamond:2"]);
    assert_eq!(names(&s, "#SANS"), ["Helvetica Neue:1"]);
    assert_eq!(names(&s, "#sans/grot"), Vec::<String>::new(), "segments match whole");
    assert_eq!(names(&s, "garamond -#serif"), ["Adobe Garamond:1"]);
    assert_eq!(s.family_tags(0), ["Serif/Old style"]);
    assert_eq!(names(&s, "garamond is:tagged"), ["Adobe Garamond:2", "EB Garamond:1"]);
    assert_eq!(names(&s, "garamond is:untagged"), ["Adobe Garamond:1"]);

    let tree: Vec<(String, u64)> = s.tag_tree().into_iter().map(|n| (n.path, n.faces.len())).collect();
    assert_eq!(
        tree,
        [
            ("sans".to_owned(), 1),
            ("sans/grotesque".to_owned(), 1),
            ("Serif".to_owned(), 3),
            ("Serif/Old style".to_owned(), 2),
            ("serif-ish".to_owned(), 1),
        ]
    );
}

#[test]
fn duplicate_files_share_tags_favourites_and_collections() {
    let mut a = face(1, "Dupe Sans", "Regular", 400);
    let mut b = face(1, "Dupe Sans", "Regular", 400);
    a.path = "/fonts/one/DupeSans.otf".into();
    b.path = "/fonts/two/DupeSans.otf".into();
    let mut s = Snapshot::build(vec![a, b], vec![]);
    let key = FaceKey { hash: FileHash(1), index: 0 };
    s.set_tags(&[("sans".into(), key)]);
    s.set_favourites(&[key]);
    s.set_collections(vec![Collection { id: 9, parent_id: None, name: "C".into(), sort: 0 }], &[(9, key)]);
    let all = |q: &str| s.search(&Query::parse(q), &|| false).unwrap().faces;
    assert_eq!(all("#sans"), 2, "both copies carry the tag");
    assert_eq!(all("is:untagged"), 0);
    assert_eq!(all("is:fav"), 2);
    assert_eq!(s.collection_faces(9).len(), 2);
}
