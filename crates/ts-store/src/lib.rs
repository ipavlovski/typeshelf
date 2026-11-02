//! SQLite persistence for the font index, organisation data and the activation journal.

mod journal;
mod org;
mod schema;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use ts_model::{Axis, Coverage, FaceInfo, FaceKey, FileHash, FontFormat};

pub use journal::{Activation, JournalState};
pub use org::Collection;

pub struct Store {
    conn: Connection,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FolderKind {
    /// Managed folder: imported fonts are copied here.
    Root,
    /// Indexed in place, never copied.
    Watched,
}

impl FolderKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Watched => "watched",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "root" { Self::Root } else { Self::Watched }
    }
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub id: i64,
    pub path: String,
    pub kind: FolderKind,
}

/// What the store already knows about a file, used to skip unchanged files.
#[derive(Clone, Copy, Debug)]
pub struct FileState {
    pub id: i64,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Clone, Debug)]
pub enum FileStatus {
    Ok,
    /// A font format the indexer cannot read yet (e.g. WOFF2).
    Unsupported(String),
    Error(String),
}

/// One indexed file, as produced by the indexer.
#[derive(Clone, Debug)]
pub struct FileRecord {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub hash: Option<FileHash>,
    pub status: FileStatus,
    pub faces: Vec<FaceInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub folders: u64,
    pub files: u64,
    pub faces: u64,
    pub families: u64,
    pub variable_faces: u64,
    pub renamed_families: u64,
    pub unsupported: u64,
    pub errors: u64,
    pub duplicate_files: u64,
    pub by_format: Vec<(String, u64)>,
}

