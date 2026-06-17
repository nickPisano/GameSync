// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri command layer for GameSync.
//!
//! Every command is a thin wrapper over `gamesync_core::Engine`. The engine
//! holds a SQLite connection (Send but not Sync), so it lives behind a `Mutex`
//! and is `None` until an encrypted store is unlocked.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use gamesync_core::{
    AutoSyncReport, AutoSyncSettings, BackupOptions, ConflictChoice, Diff, Engine, Game, GcReport,
    LanServerHandle, PluginList, RedirectReport, RetentionPolicy, SaveFile, SnapshotKind,
    StorageReport, SyncOutcome, ViewerInfo,
};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

struct AppState {
    data_dir: PathBuf,
    engine: Mutex<Option<Engine>>,
    lan: Mutex<Option<LanHostState>>,
}

/// A running LAN host plus the info shown to the user. The handle stops the
/// server when dropped.
struct LanHostState {
    _handle: LanServerHandle,
    info: LanHostInfo,
}

#[derive(Serialize, Clone)]
struct LanHostInfo {
    hosting: bool,
    ip: Option<String>,
    port: Option<u16>,
    token: Option<String>,
    spec: Option<String>,
}

impl LanHostInfo {
    fn off() -> Self {
        Self {
            hosting: false,
            ip: None,
            port: None,
            token: None,
            spec: None,
        }
    }
}

impl AppState {
    /// Run `f` against the unlocked engine, mapping errors to strings for JS.
    fn with_engine<T>(
        &self,
        f: impl FnOnce(&Engine) -> gamesync_core::Result<T>,
    ) -> Result<T, String> {
        let guard = self.engine.lock().unwrap();
        let engine = guard
            .as_ref()
            .ok_or("the store is encrypted and locked — unlock it first")?;
        f(engine).map_err(|e| e.to_string())
    }
}

#[derive(Serialize)]
struct AppStatus {
    encrypted: bool,
    unlocked: bool,
    setup_complete: bool,
    data_dir: String,
}

/// A game plus the summary fields the library view needs.
#[derive(Serialize)]
struct GameView {
    game: Game,
    version_count: usize,
    last_backup_ms: Option<i64>,
}

#[tauri::command]
fn app_status(state: State<AppState>) -> AppStatus {
    let guard = state.engine.lock().unwrap();
    let setup_complete = guard
        .as_ref()
        .and_then(|e| e.is_setup_complete().ok())
        .unwrap_or(false);
    AppStatus {
        encrypted: Engine::is_encrypted(&state.data_dir),
        unlocked: guard.is_some(),
        setup_complete,
        data_dir: state.data_dir.display().to_string(),
    }
}

#[tauri::command]
fn complete_setup(state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.mark_setup_complete())
}

#[tauri::command]
fn unlock(passphrase: String, state: State<AppState>) -> Result<(), String> {
    let engine = Engine::unlock(state.data_dir.clone(), &passphrase).map_err(|e| e.to_string())?;
    *state.engine.lock().unwrap() = Some(engine);
    Ok(())
}

/// Enable encryption on a fresh store and return the recovery key to display.
#[tauri::command]
fn init_encryption(passphrase: String, state: State<AppState>) -> Result<String, String> {
    if passphrase.len() < 8 {
        return Err("passphrase must be at least 8 characters".into());
    }
    let recovery =
        Engine::init_encryption(&state.data_dir, &passphrase).map_err(|e| e.to_string())?;
    let engine = Engine::unlock(state.data_dir.clone(), &passphrase).map_err(|e| e.to_string())?;
    *state.engine.lock().unwrap() = Some(engine);
    Ok(recovery.grouped())
}

#[tauri::command]
fn list_games(state: State<AppState>) -> Result<Vec<GameView>, String> {
    state.with_engine(|e| {
        let mut out = Vec::new();
        for game in e.list_games()? {
            let versions = e.versions(&game.id)?;
            out.push(GameView {
                last_backup_ms: versions.first().map(|v| v.created_ms),
                version_count: versions.len(),
                game,
            });
        }
        Ok(out)
    })
}

#[tauri::command]
fn scan(state: State<AppState>) -> Result<Vec<Game>, String> {
    state.with_engine(|e| e.scan_all())
}

#[tauri::command]
fn update_game_list(state: State<AppState>) -> Result<usize, String> {
    state.with_engine(|e| e.update_game_list())
}

#[tauri::command]
fn known_game_count(state: State<AppState>) -> Result<usize, String> {
    state.with_engine(|e| Ok(e.known_game_count()))
}

