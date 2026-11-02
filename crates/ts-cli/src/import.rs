use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ts_activate::Request;
use ts_model::ActivationMode;
use ts_store::{FolderKind, Store};

pub fn fontbase(
    store: &mut Store,
    from: Option<PathBuf>,
    root: Option<PathBuf>,
    dry_run: bool,
    download: bool,
    activate: &str,
) -> Result<()> {
    let dir = from.or_else(ts_import_fontbase::default_dir).context("FontBase data not found; pass --from")?;
    let fb = ts_import_fontbase::read(&dir)?;
    let root = root.or_else(|| ts_import_fontbase::root_path(&fb));
    match &root {
        Some(r) if r.is_dir() => {
            let known = store.folders()?.into_iter().find(|f| Path::new(&f.path) == r.as_path());
            match known {
                Some(f) if dry_run => println!("Root {} (already indexed)", f.path),
                Some(f) => crate::index(store, &f)?,
                None if dry_run => println!("Root {} would be added and indexed", r.display()),
                None => {
                    let f = crate::register(store, r, FolderKind::Root)?;
                    crate::index(store, &f)?;
                }
            }
        }
        Some(r) => {
            eprintln!("warning: FontBase Root {} does not exist; matching against the current index", r.display())
        }
        None => eprintln!("warning: FontBase has no Root folder set"),
    }

    let plan = ts_import_fontbase::plan(&fb, store)?;
    for c in &plan.collections {
        let remote = c.unmatched.iter().filter(|(_, url)| url.is_some()).count();
        let missing = c.unmatched.len() - remote;
        let mut note = String::new();
        if remote > 0 {
            note += &format!("  +{remote} to download");
        }
        if missing > 0 {
            note += &format!("  ({missing} not found)");
        }
        println!("{}{:<36} {:>4} fonts{note}", "  ".repeat(c.depth), c.name, c.faces.len());
    }
    println!("favourites: {}, active in FontBase: {}", plan.favourites.len(), plan.active.len());
    let missing: Vec<&str> = plan
        .collections
        .iter()
        .flat_map(|c| &c.unmatched)
        .filter(|(_, url)| url.is_none())
        .map(|(ps, _)| ps.as_str())
        .chain(plan.unmatched.iter().map(String::as_str))
        .collect();
    if !missing.is_empty() {
        println!("not found anywhere: {} (e.g. {})", missing.len(), missing[..missing.len().min(6)].join(", "));
    }
    if dry_run {
        return Ok(());
    }

    let applied = ts_import_fontbase::apply(&plan, store)?;
    println!(
        "imported: {} collections created, {} reused, {} members, {} pending, {} favourites",
        applied.created, applied.reused, applied.memberships, applied.pending, applied.favourites
    );
    if let (Some(sample), None) = (fb.samples.first(), store.setting("ui")?) {
        let size = fb.font_size.unwrap_or(42.0);
        store.set_setting("ui", &format!(r#"{{"sample":{sample:?},"size":{size}}}"#))?;
    }
    if download {
        download_pending(store)?;
    }
    let mode = match activate {
        "session" => ActivationMode::Session,
        "pinned" => ActivationMode::Pinned,
        _ => return Ok(()),
    };
    let backend = ts_activate::platform().context("font activation is not supported on this OS yet")?;
    let reqs: Vec<Request> =
        plan.active.iter().map(|a| Request { hash: a.hash, path: a.path.clone(), name: a.name.clone() }).collect();
    let out = ts_activate::activate(store, backend.as_ref(), &reqs, mode)?;
    println!("activated {} fonts ({})", out.changed.len(), mode.as_str());
    Ok(())
}

/// The registered Root folder, where provider downloads go.
fn root_folder(store: &Store) -> Result<ts_store::Folder> {
    store
        .folders()?
        .into_iter()
        .find(|f| f.kind == FolderKind::Root)
        .context("no Root folder; register one with `typeshelf index --root <dir>`")
}

/// Downloads pending collection members, indexes them and resolves the memberships.
pub fn download_pending(store: &mut Store) -> Result<()> {
    let root = root_folder(store)?;
    let dest = ts_providers::google_dir(Path::new(&root.path));
    let items: Vec<(String, PathBuf)> =
        store.pending_downloads()?.into_iter().map(|(ps, url)| (url, dest.join(format!("{ps}.ttf")))).collect();
    if items.is_empty() {
        println!("nothing to download");
        return Ok(());
    }
    eprintln!("downloading {} fonts to {}", items.len(), dest.display());
    let failed = ts_providers::download_many(&items, &|done, total| {
        if done == total || done.is_multiple_of(25) {
            eprint!("\r  {done}/{total}");
            let _ = std::io::stderr().flush();
        }
    });
    eprintln!();
    for (url, e) in &failed {
        eprintln!("  failed {url}: {e}");
    }
    crate::index(store, &root)?;
    println!("resolved {} pending members", store.resolve_pending()?);
    Ok(())
}

pub fn google(store: &mut Store, query: &str, get: bool) -> Result<()> {
    let cache = crate::default_db()?.with_file_name("google-fonts.json");
    let q = query.to_lowercase();
    let matches: Vec<ts_providers::Family> =
        ts_providers::catalogue(&cache)?.into_iter().filter(|f| f.family.to_lowercase().contains(&q)).collect();
    if !get {
        for f in matches.iter().take(40) {
            println!(
                "{:<36} {:<12} {:>2} styles {}",
                f.family,
                f.category,
                f.styles(),
                if f.axes.is_empty() { "" } else { "variable" }
            );
        }
        eprintln!("{} families", matches.len());
        return Ok(());
    }
    let Some(f) = matches.iter().find(|f| f.family.to_lowercase() == q).or(matches.first()) else {
        bail!("no Google family matches {query:?}");
    };
    let root = root_folder(store)?;
    let files = ts_providers::download_family(&f.family, &ts_providers::google_dir(Path::new(&root.path)))?;
    println!("downloaded {} files of {}", files.len(), f.family);
    crate::index(store, &root)?;
    store.resolve_pending()?;
    Ok(())
}
