//! Emulator detection. Emulators store all saves in a single location, so each
//! entry resolves to one save root that exists on disk.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use super::paths::Dirs;

const BUNDLED: &str = include_str!("../../manifests/emulators.json");

#[derive(Debug, Clone, Deserialize)]
pub struct EmulatorRule {
    pub name: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct EmulatorManifest {
    #[serde(default)]
    emulators: BTreeMap<String, EmulatorRule>,
}

/// A detected emulator and the save folder found for it.
#[derive(Debug, Clone)]
pub struct DetectedEmulator {
    pub key: String,
    pub name: String,
    pub save_root: PathBuf,
}

fn manifest() -> EmulatorManifest {
    serde_json::from_str(BUNDLED).expect("bundled emulator manifest must be valid JSON")
}

/// The bundled emulator rules (key → rule), so callers can merge in plugin rules.
pub fn bundled_rules() -> BTreeMap<String, EmulatorRule> {
    manifest().emulators
}

/// Detect emulators (using the bundled rules) whose save folder exists.
pub fn detect() -> Vec<DetectedEmulator> {
    detect_from(&bundled_rules())
}

/// Detect emulators from an explicit rule set (bundled + plugins).
pub fn detect_from(rules: &BTreeMap<String, EmulatorRule>) -> Vec<DetectedEmulator> {
    let dirs = Dirs::detect();
    let mut out = Vec::new();
    for (key, rule) in rules {
        for template in &rule.paths {
            if let Some(path) = dirs.expand(template, None) {
                if path.is_dir() {
                    out.push(DetectedEmulator {
                        key: key.clone(),
                        name: rule.name.clone(),
                        save_root: path,
                    });
                    break; // first existing path wins
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_manifest_parses_and_has_entries() {
        let m = manifest();
        assert!(m.emulators.contains_key("dolphin"));
        assert!(m.emulators.contains_key("pcsx2"));
        assert!(!m.emulators["dolphin"].paths.is_empty());
    }
}
