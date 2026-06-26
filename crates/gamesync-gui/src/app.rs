//! The egui application: state, the worker connection, and all rendering.
//!
//! Rendering follows one rule to keep egui's per-field closure borrows happy:
//! render closures only read/write individual `self` fields and send [`Cmd`]s
//! through a cloned `Sender` — they never call `&mut self` methods. Each
//! top-level `render_*` method is a separate `&mut self` call from `update`.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use gamesync_core::{AutoSyncSettings, Engine, Game, Snapshot, StorageReport};

use crate::theme::Theme;
use crate::util::{human_size, humanize_ago, short};
use crate::worker::{self, Cmd, EngineHandle, Evt, Meta};

enum ToastKind {
    Info,
    Error,
}

struct Toast {
    msg: String,
    kind: ToastKind,
    at: Instant,
}

pub struct App {
    handle: EngineHandle,
    data_dir: PathBuf,

    // Store / unlock state.
    locked: bool,
    opened: bool,
    passphrase: String,

    // Library.
    games: Vec<Game>,
    selected: Option<String>,
    versions_for: Option<String>,
    versions: Vec<Snapshot>,

    // Store-wide settings (mirrors worker `Meta`).
    auto_sync: AutoSyncSettings,
    compression: bool,
    known_games: usize,
    encrypted: bool,
    remote: Option<String>,
    storage: Option<StorageReport>,

    // game id -> (local version, remote version)
    conflicts: std::collections::HashMap<String, (String, String)>,

    // UI state.
    theme: Theme,
    theme_dirty: bool,
    search: String,
    busy: u32,
    toasts: Vec<Toast>,

