//! GOG (Galaxy) detection.
//!
//! Every game installed by GOG Galaxy drops a `goggame-<id>.info` JSON file in
//! its install folder, carrying the numeric `gameId` and the `name`. Rather than
//! read Galaxy's SQLite database or the Windows registry, we scan known library
//! roots one level deep for those files — portable and fixture-testable. The
//! save folder is resolved by matching the name into the save-path manifest
//! (GOG IDs don't appear there).

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// An installed GOG game.
#[derive(Debug, Clone)]
pub struct GogApp {
    /// GOG's numeric `gameId` — used to form the `gog:<id>` game id.
    pub game_id: String,
    /// Human-facing title, matched into the save manifest by name.
    pub name: String,
    pub install_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Info {
    #[serde(rename = "gameId")]
    game_id: Option<String>,
    name: Option<String>,
}

/// Default GOG library roots for this OS. These are best-effort common
/// locations; Galaxy lets the user pick any folder, so callers may also point
/// [`apps_in`] at custom roots.
pub fn library_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Some(pf) = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from) {
            roots.push(pf.join("GOG Galaxy").join("Games"));
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\GOG Galaxy\Games"));
        roots.push(PathBuf::from(r"C:\GOG Games"));
    }

    let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
    if let Some(h) = home {
        roots.push(h.join("GOG Games"));
        roots.push(h.join("Games"));
    }

    roots.sort();
    roots.dedup();
    roots
}

/// Find a game's `goggame-*.info` directly inside `dir`, if present.
fn read_info(dir: &Path) -> Option<GogApp> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !(file.starts_with("goggame-") && file.ends_with(".info")) {
            continue;
        }
        let text = fs::read_to_string(&path).ok()?;
        let info: Info = serde_json::from_str(&text).ok()?;
        let (Some(game_id), Some(name)) = (info.game_id, info.name) else {
            continue;
        };
        if game_id.is_empty() || name.is_empty() {
            continue;
        }
        return Some(GogApp {
            game_id,
            name,
            install_dir: dir.to_path_buf(),
        });
    }
    None
}

/// Scan each library root's immediate subdirectories (the per-game install
/// folders) for a `goggame-*.info`. Unreadable roots are skipped; the same game
/// id is never returned twice.
pub fn apps_in(roots: &[PathBuf]) -> Vec<GogApp> {
    let mut out: Vec<GogApp> = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            if let Some(app) = read_info(&dir) {
                if !out.iter().any(|a| a.game_id == app.game_id) {
                    out.push(app);
                }
            }
        }
    }
    out.sort_by(|a, b| a.game_id.cmp(&b.game_id));
    out
}

/// Detect installed GOG games under the default library roots.
pub fn installed_apps() -> Vec<GogApp> {
    apps_in(&library_roots())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_install_folders_for_goggame_info() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("GOG Games");

        // A real game folder with a goggame info file.
        let witcher = root.join("The Witcher 3");
        fs::create_dir_all(&witcher).unwrap();
        fs::write(
            witcher.join("goggame-1207664663.info"),
            r#"{"gameId":"1207664663","name":"The Witcher 3: Wild Hunt"}"#,
        )
        .unwrap();

        // A folder with no info file — ignored.
        fs::create_dir_all(root.join("Random")).unwrap();
        fs::write(root.join("Random").join("readme.txt"), "x").unwrap();

        let apps = apps_in(&[root]);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].game_id, "1207664663");
        assert_eq!(apps[0].name, "The Witcher 3: Wild Hunt");
        assert_eq!(apps[0].install_dir, witcher);
    }

    #[test]
    fn dedups_same_game_across_roots_and_skips_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("A");
        let b = tmp.path().join("B");
        for root in [&a, &b] {
            let g = root.join("Game");
            fs::create_dir_all(&g).unwrap();
            fs::write(
                g.join("goggame-42.info"),
                r#"{"gameId":"42","name":"Game"}"#,
            )
            .unwrap();
        }
        let apps = apps_in(&[a, b, tmp.path().join("does-not-exist")]);
        assert_eq!(apps.len(), 1, "the same game id must not be listed twice");
        assert_eq!(apps[0].game_id, "42");
    }
}
