use anyhow::Result;
use rusqlite::Connection;

/// Each entry upgrades the schema by one version. Append only; never edit a shipped migration.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE folders (
    id    INTEGER PRIMARY KEY,
    path  TEXT NOT NULL UNIQUE,
    kind  TEXT NOT NULL CHECK (kind IN ('root', 'watched')),
    added_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE files (
    id        INTEGER PRIMARY KEY,
    path      TEXT NOT NULL UNIQUE,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    size      INTEGER NOT NULL,
    mtime     INTEGER NOT NULL,
    hash      INTEGER,
    status    TEXT NOT NULL CHECK (status IN ('ok', 'unsupported', 'error')),
    error     TEXT
);
CREATE INDEX files_folder ON files(folder_id);
CREATE INDEX files_hash ON files(hash);

CREATE TABLE faces (
    id           INTEGER PRIMARY KEY,
    file_id      INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    hash         INTEGER NOT NULL,
    face_index   INTEGER NOT NULL,
    family       TEXT NOT NULL,
    family_raw   TEXT NOT NULL,
    subfamily    TEXT NOT NULL,
    full_name    TEXT NOT NULL,
    ps_name      TEXT NOT NULL,
    weight       INTEGER NOT NULL,
    width        INTEGER NOT NULL,
    italic       INTEGER NOT NULL,
    format       TEXT NOT NULL,
    glyph_count  INTEGER NOT NULL,
    units_per_em INTEGER NOT NULL,
    x_height     INTEGER,
    cap_height   INTEGER,
    axes         TEXT NOT NULL,
    features     TEXT NOT NULL,
    coverage     INTEGER NOT NULL,
    vendor       TEXT NOT NULL,
    license      TEXT NOT NULL,
    sample       TEXT NOT NULL,
    UNIQUE (file_id, face_index)
);
CREATE INDEX faces_key ON faces(hash, face_index);
CREATE INDEX faces_family ON faces(family COLLATE NOCASE);
CREATE INDEX faces_ps_name ON faces(ps_name);

-- Organisation refers to content (hash + face index), so it survives moves and renames.
CREATE TABLE collections (
    id        INTEGER PRIMARY KEY,
    parent_id INTEGER REFERENCES collections(id) ON DELETE CASCADE,
    name      TEXT NOT NULL,
    sort      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE collection_members (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    hash          INTEGER NOT NULL,
    face_index    INTEGER NOT NULL,
    PRIMARY KEY (collection_id, hash, face_index)
);

CREATE TABLE favourites (
    hash       INTEGER NOT NULL,
    face_index INTEGER NOT NULL,
    PRIMARY KEY (hash, face_index)
);

-- Activation journal, one row per font file (the OS activates files, so a .ttc activates all its faces).
-- Written before and after every OS change so reconcile can repair a crash.
CREATE TABLE activations (
    hash       INTEGER PRIMARY KEY,
    path       TEXT NOT NULL,
    os_key     TEXT NOT NULL,
    mode       TEXT NOT NULL CHECK (mode IN ('session', 'pinned')),
    refcount   INTEGER NOT NULL DEFAULT 1,
    state      TEXT NOT NULL CHECK (state IN ('activating', 'active', 'deactivating')),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#,
    r#"
-- Collection members known only by PostScript name (e.g. imported from FontBase but
-- never downloaded). `url` is where the font can be fetched, if known.
CREATE TABLE pending_members (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    ps_name       TEXT NOT NULL,
    url           TEXT,
    PRIMARY KEY (collection_id, ps_name)
);
"#,
];

pub fn migrate(conn: &Connection) -> Result<()> {
    let version: usize = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i + 1)?;
        tx.commit()?;
    }
    Ok(())
}
