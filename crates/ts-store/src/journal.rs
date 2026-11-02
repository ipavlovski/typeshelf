//! Activation journal queries. One row per activated font file.

use anyhow::Result;
use rusqlite::{OptionalExtension, Row, params};
use ts_model::{ActivationMode, FileHash};

use crate::Store;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JournalState {
    /// Written before the OS change. Seen at reconcile = the change may be half-done.
    Activating,
    Active,
    /// Written before the OS removal. Seen at reconcile = finish the removal.
    Deactivating,
}

impl JournalState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Activating => "activating",
            Self::Active => "active",
            Self::Deactivating => "deactivating",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "activating" => Self::Activating,
            "deactivating" => Self::Deactivating,
            _ => Self::Active,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Activation {
    pub hash: FileHash,
    pub path: String,
    /// Backend-specific key, e.g. the registry value name on Windows.
    pub os_key: String,
    pub mode: ActivationMode,
    /// Number of holders (direct activation, collections). The OS entry goes at zero.
    pub refcount: u32,
    pub state: JournalState,
}

fn row(r: &Row) -> rusqlite::Result<Activation> {
    Ok(Activation {
        hash: FileHash::from_sql(r.get(0)?),
        path: r.get(1)?,
        os_key: r.get(2)?,
        mode: ActivationMode::parse(&r.get::<_, String>(3)?).unwrap_or(ActivationMode::Session),
        refcount: r.get(4)?,
        state: JournalState::parse(&r.get::<_, String>(5)?),
    })
}

const COLUMNS: &str = "hash, path, os_key, mode, refcount, state";

impl Store {
    pub fn activations(&self) -> Result<Vec<Activation>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {COLUMNS} FROM activations ORDER BY path"))?;
        Ok(stmt.query_map([], row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn activation(&self, hash: FileHash) -> Result<Option<Activation>> {
        Ok(self
            .conn
            .query_row(&format!("SELECT {COLUMNS} FROM activations WHERE hash = ?1"), [hash.to_sql()], row)
            .optional()?)
    }

    /// Writes journal rows in one transaction.
    pub fn put_activations(&mut self, rows: &[Activation]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO activations (hash, path, os_key, mode, refcount, state, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch())
                 ON CONFLICT(hash) DO UPDATE SET path = excluded.path, os_key = excluded.os_key,
                   mode = excluded.mode, refcount = excluded.refcount, state = excluded.state,
                   updated_at = excluded.updated_at",
            )?;
            for a in rows {
                stmt.execute(params![
                    a.hash.to_sql(),
                    a.path,
                    a.os_key,
                    a.mode.as_str(),
                    a.refcount,
                    a.state.as_str()
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_activations(&mut self, hashes: &[FileHash]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached("DELETE FROM activations WHERE hash = ?1")?;
            for h in hashes {
                stmt.execute([h.to_sql()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Current indexed paths of a file's content, for repairing activations after a move.
    pub fn paths_for_hash(&self, hash: FileHash) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files WHERE hash = ?1 AND status = 'ok' ORDER BY path")?;
        Ok(stmt.query_map([hash.to_sql()], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_round_trip() {
        let mut store = Store::open_in_memory().unwrap();
        let a = Activation {
            hash: FileHash(u64::MAX - 3),
            path: "/f/a.ttf".into(),
            os_key: "A (TrueType)".into(),
            mode: ActivationMode::Pinned,
            refcount: 2,
            state: JournalState::Activating,
        };
        store.put_activations(std::slice::from_ref(&a)).unwrap();
        assert_eq!(store.activation(a.hash).unwrap(), Some(a.clone()));
        let b = Activation { state: JournalState::Active, ..a.clone() };
        store.put_activations(std::slice::from_ref(&b)).unwrap();
        assert_eq!(store.activations().unwrap(), vec![b]);
        store.delete_activations(&[a.hash]).unwrap();
        assert!(store.activations().unwrap().is_empty());
    }
}
