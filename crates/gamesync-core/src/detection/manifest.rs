//! The save-path manifest: a mapping from Steam appid to one or more save-path
//! templates, plus a resolver that expands templates for the current OS/user.
//!
//! In production this manifest would be signed and refreshed from a community
//! source. For Phase 1 a small bundled JSON is compiled in.

use std::collections::BTreeMap;
use std::path::PathBuf;

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

/// Resolve a rule to a concrete path for `app`. Prefers a template whose
/// placeholders all resolve *and* which exists on disk; otherwise falls back to
/// the first fully-resolvable template (a suggestion the user can confirm).
pub fn resolve(rule: &SaveRule, app: &SteamApp) -> Option<PathBuf> {
    let dirs = Dirs::detect();
    let mut first_resolvable: Option<PathBuf> = None;
    for template in &rule.paths {
        if let Some(path) = dirs.expand(template, Some(&app.install_dir)) {
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
}
