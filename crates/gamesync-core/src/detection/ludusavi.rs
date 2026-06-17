//! Import the community save-path manifest (the Ludusavi manifest, derived from
//! PCGamingWiki — ~13k games) and translate it into GameSync's per-appid
//! `SaveRule` format, so detection can cover far more games than the bundled
//! starter set.
//!
//! The Ludusavi manifest is a big YAML keyed by game name; each game may carry a
//! `steam: { id }` and a `files` map of path-spec → metadata (`tags`, `when`).
//! We keep only entries with a Steam id and a save-tagged (or untagged) path,
//! translating Ludusavi placeholders to ours and trimming each path to the
//! deepest folder we can actually resolve (no wildcards / unknown placeholders).

use std::collections::BTreeMap;

use super::manifest::SaveRule;
use crate::error::{Error, Result};

/// Raw community manifest (CC-BY-SA, derived from PCGamingWiki via Ludusavi).
pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";

const PLACEHOLDERS: &[(&str, &str)] = &[
    ("<winAppData>", "{APPDATA}"),
    ("<winLocalAppData>", "{LOCALAPPDATA}"),
    ("<winDocuments>", "{DOCUMENTS}"),
    ("<home>", "{HOME}"),
    ("<xdgData>", "{XDG_DATA}"),
    ("<xdgConfig>", "{XDG_CONFIG}"),
    ("<base>", "{INSTALL_DIR}"),
];

/// Download the manifest YAML via `curl` (present on modern macOS/Windows/Linux).
pub fn fetch(url: &str) -> Result<String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "120", url])
        .output()
        .map_err(|e| Error::other(format!("could not run curl to fetch the game list: {e}")))?;
    if !out.status.success() {
        return Err(Error::other(format!(
            "downloading the game list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    String::from_utf8(out.stdout).map_err(|_| Error::other("game list was not valid UTF-8"))
}

/// Parse the Ludusavi YAML into appid → `SaveRule`. Walks the document
/// defensively so one odd entry never aborts the whole import.
pub fn parse(yaml: &str) -> BTreeMap<String, SaveRule> {
    let root: serde_yaml::Value = match serde_yaml::from_str(yaml) {
        Ok(v) => v,
        Err(_) => return BTreeMap::new(),
    };
    let mut out = BTreeMap::new();
    let Some(map) = root.as_mapping() else {
        return out;
    };
    for (name_v, game_v) in map {
        let Some(name) = name_v.as_str() else {
            continue;
        };
        let appid = game_v
            .get("steam")
            .and_then(|s| s.get("id"))
            .and_then(|i| i.as_u64());
        let Some(appid) = appid else { continue };

        let mut paths: Vec<String> = Vec::new();
        if let Some(files) = game_v.get("files").and_then(|f| f.as_mapping()) {
            for (path_v, meta_v) in files {
                let Some(path) = path_v.as_str() else {
                    continue;
                };
                // Untagged entries are save data in Ludusavi; otherwise require "save".
                let is_save = match meta_v.get("tags").and_then(|t| t.as_sequence()) {
                    None => true,
                    Some(tags) => tags.iter().any(|t| t.as_str() == Some("save")),
                };
                if !is_save {
                    continue;
                }
                if let Some(p) = translate(path) {
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
            }
        }
        if paths.is_empty() {
            continue;
        }
        out.insert(
            appid.to_string(),
            SaveRule {
                name: Some(name.to_string()),
                paths,
            },
        );
    }
    out
}

/// Translate a Ludusavi path to a GameSync template, trimmed to the deepest
/// folder with no wildcard or unresolvable placeholder. Returns `None` if the
/// result would be too broad (just a root placeholder, e.g. the whole AppData).
fn translate(path: &str) -> Option<String> {
    let mut p = path.replace('\\', "/");
    for (lud, gs) in PLACEHOLDERS {
        p = p.replace(lud, gs);
    }
    let mut segs: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        if seg.is_empty() {
            continue;
        }
        if seg.contains('<') || seg.contains('*') {
            break;
        }
        segs.push(seg);
    }
    // Need a placeholder root plus at least one concrete subfolder.
    if segs.len() < 2 || !segs[0].starts_with('{') {
        return None;
    }
    Some(segs.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_translates_ludusavi_subset() {
        let yaml = r#"
Dark Souls III:
  files:
    <winAppData>/DarkSoulsIII/<storeUserId>:
      tags:
        - save
  steam:
    id: 374320
Config Only Game:
  files:
    <winAppData>/Foo/settings.ini:
      tags:
        - config
  steam:
    id: 999
No Steam Game:
  files:
    <winDocuments>/Bar/Saves: {}
Untagged Save Game:
  files:
    <winDocuments>/Baz/Saves: {}
  steam:
    id: 1000
Too Broad Game:
  files:
    <base>/save*: {}
  steam:
    id: 1001
"#;
        let m = parse(yaml);
        assert_eq!(
            m["374320"].paths,
            vec!["{APPDATA}/DarkSoulsIII".to_string()]
        );
        assert!(
            !m.contains_key("999"),
            "config-only games have no save paths"
        );
        assert!(!m
            .values()
            .any(|r| r.name.as_deref() == Some("No Steam Game")));
        assert_eq!(m["1000"].paths, vec!["{DOCUMENTS}/Baz/Saves".to_string()]);
        assert!(
            !m.contains_key("1001"),
            "a whole-install-dir path is too broad"
        );
    }
}