/// A face row together with the file it lives in.
#[derive(Clone, Debug)]
pub struct FaceRow {
    pub key: FaceKey,
    pub path: String,
    pub folder_id: i64,
    pub info: FaceInfo,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn add_folder(&self, path: &str, kind: FolderKind) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO folders (path, kind) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET kind = excluded.kind",
            params![path, kind.as_str()],
        )?;
        Ok(self.conn.query_row("SELECT id FROM folders WHERE path = ?1", [path], |r| r.get(0))?)
    }

    pub fn remove_folder(&self, path: &str) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM folders WHERE path = ?1", [path])? > 0)
    }

    pub fn folders(&self) -> Result<Vec<Folder>> {
        let mut stmt = self.conn.prepare("SELECT id, path, kind FROM folders ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            Ok(Folder { id: r.get(0)?, path: r.get(1)?, kind: FolderKind::parse(&r.get::<_, String>(2)?) })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn file_states(&self, folder_id: i64) -> Result<HashMap<String, FileState>> {
        let mut stmt = self.conn.prepare("SELECT id, path, size, mtime FROM files WHERE folder_id = ?1")?;
        let rows = stmt.query_map([folder_id], |r| {
            Ok((r.get::<_, String>(1)?, FileState { id: r.get(0)?, size: r.get(2)?, mtime: r.get(3)? }))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Inserts or replaces files and their faces in one transaction.
    pub fn write_files(&mut self, folder_id: i64, records: &[FileRecord]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upsert = tx.prepare_cached(
                "INSERT INTO files (path, folder_id, size, mtime, hash, status, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(path) DO UPDATE SET folder_id = excluded.folder_id, size = excluded.size,
                   mtime = excluded.mtime, hash = excluded.hash, status = excluded.status, error = excluded.error
                 RETURNING id",
            )?;
            let mut clear = tx.prepare_cached("DELETE FROM faces WHERE file_id = ?1")?;
            let mut insert = tx.prepare_cached(
                "INSERT INTO faces (file_id, hash, face_index, family, family_raw, subfamily, full_name, ps_name,
                   weight, width, italic, format, glyph_count, units_per_em, x_height, cap_height, axes,
                   features, coverage, vendor, license, sample)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                   ?20, ?21, ?22)",
            )?;
            for rec in records {
                let (status, error) = match &rec.status {
                    FileStatus::Ok => ("ok", None),
                    FileStatus::Unsupported(e) => ("unsupported", Some(e.as_str())),
                    FileStatus::Error(e) => ("error", Some(e.as_str())),
                };
                let file_id: i64 = upsert.query_row(
                    params![rec.path, folder_id, rec.size, rec.mtime, rec.hash.map(FileHash::to_sql), status, error],
                    |r| r.get(0),
                )?;
                clear.execute([file_id])?;
                let Some(hash) = rec.hash else { continue };
                for f in &rec.faces {
                    insert.execute(params![
                        file_id,
                        hash.to_sql(),
                        f.index,
                        f.family,
                        f.family_raw,
                        f.subfamily,
                        f.full_name,
                        f.ps_name,
                        f.weight,
                        f.width,
                        f.italic,
                        f.format.as_str(),
                        f.glyph_count,
                        f.units_per_em,
                        f.x_height,
                        f.cap_height,
                        encode_axes(&f.axes),
                        f.features.join(" "),
                        f.coverage.0 as i64,
                        f.vendor,
                        f.license,
                        f.sample,
                    ])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_files(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached("DELETE FROM files WHERE id = ?1")?;
            for id in ids {
                stmt.execute([id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn stats(&self) -> Result<Stats> {
        let one = |sql: &str| -> Result<u64> { Ok(self.conn.query_row(sql, [], |r| r.get(0))?) };
        let mut stmt = self.conn.prepare("SELECT format, COUNT(*) FROM faces GROUP BY format ORDER BY 2 DESC")?;
        let by_format = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(Stats {
            folders: one("SELECT COUNT(*) FROM folders")?,
            files: one("SELECT COUNT(*) FROM files")?,
            faces: one("SELECT COUNT(*) FROM faces")?,
            families: one("SELECT COUNT(DISTINCT family) FROM faces")?,
            variable_faces: one("SELECT COUNT(*) FROM faces WHERE axes != ''")?,
            renamed_families: one("SELECT COUNT(*) FROM faces WHERE family != family_raw")?,
            unsupported: one("SELECT COUNT(*) FROM files WHERE status = 'unsupported'")?,
            errors: one("SELECT COUNT(*) FROM files WHERE status = 'error'")?,
            duplicate_files: one("SELECT COALESCE(SUM(n - 1), 0) FROM
                   (SELECT COUNT(*) AS n FROM files WHERE hash IS NOT NULL GROUP BY hash HAVING n > 1)")?,
            by_format,
        })
    }

    /// Groups of paths whose files have identical content.
    pub fn duplicates(&self) -> Result<Vec<(FileHash, Vec<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, path FROM files WHERE hash IN
               (SELECT hash FROM files WHERE hash IS NOT NULL GROUP BY hash HAVING COUNT(*) > 1)
             ORDER BY hash, path",
        )?;
        let mut out: Vec<(FileHash, Vec<String>)> = Vec::new();
        for row in stmt.query_map([], |r| Ok((FileHash::from_sql(r.get(0)?), r.get::<_, String>(1)?)))? {
            let (hash, path) = row?;
            match out.last_mut() {
                Some((h, paths)) if *h == hash => paths.push(path),
                _ => out.push((hash, vec![path])),
            }
        }
        Ok(out)
    }

    pub fn errors(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, status, COALESCE(error, '') FROM files WHERE status != 'ok' ORDER BY status, path",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every face with its file, for building the in-memory search snapshot.
    pub fn faces(&self) -> Result<Vec<FaceRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.hash, f.face_index, files.path, files.folder_id, f.family, f.family_raw, f.subfamily,
               f.full_name, f.ps_name, f.weight, f.width, f.italic, f.format, f.glyph_count, f.units_per_em,
               f.x_height, f.cap_height, f.axes, f.features, f.coverage, f.vendor, f.license, f.sample
             FROM faces f JOIN files ON files.id = f.file_id
             ORDER BY f.family COLLATE NOCASE, f.weight, f.italic, f.subfamily",
        )?;
        let rows = stmt.query_map([], |r| {
            let index: u32 = r.get(1)?;
            let features: String = r.get(18)?;
            Ok(FaceRow {
                key: FaceKey { hash: FileHash::from_sql(r.get(0)?), index },
                path: r.get(2)?,
                folder_id: r.get(3)?,
                info: FaceInfo {
                    index,
                    family: r.get(4)?,
                    family_raw: r.get(5)?,
                    subfamily: r.get(6)?,
                    full_name: r.get(7)?,
                    ps_name: r.get(8)?,
                    weight: r.get(9)?,
                    width: r.get(10)?,
                    italic: r.get(11)?,
                    format: FontFormat::parse(&r.get::<_, String>(12)?).unwrap_or(FontFormat::Ttf),
                    glyph_count: r.get(13)?,
                    units_per_em: r.get(14)?,
                    x_height: r.get(15)?,
                    cap_height: r.get(16)?,
                    axes: decode_axes(&r.get::<_, String>(17)?),
                    features: features.split_ascii_whitespace().map(str::to_owned).collect(),
                    coverage: Coverage(r.get::<_, i64>(19)? as u64),
                    vendor: r.get(20)?,
                    license: r.get(21)?,
                    sample: r.get(22)?,
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

/// `wght:100:400:900 wdth:75:100:125`
fn encode_axes(axes: &[Axis]) -> String {
    axes.iter().map(|a| format!("{}:{}:{}:{}", a.tag, a.min, a.default, a.max)).collect::<Vec<_>>().join(" ")
}

fn decode_axes(s: &str) -> Vec<Axis> {
    s.split_ascii_whitespace()
        .filter_map(|part| {
            let mut it = part.split(':');
            Some(Axis {
                tag: it.next()?.to_owned(),
                min: it.next()?.parse().ok()?,
                default: it.next()?.parse().ok()?,
                max: it.next()?.parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn face(family: &str) -> FaceInfo {
        FaceInfo {
            index: 0,
            family: family.into(),
            family_raw: family.into(),
            subfamily: "Regular".into(),
            full_name: format!("{family} Regular"),
            ps_name: format!("{family}-Regular"),
            weight: 400,
            width: 5,
            italic: false,
            format: FontFormat::Ttf,
            glyph_count: 10,
            units_per_em: 1000,
            x_height: Some(500),
            cap_height: Some(700),
            axes: vec![Axis { tag: "wght".into(), min: 100.0, default: 400.0, max: 900.0 }],
            features: vec!["kern".into(), "liga".into()],
            coverage: Coverage(0b101),
            vendor: "TSHF".into(),
            license: String::new(),
            sample: "Abc".into(),
        }
    }

    fn record(path: &str, hash: u64, family: &str) -> FileRecord {
        FileRecord {
            path: path.into(),
            size: 100,
            mtime: 1,
            hash: Some(FileHash(hash)),
            status: FileStatus::Ok,
            faces: vec![face(family)],
        }
    }

    #[test]
    fn round_trips_faces() {
        let mut store = Store::open_in_memory().unwrap();
        let folder = store.add_folder("/fonts", FolderKind::Watched).unwrap();
        store.write_files(folder, &[record("/fonts/a.ttf", u64::MAX, "Alpha")]).unwrap();
        let faces = store.faces().unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].key.hash, FileHash(u64::MAX));
        assert_eq!(faces[0].info, face("Alpha"));
    }

    #[test]
    fn rewriting_a_file_replaces_its_faces() {
        let mut store = Store::open_in_memory().unwrap();
        let folder = store.add_folder("/fonts", FolderKind::Watched).unwrap();
        store.write_files(folder, &[record("/fonts/a.ttf", 1, "Alpha")]).unwrap();
        store.write_files(folder, &[record("/fonts/a.ttf", 2, "Beta")]).unwrap();
        let faces = store.faces().unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].info.family, "Beta");
    }

    #[test]
    fn finds_duplicates_and_counts_them() {
        let mut store = Store::open_in_memory().unwrap();
        let folder = store.add_folder("/fonts", FolderKind::Watched).unwrap();
        store
            .write_files(
                folder,
                &[record("/fonts/a.ttf", 7, "A"), record("/fonts/b.ttf", 7, "A"), record("/fonts/c.ttf", 8, "C")],
            )
            .unwrap();
        let dupes = store.duplicates().unwrap();
        assert_eq!(dupes, vec![(FileHash(7), vec!["/fonts/a.ttf".to_owned(), "/fonts/b.ttf".to_owned()])]);
        let stats = store.stats().unwrap();
        assert_eq!((stats.files, stats.faces, stats.families, stats.duplicate_files), (3, 3, 2, 1));
    }

    #[test]
    fn removing_a_folder_cascades() {
        let mut store = Store::open_in_memory().unwrap();
        let folder = store.add_folder("/fonts", FolderKind::Watched).unwrap();
        store.write_files(folder, &[record("/fonts/a.ttf", 1, "A")]).unwrap();
        assert!(store.remove_folder("/fonts").unwrap());
        assert_eq!(store.stats().unwrap().faces, 0);
    }
}
