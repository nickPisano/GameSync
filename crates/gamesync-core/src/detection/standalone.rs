//! Standalone (non-store) games — free/itch/modpack titles that carry no
//! Steam/GOG/Epic id, so they can't be matched the usual way. They're detected
//! by probing a fixed save folder. An optional `install` path (where the
//! executable lives) lets the exit-watcher find the game's process so
//! backup-on-close works, just like store-detected games.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use super::paths::Dirs;

const BUNDLED: &str = include_str!("../../manifests/standalone.json");

#[derive(Debug, Clone, Deserialize)]
pub struct StandaloneRule {
    pub name: String,
    /// Save-folder templates; the first that resolves AND exists wins.
    pub paths: Vec<String>,
    /// Install-dir templates (where the executable lives), for the exit watcher.
    #[serde(default)]
    pub install: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct StandaloneManifest {
    #[serde(default)]
    games: BTreeMap<String, StandaloneRule>,
}

/// A detected standalone game: its existing save folder, plus the install dir if
/// one of the rule's `install` templates exists (so backup-on-close can watch it).
#[derive(Debug, Clone)]
pub struct DetectedStandalone {
    pub key: String,
    pub name: String,
    pub save_root: PathBuf,
    pub install_dir: Option<PathBuf>,
}

/// The bundled standalone-game rules (key → rule).
pub fn bundled_rules() -> BTreeMap<String, StandaloneRule> {
    serde_json::from_str::<StandaloneManifest>(BUNDLED)
        .expect("bundled standalone manifest must be valid JSON")
        .games
}

/// First template that expands and exists as a directory.
fn first_existing(dirs: &Dirs, templates: &[String]) -> Option<PathBuf> {
    templates
        .iter()
        .find_map(|t| dirs.expand(t, None).filter(|p| p.is_dir()))
}

/// Detect standalone games whose save folder exists on disk.
pub fn detect() -> Vec<DetectedStandalone> {
    let dirs = Dirs::detect();
    bundled_rules()
        .into_iter()
        .filter_map(|(key, rule)| {
            let save_root = first_existing(&dirs, &rule.paths)?;
            let install_dir = first_existing(&dirs, &rule.install);
            Some(DetectedStandalone {
                key,
                name: rule.name,
                save_root,
                install_dir,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_manifest_parses_and_has_entries() {
        let rules = bundled_rules();
        assert!(rules.contains_key("votv"));
        assert!(rules.contains_key("stalker-gamma"));
        assert!(rules.values().all(|r| !r.paths.is_empty()));
    }
}
