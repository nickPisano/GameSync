//! Epic Games Launcher detection.
//!
//! The launcher writes one JSON `.item` manifest per installed game into a
//! per-OS `Manifests` directory. Each carries a `DisplayName`, an
//! `InstallLocation`, and a stable `AppName`, which is all we need: the save
//! folder itself is resolved by matching the display name into the save-path
//! manifest (Epic IDs don't appear there). Parsing is pure given a directory,
//! so it is fixture-testable without the launcher installed.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// An installed Epic application.
#[derive(Debug, Clone)]
pub struct EpicApp {
    /// Epic's stable `AppName` — used to form the `epic:<id>` game id.
    pub app_name: String,
    /// Human-facing title, matched into the save manifest by name.
    pub name: String,
    pub install_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(rename = "AppName")]
    app_name: Option<String>,
    #[serde(rename = "DisplayName")]
    display_name: Option<String>,
    #[serde(rename = "InstallLocation")]
    install_location: Option<String>,
    #[serde(rename = "bIsIncompleteInstall")]
    incomplete: Option<bool>,
}

/// The Epic launcher's `Manifests` directory for this OS, if it exists.
pub fn manifests_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        let dir = base.join(r"Epic\EpicGamesLauncher\Data\Manifests");
        return dir.is_dir().then_some(dir);
    }

    #[cfg(target_os = "macos")]
    {
        let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
        if let Some(h) = home {
            let dir = h.join("Library/Application Support/Epic/EpicGamesLauncher/Data/Manifests");
            if dir.is_dir() {
                return Some(dir);
            }
        }
        return None;
    }

    // The official launcher isn't available on Linux.
    #[allow(unreachable_code)]
    None
}

/// Parse every `.item` manifest in `dir` into an [`EpicApp`]. Skips incomplete
/// installs and any manifest missing a name or install location. A malformed
/// file is ignored rather than failing the whole scan.
pub fn apps_in(dir: &Path) -> Vec<EpicApp> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("item") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(item) = serde_json::from_str::<Item>(&text) else {
            continue;
        };
        if item.incomplete == Some(true) {
            continue;
        }
        let (Some(app_name), Some(name), Some(loc)) =
            (item.app_name, item.display_name, item.install_location)
        else {
            continue;
        };
        if app_name.is_empty() || name.is_empty() || loc.is_empty() {
            continue;
        }
        out.push(EpicApp {
            app_name,
            name,
            install_dir: PathBuf::from(loc),
        });
    }
    out.sort_by(|a, b| a.app_name.cmp(&b.app_name));
    out
}

/// Detect installed Epic apps on this machine.
pub fn installed_apps() -> Vec<EpicApp> {
    manifests_dir().map(|d| apps_in(&d)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_item_manifests_and_skips_bad_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        fs::write(
            dir.join("Hades.item"),
            r#"{"AppName":"Min","DisplayName":"Hades","InstallLocation":"/games/Hades"}"#,
        )
        .unwrap();
        // Incomplete install — must be skipped.
        fs::write(
            dir.join("Half.item"),
            r#"{"AppName":"H","DisplayName":"Half","InstallLocation":"/g/H","bIsIncompleteInstall":true}"#,
        )
        .unwrap();
        // Missing install location — skipped.
        fs::write(
            dir.join("NoLoc.item"),
            r#"{"AppName":"N","DisplayName":"NoLoc"}"#,
        )
        .unwrap();
        // Not an .item file — ignored.
        fs::write(dir.join("notes.txt"), "ignore me").unwrap();
        // Malformed JSON — ignored, doesn't abort the scan.
        fs::write(dir.join("Broken.item"), "{not json").unwrap();

        let apps = apps_in(dir);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].app_name, "Min");
        assert_eq!(apps[0].name, "Hades");
        assert_eq!(apps[0].install_dir, PathBuf::from("/games/Hades"));
    }

    #[test]
    fn missing_directory_yields_no_apps() {
        assert!(apps_in(Path::new("/no/such/epic/dir/xyzzy")).is_empty());
    }
}
