//! Snapshotting: walk a save folder, store contents in the CAS, and record an
//! immutable version row. Snapshots are append-only — we never modify or delete
//! a previous version here.

use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

use crate::cas::Cas;
use crate::db::Db;
use crate::error::{Error, Result};
use crate::model::{FileEntry, Game, Snapshot, SnapshotKind, VectorClock};
use crate::util::{file_mode, mtime_ms, new_id, now_ms, to_rel_slash};
use crate::vclock;

/// Default include pattern: everything under the save root.
pub const DEFAULT_INCLUDES: &[&str] = &["**"];

/// Sensible default excludes — transient junk that should never be part of a
/// save snapshot. Both root (`*.tmp`) and nested (`**/*.tmp`) forms are listed
/// so matching is reliable regardless of glob separator semantics.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "*.tmp",
    "**/*.tmp",
    "*.bak",
    "**/*.bak",
    "Thumbs.db",
    "**/Thumbs.db",
    ".DS_Store",
    "**/.DS_Store",
];

/// Compiled include/exclude matcher. A path is included iff it matches an
/// include and matches no exclude.
struct Matcher {
    include: GlobSet,
    exclude: GlobSet,
}

impl Matcher {
    fn build(includes: &[String], excludes: &[String]) -> Result<Self> {
        let mut inc = GlobSetBuilder::new();
        if includes.is_empty() {
            inc.add(Glob::new("**")?);
        }
        for p in includes {
            inc.add(Glob::new(p)?);
        }
        let mut exc = GlobSetBuilder::new();
        for p in excludes {
            exc.add(Glob::new(p)?);
        }
        Ok(Self {
            include: inc.build()?,
            exclude: exc.build()?,
        })
    }

    fn matches(&self, rel: &str) -> bool {
        self.include.is_match(rel) && !self.exclude.is_match(rel)
    }
}

/// Resolve the base (vector clock, parent id) for a new snapshot of `game_id`:
/// the tracked head if set, else the most recent version, else empty/none.
pub fn head_base(db: &Db, game_id: &str) -> Result<(VectorClock, Option<String>)> {
    if let Some(head_id) = db.get_head(game_id)? {
        if let Ok(s) = db.get_version(&head_id) {
            return Ok((s.vclock, Some(s.version_id)));
        }
    }
    if let Some(s) = db.latest_version(game_id)? {
        return Ok((s.vclock, Some(s.version_id)));
    }
    Ok((VectorClock::new(), None))
}

/// Capture the current state of `game.save_root` into a new immutable version.
///
/// `base_vclock` is the version vector this snapshot descends from (the head at
/// capture time, or a merged vector when resolving a conflict); this device's
/// counter is incremented from it. `parent` records the base version id.
///
/// This function assumes the caller has already established that it is safe to
/// read (game not running / folder quiescent). It does not touch the live save
/// folder beyond reading it.
pub fn create_snapshot(
    db: &Db,
    cas: &Cas,
    device_id: &str,
    game: &Game,
    kind: SnapshotKind,
    label: Option<String>,
    base_vclock: &VectorClock,
    parent: Option<String>,
) -> Result<Snapshot> {
    let (files, total) = collect_files(cas, game)?;
    let snapshot = build_and_insert(
        db,
        device_id,
        game,
        kind,
        label,
        base_vclock,
        parent,
        files,
        total,
    )?;
    Ok(snapshot)
}

/// Like [`create_snapshot`], but skips inserting a new version when the save set
/// is byte-for-byte identical to `head` (same set of `rel_path → hash`). Used by
/// auto-sync so an unchanged save doesn't accrue duplicate history entries.
/// Returns `None` when nothing changed.
pub fn create_snapshot_if_changed(
    db: &Db,
    cas: &Cas,
    device_id: &str,
    game: &Game,
    kind: SnapshotKind,
    label: Option<String>,
    base_vclock: &VectorClock,
    parent: Option<String>,
    head: Option<&Snapshot>,
) -> Result<Option<Snapshot>> {
    let (files, total) = collect_files(cas, game)?;
    // A fresh device with an empty save and no history has nothing to back up;
    // snapshotting it would create a spurious branch that conflicts with a peer
    // that does have saves. Let it pull instead.
    if files.is_empty() && head.is_none() {
        return Ok(None);
    }
    if let Some(head) = head {
        if same_contents(&files, &head.files) {
            return Ok(None);
        }
    }
    let snapshot = build_and_insert(
        db,
        device_id,
        game,
        kind,
        label,
        base_vclock,
        parent,
        files,
        total,
    )?;
    Ok(Some(snapshot))
}

/// List the save-set files (relative path, size, mtime) that *would* be backed
/// up, without hashing or storing anything. Powers the in-app file view.
pub fn list_files(game: &Game) -> Result<Vec<(String, u64, i64)>> {
    let matcher = Matcher::build(&game.includes, &game.excludes)?;
    let mut out = Vec::new();
    if game.save_root.is_dir() {
        for entry in WalkDir::new(&game.save_root).follow_links(false) {
            let entry = entry.map_err(|e| Error::other(format!("walk error: {e}")))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = to_rel_slash(&game.save_root, entry.path());
            if rel.is_empty() || !matcher.matches(&rel) {
                continue;
            }
            let meta = entry.metadata().map_err(|e| Error::other(e.to_string()))?;
            out.push((rel, meta.len(), mtime_ms(&meta)));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Walk the save set, store contents in the CAS, and return the sorted file
/// list plus total size.
fn collect_files(cas: &Cas, game: &Game) -> Result<(Vec<FileEntry>, u64)> {
    let matcher = Matcher::build(&game.includes, &game.excludes)?;
    let mut files: Vec<FileEntry> = Vec::new();
    let mut total: u64 = 0;

    if game.save_root.is_dir() {
        for entry in WalkDir::new(&game.save_root).follow_links(false) {
            let entry = entry.map_err(|e| Error::other(format!("walk error: {e}")))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = to_rel_slash(&game.save_root, entry.path());
            if rel.is_empty() || !matcher.matches(&rel) {
                continue;
            }
            let meta = entry.metadata().map_err(|e| Error::other(e.to_string()))?;
            let (hash, size) = cas.put_file(entry.path())?;
            total += size;
            files.push(FileEntry {
                rel_path: rel,
                hash,
                size,
                mtime_ms: mtime_ms(&meta),
                mode: file_mode(&meta),
            });
        }
    }
    // Deterministic ordering makes manifests stable and diffs meaningful.
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok((files, total))
}

/// Compare two (sorted) file lists by path + content hash only.
fn same_contents(a: &[FileEntry], b: &[FileEntry]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.rel_path == y.rel_path && x.hash == y.hash)
}

#[allow(clippy::too_many_arguments)]
fn build_and_insert(
    db: &Db,
    device_id: &str,
    game: &Game,
    kind: SnapshotKind,
    label: Option<String>,
    base_vclock: &VectorClock,
    parent: Option<String>,
    files: Vec<FileEntry>,
    total: u64,
) -> Result<Snapshot> {
    let snapshot = Snapshot {
        version_id: new_id(),
        game_id: game.id.clone(),
        device_id: device_id.to_string(),
        created_ms: now_ms(),
        label,
        kind,
        parent,
        vclock: vclock::bump(base_vclock, device_id),
        total_size: total,
        files,
    };
    db.insert_version(&snapshot)?;
    Ok(snapshot)
}
