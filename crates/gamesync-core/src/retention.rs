//! Retention and garbage collection.
//!
//! History is append-only during normal operation; pruning is the *only* place
//! versions are deleted, and it is deliberate and policy-driven. Object deletion
//! is a separate, reference-counted pass (`gc`) so a bug in pruning can never
//! orphan-delete bytes that another version still needs.

use crate::cas::Cas;
use crate::db::Db;
use crate::error::Result;
use crate::util::now_ms;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetentionPolicy {
    /// Always keep at least this many of the newest versions.
    pub keep_last: usize,
    /// Also keep any version newer than this many days, regardless of count.
    pub keep_days: Option<i64>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_last: 20,
            keep_days: Some(30),
        }
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct GcReport {
    pub objects_deleted: usize,
    pub bytes_freed: u64,
}

/// Apply a retention policy to one game, deleting version *records* that fall
/// outside it. Never deletes the single newest version. Returns the ids deleted.
/// Call [`gc`] afterwards to reclaim the now-unreferenced bytes.
pub fn prune(db: &Db, game_id: &str, policy: &RetentionPolicy) -> Result<Vec<String>> {
    let versions = db.list_versions(game_id)?; // newest first
    if versions.len() <= 1 {
        return Ok(Vec::new());
    }

    let cutoff = policy
        .keep_days
        .map(|d| now_ms() - d.saturating_mul(86_400_000));

    let mut deleted = Vec::new();
    for (idx, v) in versions.iter().enumerate() {
        // Protect the newest `keep_last` by position.
        if idx < policy.keep_last.max(1) {
            continue;
        }
        // Protect anything still within the time window.
        if let Some(cut) = cutoff {
            if v.created_ms >= cut {
                continue;
            }
        }
        db.delete_version(&v.version_id)?;
        deleted.push(v.version_id.clone());
    }
    Ok(deleted)
}

/// Reference-counted garbage collection: delete every stored object that no
/// surviving version references. Safe to run any time.
pub fn gc(db: &Db, cas: &Cas) -> Result<GcReport> {
    let live = db.all_referenced_hashes()?;
    let mut report = GcReport::default();
    for hash in cas.list_objects()? {
        if !live.contains(&hash) {
            report.bytes_freed += cas.object_size(&hash);
            cas.remove_object(&hash)?;
            report.objects_deleted += 1;
        }
    }
    Ok(report)
}
