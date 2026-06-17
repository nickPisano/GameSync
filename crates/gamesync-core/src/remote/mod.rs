//! Replication transport.
//!
//! A [`Remote`] is an object store + per-game version log + a head pointer,
//! with advisory locking so two devices don't write a game's head at once. The
//! engine is written against this trait, so adding rclone or LAN transports
//! later doesn't touch the sync logic.
//!
//! [`FolderRemote`] is the first implementation: it targets a plain directory,
//! which means cross-device sync rides any folder the user already syncs
//! (Dropbox, Drive, OneDrive, a network share) with zero provider integration.

pub mod folder;
pub mod lan;
pub mod rclone;

use crate::error::Result;
use crate::model::{Head, Snapshot};

pub use folder::FolderRemote;
pub use lan::{LanRemote, LanServerHandle};
pub use rclone::RcloneRemote;

/// Held for the duration of a sync; runs a transport-specific cleanup (e.g.
/// delete the lock file/object) on drop.
pub struct Lease {
    cleanup: Option<Box<dyn FnMut() + Send>>,
}

impl Lease {
    /// Create a lease whose `cleanup` runs when it is dropped.
    pub fn new(cleanup: impl FnMut() + Send + 'static) -> Self {
        Self {
            cleanup: Some(Box::new(cleanup)),
        }
    }

    /// A lease with no cleanup (e.g. a transport without locking).
    pub fn noop() -> Self {
        Self { cleanup: None }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(mut cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

pub trait Remote {
    /// Acquire the advisory lock for a game. Errors if another sync holds it.
    fn lock(&self, game_id: &str) -> Result<Lease>;

    fn has_object(&self, hash: &str) -> Result<bool>;
    /// Store an object's raw bytes (the on-disk CAS representation, which is
    /// already encrypted when encryption is enabled).
    fn put_object(&self, hash: &str, bytes: &[u8]) -> Result<()>;
    fn get_object(&self, hash: &str) -> Result<Vec<u8>>;

    fn get_version(&self, game_id: &str, version_id: &str) -> Result<Snapshot>;
    fn put_version(&self, game_id: &str, snapshot: &Snapshot) -> Result<()>;

    fn get_head(&self, game_id: &str) -> Result<Option<Head>>;
    fn set_head(&self, game_id: &str, head: &Head) -> Result<()>;
}
