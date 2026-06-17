//! Steam install + library discovery.
//!
//! Finds the Steam root, enumerates library folders via `libraryfolders.vdf`,
//! and reads each `appmanifest_*.acf` to learn installed appids, names, and
//! install directories.

use std::fs;
use std::path::{Path, PathBuf};

use super::vdf;

/// An installed Steam application.
#[derive(Debug, Clone)]
pub struct SteamApp {
    pub appid: String,
    pub name: String,
    pub install_dir: PathBuf,
}

/// Locate the Steam root directory for the current OS, if present.
pub fn steam_root() -> Option<PathBuf> {
    let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());

    #[cfg(target_os = "windows")]
    {
        let candidates = [
            PathBuf::from(r"C:\Program Files (x86)\Steam"),
            PathBuf::from(r"C:\Program Files\Steam"),
        ];
        return candidates.into_iter().find(|p| p.is_dir());
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(h) = home {
            let p = h.join("Library/Application Support/Steam");
            if p.is_dir() {
                return Some(p);
            }
        }
        return None;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(h) = home {
            let candidates = [
                h.join(".steam/steam"),
                h.join(".local/share/Steam"),
                h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            ];
            return candidates.into_iter().find(|p| p.is_dir());
        }
        return None;
    }

    #[allow(unreachable_code)]
    None
}

/// All Steam library roots (each contains a `steamapps/` directory).
pub fn library_paths(root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![root.to_path_buf()];
    let vdf_path = root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = fs::read_to_string(&vdf_path) {
        let parsed = vdf::parse(&text);
        if let Some(folders) = parsed.get("libraryfolders").and_then(|v| v.as_obj()) {
            for entry in folders.values() {
                let path = match entry {
                    vdf::Vdf::Str(s) => Some(s.clone()),
                    vdf::Vdf::Obj(_) => {
                        entry.get("path").and_then(|p| p.as_str()).map(String::from)
                    }
                };
                if let Some(p) = path {
                    libs.push(PathBuf::from(p));
                }
            }
        }
    }
    libs.sort();
    libs.dedup();
    libs
}

/// Enumerate installed apps across all libraries.
pub fn installed_apps() -> Vec<SteamApp> {
    let root = match steam_root() {
        Some(r) => r,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for lib in library_paths(&root) {
        let steamapps = lib.join("steamapps");
        let read = match fs::read_dir(&steamapps) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let text = match fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let parsed = vdf::parse(&text);
            if let Some(app) = parsed.get("AppState") {
                let appid = app.get("appid").and_then(|v| v.as_str()).unwrap_or("");
                let game_name = app.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let installdir = app.get("installdir").and_then(|v| v.as_str()).unwrap_or("");
                if appid.is_empty() {
                    continue;
                }
                out.push(SteamApp {
                    appid: appid.to_string(),
                    name: game_name.to_string(),
                    install_dir: steamapps.join("common").join(installdir),
                });
            }
        }
    }
    out
}
