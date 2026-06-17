//! A [`Remote`] backed by the `rclone` binary, for syncing directly to any of
//! rclone's 40+ cloud backends (Google Drive, S3, Dropbox, B2, OneDrive, …)
//! without a locally-synced folder.
//!
//! The `base` is an rclone target like `gdrive:GameSync` or `s3:bucket/path`
//! (a bare local path also works — rclone treats it as the local backend, which
//! is how this transport is tested). The on-remote layout mirrors
//! [`super::folder::FolderRemote`].
//!
//! Requires `rclone` on `PATH` (override with `GAMESYNC_RCLONE`) and a remote
//! configured via `rclone config`.

use std::process::{Command, Output};

use super::{Lease, Remote};
use crate::error::{Error, Result};
use crate::model::{Head, Snapshot};
use crate::util::{new_id, now_ms};

const LOCK_STALE_MS: i64 = 120_000;

pub struct RcloneRemote {
    bin: String,
    base: String,
}

impl RcloneRemote {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            bin: std::env::var("GAMESYNC_RCLONE").unwrap_or_else(|_| "rclone".to_string()),
            base: base.into(),
        }
    }

    fn object_rel(hash: &str) -> String {
        let prefix = if hash.len() >= 2 { &hash[..2] } else { "00" };
        format!("objects/{prefix}/{hash}")
    }

    fn game_rel(game_id: &str, name: &str) -> String {
        format!("games/{}/{}", game_id.replace([':', '/'], "_"), name)
    }

    /// Full rclone path for a store-relative path.
    fn full(&self, rel: &str) -> String {
        format!("{}/{}", self.base.trim_end_matches('/'), rel)
    }

    fn run(&self, args: &[&str]) -> std::io::Result<Output> {
        Command::new(&self.bin).args(args).output()
    }

    /// Run rclone, returning stdout on success or an error with stderr.
    fn checked(&self, args: &[&str]) -> Result<Vec<u8>> {
        let out = self.run(args)?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(Error::other(format!(
                "rclone {} failed: {}",
                args.first().copied().unwrap_or(""),
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }

    fn cat(&self, rel: &str) -> Result<Vec<u8>> {
        self.checked(&["cat", &self.full(rel)])
    }

    /// Upload bytes to `rel` by staging a temp file and `rclone copyto`.
    fn put(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        let tmp = std::env::temp_dir().join(format!("gsr-{}", new_id()));
        std::fs::write(&tmp, bytes)?;
        let tmp_str = tmp.to_string_lossy().into_owned();
        let result = self.checked(&["copyto", &tmp_str, &self.full(rel)]);
        let _ = std::fs::remove_file(&tmp);
        result.map(|_| ())
    }

    /// Existence check that works across backends: list the parent dir filtered
    /// to the basename. A missing parent (non-zero exit) counts as "absent".
    fn exists(&self, rel: &str) -> Result<bool> {
        let (parent, name) = match rel.rsplit_once('/') {
            Some((p, n)) => (p.to_string(), n.to_string()),
            None => (String::new(), rel.to_string()),
        };
        let parent_full = self.full(&parent);
        let out = self.run(&["lsf", &parent_full, "--include", &name])?;
        Ok(out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty())
    }
}

impl Remote for RcloneRemote {
    fn lock(&self, game_id: &str) -> Result<Lease> {
        let rel = Self::game_rel(game_id, ".lock");
        if self.exists(&rel)? {
            if let Ok(bytes) = self.cat(&rel) {
                let text = String::from_utf8_lossy(&bytes);
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
            }
        }
        self.put(&rel, format!("{}:{}", new_id(), now_ms()).as_bytes())?;

        let bin = self.bin.clone();
        let full = self.full(&rel);
        Ok(Lease::new(move || {
            let _ = Command::new(&bin).args(["deletefile", &full]).output();
        }))
    }

    fn has_object(&self, hash: &str) -> Result<bool> {
        self.exists(&Self::object_rel(hash))
    }

    fn put_object(&self, hash: &str, bytes: &[u8]) -> Result<()> {
        self.put(&Self::object_rel(hash), bytes)
    }

    fn get_object(&self, hash: &str) -> Result<Vec<u8>> {
        self.cat(&Self::object_rel(hash))
    }

    fn get_version(&self, game_id: &str, version_id: &str) -> Result<Snapshot> {
        let bytes = self.cat(&Self::game_rel(
            game_id,
            &format!("versions/{version_id}.json"),
        ))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn put_version(&self, game_id: &str, snapshot: &Snapshot) -> Result<()> {
        let rel = Self::game_rel(game_id, &format!("versions/{}.json", snapshot.version_id));
        self.put(&rel, &serde_json::to_vec_pretty(snapshot)?)
    }

    fn get_head(&self, game_id: &str) -> Result<Option<Head>> {
        let rel = Self::game_rel(game_id, "HEAD.json");
        if !self.exists(&rel)? {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&self.cat(&rel)?)?))
    }

    fn set_head(&self, game_id: &str, head: &Head) -> Result<()> {
        self.put(
            &Self::game_rel(game_id, "HEAD.json"),
            &serde_json::to_vec_pretty(head)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_remote_paths() {
        let r = RcloneRemote::new("gdrive:GameSync");
        assert_eq!(RcloneRemote::object_rel("abcd1234"), "objects/ab/abcd1234");
        assert_eq!(
            r.full(&RcloneRemote::object_rel("abcd1234")),
            "gdrive:GameSync/objects/ab/abcd1234"
        );
        assert_eq!(
            r.full(&RcloneRemote::game_rel("steam:374320", "HEAD.json")),
            "gdrive:GameSync/games/steam_374320/HEAD.json"
        );
    }

    #[test]
    fn trailing_slash_in_base_is_normalized() {
        let r = RcloneRemote::new("/tmp/remote/");
        assert_eq!(r.full("objects/ab/x"), "/tmp/remote/objects/ab/x");
    }
}
