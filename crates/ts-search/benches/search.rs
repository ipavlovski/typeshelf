//! Keystroke latency. Point `TS_BENCH_DB` at a real library database
//! (`typeshelf index` output) to measure against it; otherwise 20,000 synthetic faces.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use ts_model::{Coverage, FaceInfo, FaceKey, FileHash, FontFormat};
use ts_search::{Query, Snapshot};
use ts_store::{FaceRow, Store};

const WORDS: &[&str] = &[
    "garamond",
    "sans",
    "neue",
    "grotesk",
    "script",
    "display",
    "mono",
    "serif",
    "slab",
    "gothic",
    "brush",
    "condensed",
    "rounded",
    "stencil",
    "pro",
    "text",
    "deco",
    "retro",
    "hand",
    "classic",
];

fn synthetic(n: usize) -> Vec<FaceRow> {
    (0..n)
        .map(|i| {
            let family = format!("{} {} {}", WORDS[i % 20], WORDS[(i / 20) % 20], i / 4);
            FaceRow {
                key: FaceKey { hash: FileHash(i as u64), index: 0 },
                path: format!("/fonts/{i}.otf"),
                folder_id: 1,
                info: FaceInfo {
                    index: 0,
                    family: family.clone(),
                    family_raw: family.clone(),
                    subfamily: ["Regular", "Bold", "Italic", "Light"][i % 4].into(),
                    full_name: family,
                    ps_name: String::new(),
                    weight: [400, 700, 400, 300][i % 4],
                    width: 5,
                    italic: i % 4 == 2,
                    format: FontFormat::Otf,
                    glyph_count: 300,
                    units_per_em: 1000,
                    x_height: None,
                    cap_height: None,
                    axes: vec![],
                    features: vec!["kern".into()],
                    coverage: Coverage::from_codepoints(if i % 3 == 0 { 0x400..0x450 } else { 0x20..0x7F }),
                    vendor: String::new(),
                    license: String::new(),
                    sample: String::new(),
                },
            }
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let snap = match std::env::var("TS_BENCH_DB") {
        Ok(path) => Snapshot::load(&Store::open(path.as_ref()).unwrap()).unwrap(),
        Err(_) => Snapshot::build(synthetic(20_000), vec![]),
    };
    eprintln!("{} faces, {} families", snap.faces.len(), snap.families.len());
    for q in ["", "g", "gar", "garamond", "helvteica", "sans bold", "script:cyrillic wght:>600", "sans is:italic"] {
        let query = Query::parse(q);
        c.bench_function(&format!("search {q:?}"), |b| b.iter(|| black_box(snap.search(&query, &|| false))));
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
