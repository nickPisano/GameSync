//! Background engine thread.
//!
//! The `gamesync-core` `Engine` (and its SQLite connection) is `Send` but not
//! `Sync`, and its operations can block (hashing, network sync). So one worker
//! thread owns the `Engine`; the UI sends [`Cmd`]s and receives [`Evt`]s over
//! channels, keeping the egui thread responsive. The worker calls
//! `ctx.request_repaint()` after each event so the UI wakes to consume it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use gamesync_core::{
    AutoSyncSettings, BackupOptions, BeaconHandle, ConflictChoice, Diff, Engine, Game,
    LanServerHandle, PluginList, SaveFile, Snapshot, StorageReport, SyncOutcome,
};

use crate::util::short;

/// A request from the UI to the worker.
pub enum Cmd {
    Unlock(String),
    Scan,
    UpdateList,
    Versions(String),
    Backup(String),
    Restore {
        game: String,
        version: String,
    },
    Verify,
    AddManual {
        name: String,
        path: PathBuf,
    },
    ToggleSync {
        id: String,
        enabled: bool,
    },
    Rename {
        id: String,
        name: String,
    },
    Remove(String),
    SetCompression(bool),
    SetAutoSync(AutoSyncSettings),
    SetRemote(String),
    Sync(String),
    Resolve {
        id: String,
        keep_local: bool,
    },
    Fork(String),
    FetchStorage,
    SetExtraRoots {
        id: String,
        roots: Vec<PathBuf>,
    },
    SetGameExe {
        id: String,
        path: Option<PathBuf>,
    },
    Redirect {
        id: String,
        target: PathBuf,
    },
    ListFiles(String),
    Diff {
        game: String,
        from: String,
        to: String,
    },
    EnableEncryption(String),
    DisableEncryption,
    MarkSetupComplete,
    ListPlugins,
    SetPluginEnabled {
        id: String,
        enabled: bool,
    },
    SetCommandsAllowed(bool),
    DiscoverLan,
    StartLanHost,
    StopLanHost,
    CheckUpdate,
    SyncAll,
    Tick,
}

/// Store-wide settings snapshot, refreshed whenever they change.
pub struct Meta {
    pub encrypted: bool,
    pub auto_sync: AutoSyncSettings,
    pub compression: bool,
    pub known_games: usize,
    pub remote: Option<String>,
    pub setup_complete: bool,
}

/// An update from the worker to the UI.
pub enum Evt {
    Locked,
    Opened(Meta),
    Games(Vec<Game>),
    Versions {
        game: String,
        versions: Vec<Snapshot>,
    },
    Storage(StorageReport),
    /// game id -> (version count, last backup epoch-ms)
    Summaries(HashMap<String, (usize, Option<i64>)>),
    Files {
        game: String,
        files: Vec<SaveFile>,
    },
    Diff(Diff),
    RecoveryKey(String),
    Plugins(PluginList),
    LanHosts(Vec<(String, String)>),
    /// `Some(spec)` once this device starts hosting, `None` when it stops.
    LanHosting(Option<String>),
    Update(Option<String>),
    Conflict {
        game: String,
        local: String,
        remote: String,
    },
    Info(String),
    Error(String),
    Busy(bool),
}

pub struct EngineHandle {
    tx: Sender<Cmd>,
    pub rx: Receiver<Evt>,
}

impl EngineHandle {
    /// A cloneable command sender, for use inside render closures.
    pub fn tx(&self) -> Sender<Cmd> {
        self.tx.clone()
    }
}

/// Spawn the worker thread and return a handle. `ctx` is used to wake the UI.
pub fn spawn(ctx: egui::Context) -> EngineHandle {
    let (cmd_tx, cmd_rx) = channel::<Cmd>();
    let (evt_tx, evt_rx) = channel::<Evt>();
    let emit = move |e: Evt| {
        let _ = evt_tx.send(e);
        ctx.request_repaint();
    };
    thread::spawn(move || run(cmd_rx, emit));
    // Background ticker drives auto-sync and auto-backup-on-exit.
    let tick_tx = cmd_tx.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(30));
        if tick_tx.send(Cmd::Tick).is_err() {
            break;
        }
    });
    EngineHandle {
        tx: cmd_tx,
        rx: evt_rx,
    }
}