#[tauri::command]
fn add_game(name: String, path: String, state: State<AppState>) -> Result<Game, String> {
    state.with_engine(|e| e.add_manual_game(&name, PathBuf::from(path)))
}

#[tauri::command]
fn set_sync_enabled(id: String, enabled: bool, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.set_sync_enabled(&id, enabled))
}

#[tauri::command]
fn set_game_exe(id: String, path: Option<String>, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.set_game_exe(&id, path.map(PathBuf::from)))
}

#[tauri::command]
fn rename_game(id: String, name: String, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.rename_game(&id, &name))
}

#[tauri::command]
fn remove_game(id: String, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.remove_game(&id).map(|_| ()))
}

#[tauri::command]
fn redirect_save_folder(
    id: String,
    target: String,
    state: State<AppState>,
) -> Result<RedirectReport, String> {
    state.with_engine(|e| e.redirect_save_folder(&id, PathBuf::from(target)))
}

#[tauri::command]
fn backup(id: String, state: State<AppState>) -> Result<gamesync_core::Snapshot, String> {
    state.with_engine(|e| {
        e.backup(
            &id,
            BackupOptions {
                kind: SnapshotKind::Manual,
                ..Default::default()
            },
        )
    })
}

#[tauri::command]
fn versions(id: String, state: State<AppState>) -> Result<Vec<gamesync_core::Snapshot>, String> {
    state.with_engine(|e| e.versions(&id))
}

#[tauri::command]
fn restore(
    id: String,
    version: String,
    state: State<AppState>,
) -> Result<gamesync_core::Snapshot, String> {
    state.with_engine(|e| e.restore(&id, &version, false))
}

#[tauri::command]
fn diff(id: String, from: String, to: String, state: State<AppState>) -> Result<Diff, String> {
    state.with_engine(|e| e.diff(&id, &from, &to))
}

#[tauri::command]
fn list_save_files(id: String, state: State<AppState>) -> Result<Vec<SaveFile>, String> {
    state.with_engine(|e| e.list_save_files(&id))
}

/// Open a folder in the OS file manager.
#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(&path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer");
        c.arg(&path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&path);
        c
    };
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Reveal a file in its containing folder (selecting it where supported).
#[tauri::command]
fn reveal_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{path}"))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // No standard "reveal"; open the containing directory.
        let dir = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(path);
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[tauri::command]
fn list_plugins(state: State<AppState>) -> Result<PluginList, String> {
    state.with_engine(|e| e.list_plugins())
}

#[tauri::command]
fn set_plugin_enabled(id: String, enabled: bool, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.set_plugin_enabled(&id, enabled))
}

#[tauri::command]
fn set_plugin_commands_allowed(allowed: bool, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.set_commands_allowed(allowed))
}

#[tauri::command]
fn file_viewers(path: String, state: State<AppState>) -> Result<Vec<ViewerInfo>, String> {
    state.with_engine(|e| Ok(e.file_viewers(&path)))
}

#[tauri::command]
fn run_viewer(command: String, path: String, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.run_viewer(&command, &path))
}

#[tauri::command]
fn remote_status(state: State<AppState>) -> Result<Option<String>, String> {
    state.with_engine(|e| Ok(e.remote_path()?.map(|p| p.display().to_string())))
}

#[tauri::command]
fn set_remote(path: String, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.set_remote(&PathBuf::from(path)))
}

#[tauri::command]
fn sync_game(id: String, state: State<AppState>) -> Result<SyncOutcome, String> {
    state.with_engine(|e| e.sync_game(&id))
}

#[tauri::command]
fn resolve_conflict(
    id: String,
    keep: String,
    state: State<AppState>,
) -> Result<SyncOutcome, String> {
    let choice = match keep.as_str() {
        "remote" => ConflictChoice::KeepRemote,
        _ => ConflictChoice::KeepLocal,
    };
    state.with_engine(|e| e.resolve_conflict(&id, choice))
}

#[tauri::command]
fn get_auto_sync(state: State<AppState>) -> Result<AutoSyncSettings, String> {
    state.with_engine(|e| e.auto_sync_settings())
}

#[tauri::command]
fn set_auto_sync(settings: AutoSyncSettings, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.set_auto_sync(settings))
}

/// Run one full auto-sync pass on demand (used by the "Sync all" button).
#[tauri::command]
fn sync_all(state: State<AppState>) -> Result<AutoSyncReport, String> {
    state.with_engine(|e| e.auto_sync_pass())
}

