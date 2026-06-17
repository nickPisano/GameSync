//! A [`Remote`] backed by a plain directory.
//!
//! Layout (mirrors the local store so objects dedup across devices too):
//! ```text
//! <root>/objects/<ab>/<hash>
//! <root>/games/<game_id>/versions/<version_id>.json
//! <root>/games/<game_id>/HEAD.json
//! <root>/games/<game_id>/.lock
//! ```

use std::fs;
use std::path::PathBuf;

use super::{Lease, Remote};
use crate::error::{Error, Result};
use crate::model::{Head, Snapshot};
use crate::util::{atomic_write, new_id, now_ms};

/// A lock is considered stale (and may be stolen) after this long, so a crashed
/// sync can't wedge a game forever.
const LOCK_STALE_MS: i64 = 120_000;

pub struct FolderRemote {
    root: PathBuf,
}

impl FolderRemote {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("games"))?;
        Ok(Self { root })
    }

    fn object_path(&self, hash: &str) -> PathBuf {
        let prefix = if hash.len() >= 2 { &hash[..2] } else { "00" };
        self.root.join("objects").join(prefix).join(hash)
    }

    fn game_dir(&self, game_id: &str) -> PathBuf {
        // game ids contain ':' (e.g. "steam:374320"); make them path-safe.
        self.root
            .join("games")
            .join(game_id.replace([':', '/'], "_"))
    }
}

impl Remote for FolderRemote {
    fn lock(&self, game_id: &str) -> Result<Lease> {
        let dir = self.game_dir(game_id);
        fs::create_dir_all(&dir)?;
        let lock_path = dir.join(".lock");

        if let Ok(text) = fs::read_to_string(&lock_path) {
            // "<owner_id>:<acquired_ms>"
            let acquired: i64 = text
                .split(':')
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if now_ms() - acquired < LOCK_STALE_MS {
                return Err(Error::other(
                    "remote is locked by another sync in progress; try again shortly",
                ));
            }
            // stale — fall through and steal it
        }
        atomic_write(&lock_path, format!("{}:{}", new_id(), now_ms()).as_bytes())?;
        let cleanup_path = lock_path.clone();
        Ok(Lease::new(move || {
            let _ = fs::remove_file(&cleanup_path);
        }))
    }

    fn has_object(&self, hash: &str) -> Result<bool> {
        Ok(self.object_path(hash).is_file())
    }

    fn put_object(&self, hash: &str, bytes: &[u8]) -> Result<()> {
        let dest = self.object_path(hash);
        if dest.is_file() {
            return Ok(());
        }
        atomic_write(&dest, bytes)?;
        Ok(())
    }

    fn get_object(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.object_path(hash);
        if !path.is_file() {
            return Err(Error::Integrity(format!("remote missing object {hash}")));
        }
        Ok(fs::read(path)?)
    }

    fn get_version(&self, game_id: &str, version_id: &str) -> Result<Snapshot> {
        let path = self
            .game_dir(game_id)
            .join("versions")
            .join(format!("{version_id}.json"));
        if !path.is_file() {
            return Err(Error::NotFound(format!("remote version {version_id}")));
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    fn put_version(&self, game_id: &str, snapshot: &Snapshot) -> Result<()> {
        let path = self
            .game_dir(game_id)
            .join("versions")
            .join(format!("{}.json", snapshot.version_id));
        atomic_write(&path, &serde_json::to_vec_pretty(snapshot)?)?;
        Ok(())
    }

    fn get_head(&self, game_id: &str) -> Result<Option<Head>> {
        let path = self.game_dir(game_id).join("HEAD.json");
        if !path.is_file() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
    }

    fn set_head(&self, game_id: &str, head: &Head) -> Result<()> {
        let path = self.game_dir(game_id).join("HEAD.json");
        atomic_write(&path, &serde_json::to_vec_pretty(head)?)?;
        Ok(())
    }
}