fn meta(engine: &Engine, data_dir: &Path) -> Meta {
    Meta {
        encrypted: Engine::is_encrypted(data_dir),
        auto_sync: engine.auto_sync_settings().unwrap_or_default(),
        compression: engine.compression_enabled(),
        known_games: engine.known_game_count(),
        remote: engine
            .remote_path()
            .ok()
            .flatten()
            .map(|p| p.display().to_string())
            .filter(|s| !s.trim().is_empty()),
        setup_complete: engine.is_setup_complete().unwrap_or(true),
    }
}

/// A running LAN host: the server + discovery beacon. Dropping it stops both.
struct LanHost {
    _handle: LanServerHandle,
    _beacon: BeaconHandle,
}

fn run(rx: Receiver<Cmd>, emit: impl Fn(Evt)) {
    let data_dir = Engine::default_data_dir();
    let mut engine: Option<Engine> = None;
    let mut lan_host: Option<LanHost> = None;

    if Engine::is_encrypted(&data_dir) {
        emit(Evt::Locked);
    } else {
        match Engine::open(data_dir.clone()) {
            Ok(e) => {
                emit(Evt::Opened(meta(&e, &data_dir)));
                relist(&e, &emit);
                engine = Some(e);
            }
            Err(err) => emit(Evt::Error(format!("Couldn't open data store: {err}"))),
        }
    }

    let mut prev_running: HashSet<String> = HashSet::new();
    let mut last_pass = Instant::now();
    while let Ok(cmd) = rx.recv() {
        if matches!(cmd, Cmd::Tick) {
            tick(&engine, &mut prev_running, &mut last_pass, &emit);
            continue;
        }
        handle(cmd, &mut engine, &mut lan_host, &data_dir, &emit);
    }
}

fn relist(e: &Engine, emit: &impl Fn(Evt)) {
    match e.list_games() {
        Ok(g) => {
            let mut sums: HashMap<String, (usize, Option<i64>)> = HashMap::new();
            for game in &g {
                if let Ok(vs) = e.versions(&game.id) {
                    sums.insert(
                        game.id.clone(),
                        (vs.len(), vs.first().map(|v| v.created_ms)),
                    );
                }
            }
            emit(Evt::Games(g));
            emit(Evt::Summaries(sums));
        }
        Err(err) => emit(Evt::Error(err.to_string())),
    }
}

fn emit_versions(e: &Engine, id: &str, emit: &impl Fn(Evt)) {
    match e.versions(id) {
        Ok(v) => emit(Evt::Versions {
            game: id.to_string(),
            versions: v,
        }),
        Err(err) => emit(Evt::Error(err.to_string())),
    }
}

fn emit_plugins(e: &Engine, emit: &impl Fn(Evt)) {
    match e.list_plugins() {
        Ok(pl) => emit(Evt::Plugins(pl)),
        Err(err) => emit(Evt::Error(err.to_string())),
    }
}

/// One background tick: auto-backup games that just exited, and run a periodic
/// auto-sync pass when it's due. Runs silently except for results worth a toast.
fn tick(
    engine: &Option<Engine>,
    prev_running: &mut HashSet<String>,
    last_pass: &mut Instant,
    emit: &impl Fn(Evt),
) {
    let Some(e) = engine.as_ref() else {
        return;
    };
    let settings = e.auto_sync_settings().unwrap_or_default();

    if let Ok(running) = e.running_game_ids() {
        let now: HashSet<String> = running.into_iter().collect();
        if settings.backup_on_exit {
            for id in prev_running.difference(&now) {
                if let Ok(Some(s)) = e.backup_if_changed(id) {
                    emit(Evt::Info(format!(
                        "Auto-backed up {} file(s) for {id} on exit.",
                        s.file_count()
                    )));
                }
            }
        }
        *prev_running = now;
    }

    if settings.enabled {
        let interval = settings.interval_min.saturating_mul(60).max(60);
        if last_pass.elapsed().as_secs() >= interval {
            *last_pass = Instant::now();
            match e.auto_sync_pass() {
                Ok(rep) => {
                    if rep.backed_up + rep.pushed + rep.pulled > 0 {
                        emit(Evt::Info(format!(
                            "Auto-sync: {} backed up · {} pushed · {} pulled.",
                            rep.backed_up, rep.pushed, rep.pulled
                        )));
                    }
                    for c in rep.conflicts {
                        emit(Evt::Conflict {
                            game: c.game_id,
                            local: c.local,
                            remote: c.remote,
                        });
                    }
                    if let Ok(g) = e.list_games() {
                        emit(Evt::Games(g));
                    }
                }
                Err(err) => emit(Evt::Error(format!("Auto-sync failed: {err}"))),
            }
        }
    }
}