#[derive(Serialize)]
struct VerifyView {
    ok: bool,
    versions_checked: usize,
    objects_checked: usize,
    problems: Vec<String>,
}

#[tauri::command]
fn storage_report(state: State<AppState>) -> Result<StorageReport, String> {
    state.with_engine(|e| e.storage_report())
}

#[derive(Serialize)]
struct PruneResult {
    versions_deleted: usize,
    objects_deleted: usize,
    bytes_freed: u64,
}

#[tauri::command]
fn prune_all(policy: RetentionPolicy, state: State<AppState>) -> Result<PruneResult, String> {
    state.with_engine(|e| {
        let (versions_deleted, gc) = e.prune_all(&policy)?;
        Ok(PruneResult {
            versions_deleted,
            objects_deleted: gc.objects_deleted,
            bytes_freed: gc.bytes_freed,
        })
    })
}

#[tauri::command]
fn run_gc(state: State<AppState>) -> Result<GcReport, String> {
    state.with_engine(|e| e.gc())
}

#[tauri::command]
fn set_compression(enabled: bool, state: State<AppState>) -> Result<(), String> {
    state.with_engine(|e| e.set_compression(enabled))
}

#[tauri::command]
fn verify(state: State<AppState>) -> Result<VerifyView, String> {
    state.with_engine(|e| {
        let r = e.verify()?;
        Ok(VerifyView {
            ok: r.ok(),
            versions_checked: r.versions_checked,
            objects_checked: r.objects_checked,
            problems: r
                .problems
                .into_iter()
                .map(|(v, p, why)| format!("{v}  {p}: {why}"))
                .collect(),
        })
    })
}

/// Start hosting this device's saves over the LAN. Sets the local remote to a
/// shared folder (so the host's games land in the shared store) and starts the
/// TCP server. Returns the connect info to show peers.
#[tauri::command]
fn start_lan_host(state: State<AppState>) -> Result<LanHostInfo, String> {
    let share = state.data_dir.join("lan-share");
    {
        let guard = state.engine.lock().unwrap();
        let engine = guard.as_ref().ok_or("the store is locked")?;
        std::fs::create_dir_all(&share).map_err(|e| e.to_string())?;
        engine.set_remote(&share).map_err(|e| e.to_string())?;
    }
    let token = gamesync_core::util::new_id();
    let handle = Engine::serve_lan(share, &token, "0.0.0.0:0").map_err(|e| e.to_string())?;
    let ip = Engine::local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let port = handle.port;
    let info = LanHostInfo {
        hosting: true,
        spec: Some(format!("lan:{token}@{ip}:{port}")),
        ip: Some(ip),
        port: Some(port),
        token: Some(token),
    };
    *state.lan.lock().unwrap() = Some(LanHostState {
        _handle: handle,
        info: info.clone(),
    });
    Ok(info)
}

#[tauri::command]
fn stop_lan_host(state: State<AppState>) -> LanHostInfo {
    *state.lan.lock().unwrap() = None; // dropping the handle stops the server
    LanHostInfo::off()
}

#[tauri::command]
fn lan_host_status(state: State<AppState>) -> LanHostInfo {
    state
        .lan
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.info.clone())
        .unwrap_or_else(LanHostInfo::off)
}

/// Run an auto-sync pass and emit the result to the frontend (and on error, an
/// error event). Triggered by the background timer and the tray menu.
fn run_pass_and_emit(app: &AppHandle) {
    let state = app.state::<AppState>();
    let guard = state.engine.lock().unwrap();
    let Some(engine) = guard.as_ref() else {
        return; // locked / not unlocked yet — no start emitted, so no dangling spinner
    };
    // "start" is always paired with a completion event below (the pass always
    // returns Ok or Err), so the spinner can't get stuck.
    let _ = app.emit("auto-sync-start", ());
    let result = engine.auto_sync_pass();
    drop(guard);
    match result {
        Ok(report) => {
            let _ = app.emit("auto-sync", report);
        }
        Err(e) => {
            let _ = app.emit("auto-sync-error", e.to_string());
        }
    }
}

/// Background loop: every tick, if auto-sync is enabled and the interval has
/// elapsed, run a pass. Cheap when disabled or locked.
fn auto_sync_loop(app: AppHandle) {
    // Force a first run shortly after enable by starting "in the past".
    let mut last_run = Instant::now()
        .checked_sub(Duration::from_secs(86_400))
        .unwrap_or_else(Instant::now);
    loop {
        std::thread::sleep(Duration::from_secs(20));
        let settings = {
            let state = app.state::<AppState>();
            let guard = state.engine.lock().unwrap();
            guard.as_ref().and_then(|e| e.auto_sync_settings().ok())
        };
        let Some(settings) = settings else { continue };
        if !settings.enabled {
            continue;
        }
        if last_run.elapsed().as_secs() < settings.interval_min * 60 {
            continue;
        }
        run_pass_and_emit(&app);
        last_run = Instant::now();
    }
}

