//! Process awareness: never snapshot a save that the game is actively writing.
//!
//! Two complementary checks:
//!  - `is_running`: is any process executing from the game's install dir?
//!  - `wait_until_quiescent`: has the save folder stopped changing? (covers
//!    background writers, cloud-sync clients, and the gap between "process
//!    exits" and "OS flushes the last writes").

use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use sysinfo::System;
use walkdir::WalkDir;

use crate::model::Game;
use crate::util::mtime_ms;

/// Heuristic: true if a running process's executable lives inside the game's
/// install directory. Returns false when the install dir is unknown.
pub fn is_running(game: &Game) -> bool {
    let install = match &game.install_dir {
        Some(d) => d,
        None => return false,
    };
    let mut sys = System::new();
    sys.refresh_processes();
    for proc_ in sys.processes().values() {
        if let Some(exe) = proc_.exe() {
            if exe.starts_with(install) {
                return true;
            }
        }
    }
    false
}

/// For each install dir, whether a process is currently running from inside it.
/// Does a single process-table refresh (cheaper than calling [`is_running`] per
/// game), so it's suitable for a periodic exit watcher.
pub fn running_install_dirs(dirs: &[PathBuf]) -> Vec<bool> {
    if dirs.is_empty() {
        return Vec::new();
    }
    let mut sys = System::new();
    sys.refresh_processes();
    let exes: Vec<PathBuf> = sys
        .processes()
        .values()
        .filter_map(|p| p.exe().map(|e| e.to_path_buf()))
        .collect();
    dirs.iter()
        .map(|d| exes.iter().any(|e| e.starts_with(d)))
        .collect()
}

/// A cheap fingerprint of a folder: (total size, newest mtime, file count).
/// If this is unchanged across a settle window, writes have stopped.
fn signature(path: &Path) -> (u64, i64, u64) {
    let mut size = 0u64;
    let mut newest = 0i64;
    let mut count = 0u64;
    for entry in WalkDir::new(path).into_iter().flatten() {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                size += meta.len();
                newest = newest.max(mtime_ms(&meta));
                count += 1;
            }
        }
    }
    (size, newest, count)
}

/// Block until `path` has been stable for `settle` continuously, or until
/// `timeout` elapses. Returns true if quiescence was reached.
pub fn wait_until_quiescent(path: &Path, settle: Duration, timeout: Duration) -> bool {
    if !path.exists() {
        return true;
    }
    let start = Instant::now();
    let mut last = signature(path);
    loop {
        sleep(settle);
        let current = signature(path);
        if current == last {
            return true;
        }
        last = current;
        if start.elapsed() >= timeout {
            return false;
        }
    }
}
