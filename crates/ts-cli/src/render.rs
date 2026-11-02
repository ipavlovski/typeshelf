use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Result, bail};
use ts_render::{Renderer, Request};
use ts_store::Store;

/// Thumbnails live next to the database, so `--db` keeps test libraries separate.
pub fn cache_dir(db: &Path) -> std::path::PathBuf {
    db.with_file_name("thumbs")
}

pub fn render(store: &Store, query: &str, text: &str, size: u32, out: &Path) -> Result<()> {
    let snap = ts_search::Snapshot::load(store)?;
    let res = snap.search(&ts_search::Query::parse(query), &|| false).unwrap_or_default();
    if res.faces == 0 {
        bail!("no fonts match {query:?}");
    }
    std::fs::create_dir_all(out)?;
    let renderer = Renderer::new(std::env::temp_dir().join("typeshelf-render"))?;
    for (_, faces) in &res.families {
        for &id in faces {
            let row = &snap.faces[id as usize];
            let req = Request { key: row.key, path: Path::new(&row.path), text, size };
            let started = Instant::now();
            match renderer.live(&req, ts_render::THUMB_MAX_WIDTH, &Default::default()) {
                Ok(bytes) => {
                    let name = format!("{}.webp", row.info.full_name.replace(['/', '\\', ':'], "_"));
                    std::fs::write(out.join(&name), bytes)?;
                    println!("{name} ({:.1?})", started.elapsed());
                }
                Err(e) => eprintln!("{}: {e:#}", row.path),
            }
        }
    }
    Ok(())
}

pub fn thumbs(store: &Store, db: &Path, text: &str, sizes: &[u32]) -> Result<()> {
    let snap = ts_search::Snapshot::load(store)?;
    let renderer = Renderer::new(cache_dir(db))?;
    let done = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let total = snap.faces.len() * sizes.len();
    let started = Instant::now();
    std::thread::scope(|s| {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        for chunk in snap.faces.chunks(snap.faces.len().div_ceil(threads).max(1)) {
            let (renderer, done, failed) = (&renderer, &done, &failed);
            s.spawn(move || {
                for row in chunk {
                    for &size in sizes {
                        let req = Request { key: row.key, path: Path::new(&row.path), text, size };
                        if renderer.thumbnail(&req).is_err() {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if n % 1000 == 0 {
                            eprint!("\r  {n}/{total}");
                        }
                    }
                }
            });
        }
    });
    eprintln!("\r  {total} thumbnails, {} failed in {:.2?}", failed.load(Ordering::Relaxed), started.elapsed());
    Ok(())
}