/// Ask GitHub for the latest release tag; `Some(version)` if newer than this build.
fn check_update() -> Option<String> {
    let out = std::process::Command::new("curl")
        .args([
            "-s",
            "-H",
            "User-Agent: GameSync",
            "https://api.github.com/repos/nickPisano/GameSync/releases/latest",
        ])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let tag = v
        .get("tag_name")?
        .as_str()?
        .trim_start_matches('v')
        .to_string();
    let current = env!("CARGO_PKG_VERSION");
    if !tag.is_empty() && tag != current {
        Some(tag)
    } else {
        None
    }
}

fn note(r: gamesync_core::Result<()>, ok_msg: &str, emit: &impl Fn(Evt)) -> bool {
    match r {
        Ok(()) => {
            if !ok_msg.is_empty() {
                emit(Evt::Info(ok_msg.to_string()));
            }
            true
        }
        Err(err) => {
            emit(Evt::Error(err.to_string()));
            false
        }
    }
}

fn handle(
    cmd: Cmd,
    engine: &mut Option<Engine>,
    lan_host: &mut Option<LanHost>,
    data_dir: &Path,
    emit: &impl Fn(Evt),
) {
    // Unlock is the one command that creates the engine.
    if let Cmd::Unlock(pass) = cmd {
        emit(Evt::Busy(true));
        match Engine::unlock(data_dir.to_path_buf(), &pass) {
            Ok(e) => {
                emit(Evt::Opened(meta(&e, data_dir)));
                relist(&e, emit);
                *engine = Some(e);
            }
            Err(err) => emit(Evt::Error(format!("Unlock failed: {err}"))),
        }
        emit(Evt::Busy(false));
        return;
    }

    // Enabling encryption also re-creates the engine (now cipher-aware).
    if let Cmd::EnableEncryption(pass) = cmd {
        emit(Evt::Busy(true));
        match Engine::init_encryption(data_dir, &pass) {
            Ok(rk) => match Engine::unlock(data_dir.to_path_buf(), &pass) {
                Ok(e2) => {
                    emit(Evt::RecoveryKey(rk.grouped()));
                    emit(Evt::Opened(meta(&e2, data_dir)));
                    relist(&e2, emit);
                    *engine = Some(e2);
                }
                Err(err) => emit(Evt::Error(format!("Re-open after enabling failed: {err}"))),
            },
            Err(err) => emit(Evt::Error(format!("Couldn't enable encryption: {err}"))),
        }
        emit(Evt::Busy(false));
        return;
    }

    // Disabling decrypts the store in place, then re-opens it plaintext.
    if let Cmd::DisableEncryption = cmd {
        emit(Evt::Busy(true));
        let res = match engine.as_ref() {
            Some(e) => e.disable_encryption(),
            None => Err(gamesync_core::Error::other("the data store is not open")),
        };
        match res {
            Ok(()) => match Engine::open(data_dir.to_path_buf()) {
                Ok(e2) => {
                    emit(Evt::Info("Encryption disabled.".to_string()));
                    emit(Evt::Opened(meta(&e2, data_dir)));
                    relist(&e2, emit);
                    *engine = Some(e2);
                }
                Err(err) => emit(Evt::Error(format!("Re-open after disabling failed: {err}"))),
            },
            Err(err) => emit(Evt::Error(format!("Couldn't disable encryption: {err}"))),
        }
        emit(Evt::Busy(false));
        return;
    }

    let Some(e) = engine.as_ref() else {
        emit(Evt::Error("the data store is not open".to_string()));
        return;
    };

    emit(Evt::Busy(true));
    match cmd {
        Cmd::Unlock(_) => unreachable!("handled above"),
        Cmd::EnableEncryption(_) => unreachable!("handled above"),
        Cmd::DisableEncryption => unreachable!("handled above"),
        Cmd::Scan => match e.scan_all() {
            Ok(found) => {
                emit(Evt::Info(format!(
                    "Scan complete — {} game(s) detected.",
                    found.len()
                )));
                relist(e, emit);
            }
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::UpdateList => match e.update_game_list() {
            Ok(n) => {
                emit(Evt::Info(format!("Game list updated — {n} games known.")));
                emit(Evt::Opened(meta(e, data_dir)));
            }
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::Versions(id) => emit_versions(e, &id, emit),
        Cmd::Backup(id) => match e.backup(&id, BackupOptions::default()) {
            Ok(s) => {
                emit(Evt::Info(format!(
                    "Backed up {} file(s) → {}.",
                    s.file_count(),
                    short(&s.version_id)
                )));
                emit_versions(e, &id, emit);
                relist(e, emit);
            }
            Err(err) => emit(Evt::Error(format!("Backup failed: {err}"))),
        },
        Cmd::Restore { game, version } => match e.restore(&game, &version, false) {
            Ok(s) => {
                emit(Evt::Info(format!(
                    "Restored {} ({} files). Safety backup taken first.",
                    short(&s.version_id),
                    s.file_count()
                )));
                emit_versions(e, &game, emit);
            }
            Err(err) => emit(Evt::Error(format!("Restore failed: {err}"))),
        },
        Cmd::Verify => match e.verify() {
            Ok(r) => emit(Evt::Info(if r.ok() {
                format!("Integrity OK — {} object(s) checked.", r.objects_checked)
            } else {
                format!("Integrity: {} problem(s) found.", r.problems.len())
            })),
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::AddManual { name, path } => match e.add_manual_game(&name, path) {
            Ok(g) => {
                emit(Evt::Info(format!("Added {}.", g.name)));
                relist(e, emit);
            }
            Err(err) => emit(Evt::Error(format!("Add failed: {err}"))),
        },
        Cmd::ToggleSync { id, enabled } => {
            note(e.set_sync_enabled(&id, enabled), "", emit);
            relist(e, emit);
        }
        Cmd::Rename { id, name } => {
            note(e.rename_game(&id, &name), "Renamed.", emit);
            relist(e, emit);
        }
        Cmd::Remove(id) => match e.remove_game(&id) {
            Ok(_) => {
                emit(Evt::Info(
                    "Game removed (your save files are untouched).".to_string(),
                ));
                relist(e, emit);
            }
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::SetCompression(on) => {
            note(e.set_compression(on), "Compression updated.", emit);
            emit(Evt::Opened(meta(e, data_dir)));
        }
        Cmd::SetAutoSync(s) => {
            note(e.set_auto_sync(s), "", emit);
            emit(Evt::Opened(meta(e, data_dir)));
        }
        Cmd::SetRemote(spec) => {
            note(e.set_remote(Path::new(&spec)), "Remote updated.", emit);
            emit(Evt::Opened(meta(e, data_dir)));
        }
        Cmd::Sync(id) => match e.sync_game(&id) {
            Ok(out) => {
                sync_msg(&id, out, emit);
                relist(e, emit);
            }
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::Resolve { id, keep_local } => {
            let choice = if keep_local {
                ConflictChoice::KeepLocal
            } else {
                ConflictChoice::KeepRemote
            };
            note(
                e.resolve_conflict(&id, choice).map(|_| ()),
                "Conflict resolved.",
                emit,
            );
            relist(e, emit);
        }
        Cmd::Fork(id) => match e.fork_conflict(&id) {
            Ok(g) => {
                emit(Evt::Info(format!(
                    "Kept both — the remote branch was forked to \"{}\".",
                    g.name
                )));
                relist(e, emit);
            }
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::FetchStorage => match e.storage_report() {
            Ok(r) => emit(Evt::Storage(r)),
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::SetExtraRoots { id, roots } => {
            note(e.set_extra_roots(&id, roots), "Extra folders saved.", emit);
            relist(e, emit);
        }
        Cmd::SetGameExe { id, path } => {
            note(
                e.set_game_exe(&id, path),
                "Close-detection location saved.",
                emit,
            );
            relist(e, emit);
        }
        Cmd::Redirect { id, target } => match e.redirect_save_folder(&id, target) {
            Ok(r) => {
                emit(Evt::Info(format!(
                    "Redirected. Saves now live at {}; the original was kept at {}.",
                    r.linked_target, r.original_backup
                )));
                relist(e, emit);
            }
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::ListFiles(id) => match e.list_save_files(&id) {
            Ok(files) => emit(Evt::Files { game: id, files }),
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::Diff { game, from, to } => match e.diff(&game, &from, &to) {
            Ok(d) => emit(Evt::Diff(d)),
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::MarkSetupComplete => {
            note(e.mark_setup_complete(), "", emit);
            emit(Evt::Opened(meta(e, data_dir)));
        }
        Cmd::ListPlugins => emit_plugins(e, emit),
        Cmd::SetPluginEnabled { id, enabled } => {
            note(e.set_plugin_enabled(&id, enabled), "", emit);
            emit_plugins(e, emit);
        }
        Cmd::SetCommandsAllowed(allowed) => {
            note(e.set_commands_allowed(allowed), "", emit);
            emit_plugins(e, emit);
        }
        Cmd::DiscoverLan => match Engine::discover_lan(2000) {
            Ok(hosts) => emit(Evt::LanHosts(
                hosts
                    .into_iter()
                    .map(|h| {
                        let endpoint = h.endpoint();
                        (h.name, endpoint)
                    })
                    .collect(),
            )),
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::StartLanHost => {
            // Share a `lan-share` folder as the remote (so this device's saves
            // land in the shared store), serve it over TCP, and advertise it.
            let share = data_dir.join("lan-share");
            let started = (|| -> gamesync_core::Result<String> {
                std::fs::create_dir_all(&share)?;
                e.set_remote(&share)?;
                let token = gamesync_core::util::new_id();
                let handle = Engine::serve_lan(share.clone(), &token, "0.0.0.0:0")?;
                let ip = Engine::local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
                let port = handle.port;
                let beacon = Engine::announce_lan(&Engine::lan_hostname(), port)?;
                *lan_host = Some(LanHost {
                    _handle: handle,
                    _beacon: beacon,
                });
                Ok(format!("lan:{token}@{ip}:{port}"))
            })();
            match started {
                Ok(spec) => {
                    emit(Evt::Opened(meta(e, data_dir))); // remote now points at the share
                    emit(Evt::LanHosting(Some(spec)));
                    emit(Evt::Info("Hosting on this network.".to_string()));
                }
                Err(err) => emit(Evt::Error(format!("Couldn't start hosting: {err}"))),
            }
        }
        Cmd::StopLanHost => {
            *lan_host = None; // dropping the handle stops the server + beacon
            emit(Evt::LanHosting(None));
            emit(Evt::Info("Stopped hosting.".to_string()));
        }
        Cmd::CheckUpdate => emit(Evt::Update(check_update())),
        Cmd::SyncAll => match e.auto_sync_pass() {
            Ok(rep) => {
                emit(Evt::Info(format!(
                    "Synced all: {} backed up · {} pushed · {} pulled.",
                    rep.backed_up, rep.pushed, rep.pulled
                )));
                for c in rep.conflicts {
                    emit(Evt::Conflict {
                        game: c.game_id,
                        local: c.local,
                        remote: c.remote,
                    });
                }
                relist(e, emit);
            }
            Err(err) => emit(Evt::Error(err.to_string())),
        },
        Cmd::Tick => unreachable!("handled in the run loop"),
    }
    emit(Evt::Busy(false));
}

fn sync_msg(id: &str, out: SyncOutcome, emit: &impl Fn(Evt)) {
    match out {
        SyncOutcome::InSync => emit(Evt::Info("Already in sync.".to_string())),
        SyncOutcome::Pushed { version_id } => {
            emit(Evt::Info(format!("Pushed {}.", short(&version_id))))
        }
        SyncOutcome::Pulled { version_id } => emit(Evt::Info(format!(
            "Pulled & restored {}.",
            short(&version_id)
        ))),
        SyncOutcome::Conflict { local, remote } => emit(Evt::Conflict {
            game: id.to_string(),
            local,
            remote,
        }),
    }
}
