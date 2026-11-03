//! Hierarchical tags (`serif/slab/egyptian`). Like collections, they are stored per face by
//! content key, so they survive moves and re-imports. A parent tag exists implicitly once any
//! of its sub-tags does.

use anyhow::{Result, bail};
use rusqlite::{Transaction, params};
use ts_model::{FaceKey, FileHash, normalize_tag};

use crate::Store;

/// Matches `tag` and every tag below it (case-insensitive).
const SUBTREE: &str = "(tag = ?1 OR lower(substr(tag, 1, length(?1) + 1)) = lower(?1 || '/'))";

fn canonical(tag: &str) -> Result<String> {
    match normalize_tag(tag) {
        Some(t) => Ok(t),
        None => bail!("empty tag"),
    }
}

impl Store {
    /// `(tag, face)` for every tagging.
    pub fn face_tags(&self) -> Result<Vec<(String, FaceKey)>> {
        let mut stmt = self.conn.prepare("SELECT tag, hash, face_index FROM face_tags ORDER BY tag")?;
        let rows =
            stmt.query_map([], |r| Ok((r.get(0)?, FaceKey { hash: FileHash::from_sql(r.get(1)?), index: r.get(2)? })))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Tags faces. A tag already in use with other capitalisation keeps its spelling.
    /// Returns the tag as stored.
    pub fn add_tag(&mut self, tag: &str, faces: &[FaceKey]) -> Result<String> {
        let tag = canonical(tag)?;
        let tx = self.conn.transaction()?;
        let tag = existing_spelling(&tx, &tag)?.unwrap_or(tag);
        {
            let mut stmt =
                tx.prepare_cached("INSERT OR IGNORE INTO face_tags (tag, hash, face_index) VALUES (?1, ?2, ?3)")?;
            for f in faces {
                stmt.execute(params![tag, f.hash.to_sql(), f.index])?;
            }
        }
        tx.commit()?;
        Ok(tag)
    }

    /// Removes exactly this tag (not its sub-tags) from faces.
    pub fn remove_tag(&mut self, tag: &str, faces: &[FaceKey]) -> Result<()> {
        let tag = canonical(tag)?;
        let tx = self.conn.transaction()?;
        {
            let mut stmt =
                tx.prepare_cached("DELETE FROM face_tags WHERE tag = ?1 AND hash = ?2 AND face_index = ?3")?;
            for f in faces {
                stmt.execute(params![tag, f.hash.to_sql(), f.index])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Renames a tag and everything below it: `serif` → `classic` turns `serif/slab` into
    /// `classic/slab`. Renaming onto an existing tag merges them.
    pub fn rename_tag(&mut self, from: &str, to: &str) -> Result<()> {
        let (from, to) = (canonical(from)?, canonical(to)?);
        let tx = self.conn.transaction()?;
        let rows: Vec<(String, i64, u32)> = {
            let mut stmt = tx.prepare(&format!("SELECT tag, hash, face_index FROM face_tags WHERE {SUBTREE}"))?;
            stmt.query_map([&from], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?
        };
        tx.execute(&format!("DELETE FROM face_tags WHERE {SUBTREE}"), [&from])?;
        {
            let mut stmt =
                tx.prepare_cached("INSERT OR IGNORE INTO face_tags (tag, hash, face_index) VALUES (?1, ?2, ?3)")?;
            for (tag, hash, index) in rows {
                let renamed = format!("{to}{}", &tag[from.len()..]);
                let renamed = existing_spelling(&tx, &renamed)?.unwrap_or(renamed);
                stmt.execute(params![renamed, hash, index])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes a tag and its sub-tags from every face. Fonts are untouched.
    pub fn delete_tag(&self, tag: &str) -> Result<usize> {
        let tag = canonical(tag)?;
        Ok(self.conn.execute(&format!("DELETE FROM face_tags WHERE {SUBTREE}"), [&tag])?)
    }
}

fn existing_spelling(tx: &Transaction, tag: &str) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(tx.query_row("SELECT tag FROM face_tags WHERE tag = ?1 LIMIT 1", [tag], |r| r.get(0)).optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(h: u64) -> FaceKey {
        FaceKey { hash: FileHash(h), index: 0 }
    }

    fn tags(store: &Store) -> Vec<(String, u64)> {
        store.face_tags().unwrap().into_iter().map(|(t, k)| (t, k.hash.0)).collect()
    }

    #[test]
    fn add_remove_and_spelling() {
        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(store.add_tag("#Serif / Slab", &[key(1), key(2)]).unwrap(), "Serif/Slab");
        assert_eq!(store.add_tag("serif/slab", &[key(3), key(1)]).unwrap(), "Serif/Slab", "keeps first spelling");
        assert_eq!(tags(&store).len(), 3);
        store.remove_tag("SERIF/SLAB", &[key(2)]).unwrap();
        assert_eq!(tags(&store), [("Serif/Slab".to_owned(), 1), ("Serif/Slab".to_owned(), 3)]);
        assert!(store.add_tag(" / ", &[key(1)]).is_err());
    }

    #[test]
    fn rename_and_delete_subtrees() {
        let mut store = Store::open_in_memory().unwrap();
        store.add_tag("serif", &[key(1)]).unwrap();
        store.add_tag("serif/slab", &[key(2)]).unwrap();
        store.add_tag("serifish", &[key(3)]).unwrap();
        store.add_tag("classic/slab", &[key(2)]).unwrap();

        store.rename_tag("Serif", "classic").unwrap();
        let mut got = tags(&store);
        got.sort();
        assert_eq!(
            got,
            [("classic".to_owned(), 1), ("classic/slab".to_owned(), 2), ("serifish".to_owned(), 3)],
            "subtree renamed, merged with the existing tag, prefix-alike tag untouched"
        );

        assert_eq!(store.delete_tag("classic").unwrap(), 2);
        assert_eq!(tags(&store), [("serifish".to_owned(), 3)]);
    }
}
