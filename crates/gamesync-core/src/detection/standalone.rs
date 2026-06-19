//! Standalone (non-store) games — free/itch/modpack titles that carry no
//! Steam/GOG/Epic id, so they can't be matched the usual way. They're detected
//! by probing a fixed save path, reusing the emulator path-probe machinery
//! ([`super::emulators::detect_from`]); each entry is just a `{name, paths}`
//! rule keyed by a stable id.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::detection::emulators::EmulatorRule;

const BUNDLED: &str = include_str!("../../manifests/standalone.json");

#[derive(Deserialize)]
struct StandaloneManifest {
    #[serde(default)]
    games: BTreeMap<String, EmulatorRule>,
}

/// The bundled standalone-game rules (key → `{name, paths}`).
pub fn bundled_rules() -> BTreeMap<String, EmulatorRule> {
    serde_json::from_str::<StandaloneManifest>(BUNDLED)
        .expect("bundled standalone manifest must be valid JSON")
        .games
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
