//! Collections and favourites. Members are content keys (hash + face index), so
//! organisation survives moves, renames and re-imports.

use anyhow::Result;
use rusqlite::params;
use ts_model::{FaceKey, FileHash};

use crate::Store;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Collection {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub sort: i64,
}

impl Store {
    pub fn collections(&self) -> Result<Vec<Collection>> {
        let mut stmt =
            self.conn.prepare("SELECT id, parent_id, name, sort FROM collections ORDER BY parent_id, sort, name")?;
        let rows = stmt.query_map([], |r| {
            Ok(Collection { id: r.get(0)?, parent_id: r.get(1)?, name: r.get(2)?, sort: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn create_collection(&self, parent_id: Option<i64>, name: &str) -> Result<i64> {
        let sort: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(sort), -1) + 1 FROM collections WHERE parent_id IS ?1",
            [parent_id],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO collections (parent_id, name, sort) VALUES (?1, ?2, ?3)",
            params![parent_id, name, sort],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute("UPDATE collections SET name = ?2 WHERE id = ?1", params![id, name])?;
        Ok(())
    }

    pub fn move_collection(&self, id: i64, parent_id: Option<i64>, sort: i64) -> Result<()> {
        self.conn
            .execute("UPDATE collections SET parent_id = ?2, sort = ?3 WHERE id = ?1", params![id, parent_id, sort])?;
        Ok(())
    }

    /// Deletes a collection and its sub-collections. Fonts are untouched.
    pub fn delete_collection(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM collections WHERE id = ?1", [id])?;
        Ok(())
    }

    /// `(collection id, face)` for every membership.
    pub fn collection_members(&self) -> Result<Vec<(i64, FaceKey)>> {
        let mut stmt = self.conn.prepare("SELECT collection_id, hash, face_index FROM collection_members")?;
        let rows =
            stmt.query_map([], |r| Ok((r.get(0)?, FaceKey { hash: FileHash::from_sql(r.get(1)?), index: r.get(2)? })))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_to_collection(&mut self, id: i64, faces: &[FaceKey]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO collection_members (collection_id, hash, face_index) VALUES (?1, ?2, ?3)",
            )?;
            for f in faces {
                stmt.execute(params![id, f.hash.to_sql(), f.index])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_from_collection(&mut self, id: i64, faces: &[FaceKey]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "DELETE FROM collection_members WHERE collection_id = ?1 AND hash = ?2 AND face_index = ?3",
            )?;
            for f in faces {
                stmt.execute(params![id, f.hash.to_sql(), f.index])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn add_pending(&mut self, id: i64, members: &[(String, Option<String>)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO pending_members (collection_id, ps_name, url) VALUES (?1, ?2, ?3)
                 ON CONFLICT DO UPDATE SET url = COALESCE(excluded.url, url)",
            )?;
            for (ps, url) in members {
                stmt.execute(params![id, ps, url])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Distinct pending fonts with a download URL, as `(PostScript name, url)`.
    pub fn pending_downloads(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT ps_name, MIN(url) FROM pending_members WHERE url IS NOT NULL GROUP BY ps_name ORDER BY ps_name",
        )?;
        Ok(stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn pending_count(&self) -> Result<u64> {
        Ok(self.conn.query_row("SELECT COUNT(DISTINCT ps_name) FROM pending_members", [], |r| r.get(0))?)
    }

    /// Turns pending members whose font is now indexed into real members. Returns how many resolved.
    pub fn resolve_pending(&mut self) -> Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "CREATE TEMP TABLE resolved AS
             SELECT p.collection_id, p.ps_name, f.hash, f.face_index
             FROM pending_members p JOIN faces f ON f.ps_name = p.ps_name
             UNION
             -- Downloads are saved as <name>.ttf; the font inside may have been renamed since.
             SELECT p.collection_id, p.ps_name, f.hash, f.face_index
             FROM pending_members p JOIN files fl ON fl.path LIKE '%/' || p.ps_name || '.ttf'
                  OR fl.path LIKE '%\\' || p.ps_name || '.ttf'
             JOIN faces f ON f.file_id = fl.id",
            [],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO collection_members (collection_id, hash, face_index)
             SELECT collection_id, hash, face_index FROM resolved",
            [],
        )?;
        let n = tx.execute(
            "DELETE FROM pending_members WHERE (collection_id, ps_name) IN (SELECT collection_id, ps_name FROM resolved)",
            [],
        )?;
        tx.execute("DROP TABLE resolved", [])?;
        tx.commit()?;
        Ok(n)
    }

    pub fn favourites(&self) -> Result<Vec<FaceKey>> {
        let mut stmt = self.conn.prepare("SELECT hash, face_index FROM favourites")?;
        let rows = stmt.query_map([], |r| Ok(FaceKey { hash: FileHash::from_sql(r.get(0)?), index: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_favourite(&self, face: FaceKey, on: bool) -> Result<()> {
        let sql = if on {
            "INSERT OR IGNORE INTO favourites (hash, face_index) VALUES (?1, ?2)"
        } else {
            "DELETE FROM favourites WHERE hash = ?1 AND face_index = ?2"
        };
        self.conn.execute(sql, params![face.hash.to_sql(), face.index])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(h: u64) -> FaceKey {
        FaceKey { hash: FileHash(h), index: 0 }
    }

    #[test]
    fn nested_collections_and_members() {
        let mut store = Store::open_in_memory().unwrap();
        let serif = store.create_collection(None, "1. SERIF").unwrap();
        let slab = store.create_collection(Some(serif), "Slab").unwrap();
        let sans = store.create_collection(None, "2. SANS").unwrap();
        assert_eq!(store.collections().unwrap().iter().map(|c| c.sort).collect::<Vec<_>>(), [0, 1, 0]);

        store.add_to_collection(slab, &[key(1), key(2), key(1)]).unwrap();
        store.add_to_collection(sans, &[key(3)]).unwrap();
        store.remove_from_collection(slab, &[key(2)]).unwrap();
        assert_eq!(store.collection_members().unwrap().len(), 2);

        store.delete_collection(serif).unwrap();
        assert_eq!(store.collections().unwrap().len(), 1, "child deleted with parent");
        assert_eq!(store.collection_members().unwrap(), vec![(sans, key(3))]);
    }

    #[test]
    fn pending_members_resolve_once_indexed() {
        use crate::{FileRecord, FileStatus, FolderKind};
        let mut store = Store::open_in_memory().unwrap();
        let col = store.create_collection(None, "Google").unwrap();
        store
            .add_pending(col, &[("Joan-Regular".into(), Some("https://x/joan.ttf".into())), ("Gone".into(), None)])
            .unwrap();
        assert_eq!(store.pending_downloads().unwrap(), [("Joan-Regular".to_owned(), "https://x/joan.ttf".to_owned())]);
        assert_eq!(store.resolve_pending().unwrap(), 0);

        let folder = store.add_folder("/f", FolderKind::Root).unwrap();
        let mut face = crate::tests::face("Joan");
        face.ps_name = "Joan-Regular".into();
        let rec = FileRecord {
            path: "/f/joan.ttf".into(),
            size: 1,
            mtime: 0,
            hash: Some(FileHash(5)),
            status: FileStatus::Ok,
            faces: vec![face],
        };
        store.write_files(folder, &[rec]).unwrap();
        assert_eq!(store.resolve_pending().unwrap(), 1);
        assert_eq!(store.collection_members().unwrap(), vec![(col, key(5))]);
        assert_eq!(store.pending_count().unwrap(), 1, "the font without a URL stays pending");

        // A download saved as Renamed-Old.ttf whose font now has another PostScript name.
        store.add_pending(col, &[("Renamed-Old".into(), Some("https://x/r.ttf".into()))]).unwrap();
        let mut face = crate::tests::face("Renamed");
        face.ps_name = "Renamed-New".into();
        let rec = FileRecord {
            path: "/f/Providers/Google/Renamed-Old.ttf".into(),
            size: 1,
            mtime: 0,
            hash: Some(FileHash(6)),
            status: FileStatus::Ok,
            faces: vec![face],
        };
        store.write_files(folder, &[rec]).unwrap();
        assert_eq!(store.resolve_pending().unwrap(), 1);
    }

    #[test]
    fn favourites_toggle() {
        let store = Store::open_in_memory().unwrap();
        store.set_favourite(key(9), true).unwrap();
        store.set_favourite(key(9), true).unwrap();
        assert_eq!(store.favourites().unwrap(), vec![key(9)]);
        store.set_favourite(key(9), false).unwrap();
        assert!(store.favourites().unwrap().is_empty());
    }
}
