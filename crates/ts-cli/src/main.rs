mod activation;
mod import;
mod render;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ts_store::{Folder, FolderKind, Store};

#[derive(Parser)]
#[command(name = "typeshelf", version, about = "Fast font library: index, preview and activate fonts")]
struct Cli {
    /// Library database (default: the per-user data directory).
    #[arg(long, global = true, env = "TYPESHELF_DB")]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index folders. Paths given here are registered as watched folders first;
    /// with no paths, every registered folder is re-indexed.
    Index {
        paths: Vec<PathBuf>,
        /// Register the paths as the managed Root folder instead.
        #[arg(long)]
        root: bool,
    },
    /// List registered folders.
    Folders,
    /// Unregister a folder and drop its fonts from the index (files are untouched).
    RemoveFolder { path: PathBuf },
    /// Library statistics.
    Stats,
    /// Files with identical content.
    Dupes,
    /// Files that could not be indexed.
    Errors,
    /// Keep the index up to date while folders change.
    Watch,
    /// Search the library, e.g. `garamond script:cyrillic wght:>600 is:active`.
    Search {
        query: Vec<String>,
        /// Families to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Hierarchical tags (`serif/slab`); search them with `#serif`.
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },
    /// Render previews of matching faces as WebP files into a folder.
    Render {
        query: String,
        #[arg(long, default_value = ts_index::DEFAULT_SAMPLE)]
        text: String,
        #[arg(long, default_value_t = 64)]
        size: u32,
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// Pre-render list thumbnails for the whole library.
    Thumbs {
        #[arg(long, default_value = ts_index::DEFAULT_SAMPLE)]
        text: String,
        /// Pixel sizes to render.
        #[arg(long, value_delimiter = ',', default_value = "32,64")]
        sizes: Vec<u32>,
    },
    /// Import collections, favourites, active fonts and the Root folder from FontBase.
    ImportFontbase {
        /// FontBase's IndexedDB folder (default: %APPDATA%\FontBase\IndexedDB\file__0.indexeddb.leveldb).
        #[arg(long)]
        from: Option<PathBuf>,
        /// Use this folder instead of FontBase's Root (e.g. a copy or a mounted path).
        #[arg(long)]
        root: Option<PathBuf>,
        /// Show what would be imported without changing anything.
        #[arg(long)]
        dry_run: bool,
        /// Re-activate the fonts FontBase had active.
        #[arg(long, value_parser = ["none", "session", "pinned"], default_value = "none")]
        activate: String,
        /// Also download collection members FontBase never downloaded (Google Fonts).
        #[arg(long)]
        download: bool,
    },
    /// Download pending collection members and add them to their collections.
    DownloadPending,
    /// Search the Google Fonts catalogue; with --get, download a family into the Root folder.
    Google {
        query: String,
        #[arg(long)]
        get: bool,
    },
    /// Activate fonts matching a family, full name or PostScript name.
    Activate {
        query: String,
        /// Keep active across reboots instead of for this session only.
        #[arg(long)]
        pin: bool,
    },
    /// Release fonts matching a query (removed once no collection holds them).
    Deactivate {
        /// Omit with --all-session.
        query: Option<String>,
        /// Remove even if other holders remain.
        #[arg(long)]
        force: bool,
        /// Deactivate every session font.
        #[arg(long, conflicts_with = "query")]
        all_session: bool,
    },
    /// Keep active fonts across reboots.
    Pin { query: String },
    /// Make pinned fonts session-only again.
    Unpin { query: String },
    /// List active fonts.
    Active,
    /// Repair activation state after a crash; with --logon, also clear the last session's fonts.
    Reconcile {
        #[arg(long)]
        logon: bool,
    },
    /// Run `reconcile --logon` at every logon (on) or stop doing so (off).
    Autostart {
        #[arg(value_parser = ["on", "off", "status"])]
        state: String,
    },
}

#[derive(Subcommand)]
enum TagAction {
    /// List tags with their face counts.
    List,
    /// Tag every face matching a search query.
    Add { tag: String, query: Vec<String> },
    /// Remove a tag from every face matching a search query.
    Remove { tag: String, query: Vec<String> },
    /// Rename a tag and its sub-tags.
    Rename { from: String, to: String },
    /// Remove a tag and its sub-tags from every font.
    Delete { tag: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db = match cli.db {
        Some(p) => p,
        None => default_db()?,
    };
    let mut store = Store::open(&db)?;
    match cli.command {
        Command::ImportFontbase { from, root, dry_run, activate, download } => {
            import::fontbase(&mut store, from, root, dry_run, download, &activate)?
        }
        Command::DownloadPending => import::download_pending(&mut store)?,
        Command::Google { query, get } => import::google(&mut store, &query, get)?,
        Command::Activate { query, pin } => activation::activate(&mut store, &query, pin)?,
        Command::Deactivate { query, force, all_session } => {
            activation::deactivate(&mut store, query.as_deref(), force, all_session)?
        }
        Command::Pin { query } => activation::set_mode(&mut store, &query, ts_model::ActivationMode::Pinned)?,
        Command::Unpin { query } => activation::set_mode(&mut store, &query, ts_model::ActivationMode::Session)?,
        Command::Active => activation::list(&store)?,
        Command::Reconcile { logon } => activation::reconcile(&mut store, logon)?,
        Command::Autostart { state } => activation::autostart(&db, &state)?,
        Command::Index { paths, root } => {
            let kind = if root { FolderKind::Root } else { FolderKind::Watched };
            let folders = if paths.is_empty() {
                store.folders()?
            } else {
                paths.iter().map(|p| register(&store, p, kind)).collect::<Result<_>>()?
            };
            if folders.is_empty() {
                bail!("no folders registered; run `typeshelf index <dir>`");
            }
            for folder in &folders {
                index(&mut store, folder)?;
            }
        }
        Command::Folders => {
            for f in store.folders()? {
                println!("{:<8} {}", format!("{:?}", f.kind).to_lowercase(), f.path);
            }
        }
        Command::RemoveFolder { path } => {
            let path = absolute(&path)?;
            if !store.remove_folder(&path)? {
                bail!("{path} is not a registered folder");
            }
        }
        Command::Stats => print_stats(&store)?,
        Command::Search { query, limit } => search(&store, &query.join(" "), limit)?,
        Command::Tag { action } => tag(&mut store, action)?,
        Command::Render { query, text, size, out } => render::render(&store, &query, &text, size, &out)?,
        Command::Thumbs { text, sizes } => render::thumbs(&store, &db, &text, &sizes)?,
        Command::Dupes => {
            for (hash, paths) in store.duplicates()? {
                println!("{hash}");
                for p in paths {
                    println!("  {p}");
                }
            }
        }
        Command::Errors => {
            for (path, status, error) in store.errors()? {
                println!("{status:<12} {path}: {error}");
            }
        }
        Command::Watch => {
            let folders = store.folders()?;
            let roots: Vec<PathBuf> = folders.iter().map(|f| PathBuf::from(&f.path)).collect();
            eprintln!("watching {} folders, Ctrl+C to stop", roots.len());
            ts_index::watch(&roots, Duration::from_millis(500), |changed| {
                for folder in folders.iter().filter(|f| changed.iter().any(|p| p.starts_with(&f.path))) {
                    if let Err(e) = index(&mut store, folder) {
                        eprintln!("{}: {e:#}", folder.path);
                    }
                }
            })?;
        }
    }
    Ok(())
}

pub(crate) fn default_db() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "typeshelf").context("no home directory")?;
    Ok(dirs.data_local_dir().join("library.db"))
}

