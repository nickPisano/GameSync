//! Domain types. These are the shapes that cross the engine boundary and will
//! also be the IPC surface for the Tauri UI (serializable via serde).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A BLAKE3 content hash, lowercase hex.
pub type Hash = String;

/// A version vector: device id → monotonic counter. Two snapshots are
/// *concurrent* (a conflict) when neither vector dominates the other.
pub type VectorClock = BTreeMap<String, u64>;

/// The agreed "current" version of a game on a remote, with its clock so peers
/// can decide fast-forward vs. conflict without downloading the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Head {
    pub version_id: String,
    #[serde(default)]
    pub vclock: VectorClock,
}

/// Where a game was discovered. Drives detection logic and display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Steam,
    Gog,
    Epic,
    Emulator,
    Manual,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Steam => "steam",
            Platform::Gog => "gog",
            Platform::Epic => "epic",
            Platform::Emulator => "emulator",
            Platform::Manual => "manual",
        }
    }

    pub fn from_str(s: &str) -> Platform {
        match s {
            "steam" => Platform::Steam,
            "gog" => Platform::Gog,
            "epic" => Platform::Epic,
            "emulator" => Platform::Emulator,
            _ => Platform::Manual,
        }
    }
}

/// A game whose saves GameSync manages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    /// Stable id, e.g. `steam:374320` or `manual:ab12cd34ef567890`.
    pub id: String,
    pub name: String,
    pub platform: Platform,
    /// Absolute path to the save folder (the unit we snapshot).
    pub save_root: PathBuf,
    /// Install directory, if known. Used to detect whether the game is running.
    pub install_dir: Option<PathBuf>,
    /// Glob includes (relative to `save_root`). Defaults to everything.
    pub includes: Vec<String>,
    /// Glob excludes (relative to `save_root`).
    pub excludes: Vec<String>,
    /// Whether automatic sync/backup is enabled for this game.
    pub sync_enabled: bool,
}

/// Why a snapshot was taken. `PreRestore` snapshots are the safety net captured
/// immediately before any restore, so a restore is always itself undoable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotKind {
    Manual,
    Auto,
    PreRestore,
}

impl SnapshotKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotKind::Manual => "manual",
            SnapshotKind::Auto => "auto",
            SnapshotKind::PreRestore => "pre_restore",
        }
    }

    pub fn from_str(s: &str) -> SnapshotKind {
        match s {
            "manual" => SnapshotKind::Manual,
            "pre_restore" => SnapshotKind::PreRestore,
            _ => SnapshotKind::Auto,
        }
    }
}

/// One file inside a snapshot. Contents live in the content-addressed store
/// keyed by `hash`; this entry records how to put it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path relative to `save_root`, forward-slash separated.
    pub rel_path: String,
    pub hash: Hash,
    pub size: u64,
    pub mtime_ms: i64,
    /// Unix mode bits, 0 where not applicable.
    pub mode: u32,
}

/// An immutable, point-in-time capture of a game's save set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version_id: String,
    pub game_id: String,
    pub device_id: String,
    pub created_ms: i64,
    pub label: Option<String>,
    pub kind: SnapshotKind,
    /// Previous version id this snapshot was based on (the head at capture time).
    pub parent: Option<String>,
    /// Version vector for cross-device causality / conflict detection.
    #[serde(default)]
    pub vclock: VectorClock,
    pub total_size: u64,
    pub files: Vec<FileEntry>,
}

impl Snapshot {
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}
