//! Per-OS base directories and template expansion, shared by all save-path
//! resolvers (Steam manifest, emulators, …).
//!
//! Templates use `{PLACEHOLDER}` tokens. Expansion is pure given a [`Dirs`], so
//! it is unit-testable without touching the real filesystem or environment.

use std::path::{Path, PathBuf};

/// Resolved base directories for the current platform. Any that don't apply to
/// this OS are `None`, and templates referencing them are skipped.
#[derive(Debug, Clone, Default)]
pub struct Dirs {
    pub home: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub appdata: Option<PathBuf>,      // Windows %APPDATA% (Roaming)
    pub localappdata: Option<PathBuf>, // Windows %LOCALAPPDATA%
    pub saved_games: Option<PathBuf>,  // Windows %USERPROFILE%\Saved Games
    pub app_support: Option<PathBuf>,  // macOS ~/Library/Application Support
    pub xdg_data: Option<PathBuf>,     // Linux $XDG_DATA_HOME or ~/.local/share
    pub xdg_config: Option<PathBuf>,   // Linux $XDG_CONFIG_HOME or ~/.config
}

impl Dirs {
    /// Detect the real base directories for the running OS.
    pub fn detect() -> Self {
        let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
        let documents = directories::UserDirs::new()
            .and_then(|u| u.document_dir().map(Path::to_path_buf))
            .or_else(|| home.as_ref().map(|h| h.join("Documents")));

        let app_support = if cfg!(target_os = "macos") {
            home.as_ref().map(|h| h.join("Library/Application Support"))
        } else {
            None
        };

        let (xdg_data, xdg_config) = if cfg!(all(unix, not(target_os = "macos"))) {
            let data = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(".config")));
            (data, config)
        } else {
            (None, None)
        };

        Dirs {
            home,
            documents,
            appdata: std::env::var_os("APPDATA").map(PathBuf::from),
            localappdata: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            saved_games: std::env::var_os("USERPROFILE")
                .map(|p| PathBuf::from(p).join("Saved Games")),
            app_support,
            xdg_data,
            xdg_config,
        }
    }

    fn lookup(&self, key: &str, install_dir: Option<&Path>) -> Option<PathBuf> {
        match key {
            "INSTALL_DIR" => install_dir.map(Path::to_path_buf),
            "HOME" | "USERPROFILE" => self.home.clone(),
            "DOCUMENTS" => self.documents.clone(),
            "APPDATA" => self.appdata.clone(),
            "LOCALAPPDATA" => self.localappdata.clone(),
            "SAVEDGAMES" => self.saved_games.clone(),
            "APPSUPPORT" => self.app_support.clone(),
            "XDG_DATA" => self.xdg_data.clone(),
            "XDG_CONFIG" => self.xdg_config.clone(),
            _ => None,
        }
    }

    /// Expand a `{PLACEHOLDER}`-style template. Returns `None` if any
    /// placeholder can't be resolved on this platform (so the caller skips it).
    pub fn expand(&self, template: &str, install_dir: Option<&Path>) -> Option<PathBuf> {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let end = rest[start..].find('}')? + start;
            let key = &rest[start + 1..end];
            let value = self.lookup(key, install_dir)?;
            out.push_str(&value.to_string_lossy());
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        Some(PathBuf::from(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_dirs() -> Dirs {
        Dirs {
            home: Some(PathBuf::from("/home/u")),
            documents: Some(PathBuf::from("/home/u/Documents")),
            xdg_data: Some(PathBuf::from("/home/u/.local/share")),
            ..Default::default()
        }
    }

    #[test]
    fn expands_known_placeholders() {
        let d = fake_dirs();
        assert_eq!(
            d.expand("{DOCUMENTS}/PCSX2/memcards", None),
            Some(PathBuf::from("/home/u/Documents/PCSX2/memcards"))
        );
        assert_eq!(
            d.expand("{INSTALL_DIR}/saves", Some(Path::new("/games/foo"))),
            Some(PathBuf::from("/games/foo/saves"))
        );
    }

    #[test]
    fn unresolvable_placeholder_yields_none() {
        let d = fake_dirs();
        // APPDATA is unset in fake_dirs (non-Windows), so this template is skipped.
        assert_eq!(d.expand("{APPDATA}/Game", None), None);
    }
}