    // Modals / inline editors.
    show_settings: bool,
    show_add: bool,
    add_name: String,
    add_path: String,
    renaming: Option<String>,
    rename_buf: String,
    remote_buf: String,
    confirm_remove: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let data_dir = Engine::default_data_dir();
        let theme = crate::theme::load(&data_dir);
        cc.egui_ctx.set_visuals(theme.visuals());
        let handle = worker::spawn(cc.egui_ctx.clone());
        Self {
            handle,
            data_dir,
            locked: false,
            opened: false,
            passphrase: String::new(),
            games: Vec::new(),
            selected: None,
            versions_for: None,
            versions: Vec::new(),
            auto_sync: AutoSyncSettings::default(),
            compression: false,
            known_games: 0,
            encrypted: false,
            remote: None,
            storage: None,
            conflicts: std::collections::HashMap::new(),
            theme,
            theme_dirty: false,
            search: String::new(),
            busy: 0,
            toasts: Vec::new(),
            show_settings: false,
            show_add: false,
            add_name: String::new(),
            add_path: String::new(),
            renaming: None,
            rename_buf: String::new(),
            remote_buf: String::new(),
            confirm_remove: None,
        }
    }

    fn drain_events(&mut self) {
        while let Ok(evt) = self.handle.rx.try_recv() {
            match evt {
                Evt::Locked => {
                    self.locked = true;
                    self.opened = false;
                }
                Evt::Opened(m) => {
                    self.apply_meta(m);
                    self.locked = false;
                    self.opened = true;
                }
                Evt::Games(mut g) => {
                    g.sort_by_key(|x| x.name.to_lowercase());
                    self.games = g;
                    if let Some(sel) = self.selected.clone() {
                        if self.games.iter().any(|x| x.id == sel) {
                            let _ = self.handle.tx().send(Cmd::Versions(sel));
                        } else {
                            self.selected = None;
                            self.versions.clear();
                            self.versions_for = None;
                        }
                    }
                }
                Evt::Versions { game, versions } => {
                    self.versions_for = Some(game);
                    self.versions = versions;
                }
                Evt::Storage(s) => self.storage = Some(s),
                Evt::Conflict {
                    game,
                    local,
                    remote,
                } => {
                    self.toasts.push(Toast {
                        msg: format!("Sync conflict for {game} — resolve it below."),
                        kind: ToastKind::Error,
                        at: Instant::now(),
                    });
                    self.conflicts.insert(game, (local, remote));
                }
                Evt::Info(m) => self.toasts.push(Toast {
                    msg: m,
                    kind: ToastKind::Info,
                    at: Instant::now(),
                }),
                Evt::Error(m) => self.toasts.push(Toast {
                    msg: m,
                    kind: ToastKind::Error,
                    at: Instant::now(),
                }),
                Evt::Busy(b) => {
                    if b {
                        self.busy += 1;
                    } else {
                        self.busy = self.busy.saturating_sub(1);
                    }
                }
            }
        }
    }

    fn apply_meta(&mut self, m: Meta) {
        self.encrypted = m.encrypted;
        self.auto_sync = m.auto_sync;
        self.compression = m.compression;
        self.known_games = m.known_games;
        self.remote = m.remote.clone();
        self.remote_buf = m.remote.unwrap_or_default();
    }

    // ---- rendering -------------------------------------------------------

    fn render_unlock(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading("GameSync");
                ui.add_space(6.0);
                ui.label("This data store is encrypted. Enter your passphrase to unlock.");
                ui.add_space(10.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.passphrase)
                        .password(true)
                        .hint_text("Passphrase")
                        .desired_width(260.0),
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                if ui.button("Unlock").clicked() || enter {
                    let _ = tx.send(Cmd::Unlock(self.passphrase.clone()));
                }
            });
        });
    }

    fn render_topbar(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("GameSync");
                ui.label(RichText::new("native").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙ Settings").clicked() {
                        self.show_settings = true;
                    }
                    if ui.button("Verify").clicked() {
                        let _ = tx.send(Cmd::Verify);
                    }
                    if ui.button("Update list").clicked() {
                        let _ = tx.send(Cmd::UpdateList);
                    }
                    if ui.button("Scan").clicked() {
                        let _ = tx.send(Cmd::Scan);
                    }
                    if ui.button("➕ Add").clicked() {
                        self.show_add = true;
                    }
                });
            });
            ui.add_space(4.0);
        });
    }

    fn render_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if self.busy > 0 {
                    ui.spinner();
                    ui.label("Working…");
                } else {
                    let text = match &self.remote {
                        Some(r) => format!("Remote: {r}"),
                        None => format!("Data: {}", self.data_dir.display()),
                    };
                    ui.label(RichText::new(text).weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} games known", self.known_games)).weak());
                });
            });
            ui.add_space(2.0);
        });
    }

    fn render_sidebar(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        egui::SidePanel::left("games")
            .resizable(true)
            .default_width(300.0)
            .min_width(220.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.strong("Games");
                    ui.label(RichText::new(format!("({})", self.games.len())).weak());
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search…")
                        .desired_width(f32::INFINITY),
                );
                ui.separator();
                let q = self.search.trim().to_lowercase();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if self.games.is_empty() {
                            ui.label("No games tracked yet.");
                            ui.label("Click Scan, or Add one manually.");
                        }
                        for g in &self.games {
                            if !q.is_empty() && !g.name.to_lowercase().contains(&q) {
                                continue;
                            }
                            let is_sel = self.selected.as_deref() == Some(g.id.as_str());
                            let mark = if g.sync_enabled { " ⟳" } else { "" };
                            let label = format!("{}\n{}{}", g.name, g.platform.as_str(), mark);
                            if ui.selectable_label(is_sel, label).clicked() {
                                self.selected = Some(g.id.clone());
                                let _ = tx.send(Cmd::Versions(g.id.clone()));
                            }
                        }
                    });
            });
    }

    fn render_central(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(sel_id) = self.selected.clone() else {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.heading("Select a game");
                    ui.label("Choose a game on the left, Scan for installed games, or Add one.");
                });
                return;
            };
            let Some(g) = self.games.iter().find(|x| x.id == sel_id).cloned() else {
                return;
            };

            // Header: name (or rename editor) + actions.
            ui.horizontal(|ui| {
                if self.renaming.as_deref() == Some(g.id.as_str()) {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.rename_buf).desired_width(240.0),
                    );
                    if ui.button("Save").clicked() {
                        let name = self.rename_buf.trim().to_string();
                        if !name.is_empty() {
                            let _ = tx.send(Cmd::Rename { id: g.id.clone(), name });
                        }
                        self.renaming = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.renaming = None;
                    }
                } else {
                    ui.heading(g.name.as_str());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Remove").clicked() {
                            self.confirm_remove = Some(g.id.clone());
                        }
                        if ui.button("Rename").clicked() {
                            self.renaming = Some(g.id.clone());
                            self.rename_buf = g.name.clone();
                        }
                    });
                }
            });
            ui.label(
                RichText::new(format!("{} · {}", g.platform.as_str(), g.save_root.display()))
                    .weak(),
            );

            // Conflict banner.
            if let Some((local, remote)) = self.conflicts.get(&g.id).cloned() {
                ui.add_space(6.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.colored_label(
                        Color32::from_rgb(210, 150, 60),
                        format!(
                            "Sync conflict: your save ({}) and the remote ({}) diverged. Your live save was not changed.",
                            short(&local),
                            short(&remote)
                        ),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Keep mine").clicked() {
                            let _ = tx.send(Cmd::Resolve { id: g.id.clone(), keep_local: true });
                            self.conflicts.remove(&g.id);
                        }
                        if ui.button("Take remote").clicked() {
                            let _ = tx.send(Cmd::Resolve { id: g.id.clone(), keep_local: false });
                            self.conflicts.remove(&g.id);
                        }
                        if ui.button("Keep both").clicked() {
                            let _ = tx.send(Cmd::Fork(g.id.clone()));
                            self.conflicts.remove(&g.id);
                        }
                    });
                });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Back up now").clicked() {
                    let _ = tx.send(Cmd::Backup(g.id.clone()));
                }
                if ui.button("Restore latest").clicked() {
                    let _ = tx.send(Cmd::RestoreLatest(g.id.clone()));
                }
                if self.remote.is_some() && ui.button("Sync now").clicked() {
                    let _ = tx.send(Cmd::Sync(g.id.clone()));
                }
                let mut sync = g.sync_enabled;
                if ui.checkbox(&mut sync, "Auto-sync this game").changed() {
                    let _ = tx.send(Cmd::ToggleSync {
                        id: g.id.clone(),
                        enabled: sync,
                    });
                }
            });

            ui.separator();
            if self.versions_for.as_deref() == Some(g.id.as_str()) {
                ui.label(format!("History — {} version(s), newest first", self.versions.len()));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for v in &self.versions {
                        ui.horizontal(|ui| {
                            ui.monospace(short(&v.version_id));
                            ui.label(v.kind.as_str());
                            ui.label(format!("{} files", v.file_count()));
                            ui.label(human_size(v.total_size));
                            ui.label(humanize_ago(v.created_ms));
                            if let Some(l) = &v.label {
                                ui.label(format!("— {l}"));
                            }
                            if ui.small_button("Restore").clicked() {
                                let _ = tx.send(Cmd::Restore {
                                    game: g.id.clone(),
                                    version: v.version_id.clone(),
                                });
                            }
                        });
                    }
                });
            } else {
                ui.label("Loading history…");
            }
        });
    }

    fn render_settings(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.heading("Appearance");
                ui.horizontal_wrapped(|ui| {
                    for t in Theme::ALL {
                        if ui.selectable_label(self.theme == t, t.name()).clicked() {
                            self.theme = t;
                            self.theme_dirty = true;
                        }
                    }
                });

                ui.separator();
                ui.heading("Sync");
                let mut s = self.auto_sync;
                let mut changed = false;
                changed |= ui
                    .checkbox(&mut s.enabled, "Automatic background sync")
                    .changed();
                ui.horizontal(|ui| {
                    ui.label("Every");
                    changed |= ui
                        .add(egui::DragValue::new(&mut s.interval_min).range(1..=1440))
                        .changed();
                    ui.label("minutes");
                });
                changed |= ui
                    .checkbox(
                        &mut s.backup_on_exit,
                        "Back up automatically when a game closes",
                    )
                    .changed();
                if changed {
                    let _ = tx.send(Cmd::SetAutoSync(s));
                }

                ui.separator();
                ui.heading("Remote");
                ui.label(
                    RichText::new("A folder path, rclone:<remote>, or lan:<token>@<host:port>")
                        .weak(),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.remote_buf)
                            .desired_width(280.0)
                            .hint_text("/path/to/Dropbox/GameSync"),
                    );
                    if ui.button("Save").clicked() {
                        let _ = tx.send(Cmd::SetRemote(self.remote_buf.trim().to_string()));
                    }
                });

                ui.separator();
                ui.heading("Storage");
                let mut comp = self.compression;
                if ui
                    .checkbox(&mut comp, "Compress stored saves (LZMA2)")
                    .changed()
                {
                    let _ = tx.send(Cmd::SetCompression(comp));
                }
                if ui.button("Calculate usage").clicked() {
                    let _ = tx.send(Cmd::FetchStorage);
                }
                if let Some(st) = &self.storage {
                    ui.label(format!(
                        "{} objects · {} on disk{}",
                        st.total_objects,
                        human_size(st.total_bytes),
                        if st.compressed { " (compressed)" } else { "" }
                    ));
                }

                ui.separator();
                ui.heading("Encryption");
                ui.label(if self.encrypted {
                    "Enabled (zero-knowledge)."
                } else {
                    "Disabled. Enable on a fresh store with the CLI: gamesync encrypt-init."
                });
            });
        self.show_settings = open;
    }

    fn render_add(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        if !self.show_add {
            return;
        }
        let mut open = self.show_add;
        egui::Window::new("Add a game")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Track a game by its save folder:");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut self.add_name).desired_width(240.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Folder");
                    ui.add(egui::TextEdit::singleline(&mut self.add_path).desired_width(240.0));
                });
                ui.add_space(6.0);
                if ui.button("Add").clicked() {
                    let name = self.add_name.trim().to_string();
                    let path = self.add_path.trim().to_string();
                    if !name.is_empty() && !path.is_empty() {
                        let _ = tx.send(Cmd::AddManual {
                            name,
                            path: PathBuf::from(path),
                        });
                        self.add_name.clear();
                        self.add_path.clear();
                        self.show_add = false;
                    }
                }
            });
        // If the window was closed via its X, `open` is false.
        if !open {
            self.show_add = false;
        }
    }

    fn render_confirm_remove(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        let Some(id) = self.confirm_remove.clone() else {
            return;
        };
        let name = self
            .games
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| id.clone());
        let mut open = true;
        egui::Window::new("Remove game?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Remove \"{name}\" from GameSync?"));
                ui.label(
                    RichText::new(
                        "This deletes its backup history. Your actual save files are NOT touched.",
                    )
                    .weak(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Remove").clicked() {
                        let _ = tx.send(Cmd::Remove(id.clone()));
                        self.confirm_remove = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_remove = None;
                    }
                });
            });
        if !open {
            self.confirm_remove = None;
        }
    }

    fn render_toasts(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -44.0))
            .show(ctx, |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    for t in self.toasts.iter().rev().take(4) {
                        let col = match t.kind {
                            ToastKind::Info => Color32::from_rgb(120, 200, 120),
                            ToastKind::Error => Color32::from_rgb(225, 110, 110),
                        };
                        ui.colored_label(col, t.msg.as_str());
                    }
                });
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.toasts
            .retain(|t| t.at.elapsed() < Duration::from_secs(6));

        let tx = self.handle.tx();

        if self.locked && !self.opened {
            self.render_unlock(ctx, &tx);
            self.render_toasts(ctx);
            return;
        }

        self.render_topbar(ctx, &tx);
        self.render_status(ctx);
        self.render_sidebar(ctx, &tx);
        self.render_central(ctx, &tx);
        self.render_settings(ctx, &tx);
        self.render_add(ctx, &tx);
        self.render_confirm_remove(ctx, &tx);
        self.render_toasts(ctx);

        if self.theme_dirty {
            ctx.set_visuals(self.theme.visuals());
            crate::theme::save(&self.data_dir, self.theme);
            self.theme_dirty = false;
        }
    }
}
