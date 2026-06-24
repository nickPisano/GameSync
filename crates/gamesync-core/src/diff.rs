//! Compare two snapshots. Powers the "what changed between versions" view and
//! lets a restore preview show exactly what it will add, remove, and overwrite.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::{FileEntry, Snapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diff {
    /// Present in `to` but not `from`.
    pub added: Vec<String>,
    /// Present in `from` but not `to`.
    pub removed: Vec<String>,
    /// Present in both, but contents differ.
    pub modified: Vec<String>,
    /// Count of files identical in both.
    pub unchanged: usize,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    pub fn changed_count(&self) -> usize {
        self.added.len() + self.removed.len() + self.modified.len()
    }
}

/// A file's identity for diffing: (root index, relative path). Keyed on both so
/// same-named files in different roots don't collide.
fn key(f: &FileEntry) -> (u32, &str) {
    (f.root, f.rel_path.as_str())
}

/// Human label for a diff entry; extra-root files get a `[N]` prefix.
fn label(root: u32, rel: &str) -> String {
    if root == 0 {
        rel.to_string()
    } else {
        format!("[folder {}] {rel}", root + 1)
    }
}

/// Diff `from` (older) against `to` (newer): the changes that turn `from` into
/// `to`.
pub fn diff(from: &Snapshot, to: &Snapshot) -> Diff {
    let from_map: BTreeMap<(u32, &str), &str> = from
        .files
        .iter()
        .map(|f| (key(f), f.hash.as_str()))
        .collect();
    let to_map: BTreeMap<(u32, &str), &str> =
        to.files.iter().map(|f| (key(f), f.hash.as_str())).collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged = 0usize;

    for ((root, path), to_hash) in &to_map {
        match from_map.get(&(*root, *path)) {
            None => added.push(label(*root, path)),
            Some(from_hash) if from_hash == to_hash => unchanged += 1,
            Some(_) => modified.push(label(*root, path)),
        }
    }
    for (root, path) in from_map.keys() {
        if !to_map.contains_key(&(*root, *path)) {
            removed.push(label(*root, path));
        }
    }

    added.sort();
    removed.sort();
    modified.sort();
    Diff {
        added,
        removed,
        modified,
        unchanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileEntry, SnapshotKind};

    fn snap(files: &[(&str, &str)]) -> Snapshot {
        Snapshot {
            version_id: "v".into(),
            game_id: "g".into(),
            device_id: "d".into(),
            created_ms: 0,
            label: None,
            kind: SnapshotKind::Manual,
            parent: None,
            vclock: Default::default(),
            total_size: 0,
            files: files
                .iter()
                .map(|(p, h)| FileEntry {
                    rel_path: (*p).into(),
                    hash: (*h).into(),
                    size: 0,
                    mtime_ms: 0,
                    mode: 0,
                    root: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn detects_all_change_kinds() {
        let from = snap(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let to = snap(&[("a", "1"), ("b", "9"), ("d", "4")]);
        let d = diff(&from, &to);
        assert_eq!(d.added, vec!["d"]);
        assert_eq!(d.removed, vec!["c"]);
        assert_eq!(d.modified, vec!["b"]);
        assert_eq!(d.unchanged, 1); // "a"
        assert_eq!(d.changed_count(), 3);
    }
}
