//! The engine ties storage, detection, snapshotting, and restore together into
//! the operations the UI/CLI call. It owns the DB connection and CAS, and is
//! the single entry point for everything stateful.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cas::Cas;
use crate::crypto::{Dek, KeyStore, RecoveryKey};
use crate::db::Db;
use crate::detection::emulators::EmulatorRule;
use crate::detection::manifest::SaveRule;
use crate::detection::{emulators, epic, gog, ludusavi, manifest, standalone, steam};
use crate::error::{Error, Result};
use crate::model::{Game, Head, Platform, Snapshot, SnapshotKind, VectorClock};
use crate::plugins;
use crate::remote::{lan, FolderRemote, LanRemote, LanServerHandle, RcloneRemote, Remote};
use crate::restore::{restore_version, RestoreOptions};
use crate::retention::{self, GcReport, RetentionPolicy};
use crate::snapshot::{
    create_snapshot, create_snapshot_if_changed, head_base, DEFAULT_EXCLUDES, DEFAULT_INCLUDES,
};
use crate::vclock;
use crate::{process, restore};

const CFG_REMOTE: &str = "remote_path";
const CFG_AUTO_SYNC: &str = "auto_sync_enabled";
const CFG_AUTO_INTERVAL: &str = "auto_sync_interval_min";
const CFG_SETUP_DONE: &str = "setup_complete";
const CFG_BACKUP_ON_EXIT: &str = "backup_on_exit";
const CFG_COMPRESS: &str = "compress_objects";
const CFG_DISABLED_PLUGINS: &str = "disabled_plugins";
const CFG_PLUGIN_CMDS: &str = "plugin_commands_allowed";

/// Summary of an installed plugin for the Plugins screen.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub games: usize,
    pub emulators: usize,
    pub hooks: usize,
    pub viewers: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginList {
    pub dir: String,
    /// Whether the user has opted in to letting plugins run commands.
    pub commands_allowed: bool,
    pub plugins: Vec<PluginInfo>,
    /// (plugin id, parse error) for files that failed to load.
    pub errors: Vec<(String, String)>,
}

/// A file-viewer offered by a plugin for a particular file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ViewerInfo {
    pub plugin: String,
    pub name: String,
    pub command: String,
}

/// Auto-sync configuration.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct AutoSyncSettings {
    pub enabled: bool,
    pub interval_min: u64,
    /// Back up (and sync) a game automatically a few seconds after it closes.
    #[serde(default = "default_true")]
    pub backup_on_exit: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AutoSyncSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_min: 15,
            backup_on_exit: true,
        }
    }
}

/// A game that ended a sync in conflict, with the diverging version ids so the
/// UI can diff local vs remote.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConflictInfo {
    pub game_id: String,
    pub game_name: String,
    pub local: String,
    pub remote: String,
}

/// Summary of one automatic sync pass over all enabled games.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct AutoSyncReport {
    pub games: usize,
    pub backed_up: usize,
    pub pushed: usize,
    pub pulled: usize,
    pub in_sync: usize,
    pub skipped_running: usize,
    pub conflicts: Vec<ConflictInfo>,
    /// (game_name, error message) for games that failed this pass.
    pub errors: Vec<(String, String)>,
}

/// The result of a sync attempt for one game.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncOutcome {
    /// Local and remote already agree.
    InSync,
    /// Local was ahead; the local head was uploaded.
    Pushed { version_id: String },
    /// Remote was ahead; it was downloaded and restored into the save folder.
    Pulled { version_id: String },
    /// Both sides diverged. Nothing was overwritten; resolve with
    /// [`Engine::resolve_conflict`].
    Conflict { local: String, remote: String },
}

/// How to resolve a divergence between two devices.
#[derive(Debug, Clone, Copy)]
pub enum ConflictChoice {
    KeepLocal,
    KeepRemote,
}

/// Deduplicated storage footprint of one game (distinct objects its versions
/// reference). Objects shared with other games are counted in each.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GameStorage {
    pub game_id: String,
    pub name: String,
    pub versions: usize,
    pub bytes: u64,
}

/// Overall storage usage of the content store.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StorageReport {
    pub total_objects: usize,
    /// Actual bytes on disk in the store (after dedup; compressed/encrypted size).
    pub total_bytes: u64,
    /// Whether LZMA2 compression is currently enabled for stored objects.
    pub compressed: bool,
    pub games: Vec<GameStorage>,
}

/// Result of redirecting a save folder via a symlink.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RedirectReport {
    /// Where the saves now physically live (and what the symlink points to).
    pub linked_target: String,
    /// Where the original folder was moved for safety (delete once verified).
    pub original_backup: String,
}

/// A file currently present in a game's save folder (for the in-app file view).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SaveFile {
    /// Path relative to the save root, forward-slash separated.
    pub rel_path: String,
    /// Absolute path on disk (for "reveal in file manager").
    pub abs_path: String,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Options for a backup operation.
#[derive(Debug, Clone)]
pub struct BackupOptions {
    /// Snapshot even if the game appears to be running. Dangerous; off by default.
    pub force: bool,
    /// Wait for the save folder to go quiet before snapshotting.
    pub wait_quiescent: bool,
    pub kind: SnapshotKind,
    pub label: Option<String>,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            force: false,
            wait_quiescent: false,
            kind: SnapshotKind::Manual,
            label: None,
        }
    }
}

