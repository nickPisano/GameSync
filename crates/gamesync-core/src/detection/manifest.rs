//! The save-path manifest: a mapping from Steam appid to one or more save-path
//! templates, plus a resolver that expands templates for the current OS/user.
//!
//! In production this manifest would be signed and refreshed from a community
//! source. For Phase 1 a small bundled JSON is compiled in.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths::Dirs;
use super::steam::SteamApp;

const BUNDLED: &str = include_str!("../../manifests/saves.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveRule {
    pub name: Option<String>,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveManifest {
    #[serde(default)]
    pub games: BTreeMap<String, SaveRule>,
}

/// Load the compiled-in starter manifest.
pub fn bundled() -> SaveManifest {
    serde_json::from_str(BUNDLED).expect("bundled manifest must be valid JSON")
}

/// Resolve a rule to a concrete path for a Steam `app`.
pub fn resolve(rule: &SaveRule, app: &SteamApp) -> Option<PathBuf> {
    resolve_in(rule, Some(&app.install_dir))
}

/// Resolve a rule to a concrete path given an optional install directory (used
/// for `{INSTALL_DIR}` templates). Prefers a template whose placeholders all
/// resolve *and* which exists on disk; otherwise falls back to the first
/// fully-resolvable template (a suggestion the user can confirm). Shared by the
/// Steam, GOG, and Epic scanners.
pub fn resolve_in(rule: &SaveRule, install_dir: Option<&Path>) -> Option<PathBuf> {
    let dirs = Dirs::detect();
    let mut first_resolvable: Option<PathBuf> = None;
    for template in &rule.paths {
        if let Some(path) = dirs.expand(template, install_dir) {
            if path.exists() {
                return Some(path);
            }
            if first_resolvable.is_none() {
                first_resolvable = Some(path);
            }
        }
    }
    first_resolvable
}

/// Normalize a game name for cross-store matching: lowercase, alphanumerics
/// only (so "Dark Souls III™" and "dark souls iii" both become "darksoulsiii").
/// GOG/Epic carry no Steam appid locally, so we match their games into the
/// (Steam-appid-keyed) manifest by name instead.
pub fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Build a normalized-name → rule index from a rule set, for stores whose local
/// metadata has no Steam appid. Rules without a name, or that collapse to an
/// empty key, are skipped; on a name collision the later rule wins.
pub fn name_index(rules: &BTreeMap<String, SaveRule>) -> HashMap<String, SaveRule> {
    let mut idx = HashMap::new();
    for rule in rules.values() {
        if let Some(name) = &rule.name {
            let key = normalize_name(name);
            if !key.is_empty() {
                idx.insert(key, rule.clone());
            }
        }
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_manifest_parses_and_is_large() {
        let m = bundled();
        assert!(
            m.games.len() >= 30,
            "expected a sizable manifest, got {}",
            m.games.len()
        );
        assert!(
            m.games.contains_key("374320"),
            "Dark Souls III should be present"
        );
        assert!(
            m.games.values().all(|r| !r.paths.is_empty()),
            "every rule must have at least one path"
        );
    }

    #[test]
    fn normalize_name_strips_case_punctuation_and_trademarks() {
        assert_eq!(normalize_name("Dark Souls III"), "darksoulsiii");
        assert_eq!(normalize_name("DARK SOULS III™"), "darksoulsiii");
        assert_eq!(
            normalize_name("Tom Clancy's: Splinter-Cell"),
            "tomclancyssplintercell"
        );
        assert_eq!(normalize_name("   "), "");
    }

    #[test]
    fn name_index_keys_by_normalized_name() {
        let mut rules = BTreeMap::new();
        rules.insert(
            "374320".to_string(),
            SaveRule {
                name: Some("Dark Souls III".to_string()),
                paths: vec!["{APPDATA}/DarkSoulsIII".to_string()],
            },
        );
        // A rule with no name is not indexable by name.
        rules.insert(
            "999".to_string(),
            SaveRule {
                name: None,
                paths: vec!["{HOME}/x".to_string()],
            },
        );
        let idx = name_index(&rules);
        assert!(idx.contains_key("darksoulsiii"));
        assert_eq!(idx.len(), 1, "the nameless rule must be skipped");
    }
}