/// Handle a tracked game closing: wait briefly for the save to flush, back up
/// if it changed, and (if the game syncs and a remote is set) sync.
fn handle_game_exit(app: &AppHandle, game_id: &str) {
    std::thread::sleep(Duration::from_secs(4));
    let (made, name, do_sync) = {
        let state = app.state::<AppState>();
        let guard = state.engine.lock().unwrap();
        let Some(engine) = guard.as_ref() else {
            return;
        };
        let name = engine
            .get_game(game_id)
            .map(|g| g.name)
            .unwrap_or_else(|_| game_id.to_string());
        let made = engine.backup_if_changed(game_id).ok().flatten().is_some();
        let do_sync = made
            && engine
                .get_game(game_id)
                .map(|g| g.sync_enabled)
                .unwrap_or(false)
            && engine.remote_path().ok().flatten().is_some();
        (made, name, do_sync)
    };
    if made {
        let _ = app.emit("game-exit-backup", name);
    }
    if do_sync {
        run_pass_and_emit(app);
    }
}

/// Poll which tracked games are running; when one stops, fire the exit handler.
fn exit_watch_loop(app: AppHandle) {
    let mut prev: HashSet<String> = HashSet::new();
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let snapshot = {
            let state = app.state::<AppState>();
            let guard = state.engine.lock().unwrap();
            guard.as_ref().map(|e| {
                (
                    e.auto_sync_settings()
                        .map(|s| s.backup_on_exit)
                        .unwrap_or(true),
                    e.running_game_ids()
                        .unwrap_or_default()
                        .into_iter()
                        .collect::<HashSet<String>>(),
                )
            })
        };
        let Some((enabled, current)) = snapshot else {
            continue; // store locked — leave `prev` as-is
        };
        let exited: Vec<String> = prev.difference(&current).cloned().collect();
        prev = current;
        if !enabled {
            continue;
        }
        for id in exited {
            let app = app.clone();
            std::thread::spawn(move || handle_game_exit(&app, &id));
        }
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn main() {
    let data_dir = Engine::default_data_dir();
    // Open immediately unless the store is encrypted (then the UI prompts).
    let engine = if Engine::is_encrypted(&data_dir) {
        None
    } else {
        Some(Engine::open(data_dir.clone()).expect("failed to open GameSync data store"))
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            data_dir,
            engine: Mutex::new(engine),
            lan: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            app_status,
            complete_setup,
            unlock,
            init_encryption,
            list_games,
            scan,
            update_game_list,
            known_game_count,
            add_game,
            set_sync_enabled,
            set_game_exe,
            rename_game,
            remove_game,
            redirect_save_folder,
            backup,
            versions,
            restore,
            diff,
            list_save_files,
            open_folder,
            reveal_file,
            list_plugins,
            set_plugin_enabled,
            set_plugin_commands_allowed,
            file_viewers,
            run_viewer,
            remote_status,
            set_remote,
            sync_game,
            resolve_conflict,
            get_auto_sync,
            set_auto_sync,
            sync_all,
            verify,
            storage_report,
            prune_all,
            run_gc,
            set_compression,
            start_lan_host,
            stop_lan_host,
            lan_host_status
        ])
        .setup(|app| {
            // ---- system tray ----
            let open_i = MenuItem::with_id(app, "open", "Open GameSync", true, None::<&str>)?;
            let sync_i = MenuItem::with_id(app, "sync", "Sync all now", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit GameSync", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_i, &sync_i, &quit_i])?;

            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("GameSync")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main_window(app),
                    "sync" => {
                        let app = app.clone();
                        std::thread::spawn(move || run_pass_and_emit(&app));
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // ---- background auto-sync ----
            let handle = app.handle().clone();
            std::thread::spawn(move || auto_sync_loop(handle));

            // ---- back up automatically when a tracked game closes ----
            let exit_handle = app.handle().clone();
            std::thread::spawn(move || exit_watch_loop(exit_handle));
            Ok(())
        })
        // Closing the window hides to the tray instead of quitting, so
        // background auto-sync keeps running. Quit from the tray menu.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running GameSync");
}