/// Result of an integrity check.
#[derive(Debug, Default)]
pub struct VerifyReport {
    pub versions_checked: usize,
    pub objects_checked: usize,
    /// (version_id, rel_path, reason) for each problem found.
    pub problems: Vec<(String, String, String)>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub struct Engine {
    pub db: Db,
    pub cas: Cas,
    pub device_id: String,
    pub data_dir: PathBuf,
}

impl Engine {
    /// Default data directory: `$GAMESYNC_DATA` if set, else the per-OS app
    /// data dir (`%APPDATA%`, `~/Library/Application Support`, `$XDG_DATA_HOME`).
    pub fn default_data_dir() -> PathBuf {
        if let Ok(p) = std::env::var("GAMESYNC_DATA") {
            return PathBuf::from(p);
        }
        directories::ProjectDirs::from("dev", "GameSync", "GameSync")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".gamesync"))
    }

    /// Open the store. If the store is encrypted, this returns [`Error::Locked`]
    /// — call [`Engine::unlock`] (or [`Engine::unlock_with_recovery`]) instead.
    pub fn open(data_dir: PathBuf) -> Result<Self> {
        if Self::is_encrypted(&data_dir) {
            return Err(Error::Locked);
        }
        std::fs::create_dir_all(&data_dir)?;
        let db = Db::open(&data_dir.join("gamesync.db"))?;
        let cas = Cas::open(data_dir.join("store"))?;
        cas.set_compress(db.get_config(CFG_COMPRESS)?.as_deref() == Some("1"));
        let device_id = db.device_id()?;
        Ok(Self {
            db,
            cas,
            device_id,
            data_dir,
        })
    }

    fn keystore_path(data_dir: &Path) -> PathBuf {
        data_dir.join("keystore.json")
    }

    /// Whether this data store has encryption enabled.
    pub fn is_encrypted(data_dir: &Path) -> bool {
        Self::keystore_path(data_dir).is_file()
    }

    /// Turn on client-side encryption for a *fresh* store and return the
    /// recovery key (shown to the user exactly once). Refuses if the store
    /// already contains data, since transparent re-encryption isn't implemented
    /// yet.
    pub fn init_encryption(data_dir: &Path, passphrase: &str) -> Result<RecoveryKey> {
        std::fs::create_dir_all(data_dir)?;
        if Self::is_encrypted(data_dir) {
            return Err(Error::other("encryption is already enabled for this store"));
        }
        let cas = Cas::open(data_dir.join("store"))?;
        if !cas.list_objects()?.is_empty() {
            return Err(Error::other(
                "cannot enable encryption on a store that already has data \
                 (re-encryption of existing saves is not yet supported)",
            ));
        }
        let (keystore, recovery) = KeyStore::init(passphrase)?;
        let json = serde_json::to_vec_pretty(&keystore)?;
        crate::util::atomic_write(&Self::keystore_path(data_dir), &json)?;
        Ok(recovery)
    }

    /// Disable client-side encryption: decrypt every stored object back to
    /// plaintext-at-rest (the compression mode is preserved) and remove the
    /// keystore. Your saves stay intact — only the at-rest encryption is undone.
    ///
    /// The store must be currently unlocked (this engine holds the data key).
    /// Objects are decrypted *before* the keystore is removed, so an interruption
    /// is recoverable: the keystore stays until the data is fully plaintext, and
    /// re-running finishes the job (only still-encrypted objects are rewritten).
    /// After this returns, reopen the store with [`Engine::open`].
    pub fn disable_encryption(&self) -> Result<()> {
        if !self.cas.is_encrypted() {
            return Ok(());
        }
        self.cas.decrypt_all_to_plaintext()?;
        let ks = Self::keystore_path(&self.data_dir);
        if ks.is_file() {
            std::fs::remove_file(&ks)?;
        }
        Ok(())
    }

    /// Open an encrypted store using the passphrase.
    pub fn unlock(data_dir: PathBuf, passphrase: &str) -> Result<Self> {
        let keystore = Self::load_keystore(&data_dir)?;
        let dek = keystore.unlock_with_passphrase(passphrase)?;
        Self::open_with_dek(data_dir, &dek)
    }

    /// Open an encrypted store using the recovery key.
    pub fn unlock_with_recovery(data_dir: PathBuf, recovery_hex: &str) -> Result<Self> {
        let keystore = Self::load_keystore(&data_dir)?;
        let dek = keystore.unlock_with_recovery(recovery_hex)?;
        Self::open_with_dek(data_dir, &dek)
    }

    fn load_keystore(data_dir: &Path) -> Result<KeyStore> {
        let path = Self::keystore_path(data_dir);
        if !path.is_file() {
            return Err(Error::other("this store is not encrypted"));
        }
        let text = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&text)?)
    }

    fn open_with_dek(data_dir: PathBuf, dek: &Dek) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let db = Db::open(&data_dir.join("gamesync.db"))?;
        let cas = Cas::open_encrypted(data_dir.join("store"), dek)?;
        cas.set_compress(db.get_config(CFG_COMPRESS)?.as_deref() == Some("1"));
        let device_id = db.device_id()?;
        Ok(Self {
            db,
            cas,
            device_id,
            data_dir,
        })
    }

    // ---- detection -------------------------------------------------------

    /// Scan Steam, resolve known save paths via the manifest, and upsert the
    /// matched games. Returns the games found this scan. User toggles are
    /// preserved across scans.
    pub fn scan_steam(&self) -> Result<Vec<Game>> {
        let rules = self.merged_game_rules();
        let mut found = Vec::new();
        for app in steam::installed_apps() {
            let Some(rule) = rules.get(&app.appid) else {
                continue;
            };
            let Some(save_root) = manifest::resolve(rule, &app) else {
                continue;
            };
            let id = format!("steam:{}", app.appid);
            let previous = self.db.get_game(&id).ok();
            let game = Game {
                id,
                name: rule.name.clone().unwrap_or_else(|| app.name.clone()),
                platform: Platform::Steam,
                save_root,
                install_dir: Some(app.install_dir.clone()),
                includes: default_includes(),
                excludes: default_excludes(),
                sync_enabled: previous.as_ref().map(|p| p.sync_enabled).unwrap_or(false),
                extra_roots: previous
                    .as_ref()
                    .map(|p| p.extra_roots.clone())
                    .unwrap_or_default(),
            };
            self.db.upsert_game(&game)?;
            found.push(game);
        }
        Ok(found)
    }

    /// Scan installed emulators and register those whose save folder exists.
    /// User toggles are preserved across scans.
    pub fn scan_emulators(&self) -> Result<Vec<Game>> {
        let rules = self.merged_emulator_rules();
        let mut found = Vec::new();
        for emu in emulators::detect_from(&rules) {
            let id = format!("emu:{}", emu.key);
            let previous = self.db.get_game(&id).ok();
            let game = Game {
                id,
                name: emu.name,
                platform: Platform::Emulator,
                save_root: emu.save_root,
                install_dir: None,
                includes: default_includes(),
                excludes: default_excludes(),
                sync_enabled: previous.as_ref().map(|p| p.sync_enabled).unwrap_or(false),
                extra_roots: previous
                    .as_ref()
                    .map(|p| p.extra_roots.clone())
                    .unwrap_or_default(),
            };
            self.db.upsert_game(&game)?;
            found.push(game);
        }
        Ok(found)
    }

    /// Detect standalone (non-store) games — free/itch/modpack titles with no
    /// store id — by probing their fixed save folder. Reuses the emulator
    /// path-probe; a game is added only if its save folder exists.
    pub fn scan_standalone(&self) -> Result<Vec<Game>> {
        let mut found = Vec::new();
        for d in standalone::detect() {
            let id = format!("standalone:{}", d.key);
            let previous = self.db.get_game(&id).ok();
            let game = Game {
                id,
                name: d.name,
                platform: Platform::Standalone,
                save_root: d.save_root,
                install_dir: d.install_dir,
                includes: default_includes(),
                excludes: default_excludes(),
                sync_enabled: previous.as_ref().map(|p| p.sync_enabled).unwrap_or(false),
                extra_roots: previous
                    .as_ref()
                    .map(|p| p.extra_roots.clone())
                    .unwrap_or_default(),
            };
            self.db.upsert_game(&game)?;
            found.push(game);
        }
        Ok(found)
    }

    /// Scan GOG games under the default library roots ([`gog::library_roots`]).
    /// Saves are resolved by matching each game's name into the save manifest.
    pub fn scan_gog(&self) -> Result<Vec<Game>> {
        self.scan_gog_in(&gog::library_roots())
    }

    /// Scan GOG games under explicit library roots. Exposed so callers can point
    /// at non-default install locations (and for tests).
    pub fn scan_gog_in(&self, roots: &[PathBuf]) -> Result<Vec<Game>> {
        let index = manifest::name_index(&self.merged_game_rules());
        let mut found = Vec::new();
        for app in gog::apps_in(roots) {
            let id = format!("gog:{}", app.game_id);
            if let Some(game) =
                self.register_named_game(&index, id, &app.name, app.install_dir, Platform::Gog)?
            {
                found.push(game);
            }
        }
        Ok(found)
    }

    /// Scan Epic games from the launcher's manifests directory. Saves are
    /// resolved by matching each game's display name into the save manifest.
    pub fn scan_epic(&self) -> Result<Vec<Game>> {
        match epic::manifests_dir() {
            Some(dir) => self.scan_epic_in(&dir),
            None => Ok(Vec::new()),
        }
    }

    /// Scan Epic games from an explicit manifests directory (and for tests).
    pub fn scan_epic_in(&self, manifests_dir: &Path) -> Result<Vec<Game>> {
        let index = manifest::name_index(&self.merged_game_rules());
        let mut found = Vec::new();
        for app in epic::apps_in(manifests_dir) {
            let id = format!("epic:{}", app.app_name);
            if let Some(game) =
                self.register_named_game(&index, id, &app.name, app.install_dir, Platform::Epic)?
            {
                found.push(game);
            }
        }
        Ok(found)
    }

    /// Resolve a detected game's save folder by matching its name into the
    /// manifest, then upsert it (preserving the user's sync toggle). Returns
    /// `None` when no rule matches or no save path resolves. Used by the GOG and
    /// Epic scanners, which (unlike Steam) have no appid to key the manifest on.
    fn register_named_game(
        &self,
        index: &HashMap<String, SaveRule>,
        id: String,
        detected_name: &str,
        install_dir: PathBuf,
        platform: Platform,
    ) -> Result<Option<Game>> {
        let Some(rule) = index.get(&manifest::normalize_name(detected_name)) else {
            return Ok(None);
        };
        let Some(save_root) = manifest::resolve_in(rule, Some(&install_dir)) else {
            return Ok(None);
        };
        let previous = self.db.get_game(&id).ok();
        let game = Game {
            id,
            name: rule
                .name
                .clone()
                .unwrap_or_else(|| detected_name.to_string()),
            platform,
            save_root,
            install_dir: Some(install_dir),
            includes: default_includes(),
            excludes: default_excludes(),
            sync_enabled: previous.as_ref().map(|p| p.sync_enabled).unwrap_or(false),
            extra_roots: previous
                .as_ref()
                .map(|p| p.extra_roots.clone())
                .unwrap_or_default(),
        };
        self.db.upsert_game(&game)?;
        Ok(Some(game))
    }

    /// Run all automatic detectors (Steam + GOG + Epic + emulators).
    pub fn scan_all(&self) -> Result<Vec<Game>> {
        let mut found = self.scan_steam()?;
        found.extend(self.scan_gog()?);
        found.extend(self.scan_epic()?);
        found.extend(self.scan_emulators()?);
        found.extend(self.scan_standalone()?);
        Ok(found)
    }

    // ---- plugins ---------------------------------------------------------

    pub fn plugins_dir(&self) -> PathBuf {
        self.data_dir.join("plugins")
    }

    fn disabled_plugins(&self) -> Vec<String> {
        self.db
            .get_config(CFG_DISABLED_PLUGINS)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Whether the user has opted in to letting plugins run shell commands
    /// (hooks and viewers). Off by default.
    pub fn commands_allowed(&self) -> bool {
        self.db
            .get_config(CFG_PLUGIN_CMDS)
            .ok()
            .flatten()
            .as_deref()
            == Some("1")
    }

    pub fn set_commands_allowed(&self, allowed: bool) -> Result<()> {
        self.db
            .set_config(CFG_PLUGIN_CMDS, if allowed { "1" } else { "0" })
    }

    /// Enabled plugins (loaded fresh from the plugins folder).
    fn enabled_plugins(&self) -> Vec<plugins::Plugin> {
        let disabled = self.disabled_plugins();
        let (plugins, _errors) = plugins::load_plugins(&self.plugins_dir());
        plugins
            .into_iter()
            .filter(|p| !disabled.contains(&p.id))
            .collect()
    }

    /// All known game rules: the downloaded community list (if any) as the base,
    /// overridden by the curated bundled manifest, overridden by enabled plugins.
    fn merged_game_rules(&self) -> BTreeMap<String, SaveRule> {
        let mut map = self.community_rules();
        for (k, v) in manifest::bundled().games {
            map.insert(k, v);
        }
        for plugin in self.enabled_plugins() {
            for (k, v) in plugin.file.games {
                map.insert(k, v);
            }
        }
        map
    }

    fn community_manifest_path(&self) -> PathBuf {
        self.data_dir.join("community-manifest.json")
    }

    /// The downloaded community rules (appid → rule), or empty if not fetched.
    fn community_rules(&self) -> BTreeMap<String, SaveRule> {
        std::fs::read_to_string(self.community_manifest_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Total number of games GameSync can auto-detect (community + bundled + plugins).
    pub fn known_game_count(&self) -> usize {
        self.merged_game_rules().len()
    }

    /// Download the community game list, translate it, and cache it. Returns how
    /// many games are now known in total. Network-bound (uses `curl`).
    pub fn update_game_list(&self) -> Result<usize> {
        let yaml = ludusavi::fetch(ludusavi::MANIFEST_URL)?;
        let rules = ludusavi::parse(&yaml);
        if rules.is_empty() {
            return Err(Error::other(
                "the downloaded game list was empty or could not be read",
            ));
        }
        crate::util::atomic_write(
            &self.community_manifest_path(),
            &serde_json::to_vec(&rules)?,
        )?;
        Ok(self.known_game_count())
    }

    fn merged_emulator_rules(&self) -> BTreeMap<String, EmulatorRule> {
        let mut map = emulators::bundled_rules();
        for plugin in self.enabled_plugins() {
            for (k, v) in plugin.file.emulators {
                map.insert(k, v);
            }
        }
        map
    }

    pub fn list_plugins(&self) -> Result<PluginList> {
        std::fs::create_dir_all(self.plugins_dir())?;
        let disabled = self.disabled_plugins();
        let (plugins, errors) = plugins::load_plugins(&self.plugins_dir());
        let infos = plugins
            .into_iter()
            .map(|p| PluginInfo {
                enabled: !disabled.contains(&p.id),
                games: p.file.games.len(),
                emulators: p.file.emulators.len(),
                hooks: [
                    &p.file.hooks.pre_backup,
                    &p.file.hooks.post_backup,
                    &p.file.hooks.pre_restore,
                    &p.file.hooks.post_restore,
                ]
                .iter()
                .filter(|h| h.is_some())
                .count(),
                viewers: p.file.viewers.len(),
                id: p.id,
                name: p.name,
            })
            .collect();
        Ok(PluginList {
            dir: self.plugins_dir().display().to_string(),
            commands_allowed: self.commands_allowed(),
            plugins: infos,
            errors,
        })
    }

    pub fn set_plugin_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut disabled = self.disabled_plugins();
        disabled.retain(|d| d != id);
        if !enabled {
            disabled.push(id.to_string());
        }
        self.db
            .set_config(CFG_DISABLED_PLUGINS, &serde_json::to_string(&disabled)?)
    }

    /// Viewers (from enabled plugins) that match a given absolute file path.
    pub fn file_viewers(&self, abs_path: &str) -> Vec<ViewerInfo> {
        let name = Path::new(abs_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(abs_path);
        let mut out = Vec::new();
        for plugin in self.enabled_plugins() {
            for v in &plugin.file.viewers {
                if plugins::viewer_matches(&v.pattern, name) {
                    out.push(ViewerInfo {
                        plugin: plugin.name.clone(),
                        name: v.name.clone(),
                        command: v.command.clone(),
                    });
                }
            }
        }
        out
    }

    /// Open a file with a viewer command (gated on the commands opt-in).
    pub fn run_viewer(&self, command: &str, abs_path: &str) -> Result<()> {
        if !self.commands_allowed() {
            return Err(Error::other(
                "plugin commands are disabled — enable them in Plugins settings first",
            ));
        }
        plugins::run_detached(&plugins::substitute(command, &[("file", abs_path)]))
    }

    /// Run a hook (chosen by `pick`) from each enabled plugin, if commands are
    /// allowed. Best-effort: a failing hook is ignored so it can't wedge a
    /// backup/restore.
    fn run_hooks(
        &self,
        pick: impl Fn(&plugins::PluginHooks) -> Option<String>,
        vars: &[(&str, &str)],
    ) {
        if !self.commands_allowed() {
            return;
        }
        for plugin in self.enabled_plugins() {
            if let Some(cmd) = pick(&plugin.file.hooks) {
                let _ = plugins::run_blocking(&plugins::substitute(&cmd, vars));
            }
        }
    }

    /// Register a game by hand (the manual-add flow). Validates that the save
    /// folder exists.
    pub fn add_manual_game(&self, name: &str, save_root: PathBuf) -> Result<Game> {
        if !save_root.exists() {
            return Err(Error::NotFound(format!(
                "save folder does not exist: {}",
                save_root.display()
            )));
        }
        let game = Game {
            // Derived from the name so the same game added on two devices shares
            // an id and can therefore sync.
            id: format!("manual:{}", crate::util::hash_id(name)),
            name: name.to_string(),
            platform: Platform::Manual,
            save_root,
            install_dir: None,
            includes: default_includes(),
            excludes: default_excludes(),
            sync_enabled: false,
            extra_roots: Vec::new(),
        };
        self.db.upsert_game(&game)?;
        Ok(game)
    }

    pub fn list_games(&self) -> Result<Vec<Game>> {
        self.db.list_games()
    }

    /// The files currently in a game's save folder (those that would be backed
    /// up), with absolute paths for revealing in the OS file manager.
    pub fn list_save_files(&self, game_id: &str) -> Result<Vec<SaveFile>> {
        let game = self.db.get_game(game_id)?;
        let roots = game.roots();
        let files = crate::snapshot::list_files(&game)?;
        Ok(files
            .into_iter()
            .map(|(root, rel, size, mtime_ms)| {
                let base = roots.get(root as usize).unwrap_or(&game.save_root);
                SaveFile {
                    abs_path: crate::util::rel_to_path(base, &rel)
                        .to_string_lossy()
                        .into_owned(),
                    rel_path: rel,
                    size,
                    mtime_ms,
                }
            })
            .collect())
    }

    /// Replace a game's extra backup roots (folders snapshotted/restored
    /// together with its save folder). Non-existent paths are kept but simply
    /// contribute nothing until they exist.
    pub fn set_extra_roots(&self, game_id: &str, roots: Vec<PathBuf>) -> Result<()> {
        self.db.set_extra_roots(game_id, &roots)
    }

    pub fn get_game(&self, id: &str) -> Result<Game> {
        self.db.get_game(id)
    }

    pub fn set_sync_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.db.set_sync_enabled(id, enabled)
    }

    /// Redirect a game's save folder into `target_base` (e.g. a cloud-synced
    /// folder) and leave a symlink behind so the game still finds its saves.
    ///
    /// Safety: takes a GameSync backup first, never deletes the original (it's
    /// renamed aside and the path is returned), verifies the link, and rolls
    /// back on failure. If the chosen folder already contains saves (e.g. synced
    /// from another device), it links to those instead of overwriting them.
    pub fn redirect_save_folder(
        &self,
        game_id: &str,
        target_base: PathBuf,
    ) -> Result<RedirectReport> {
        let game = self.db.get_game(game_id)?;
        let src = game.save_root.clone();

        let meta = std::fs::symlink_metadata(&src)
            .map_err(|_| Error::other("save folder does not exist"))?;
        if meta.file_type().is_symlink() {
            return Err(Error::other(
                "this save folder is already redirected (a symlink)",
            ));
        }
        if !src.is_dir() {
            return Err(Error::other("save root is not a directory"));
        }

        let safe: String = game
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let target = target_base.join(safe.trim());

        // Guard against nesting the target inside the source (or vice-versa).
        if target == src || target.starts_with(&src) || src.starts_with(&target) {
            return Err(Error::other(
                "target folder must be outside the save folder",
            ));
        }

        // 1. Safety backup of the current saves.
        let (base, parent) = head_base(&self.db, &game.id)?;
        let snap = create_snapshot(
            &self.db,
            &self.cas,
            &self.device_id,
            &game,
            SnapshotKind::Manual,
            Some("before redirecting save folder".to_string()),
            &base,
            parent,
        )?;
        self.db.set_head(&game.id, &snap.version_id)?;

        // 2. Make sure the target holds the saves. If it already has data (e.g.
        //    synced from another device), keep it; otherwise copy ours in.
        let target_has_data = target.is_dir()
            && std::fs::read_dir(&target)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
        if !target_has_data {
            crate::util::copy_dir_all(&src, &target)?;
        }

        // 3. Move the original aside (atomic rename, same parent) — never deleted.
        let orig_bak = src.with_file_name(format!(
            "{}.gamesync-orig-{}",
            src.file_name().and_then(|n| n.to_str()).unwrap_or("save"),
            crate::util::new_id()
        ));
        std::fs::rename(&src, &orig_bak)?;

        // 4. Symlink the original path to the target; roll back on failure.
        if let Err(e) = crate::util::make_dir_symlink(&target, &src) {
            let _ = std::fs::rename(&orig_bak, &src);
            return Err(Error::other(format!(
                "couldn't create the symlink (on Windows, enable Developer Mode or run as admin): {e}"
            )));
        }

        // 5. Verify the link resolves to a directory.
        if !src.is_dir() {
            let _ = std::fs::remove_file(&src);
            let _ = std::fs::rename(&orig_bak, &src);
            return Err(Error::Integrity("redirect could not be verified".into()));
        }

        // 6. Point GameSync at the real target so backups don't traverse the link.
        let mut updated = game;
        updated.save_root = target.clone();
        self.db.upsert_game(&updated)?;

        Ok(RedirectReport {
            linked_target: target.display().to_string(),
            original_backup: orig_bak.display().to_string(),
        })
    }

    /// Rename a game (display name only; its id and sync identity are unchanged).
    pub fn rename_game(&self, game_id: &str, name: &str) -> Result<()> {
        let mut game = self.db.get_game(game_id)?;
        game.name = name.to_string();
        self.db.upsert_game(&game)
    }

    /// Stop tracking a game and delete its backup history, then reclaim the
    /// freed space. The actual save folder on disk is left untouched.
    pub fn remove_game(&self, game_id: &str) -> Result<GcReport> {
        self.db.delete_game(game_id)?;
        retention::gc(&self.db, &self.cas)
    }

    /// Set (or clear) the folder/executable to watch so this game gets
    /// auto-backup-on-exit. Detection matches any running process whose exe
    /// lives under this path, so a game folder, an `.app`, or an exact
    /// executable all work. Lets manually-added games opt into exit detection.
    pub fn set_game_exe(&self, game_id: &str, path: Option<PathBuf>) -> Result<()> {
        let mut game = self.db.get_game(game_id)?;
        game.install_dir = path;
        self.db.upsert_game(&game)
    }

    // ---- backup / restore ------------------------------------------------

    pub fn backup(&self, game_id: &str, opts: BackupOptions) -> Result<Snapshot> {
        let game = self.db.get_game(game_id)?;
        if !opts.force && process::is_running(&game) {
            return Err(Error::GameRunning(game.name, "back up"));
        }
        if opts.wait_quiescent {
            process::wait_until_quiescent(
                &game.save_root,
                Duration::from_millis(1500),
                Duration::from_secs(30),
            );
        }
        let save_dir = game.save_root.to_string_lossy().into_owned();
        let vars = [
            ("game", game.name.as_str()),
            ("game_id", game.id.as_str()),
            ("save_dir", save_dir.as_str()),
        ];
        self.run_hooks(|h| h.pre_backup.clone(), &vars);

        let (base, parent) = head_base(&self.db, &game.id)?;
        let snap = create_snapshot(
            &self.db,
            &self.cas,
            &self.device_id,
            &game,
            opts.kind,
            opts.label,
            &base,
            parent,
        )?;
        self.db.set_head(&game.id, &snap.version_id)?;
        self.run_hooks(|h| h.post_backup.clone(), &vars);
        Ok(snap)
    }

    /// Snapshot only if the save set changed since the head version. Returns the
    /// new snapshot, or `None` when nothing changed. Used by auto-sync.
    pub fn backup_if_changed(&self, game_id: &str) -> Result<Option<Snapshot>> {
        let game = self.db.get_game(game_id)?;
        let head = self.head_snapshot(game_id)?;
        let (base, parent) = head_base(&self.db, &game.id)?;
        let snap = create_snapshot_if_changed(
            &self.db,
            &self.cas,
            &self.device_id,
            &game,
            SnapshotKind::Auto,
            None,
            &base,
            parent,
            head.as_ref(),
        )?;
        if let Some(s) = &snap {
            self.db.set_head(&game.id, &s.version_id)?;
        }
        Ok(snap)
    }

    pub fn versions(&self, game_id: &str) -> Result<Vec<Snapshot>> {
        self.db.list_versions(game_id)
    }

    pub fn restore(&self, game_id: &str, version_id: &str, force: bool) -> Result<Snapshot> {
        let game = self.db.get_game(game_id)?;
        if !force && process::is_running(&game) {
            return Err(Error::GameRunning(game.name, "restore"));
        }
        let save_dir = game.save_root.to_string_lossy().into_owned();
        let vars = [
            ("game", game.name.as_str()),
            ("game_id", game.id.as_str()),
            ("save_dir", save_dir.as_str()),
            ("version", version_id),
        ];
        self.run_hooks(|h| h.pre_restore.clone(), &vars);
        let snap = restore_version(
            &self.db,
            &self.cas,
            &self.device_id,
            &game,
            version_id,
            RestoreOptions::default(),
        )?;
        self.run_hooks(|h| h.post_restore.clone(), &vars);
        Ok(snap)
    }

    /// Restore the most recent version for a game (the common "sync me back"
    /// action). Convenience over `versions` + `restore`.
    pub fn restore_latest(&self, game_id: &str, force: bool) -> Result<Snapshot> {
        let latest = self
            .db
            .latest_version(game_id)?
            .ok_or_else(|| Error::NotFound(format!("no versions for game {game_id}")))?;
        self.restore(game_id, &latest.version_id, force)
    }

    /// Allow tests/UI to opt out of the safety snapshot explicitly.
    pub fn restore_with(
        &self,
        game_id: &str,
        version_id: &str,
        opts: RestoreOptions,
    ) -> Result<Snapshot> {
        let game = self.db.get_game(game_id)?;
        restore::restore_version(
            &self.db,
            &self.cas,
            &self.device_id,
            &game,
            version_id,
            opts,
        )
    }

    // ---- history maintenance --------------------------------------------

    /// Compare two versions of a game.
    pub fn diff(&self, game_id: &str, from_id: &str, to_id: &str) -> Result<crate::diff::Diff> {
        let from = self.db.get_version(from_id)?;
        let to = self.db.get_version(to_id)?;
        if from.game_id != game_id || to.game_id != game_id {
            return Err(Error::NotFound(format!(
                "both versions must belong to game {game_id}"
            )));
        }
        Ok(crate::diff::diff(&from, &to))
    }

    /// Apply a retention policy to a game's history, then reclaim orphaned bytes.
    /// Returns the deleted version ids and the GC report.
    pub fn prune(
        &self,
        game_id: &str,
        policy: &RetentionPolicy,
    ) -> Result<(Vec<String>, GcReport)> {
        let deleted = retention::prune(&self.db, game_id, policy)?;
        let report = retention::gc(&self.db, &self.cas)?;
        Ok((deleted, report))
    }

    /// Run garbage collection alone (delete objects no version references).
    pub fn gc(&self) -> Result<GcReport> {
        retention::gc(&self.db, &self.cas)
    }

    /// Apply a retention policy to *every* game, then reclaim space once.
    /// Returns (versions deleted, gc report).
    pub fn prune_all(&self, policy: &RetentionPolicy) -> Result<(usize, GcReport)> {
        let mut deleted = 0;
        for game in self.db.list_games()? {
            deleted += retention::prune(&self.db, &game.id, policy)?.len();
        }
        let report = retention::gc(&self.db, &self.cas)?;
        Ok((deleted, report))
    }

    /// Storage usage: total on-disk size + per-game deduplicated footprint.
    pub fn storage_report(&self) -> Result<StorageReport> {
        let objects = self.cas.list_objects()?;
        let total_objects = objects.len();
        let total_bytes: u64 = objects.iter().map(|h| self.cas.object_size(h)).sum();

        let mut games = Vec::new();
        for game in self.db.list_games()? {
            let versions = self.db.list_versions(&game.id)?;
            let mut seen: HashSet<String> = HashSet::new();
            let mut bytes = 0u64;
            for v in &versions {
                for f in &v.files {
                    if seen.insert(f.hash.clone()) {
                        bytes += self.cas.object_size(&f.hash);
                    }
                }
            }
            games.push(GameStorage {
                game_id: game.id,
                name: game.name,
                versions: versions.len(),
                bytes,
            });
        }
        games.sort_by_key(|g| std::cmp::Reverse(g.bytes));
        Ok(StorageReport {
            total_objects,
            total_bytes,
            compressed: self.cas.compress_enabled(),
            games,
        })
    }

    // ---- replication / sync ---------------------------------------------

    /// Configure the replication target (a folder — e.g. inside Dropbox/Drive).
    pub fn set_remote(&self, path: &Path) -> Result<()> {
        self.db.set_config(CFG_REMOTE, &path.to_string_lossy())
    }

    pub fn remote_path(&self) -> Result<Option<PathBuf>> {
        Ok(self.db.get_config(CFG_REMOTE)?.map(PathBuf::from))
    }

    /// Build the configured transport. A spec of `rclone:<target>` uses the
    /// rclone backend (e.g. `rclone:gdrive:GameSync`); anything else is treated
    /// as a local/shared folder path.
    fn remote(&self) -> Result<Box<dyn Remote>> {
        let spec = self
            .db
            .get_config(CFG_REMOTE)?
            .ok_or_else(|| Error::other("no remote configured — set one in the Remote bar"))?;
        if let Some(target) = spec.strip_prefix("rclone:") {
            Ok(Box::new(RcloneRemote::new(target.to_string())))
        } else if let Some(rest) = spec.strip_prefix("lan:") {
            // lan:<token>@<host:port>  (token optional)
            let (token, addr) = match rest.split_once('@') {
                Some((t, a)) => (t.to_string(), a.to_string()),
                None => (String::new(), rest.to_string()),
            };
            Ok(Box::new(LanRemote::connect(addr, token)))
        } else {
            Ok(Box::new(FolderRemote::open(PathBuf::from(spec))?))
        }
    }

    /// Start hosting a directory over LAN. Returns the handle (keep it alive to
    /// keep serving) — the bound port is on the handle.
    pub fn serve_lan(dir: PathBuf, token: &str, bind: &str) -> Result<LanServerHandle> {
        lan::serve(dir, token.to_string(), bind)
    }

    /// Best-effort primary LAN IP, for showing a connect string to peers.
    pub fn local_ip() -> Option<String> {
        lan::local_ip()
    }

    /// Broadcast a discovery beacon so peers can find this host without typing
    /// its address. `name` is a friendly label; `port` is the LAN host's TCP
    /// port. The token is never broadcast. Keep the handle alive while hosting.
    pub fn announce_lan(name: &str, port: u16) -> Result<lan::BeaconHandle> {
        lan::announce(name.to_string(), port)
    }

    /// Listen for host beacons for `timeout_ms`, returning the hosts found.
    pub fn discover_lan(timeout_ms: u64) -> Result<Vec<lan::DiscoveredHost>> {
        lan::discover(std::time::Duration::from_millis(timeout_ms))
    }

    /// A friendly label for this device (its hostname), for the beacon.
    pub fn lan_hostname() -> String {
        lan::hostname()
    }

    pub fn compression_enabled(&self) -> bool {
        self.cas.compress_enabled()
    }

    /// Turn LZMA2 (7-Zip-codec) compression of stored backups on/off. Only
    /// allowed while the store has no objects yet (changing it would otherwise
    /// require re-encoding everything), matching the encryption constraint.
    pub fn set_compression(&self, enabled: bool) -> Result<()> {
        if !self.cas.list_objects()?.is_empty() {
            return Err(Error::other(
                "compression can only be changed before any backups exist",
            ));
        }
        self.cas.set_compress(enabled);
        self.db
            .set_config(CFG_COMPRESS, if enabled { "1" } else { "0" })
    }

    /// Whether the first-run setup wizard has been completed/skipped.
    pub fn is_setup_complete(&self) -> Result<bool> {
        Ok(self.db.get_config(CFG_SETUP_DONE)?.as_deref() == Some("1"))
    }

    pub fn mark_setup_complete(&self) -> Result<()> {
        self.db.set_config(CFG_SETUP_DONE, "1")
    }

    pub fn auto_sync_settings(&self) -> Result<AutoSyncSettings> {
        let enabled = self.db.get_config(CFG_AUTO_SYNC)?.as_deref() == Some("1");
        let interval_min = self
            .db
            .get_config(CFG_AUTO_INTERVAL)?
            .and_then(|s| s.parse().ok())
            .unwrap_or(15)
            .max(1);
        // Defaults to true (anything other than an explicit "0").
        let backup_on_exit = self.db.get_config(CFG_BACKUP_ON_EXIT)?.as_deref() != Some("0");
        Ok(AutoSyncSettings {
            enabled,
            interval_min,
            backup_on_exit,
        })
    }

    pub fn set_auto_sync(&self, settings: AutoSyncSettings) -> Result<()> {
        self.db
            .set_config(CFG_AUTO_SYNC, if settings.enabled { "1" } else { "0" })?;
        self.db
            .set_config(CFG_AUTO_INTERVAL, &settings.interval_min.max(1).to_string())?;
        self.db.set_config(
            CFG_BACKUP_ON_EXIT,
            if settings.backup_on_exit { "1" } else { "0" },
        )?;
        Ok(())
    }

    /// Ids of tracked games that currently appear to be running (only games
    /// with a known install dir can be detected). Used by the exit watcher.
    pub fn running_game_ids(&self) -> Result<Vec<String>> {
        let games: Vec<Game> = self
            .db
            .list_games()?
            .into_iter()
            .filter(|g| g.install_dir.is_some())
            .collect();
        let dirs: Vec<PathBuf> = games
            .iter()
            .map(|g| g.install_dir.clone().unwrap())
            .collect();
        let running = process::running_install_dirs(&dirs);
        Ok(games
            .into_iter()
            .zip(running)
            .filter_map(|(g, r)| r.then_some(g.id))
            .collect())
    }

    /// One automatic pass: for every sync-enabled game (when a remote is set),
    /// snapshot any local changes and sync. Skips games that look like they're
    /// running, and never auto-resolves conflicts — those are reported back.
    pub fn auto_sync_pass(&self) -> Result<AutoSyncReport> {
        let mut report = AutoSyncReport::default();
        if self.remote_path()?.is_none() {
            return Ok(report);
        }
        for game in self.db.list_games()? {
            if !game.sync_enabled {
                continue;
            }
            report.games += 1;
            if process::is_running(&game) {
                report.skipped_running += 1;
                continue;
            }
            // Each game is isolated: one failure doesn't abort the whole pass.
            let outcome = (|| -> Result<SyncOutcome> {
                if self.backup_if_changed(&game.id)?.is_some() {
                    report.backed_up += 1;
                }
                self.sync_game(&game.id)
            })();
            match outcome {
                Ok(SyncOutcome::InSync) => report.in_sync += 1,
                Ok(SyncOutcome::Pushed { .. }) => report.pushed += 1,
                Ok(SyncOutcome::Pulled { .. }) => report.pulled += 1,
                Ok(SyncOutcome::Conflict { local, remote }) => {
                    report.conflicts.push(ConflictInfo {
                        game_id: game.id.clone(),
                        game_name: game.name.clone(),
                        local,
                        remote,
                    })
                }
                Err(e) => report.errors.push((game.name.clone(), e.to_string())),
            }
        }
        Ok(report)
    }

    /// The snapshot the live save folder currently corresponds to.
    fn head_snapshot(&self, game_id: &str) -> Result<Option<Snapshot>> {
        if let Some(head_id) = self.db.get_head(game_id)? {
            if let Ok(s) = self.db.get_version(&head_id) {
                return Ok(Some(s));
            }
        }
        self.db.latest_version(game_id)
    }

    /// Synchronize one game with the configured remote. See [`SyncOutcome`].
    pub fn sync_game(&self, game_id: &str) -> Result<SyncOutcome> {
        let game = self.db.get_game(game_id)?;
        let remote = self.remote()?;
        let _lease = remote.lock(game_id)?;

        let local_head = self.head_snapshot(game_id)?;
        let remote_head = remote.get_head(game_id)?;

        match (local_head, remote_head) {
            (None, None) => Ok(SyncOutcome::InSync),
            (Some(lh), None) => {
                self.push_version(remote.as_ref(), game_id, &lh)?;
                Ok(SyncOutcome::Pushed {
                    version_id: lh.version_id,
                })
            }
            (None, Some(rh)) => {
                let rsnap = self.fetch_remote_version(remote.as_ref(), game_id, &rh.version_id)?;
                self.apply_pulled(&game, &rsnap)?;
                Ok(SyncOutcome::Pulled {
                    version_id: rsnap.version_id,
                })
            }
            (Some(lh), Some(rh)) => {
                if lh.version_id == rh.version_id {
                    return Ok(SyncOutcome::InSync);
                }
                match vclock::compare(&lh.vclock, &rh.vclock) {
                    vclock::Ordering::Equal => Ok(SyncOutcome::InSync),
                    vclock::Ordering::Dominates => {
                        self.push_version(remote.as_ref(), game_id, &lh)?;
                        Ok(SyncOutcome::Pushed {
                            version_id: lh.version_id,
                        })
                    }
                    vclock::Ordering::DominatedBy => {
                        let rsnap =
                            self.fetch_remote_version(remote.as_ref(), game_id, &rh.version_id)?;
                        self.apply_pulled(&game, &rsnap)?;
                        Ok(SyncOutcome::Pulled {
                            version_id: rsnap.version_id,
                        })
                    }
                    vclock::Ordering::Concurrent => {
                        // Pull the remote version into local history so the user
                        // can inspect/diff it, but DO NOT touch the live save.
                        let rsnap =
                            self.fetch_remote_version(remote.as_ref(), game_id, &rh.version_id)?;
                        Ok(SyncOutcome::Conflict {
                            local: lh.version_id,
                            remote: rsnap.version_id,
                        })
                    }
                }
            }
        }
    }

    /// Resolve a conflict by choosing a side. The chosen state is captured as a
    /// new version whose clock supersedes both, then pushed.
    pub fn resolve_conflict(&self, game_id: &str, choice: ConflictChoice) -> Result<SyncOutcome> {
        let game = self.db.get_game(game_id)?;
        let remote = self.remote()?;
        let _lease = remote.lock(game_id)?;

        let rh = remote
            .get_head(game_id)?
            .ok_or_else(|| Error::other("no remote head; nothing to resolve"))?;
        let lh = self
            .head_snapshot(game_id)?
            .ok_or_else(|| Error::other("no local head; nothing to resolve"))?;
        let rsnap = self.fetch_remote_version(remote.as_ref(), game_id, &rh.version_id)?;

        // A clock that dominates both sides once this device increments it.
        let merged = vclock::merge(&lh.vclock, &rsnap.vclock);

        let parent = match choice {
            ConflictChoice::KeepRemote => {
                // Make the live save match the remote side first.
                restore_version(
                    &self.db,
                    &self.cas,
                    &self.device_id,
                    &game,
                    &rsnap.version_id,
                    RestoreOptions::default(),
                )?;
                rsnap.version_id.clone()
            }
            // KeepLocal: the live save already reflects the local head.
            ConflictChoice::KeepLocal => lh.version_id.clone(),
        };

        let resolved = create_snapshot(
            &self.db,
            &self.cas,
            &self.device_id,
            &game,
            SnapshotKind::Manual,
            Some("conflict resolution".to_string()),
            &merged,
            Some(parent),
        )?;
        self.db.set_head(&game.id, &resolved.version_id)?;
        self.push_version(remote.as_ref(), game_id, &resolved)?;
        Ok(SyncOutcome::Pushed {
            version_id: resolved.version_id,
        })
    }

    /// Resolve a conflict by **keeping both** save lines. The local save stays
    /// live and the conflict converges as keep-local (so every device agrees on
    /// the main line), while the remote branch is preserved as a brand-new,
    /// independent **fork** game — its own folder, its own history, sync off —
    /// so the other device's progress isn't discarded. Returns the fork game.
    pub fn fork_conflict(&self, game_id: &str) -> Result<Game> {
        let game = self.db.get_game(game_id)?;
        let remote = self.remote()?;
        let _lease = remote.lock(game_id)?;

        let rh = remote
            .get_head(game_id)?
            .ok_or_else(|| Error::other("no remote head; nothing to fork"))?;
        let lh = self
            .head_snapshot(game_id)?
            .ok_or_else(|| Error::other("no local head; nothing to fork"))?;
        let rsnap = self.fetch_remote_version(remote.as_ref(), game_id, &rh.version_id)?;

        // 1. Create the fork game with its own folder, so materializing the
        //    remote branch never touches the original's live save.
        let fork = self.create_fork_game(&game, &rsnap)?;

        // 2. Seed the fork's history with a fork-owned copy of the remote
        //    snapshot (same CAS objects, new id + game_id), then materialize it
        //    into the fork's (empty) folder.
        let fork_version = Snapshot {
            version_id: crate::util::new_id(),
            game_id: fork.id.clone(),
            device_id: self.device_id.clone(),
            created_ms: crate::util::now_ms(),
            label: Some(format!("forked from \"{}\" on conflict", game.name)),
            kind: SnapshotKind::Manual,
            parent: None,
            vclock: VectorClock::new(),
            total_size: rsnap.total_size,
            files: rsnap.files.clone(),
        };
        self.db.insert_version(&fork_version)?;
        self.db.set_head(&fork.id, &fork_version.version_id)?;
        restore_version(
            &self.db,
            &self.cas,
            &self.device_id,
            &fork,
            &fork_version.version_id,
            RestoreOptions {
                safety_snapshot: false,
            },
        )?;

        // 3. Converge the original game on the local side (keep-local), so all
        //    devices agree on the main line.
        let merged = vclock::merge(&lh.vclock, &rsnap.vclock);
        let resolved = create_snapshot(
            &self.db,
            &self.cas,
            &self.device_id,
            &game,
            SnapshotKind::Manual,
            Some("conflict resolution (kept both)".to_string()),
            &merged,
            Some(lh.version_id.clone()),
        )?;
        self.db.set_head(&game.id, &resolved.version_id)?;
        self.push_version(remote.as_ref(), game_id, &resolved)?;

        Ok(fork)
    }

    /// Build + persist a fork game beside the original: a unique sibling folder
    /// (`<save_root> (fork)`), name `<name> (fork)`, sync disabled.
    fn create_fork_game(&self, game: &Game, rsnap: &Snapshot) -> Result<Game> {
        let save_root = unique_fork_dir(fork_save_root(&game.save_root));
        std::fs::create_dir_all(&save_root)?;
        let short = &rsnap.version_id[..rsnap.version_id.len().min(6)];
        let fork = Game {
            id: format!("{}::fork::{short}", game.id),
            name: format!("{} (fork)", game.name),
            platform: Platform::Manual,
            save_root,
            install_dir: None,
            includes: game.includes.clone(),
            excludes: game.excludes.clone(),
            sync_enabled: false,
            extra_roots: Vec::new(),
        };
        self.db.upsert_game(&fork)?;
        Ok(fork)
    }

    /// Upload a version's objects + manifest and advance the remote head.
    fn push_version(&self, remote: &dyn Remote, game_id: &str, snap: &Snapshot) -> Result<()> {
        for f in &snap.files {
            if !remote.has_object(&f.hash)? {
                let bytes = self.cas.object_bytes(&f.hash)?;
                remote.put_object(&f.hash, &bytes)?;
            }
        }
        remote.put_version(game_id, snap)?;
        remote.set_head(
            game_id,
            &Head {
                version_id: snap.version_id.clone(),
                vclock: snap.vclock.clone(),
            },
        )?;
        Ok(())
    }

    /// Download a remote version's objects + manifest into the local store.
    fn fetch_remote_version(
        &self,
        remote: &dyn Remote,
        game_id: &str,
        version_id: &str,
    ) -> Result<Snapshot> {
        let snap = remote.get_version(game_id, version_id)?;
        for f in &snap.files {
            if !self.cas.exists(&f.hash) {
                let bytes = remote.get_object(&f.hash)?;
                self.cas.ingest_raw(&f.hash, &bytes)?;
            }
        }
        self.db.insert_version_if_absent(&snap)?;
        Ok(snap)
    }

    /// Fast-forward the live save to a pulled version (objects already local).
    fn apply_pulled(&self, game: &Game, rsnap: &Snapshot) -> Result<()> {
        // restore_version takes a pre-restore safety snapshot and sets the head.
        restore_version(
            &self.db,
            &self.cas,
            &self.device_id,
            game,
            &rsnap.version_id,
            RestoreOptions::default(),
        )?;
        Ok(())
    }

    // ---- integrity -------------------------------------------------------

    /// Verify every stored object referenced by every version re-hashes
    /// correctly. The whole point of the app is to not lose data, so this is a
    /// first-class operation.
    pub fn verify(&self) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        for game in self.db.list_games()? {
            for version in self.db.list_versions(&game.id)? {
                report.versions_checked += 1;
                for fe in &version.files {
                    report.objects_checked += 1;
                    match self.cas.verify_object(&fe.hash) {
                        Ok(true) => {}
                        Ok(false) => report.problems.push((
                            version.version_id.clone(),
                            fe.rel_path.clone(),
                            "missing or corrupt object".to_string(),
                        )),
                        Err(e) => report.problems.push((
                            version.version_id.clone(),
                            fe.rel_path.clone(),
                            e.to_string(),
                        )),
                    }
                }
            }
        }
        Ok(report)
    }
}

fn default_includes() -> Vec<String> {
    DEFAULT_INCLUDES.iter().map(|s| s.to_string()).collect()
}

fn default_excludes() -> Vec<String> {
    DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect()
}

/// A sibling folder for a forked save line: `<dir> (fork)` next to `dir`.
fn fork_save_root(save_root: &Path) -> PathBuf {
    let name = save_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "save".to_string());
    let parent = save_root.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{name} (fork)"))
}

/// Ensure the fork folder doesn't collide with an existing one, appending a
/// number (`… (fork) 2`, `3`, …) until a free path is found.
fn unique_fork_dir(base: PathBuf) -> PathBuf {
    if !base.exists() {
        return base;
    }
    let name = base
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "save (fork)".to_string());
    let parent = base
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    for n in 2..1000 {
        let cand = parent.join(format!("{name} {n}"));
        if !cand.exists() {
            return cand;
        }
    }
    base
}