fn absolute(path: &Path) -> Result<String> {
    let p = std::path::absolute(path).with_context(|| format!("resolving {}", path.display()))?;
    Ok(p.to_string_lossy().trim_end_matches(['/', '\\']).to_owned())
}

pub(crate) fn register(store: &Store, path: &Path, kind: FolderKind) -> Result<Folder> {
    if !path.is_dir() {
        bail!("{} is not a directory", path.display());
    }
    let path = absolute(path)?;
    let id = store.add_folder(&path, kind)?;
    Ok(Folder { id, path, kind })
}

pub(crate) fn index(store: &mut Store, folder: &Folder) -> Result<()> {
    eprintln!("indexing {}", folder.path);
    let report = ts_index::index_folder(store, folder, &|done, total| {
        if done == total || done % 250 == 0 {
            eprint!("\r  {done}/{total}");
            let _ = std::io::stderr().flush();
        }
    })?;
    if report.indexed > 0 {
        eprintln!();
    }
    eprintln!(
        "  {} files: {} indexed, {} unchanged, {} removed, {} unsupported, {} failed in {:.2?}",
        report.scanned,
        report.indexed,
        report.unchanged,
        report.removed,
        report.unsupported,
        report.failed,
        report.elapsed
    );
    Ok(())
}

fn search(store: &Store, query: &str, limit: usize) -> Result<()> {
    let started = std::time::Instant::now();
    let snap = ts_search::Snapshot::load(store)?;
    let loaded = started.elapsed();
    let started = std::time::Instant::now();
    let res = snap.search(&ts_search::Query::parse(query), &|| false).unwrap_or_default();
    let took = started.elapsed();
    for (fam, faces) in res.families.iter().take(limit) {
        let active = faces.iter().filter(|&&f| snap.is_active(f)).count();
        let dot = if active == 0 {
            " "
        } else if active == faces.len() {
            "●"
        } else {
            "◐"
        };
        println!("{dot} {:<40} {} styles", snap.families[*fam as usize].name, faces.len());
    }
    eprintln!("{} families, {} faces (snapshot {:.0?}, query {:.2?})", res.families.len(), res.faces, loaded, took);
    Ok(())
}

