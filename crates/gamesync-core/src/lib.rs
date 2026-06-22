//! # gamesync-core
//!
//! The engine behind GameSync: safe, versioned, content-addressed backup and
//! restore of game save folders for titles without reliable cloud saves.
//!
//! Design principles (see `docs/ARCHITECTURE.md`):
//!  - **Snapshot-and-replicate, not live file sync.** A save set is captured
//!    atomically at a safe moment; we never sync individual files mid-write.
//!  - **Append-only history.** Versions are immutable; restores are undoable.
//!  - **Integrity by construction.** Content addressing + checksums everywhere.
//!
//! The public entry point is [`Engine`]. The Tauri UI and the CLI are both thin
//! layers over it.

pub mod cas;
pub mod crypto;
pub mod db;
pub mod detection;
pub mod diff;
pub mod engine;
pub mod error;
pub mod model;
pub mod plugins;
pub mod process;
pub mod remote;
pub mod restore;
pub mod retention;
pub mod snapshot;
pub mod util;
pub mod vclock;

pub use crypto::{KeyStore, RecoveryKey};
pub use diff::Diff;
pub use engine::{
    AutoSyncReport, AutoSyncSettings, BackupOptions, ConflictChoice, ConflictInfo, Engine,
    GameStorage, PluginInfo, PluginList, RedirectReport, SaveFile, StorageReport, SyncOutcome,
    VerifyReport, ViewerInfo,
};
pub use error::{Error, Result};
pub use model::{FileEntry, Game, Head, Platform, Snapshot, SnapshotKind};
pub use remote::{
    BeaconHandle, DiscoveredHost, FolderRemote, LanRemote, LanServerHandle, RcloneRemote, Remote,
};
pub use restore::RestoreOptions;
pub use retention::{GcReport, RetentionPolicy};
