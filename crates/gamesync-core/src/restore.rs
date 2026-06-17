//! Restore: materialize a stored version back into the live save folder.
//!
//! Safety properties enforced here:
//!  1. A pre-restore safety snapshot of the *current* state is taken first
//!     (unless disabled), so any restore is itself undoable.
//!  2. Files are materialized into a staging directory and verified by hash
//!     before anything live is touched.
//!  3. The swap uses renames within the same filesystem so a crash leaves
//!     either the old folder or the new one — never a half-written mix. On
//!     failure we roll the previous folder back.

use std::fs;
use std::path::PathBuf;

use crate::cas::Cas;
use crate::db::Db;
use crate::error::{Error, Result};
use crate::model::{Game, Snapshot, SnapshotKind};
use crate::snapshot::{create_snapshot, head_base};
use crate::util::{new_id, rel_to_path};

#[derive(Debug, Clone, Copy)]
pub struct RestoreOptions {
    /// Capture the current save state before overwriting it. Strongly
    /// recommended; only disabled in tests or by explicit user choice.
    pub safety_snapshot: bool,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            safety_snapshot: true,
        }
    }
}

/// Restore `version_id` into `game.save_root`. Returns the target snapshot that
/// was restored.
pub fn restore_version(
    db: &Db,
    cas: &Cas,
    device_id: &str,
    game: &Game,
    version_id: &str,
    opts: RestoreOptions,
) -> Result<Snapshot> {
    let target = db.get_version(version_id)?;
    if target.game_id != game.id {
        return Err(Error::NotFound(format!(
            "version {version_id} does not belong to game {}",
            game.id
        )));
    }

    // 1. Safety snapshot of the current state (only if there's something there).
    if opts.safety_snapshot && game.save_root.is_dir() {
        let short = &version_id[..version_id.len().min(8)];
        let (base, parent) = head_base(db, &game.id)?;
        create_snapshot(
            db,
            cas,
            device_id,
            game,
            SnapshotKind::PreRestore,
            Some(format!("auto safety backup before restoring {short}")),
            &base,
            parent,
        )?;
    }

    // Stage adjacent to the save root so the final rename stays on one fs.
    let parent = game
        .save_root
        .parent()
        .ok_or_else(|| Error::other("save_root has no parent directory"))?;
    let staging = parent.join(format!(".gamesync-staging-{}", new_id()));
    fs::create_dir_all(&staging)?;

    // 2. Materialize + verify into staging.
    let staged: Result<()> = (|| {
        for fe in &target.files {
            let dest = rel_to_path(&staging, &fe.rel_path);
            cas.copy_to(&fe.hash, &dest)?;
            if !Cas::verify_file(&dest, &fe.hash)? {
                return Err(Error::Integrity(format!(
                    "restored file {} failed checksum",
                    fe.rel_path
                )));
            }
        }
        Ok(())
    })();
    if let Err(e) = staged {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    // 3. Atomic-ish swap with rollback.
    let had_existing = game.save_root.is_dir();
    let old: PathBuf = parent.join(format!(".gamesync-old-{}", new_id()));
    if had_existing {
        fs::rename(&game.save_root, &old)?;
    } else if let Some(p) = game.save_root.parent() {
        fs::create_dir_all(p)?;
    }

    match fs::rename(&staging, &game.save_root) {
        Ok(()) => {
            if had_existing {
                let _ = fs::remove_dir_all(&old);
            }
            // The live save now corresponds to the restored version.
            db.set_head(&game.id, &target.version_id)?;
            Ok(target)
        }
        Err(e) => {
            // Roll the previous folder back into place.
            if had_existing {
                let _ = fs::rename(&old, &game.save_root);
            }
            let _ = fs::remove_dir_all(&staging);
            Err(e.into())
        }
    }
}
