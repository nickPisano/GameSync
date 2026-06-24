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
use crate::model::{FileEntry, Game, Snapshot, SnapshotKind};
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

    let roots = game.roots();

    // 1. Safety snapshot of the current state of every root (so the restore is
    //    undoable), only if at least one root has something to capture.
    if opts.safety_snapshot && roots.iter().any(|r| r.is_dir()) {
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

    // 2. Stage + verify each root the target has files for. We only touch a root
    //    the snapshot actually has files for, so restoring an older version
    //    never empties a root that was added later. Stage everything (verifying
    //    by hash) before swapping anything live.
    struct Staged {
        root: PathBuf,
        staging: PathBuf,
    }
    let mut staged: Vec<Staged> = Vec::new();
    let stage_all: Result<()> = (|| {
        for (idx, root) in roots.iter().enumerate() {
            let files: Vec<&FileEntry> = target
                .files
                .iter()
                .filter(|f| f.root as usize == idx)
                .collect();
            if files.is_empty() {
                continue;
            }
            // Stage adjacent to the root so the final rename stays on one fs.
            let parent = root
                .parent()
                .ok_or_else(|| Error::other("restore root has no parent directory"))?;
            let staging = parent.join(format!(".gamesync-staging-{}", new_id()));
            fs::create_dir_all(&staging)?;
            // Record the staging dir *before* materializing into it, so a
            // mid-materialize failure still cleans it up below.
            staged.push(Staged {
                root: root.clone(),
                staging,
            });
            let staging = &staged.last().unwrap().staging;
            for fe in files {
                let dest = rel_to_path(staging, &fe.rel_path);
                cas.copy_to(&fe.hash, &dest)?;
                if !Cas::verify_file(&dest, &fe.hash)? {
                    return Err(Error::Integrity(format!(
                        "restored file {} failed checksum",
                        fe.rel_path
                    )));
                }
            }
        }
        Ok(())
    })();
    if let Err(e) = stage_all {
        for s in &staged {
            let _ = fs::remove_dir_all(&s.staging);
        }
        return Err(e);
    }

    // 3. Swap each staged root into place (atomic per folder), rolling back
    //    every root already swapped if any swap fails.
    struct Swapped {
        root: PathBuf,
        old: Option<PathBuf>,
    }
    let mut swapped: Vec<Swapped> = Vec::new();
    let swap_all: Result<()> = (|| {
        for s in &staged {
            let old = if s.root.is_dir() {
                let old = s.root.with_file_name(format!(".gamesync-old-{}", new_id()));
                fs::rename(&s.root, &old)?;
                Some(old)
            } else {
                if let Some(p) = s.root.parent() {
                    fs::create_dir_all(p)?;
                }
                None
            };
            fs::rename(&s.staging, &s.root)?;
            swapped.push(Swapped {
                root: s.root.clone(),
                old,
            });
        }
        Ok(())
    })();
    match swap_all {
        Ok(()) => {
            for s in &swapped {
                if let Some(old) = &s.old {
                    let _ = fs::remove_dir_all(old);
                }
            }
            // The live save now corresponds to the restored version.
            db.set_head(&game.id, &target.version_id)?;
            Ok(target)
        }
        Err(e) => {
            // Roll back: drop the swapped-in folders, move the originals back.
            for s in &swapped {
                let _ = fs::remove_dir_all(&s.root);
                if let Some(old) = &s.old {
                    let _ = fs::rename(old, &s.root);
                }
            }
            // Clean up stagings that hadn't been swapped yet.
            for st in &staged {
                if !swapped.iter().any(|s| s.root == st.root) {
                    let _ = fs::remove_dir_all(&st.staging);
                }
            }
            Err(e)
        }
    }
}
