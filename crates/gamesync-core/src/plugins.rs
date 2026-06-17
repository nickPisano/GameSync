//! Plugin system: drop-in JSON files in the data dir's `plugins/` folder.
//!
//! A plugin can declare any combination of:
//!  - **game definitions** — extra Steam appid / emulator save-path rules merged
//!    into detection (pure data, always safe);
//!  - **hooks** — commands run before/after a backup or restore;
//!  - **viewers** — commands to open matching save files with an external tool.
//!
//! Hooks and viewers run shell commands, so they only execute when the user has
//! explicitly enabled "allow plugins to run commands". Plugins are loaded only
//! from the local plugins folder — never from synced or network data.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::detection::emulators::EmulatorRule;
use crate::detection::manifest::SaveRule;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginHooks {
    pub pre_backup: Option<String>,
    pub post_backup: Option<String>,
    pub pre_restore: Option<String>,
    pub post_restore: Option<String>,
}

/// Associates a file pattern with an external program to open it.
#[derive(Debug, Clone, Deserialize)]
pub struct Viewer {
    pub name: String,
    /// Glob matched against a file's name, e.g. `*.sl2`.
    #[serde(rename = "match")]
    pub pattern: String,
    /// Command with a `{file}` placeholder, e.g. `hexedit {file}`.
    pub command: String,
}

/// The on-disk shape of a plugin file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginFile {
    pub name: Option<String>,
    #[serde(default)]
    pub games: BTreeMap<String, SaveRule>,
    #[serde(default)]
    pub emulators: BTreeMap<String, EmulatorRule>,
    #[serde(default)]
    pub hooks: PluginHooks,
    #[serde(default)]
    pub viewers: Vec<Viewer>,
}

#[derive(Debug, Clone)]
pub struct Plugin {
    /// Filename stem — the stable id used for enable/disable.
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub file: PluginFile,
}

/// Load every `*.json` plugin in `dir`. Returns the parsed plugins plus
/// `(id, error)` for any that failed to parse (surfaced in the UI).
pub fn load_plugins(dir: &Path) -> (Vec<Plugin>, Vec<(String, String)>) {
    let mut plugins = Vec::new();
    let mut errors = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return (plugins, errors),
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin")
            .to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<PluginFile>(&text) {
                Ok(file) => {
                    let name = file.name.clone().unwrap_or_else(|| id.clone());
                    plugins.push(Plugin {
                        id,
                        name,
                        path,
                        file,
                    });
                }
                Err(e) => errors.push((id, e.to_string())),
            },
            Err(e) => errors.push((id, e.to_string())),
        }
    }
    plugins.sort_by(|a, b| a.id.cmp(&b.id));
    (plugins, errors)
}

/// Replace `{key}` placeholders in a command template.
pub fn substitute(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// Does a viewer's glob match this file name?
pub fn viewer_matches(pattern: &str, file_name: &str) -> bool {
    globset::Glob::new(pattern)
        .map(|g| g.compile_matcher().is_match(file_name))
        .unwrap_or(false)
}

fn shell(cmd: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    }
}

/// Run a command and wait for it (used for hooks — the action waits on it).
pub fn run_blocking(cmd: &str) -> Result<()> {
    let status = shell(cmd).status()?;
    if !status.success() {
        return Err(Error::other(format!("plugin command failed: {cmd}")));
    }
    Ok(())
}

/// Launch a command without waiting (used for viewers — typically a GUI app).
pub fn run_detached(cmd: &str) -> Result<()> {
    shell(cmd).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_and_matches() {
        assert_eq!(
            substitute("echo {a} {b}", &[("a", "1"), ("b", "2")]),
            "echo 1 2"
        );
        assert!(viewer_matches("*.sl2", "DS30000.sl2"));
        assert!(!viewer_matches("*.sl2", "save.dat"));
    }

    #[test]
    fn loads_plugin_with_game_and_viewer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("p.json"),
            r#"{ "name": "Test", "games": { "steam:1": { "name": "G", "paths": ["{HOME}/g"] } },
                "viewers": [ { "name": "Hex", "match": "*.sav", "command": "hexedit {file}" } ] }"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("bad.json"), "{ not json").unwrap();

        let (plugins, errors) = load_plugins(dir.path());
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "Test");
        assert_eq!(plugins[0].file.games.len(), 1);
        assert_eq!(plugins[0].file.viewers.len(), 1);
        assert_eq!(errors.len(), 1, "the malformed file is reported");
    }
}