fn tag(store: &mut Store, action: TagAction) -> Result<()> {
    let matching = |store: &Store, query: &[String]| -> Result<Vec<ts_model::FaceKey>> {
        let query = query.join(" ");
        if query.trim().is_empty() {
            bail!("give a search query for the fonts to change");
        }
        let snap = ts_search::Snapshot::load(store)?;
        let res = snap.search(&ts_search::Query::parse(&query), &|| false).unwrap_or_default();
        Ok(res.families.iter().flat_map(|(_, faces)| faces).map(|&id| snap.faces[id as usize].key).collect())
    };
    match action {
        TagAction::List => {
            let snap = ts_search::Snapshot::load(store)?;
            for node in snap.tag_tree() {
                println!("{:>6}  #{}", node.faces.len(), node.path);
            }
        }
        TagAction::Add { tag, query } => {
            let keys = matching(store, &query)?;
            let tag = store.add_tag(&tag, &keys)?;
            eprintln!("tagged {} faces #{tag}", keys.len());
        }
        TagAction::Remove { tag, query } => {
            let keys = matching(store, &query)?;
            store.remove_tag(&tag, &keys)?;
            eprintln!("removed #{tag} from {} faces", keys.len());
        }
        TagAction::Rename { from, to } => store.rename_tag(&from, &to)?,
        TagAction::Delete { tag } => eprintln!("removed {} taggings", store.delete_tag(&tag)?),
    }
    Ok(())
}

fn print_stats(store: &Store) -> Result<()> {
    let s = store.stats()?;
    println!("folders            {}", s.folders);
    println!("files              {}", s.files);
    println!("faces              {}", s.faces);
    println!("families           {}", s.families);
    println!("variable faces     {}", s.variable_faces);
    println!("renamed families   {}", s.renamed_families);
    println!("duplicate files    {}", s.duplicate_files);
    println!("unsupported files  {}", s.unsupported);
    println!("unreadable files   {}", s.errors);
    for (format, n) in s.by_format {
        println!("  {format:<6} {n}");
    }
    Ok(())
}
