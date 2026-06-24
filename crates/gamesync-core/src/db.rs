//! SQLite metadata store: games, snapshot versions, and engine metadata.
//!
//! The DB is the source of truth for *what* exists; the CAS holds the bytes.
//! Snapshot file manifests are stored as JSON in the `versions` row, which is
//! plenty for the save sizes we deal with and keeps Phase 1 simple.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Error, Result};
use crate::model::{FileEntry, Game, Platform, Snapshot, SnapshotKind};
use crate::util::new_id;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS games (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    platform        TEXT NOT NULL,
    save_root       TEXT NOT NULL,
    install_dir     TEXT,
    includes        TEXT NOT NULL,
    excludes        TEXT NOT NULL,
    sync_enabled    INTEGER NOT NULL DEFAULT 0,
    head_version_id TEXT,
    extra_roots     TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS versions (
    version_id  TEXT PRIMARY KEY,
    game_id     TEXT NOT NULL,
    device_id   TEXT NOT NULL,
    created_ms  INTEGER NOT NULL,
    label       TEXT,
    kind        TEXT NOT NULL,
    parent      TEXT,
    vclock      TEXT NOT NULL DEFAULT '{}',
    total_size  INTEGER NOT NULL,
    manifest    TEXT NOT NULL,
    FOREIGN KEY (game_id) REFERENCES games(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_versions_game
    ON versions (game_id, created_ms DESC);
"#;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        // WAL gives better crash resistance and concurrent reads.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let db = Self { conn };
        db.conn.execute_batch(SCHEMA)?;
        db.migrate();
        Ok(db)
    }

    /// Best-effort additive migrations for stores created by older schemas.
    /// `ADD COLUMN` errors if the column already exists (fresh DBs), which we
    /// intentionally ignore.
    fn migrate(&self) {
        let _ = self
            .conn
            .execute("ALTER TABLE games ADD COLUMN head_version_id TEXT", []);
        let _ = self.conn.execute(
            "ALTER TABLE versions ADD COLUMN vclock TEXT NOT NULL DEFAULT '{}'",
            [],
        );
        let _ = self.conn.execute(
            "ALTER TABLE games ADD COLUMN extra_roots TEXT NOT NULL DEFAULT '[]'",
            [],
        );
    }

    /// The version id the live save folder currently corresponds to.
    pub fn get_head(&self, game_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT head_version_id FROM games WHERE id = ?1",
                params![game_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    pub fn set_head(&self, game_id: &str, version_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE games SET head_version_id = ?2 WHERE id = ?1",
            params![game_id, version_id],
        )?;
        Ok(())
    }

    /// Stable per-install device id, created on first use.
    pub fn device_id(&self) -> Result<String> {
        if let Some(v) = self.get_meta("device_id")? {
            return Ok(v);
        }
        let id = new_id();
        self.set_meta("device_id", &id)?;
        Ok(id)
    }

    /// Persist a configuration value (e.g. the remote path).
    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        self.set_meta(key, value)
    }

    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        self.get_meta(key)
    }

    fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let v = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        Ok(v)
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- games -----------------------------------------------------------

    /// Insert or update a game. Crucially, this does NOT clobber the user's
    /// `sync_enabled` choice on re-scan — only the discovered fields update.
    pub fn upsert_game(&self, g: &Game) -> Result<()> {
        let includes = serde_json::to_string(&g.includes)?;
        let excludes = serde_json::to_string(&g.excludes)?;
        let extra_roots = serde_json::to_string(&g.extra_roots)?;
        let install = g
            .install_dir
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        self.conn.execute(
            "INSERT INTO games (id, name, platform, save_root, install_dir, includes, excludes, sync_enabled, extra_roots)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                name        = excluded.name,
                platform    = excluded.platform,
                save_root   = excluded.save_root,
                install_dir = excluded.install_dir,
                includes    = excluded.includes,
                excludes    = excluded.excludes",
            params![
                g.id,
                g.name,
                g.platform.as_str(),
                g.save_root.to_string_lossy(),
                install,
                includes,
                excludes,
                g.sync_enabled as i64,
                extra_roots,
            ],
        )?;
        Ok(())
    }

    pub fn set_sync_enabled(&self, game_id: &str, enabled: bool) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE games SET sync_enabled = ?2 WHERE id = ?1",
            params![game_id, enabled as i64],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("game {game_id}")));
        }
        Ok(())
    }

    /// Replace a game's extra backup roots (preserved across re-scan, like
    /// `sync_enabled`, so it lives outside the upsert's conflict update).
    pub fn set_extra_roots(&self, game_id: &str, roots: &[std::path::PathBuf]) -> Result<()> {
        let json = serde_json::to_string(roots)?;
        let n = self.conn.execute(
            "UPDATE games SET extra_roots = ?2 WHERE id = ?1",
            params![game_id, json],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("game {game_id}")));
        }
        Ok(())
    }

    /// Delete a game and (via the cascade) all its versions. Does not touch the
    /// live save folder — only the tracking + backup history.
    pub fn delete_game(&self, id: &str) -> Result<()> {
        let n = self
            .conn
            .execute("DELETE FROM games WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(Error::NotFound(format!("game {id}")));
        }
        Ok(())
    }

    pub fn get_game(&self, id: &str) -> Result<Game> {
        self.conn
            .query_row(
                "SELECT id, name, platform, save_root, install_dir, includes, excludes, sync_enabled, extra_roots
                 FROM games WHERE id = ?1",
                params![id],
                Self::row_to_game,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("game {id}")))
    }

    pub fn list_games(&self) -> Result<Vec<Game>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, platform, save_root, install_dir, includes, excludes, sync_enabled, extra_roots
             FROM games ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], Self::row_to_game)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn row_to_game(row: &rusqlite::Row) -> rusqlite::Result<Game> {
        let includes: String = row.get(5)?;
        let excludes: String = row.get(6)?;
        let install: Option<String> = row.get(4)?;
        let save_root: String = row.get(3)?;
        let extra_roots: String = row.get(8).unwrap_or_else(|_| "[]".to_string());
        Ok(Game {
            id: row.get(0)?,
            name: row.get(1)?,
            platform: Platform::from_str(&row.get::<_, String>(2)?),
            save_root: save_root.into(),
            install_dir: install.map(Into::into),
            includes: serde_json::from_str(&includes).unwrap_or_default(),
            excludes: serde_json::from_str(&excludes).unwrap_or_default(),
            sync_enabled: row.get::<_, i64>(7)? != 0,
            extra_roots: serde_json::from_str(&extra_roots).unwrap_or_default(),
        })
    }

    // ---- versions --------------------------------------------------------

    pub fn insert_version(&self, s: &Snapshot) -> Result<()> {
        let manifest = serde_json::to_string(&s.files)?;
        let vclock = serde_json::to_string(&s.vclock)?;
        self.conn.execute(
            "INSERT INTO versions
                (version_id, game_id, device_id, created_ms, label, kind, parent, vclock, total_size, manifest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                s.version_id,
                s.game_id,
                s.device_id,
                s.created_ms,
                s.label,
                s.kind.as_str(),
                s.parent,
                vclock,
                s.total_size as i64,
                manifest,
            ],
        )?;
        Ok(())
    }

    /// Insert a version only if it isn't already present (used when pulling a
    /// peer's version into local history).
    pub fn insert_version_if_absent(&self, s: &Snapshot) -> Result<()> {
        if self.get_version(&s.version_id).is_ok() {
            return Ok(());
        }
        self.insert_version(s)
    }

    pub fn get_version(&self, version_id: &str) -> Result<Snapshot> {
        self.conn
            .query_row(
                "SELECT version_id, game_id, device_id, created_ms, label, kind, parent, vclock, total_size, manifest
                 FROM versions WHERE version_id = ?1",
                params![version_id],
                Self::row_to_snapshot,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("version {version_id}")))
    }

    /// All versions for a game, newest first.
    pub fn list_versions(&self, game_id: &str) -> Result<Vec<Snapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT version_id, game_id, device_id, created_ms, label, kind, parent, vclock, total_size, manifest
             FROM versions WHERE game_id = ?1 ORDER BY created_ms DESC",
        )?;
        let rows = stmt.query_map(params![game_id], Self::row_to_snapshot)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_version(&self, version_id: &str) -> Result<()> {
        let n = self.conn.execute(
            "DELETE FROM versions WHERE version_id = ?1",
            params![version_id],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("version {version_id}")));
        }
        Ok(())
    }

    /// Every content hash referenced by any surviving version, across all games.
    /// This is the "live set" the garbage collector keeps.
    pub fn all_referenced_hashes(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT manifest FROM versions")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut set = std::collections::HashSet::new();
        for manifest in rows {
            let files: Vec<FileEntry> = serde_json::from_str(&manifest?).unwrap_or_default();
            for f in files {
                set.insert(f.hash);
            }
        }
        Ok(set)
    }

    pub fn latest_version(&self, game_id: &str) -> Result<Option<Snapshot>> {
        self.conn
            .query_row(
                "SELECT version_id, game_id, device_id, created_ms, label, kind, parent, vclock, total_size, manifest
                 FROM versions WHERE game_id = ?1 ORDER BY created_ms DESC LIMIT 1",
                params![game_id],
                Self::row_to_snapshot,
            )
            .optional()
            .map_err(Into::into)
    }

    fn row_to_snapshot(row: &rusqlite::Row) -> rusqlite::Result<Snapshot> {
        let vclock: String = row.get(7)?;
        let manifest: String = row.get(9)?;
        let files: Vec<FileEntry> = serde_json::from_str(&manifest).unwrap_or_default();
        Ok(Snapshot {
            version_id: row.get(0)?,
            game_id: row.get(1)?,
            device_id: row.get(2)?,
            created_ms: row.get(3)?,
            label: row.get(4)?,
            kind: SnapshotKind::from_str(&row.get::<_, String>(5)?),
            parent: row.get(6)?,
            vclock: serde_json::from_str(&vclock).unwrap_or_default(),
            total_size: row.get::<_, i64>(8)? as u64,
            files,
        })
    }
}
