//! Background engine thread.
//!
//! The `gamesync-core` `Engine` (and its SQLite connection) is `Send` but not
//! `Sync`, and its operations can block (hashing, network sync). So one worker
//! thread owns the `Engine`; the UI sends [`Cmd`]s and receives [`Evt`]s over
//! channels, keeping the egui thread responsive. The worker calls
//! `ctx.request_repaint()` after each event so the UI wakes to consume it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use eframe::egui;
use gamesync_core::{
    AutoSyncSettings, BackupOptions, ConflictChoice, Engine, Game, Snapshot, StorageReport,
    SyncOutcome,
};

use crate::util::short;

/// A request from the UI to the worker.
pub enum Cmd {
    Unlock(String),
    Scan,
    UpdateList,
    Versions(String),
    Backup(String),
    RestoreLatest(String),
    Restore { game: String, version: String },
    Verify,
    AddManual { name: String, path: PathBuf },
    ToggleSync { id: String, enabled: bool },
    Rename { id: String, name: String },
    Remove(String),
    SetCompression(bool),
    SetAutoSync(AutoSyncSettings),
    SetRemote(String),
    Sync(String),
    Resolve { id: String, keep_local: bool },
    Fork(String),
    FetchStorage,
}

/// Store-wide settings snapshot, refreshed whenever they change.
pub struct Meta {
    pub encrypted: bool,
    pub auto_sync: AutoSyncSettings,
    pub compression: bool,
    pub known_games: usize,
    pub remote: Option<String>,
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
            .map(|p| p.display().to_string()),
    }
}

fn run(rx: Receiver<Cmd>, emit: impl Fn(Evt)) {
    let data_dir = Engine::default_data_dir();
    let mut engine: Option<Engine> = None;

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

    while let Ok(cmd) = rx.recv() {
        handle(cmd, &mut engine, &data_dir, &emit);
    }
}

fn relist(e: &Engine, emit: &impl Fn(Evt)) {
    match e.list_games() {
        Ok(g) => emit(Evt::Games(g)),
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

fn handle(cmd: Cmd, engine: &mut Option<Engine>, data_dir: &Path, emit: &impl Fn(Evt)) {
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

    let Some(e) = engine.as_ref() else {
        emit(Evt::Error("the data store is not open".to_string()));
        return;
    };

    emit(Evt::Busy(true));
    match cmd {
        Cmd::Unlock(_) => unreachable!("handled above"),
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
        Cmd::RestoreLatest(id) => match e.restore_latest(&id, false) {
            Ok(s) => {
                emit(Evt::Info(format!(
                    "Restored latest ({} files). Safety backup taken first.",
                    s.file_count()
                )));
                emit_versions(e, &id, emit);
            }
            Err(err) => emit(Evt::Error(format!("Restore failed: {err}"))),
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
