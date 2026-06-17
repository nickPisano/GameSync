//! Filesystem, time, and id helpers shared across the engine.
//!
//! The functions here are deliberately small and conservative — they are the
//! primitives the safety guarantees are built on (atomic writes, stable ids,
//! consistent relative-path formatting).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a short, collision-resistant id (16 hex chars). Derived from the
/// monotonic-ish wall clock plus a process-local counter so two ids minted in
/// the same nanosecond still differ.
pub fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&nanos.to_le_bytes());
    hasher.update(&c.to_le_bytes());
    hasher.finalize().to_hex()[..16].to_string()
}

/// A stable 16-hex id derived from an input string (e.g. a game name). Unlike
/// [`new_id`], this is deterministic, so two devices deriving an id from the
/// same name agree — which is what lets manually-added games sync.
pub fn hash_id(input: &str) -> String {
    blake3::hash(input.as_bytes()).to_hex()[..16].to_string()
}

/// Current wall-clock time in milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Modification time of a file in milliseconds since the Unix epoch (0 if
/// unavailable).
pub fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Unix permission bits, or 0 on platforms without them.
#[cfg(unix)]
pub fn file_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(unix))]
pub fn file_mode(_meta: &fs::Metadata) -> u32 {
    0
}

/// Format a path relative to `base` using forward slashes, regardless of OS.
/// This keeps snapshot manifests portable between Windows and Unix.
pub fn to_rel_slash(base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Turn a forward-slash relative path (from a manifest) back into a native
/// `PathBuf` joined under `base`.
pub fn rel_to_path(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

/// Lowercase hex encoding.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

/// Decode lowercase/uppercase hex; returns None on malformed input.
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

/// Recursively copy a directory's contents (files + subdirs) to `dst`. Skips
/// symlinks and other special files.
pub fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Create a directory symlink at `link` pointing to `target` (a junction-style
/// redirect). On Windows this needs Developer Mode or admin rights.
pub fn make_dir_symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "symlinks not supported on this platform",
        ))
    }
}

/// Write `data` to `path` atomically: write to a temp file in the same
/// directory, fsync it, then rename over the destination. A crash leaves either
/// the old file or the new file, never a truncated one.
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".gstmp-{}", new_id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}
