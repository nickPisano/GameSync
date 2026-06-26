//! The egui application: state, the worker connection, and all rendering.
//!
//! Layout mirrors the v0.2.5 web build: a brand topbar, an always-visible remote
//! bar, and a single column of game "cards" (not a master-detail list). Render
//! closures only read/write individual `self` fields and send [`Cmd`]s through a
//! cloned `Sender` — they never call `&mut self` methods.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use gamesync_core::{
    AutoSyncSettings, Diff, Engine, Game, PluginList, SaveFile, Snapshot, StorageReport,
};
use tray_icon::TrayIcon;

use crate::theme::{Custom, Theme};
use crate::tray::{self, TrayAction};
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

    locked: bool,
    opened: bool,
    passphrase: String,

    games: Vec<Game>,
    summaries: std::collections::HashMap<String, (usize, Option<i64>)>,
    versions_for: Option<String>,
    versions: Vec<Snapshot>,

    auto_sync: AutoSyncSettings,
    compression: bool,
    known_games: usize,
    encrypted: bool,
    remote: Option<String>,
    storage: Option<StorageReport>,

    conflicts: std::collections::HashMap<String, (String, String)>,

    theme: Theme,
    theme_dirty: bool,
    search: String,
    busy: u32,
    toasts: Vec<Toast>,

    show_settings: bool,
    show_add: bool,
    add_name: String,
    add_path: String,
    renaming: Option<String>,
    rename_buf: String,
    remote_buf: String,
    confirm_remove: Option<String>,

    gs_game: Option<String>,
    gs_extra: Vec<String>,
    gs_exe: String,

    files_game: Option<String>,
    files_for: Option<String>,
    files: Vec<SaveFile>,

    show_history: Option<String>,

    diff_result: Option<Diff>,
    show_diff: bool,

    setup_complete: bool,
    wizard_dismissed: bool,

    enc_pass: String,
    recovery_key: Option<String>,

    show_plugins: bool,
    plugins: Option<PluginList>,

    lan_hosts: Vec<(String, String)>,
    update_status: Option<String>,

    custom_theme: Option<Custom>,
    use_custom: bool,

    _tray: Option<TrayIcon>,
    tray_rx: Option<Receiver<TrayAction>>,
    quitting: bool,

    brand: Option<egui::TextureHandle>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let data_dir = Engine::default_data_dir();
        let loaded = crate::theme::load(&data_dir);
        let visuals = match (loaded.use_custom, &loaded.custom) {
            (true, Some(c)) => c.visuals(),
            _ => loaded.theme.visuals(),
        };
        cc.egui_ctx.set_visuals(visuals);
        crate::theme::apply_style(&cc.egui_ctx);
        let handle = worker::spawn(cc.egui_ctx.clone());
        let (tray, tray_rx) = match tray::setup(cc.egui_ctx.clone()) {
            Some((t, r)) => (Some(t), Some(r)),
            None => (None, None),
        };
        let brand = load_brand(&cc.egui_ctx);
        Self {
            handle,
            data_dir,
            locked: false,
            opened: false,
            passphrase: String::new(),
            games: Vec::new(),
            summaries: std::collections::HashMap::new(),
            versions_for: None,
            versions: Vec::new(),
            auto_sync: AutoSyncSettings::default(),
            compression: false,
            known_games: 0,
            encrypted: false,
            remote: None,
            storage: None,
            conflicts: std::collections::HashMap::new(),
            theme: loaded.theme,
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
            gs_game: None,
            gs_extra: Vec::new(),
            gs_exe: String::new(),
            files_game: None,
            files_for: None,
            files: Vec::new(),
            show_history: None,
            diff_result: None,
            show_diff: false,
            setup_complete: true,
            wizard_dismissed: false,
            enc_pass: String::new(),
            recovery_key: None,
            show_plugins: false,
            plugins: None,
            lan_hosts: Vec::new(),
            update_status: None,
            custom_theme: loaded.custom,
            use_custom: loaded.use_custom,
            _tray: tray,
            tray_rx,
            quitting: false,
            brand,
        }
    }

    fn accent(&self) -> Color32 {
        match (self.use_custom, &self.custom_theme) {
            (true, Some(c)) => c.accent_color(),
            _ => self.theme.accent(),
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
                    if let Some(h) = self.show_history.clone() {
                        if self.games.iter().any(|x| x.id == h) {
                            let _ = self.handle.tx().send(Cmd::Versions(h));
                        }
                    }
                }
                Evt::Summaries(s) => self.summaries = s,
                Evt::Versions { game, versions } => {
                    self.versions_for = Some(game);
                    self.versions = versions;
                }
                Evt::Storage(s) => self.storage = Some(s),
                Evt::Files { game, files } => {
                    self.files_for = Some(game);
                    self.files = files;
                }
                Evt::Diff(d) => {
                    self.diff_result = Some(d);
                    self.show_diff = true;
                }
                Evt::RecoveryKey(k) => self.recovery_key = Some(k),
                Evt::Plugins(p) => self.plugins = Some(p),
                Evt::LanHosts(h) => self.lan_hosts = h,
                Evt::Update(opt) => {
                    self.update_status = Some(match opt {
                        Some(v) => format!("Update available: v{v}"),
                        None => "You're on the latest version.".to_string(),
                    })
                }
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
        self.setup_complete = m.setup_complete;
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
        let accent = self.accent();
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if let Some(tex) = &self.brand {
                    ui.image(egui::load::SizedTexture::new(
                        tex.id(),
                        egui::vec2(30.0, 30.0),
                    ));
                } else {
                    ui.label(
                        RichText::new(" GS ")
                            .size(18.0)
                            .strong()
                            .color(Color32::WHITE)
                            .background_color(accent),
                    );
                }
                ui.add_space(4.0);
                ui.label(RichText::new("GameSync").size(18.0).strong());
                ui.label(RichText::new("safe save backup & sync").weak().small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Settings").clicked() {
                        self.show_settings = true;
                    }
                    if ui.button("Plugins").clicked() {
                        self.show_plugins = true;
                        let _ = tx.send(Cmd::ListPlugins);
                    }
                    if ui.button("Add game").clicked() {
                        self.show_add = true;
                    }
                    if ui.button("Scan").clicked() {
                        let _ = tx.send(Cmd::Scan);
                    }
                    if ui
                        .add_enabled(
                            self.remote.is_some(),
                            egui::Button::new(RichText::new("Sync all").color(Color32::WHITE))
                                .fill(accent),
                        )
                        .clicked()
                    {
                        let _ = tx.send(Cmd::SyncAll);
                    }
                });
            });
            ui.add_space(6.0);
        });
    }

    fn render_remote_bar(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        let accent = self.accent();
        egui::TopBottomPanel::top("remote").show(ctx, |ui| {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("REMOTE").small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(if self.remote.is_some() {
                            "set"
                        } else {
                            "not set"
                        })
                        .small()
                        .weak(),
                    );
                    if primary_button(ui, "Save", accent).clicked() {
                        let _ = tx.send(Cmd::SetRemote(self.remote_buf.trim().to_string()));
                    }
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.remote_buf = p.display().to_string();
                        }
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.remote_buf)
                            .desired_width(f32::INFINITY)
                            .hint_text("Path to a shared/cloud folder (e.g. ~/Dropbox/GameSync)"),
                    );
                });
            });
            ui.add_space(5.0);
        });
    }

    fn render_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                if self.busy > 0 {
                    ui.spinner();
                    ui.label("Working…");
                } else {
                    let enc = if self.encrypted {
                        "Encrypted"
                    } else {
                        "Unencrypted"
                    };
                    let rem = if self.remote.is_some() {
                        "remote set"
                    } else {
                        "no remote"
                    };
                    ui.label(
                        RichText::new(format!("{enc} · {} games · {rem}", self.games.len())).weak(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(self.data_dir.display().to_string())
                            .weak()
                            .small(),
                    );
                });
            });
            ui.add_space(3.0);
        });
    }

    fn render_library(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        let accent = self.accent();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Games ({})", self.games.len())).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Filter…")
                            .desired_width(200.0),
                    );
                });
            });
            ui.add_space(4.0);
            let q = self.search.trim().to_lowercase();
            let card_fill = ui.visuals().faint_bg_color;
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    if self.games.is_empty() {
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            ui.label("No games tracked yet.");
                            ui.label("Click Scan to detect installed games, or Add game.");
                        });
                    }
                    for g in &self.games {
                        if !q.is_empty() && !g.name.to_lowercase().contains(&q) {
                            continue;
                        }
                        let (count, last) = self.summaries.get(&g.id).copied().unwrap_or((0, None));
                        egui::Frame::group(ui.style())
                            .fill(card_fill)
                            .inner_margin(egui::Margin::same(14))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                let full = ui.available_width();
                                let left_w = (full - 360.0).max(160.0);
                                let right_w = full - left_w;
                                ui.horizontal(|ui| {
                                    // Left column: name/badge, path, status, links.
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(left_w, 0.0),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.set_width(left_w);
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(g.name.as_str())
                                                        .size(16.0)
                                                        .strong(),
                                                );
                                                badge(ui, g.platform.as_str());
                                            });
                                            ui.label(
                                                RichText::new(g.save_root.display().to_string())
                                                    .weak()
                                                    .small(),
                                            );
                                            let last_str = last
                                                .map(humanize_ago)
                                                .unwrap_or_else(|| "never".to_string());
                                            ui.label(
                                                RichText::new(format!(
                                                    "{count} versions · last backup {last_str}"
                                                ))
                                                .weak()
                                                .small(),
                                            );
                                            if self.auto_sync.backup_on_exit {
                                                ui.label(
                                                    RichText::new(
                                                        "backs up automatically when it closes",
                                                    )
                                                    .weak()
                                                    .small(),
                                                );
                                            }
                                            if let Some((local, remote)) =
                                                self.conflicts.get(&g.id).cloned()
                                            {
                                                ui.add_space(4.0);
                                                ui.colored_label(
                                                    Color32::from_rgb(220, 150, 60),
                                                    format!(
                                                        "Conflict: yours ({}) vs remote ({}).",
                                                        short(&local),
                                                        short(&remote)
                                                    ),
                                                );
                                                ui.horizontal(|ui| {
                                                    if ui.button("Keep mine").clicked() {
                                                        let _ = tx.send(Cmd::Resolve {
                                                            id: g.id.clone(),
                                                            keep_local: true,
                                                        });
                                                        self.conflicts.remove(&g.id);
                                                    }
                                                    if ui.button("Take remote").clicked() {
                                                        let _ = tx.send(Cmd::Resolve {
                                                            id: g.id.clone(),
                                                            keep_local: false,
                                                        });
                                                        self.conflicts.remove(&g.id);
                                                    }
                                                    if ui.button("Keep both").clicked() {
                                                        let _ = tx.send(Cmd::Fork(g.id.clone()));
                                                        self.conflicts.remove(&g.id);
                                                    }
                                                });
                                            }
                                            ui.add_space(2.0);
                                            ui.horizontal(|ui| {
                                                if ui.link("Rename").clicked() {
                                                    self.renaming = Some(g.id.clone());
                                                    self.rename_buf = g.name.clone();
                                                }
                                                ui.label(RichText::new("·").weak());
                                                if ui.link("Redirect to synced folder").clicked() {
                                                    self.gs_game = Some(g.id.clone());
                                                    self.gs_extra = g
                                                        .extra_roots
                                                        .iter()
                                                        .map(|p| p.display().to_string())
                                                        .collect();
                                                    self.gs_exe = g
                                                        .install_dir
                                                        .as_ref()
                                                        .map(|p| p.display().to_string())
                                                        .unwrap_or_default();
                                                }
                                                ui.label(RichText::new("·").weak());
                                                if ui
                                                    .add(
                                                        egui::Label::new(
                                                            RichText::new("Remove").color(
                                                                Color32::from_rgb(0xe0, 0x6c, 0x6c),
                                                            ),
                                                        )
                                                        .sense(egui::Sense::click()),
                                                    )
                                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                    .clicked()
                                                {
                                                    self.confirm_remove = Some(g.id.clone());
                                                }
                                            });
                                        },
                                    );
                                    // Right column: toggle + action buttons, right-aligned.
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(right_w, 0.0),
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if tonal_button(
                                                ui,
                                                "Sync now",
                                                accent,
                                                self.remote.is_some(),
                                            )
                                            .clicked()
                                            {
                                                let _ = tx.send(Cmd::Sync(g.id.clone()));
                                            }
                                            if tonal_button(ui, "History", accent, count > 0)
                                                .clicked()
                                            {
                                                self.show_history = Some(g.id.clone());
                                                let _ = tx.send(Cmd::Versions(g.id.clone()));
                                            }
                                            if tonal_button(ui, "Files", accent, true).clicked() {
                                                self.files_game = Some(g.id.clone());
                                                self.files.clear();
                                                self.files_for = None;
                                                let _ = tx.send(Cmd::ListFiles(g.id.clone()));
                                            }
                                            if tonal_button(ui, "Back up", accent, true).clicked() {
                                                let _ = tx.send(Cmd::Backup(g.id.clone()));
                                            }
                                            ui.label(RichText::new("Sync").small().weak());
                                            let mut sync = g.sync_enabled;
                                            if toggle_switch(ui, &mut sync, accent).changed() {
                                                let _ = tx.send(Cmd::ToggleSync {
                                                    id: g.id.clone(),
                                                    enabled: sync,
                                                });
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(8.0);
                    }
                });
        });
    }

    fn render_history(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        let Some(game) = self.show_history.clone() else {
            return;
        };
        let accent = self.accent();
        let mut open = true;
        egui::Window::new("History")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(580.0)
            .show(ctx, |ui| {
                if self.versions_for.as_deref() != Some(game.as_str()) {
                    ui.label("Loading…");
                } else if self.versions.is_empty() {
                    ui.label("No versions yet. Use Back up to create one.");
                } else {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .max_height(400.0)
                        .show(ui, |ui| {
                            for v in &self.versions {
                                ui.horizontal(|ui| {
                                    if primary_button(ui, "Restore", accent).clicked() {
                                        let _ = tx.send(Cmd::Restore {
                                            game: game.clone(),
                                            version: v.version_id.clone(),
                                        });
                                    }
                                    if let Some(parent) = &v.parent {
                                        if tonal_button(ui, "Diff", accent, true).clicked() {
                                            let _ = tx.send(Cmd::Diff {
                                                game: game.clone(),
                                                from: parent.clone(),
                                                to: v.version_id.clone(),
                                            });
                                        }
                                    }
                                    ui.monospace(short(&v.version_id));
                                    ui.label(v.kind.as_str());
                                    ui.label(format!("{} files", v.file_count()));
                                    ui.label(human_size(v.total_size));
                                    ui.label(humanize_ago(v.created_ms));
                                    if let Some(l) = &v.label {
                                        ui.label(format!("— {l}"));
                                    }
                                });
                            }
                        });
                }
            });
        if !open {
            self.show_history = None;
        }
    }

    fn render_rename(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        let Some(id) = self.renaming.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Rename game")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.rename_buf).desired_width(260.0));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        let name = self.rename_buf.trim().to_string();
                        if !name.is_empty() {
                            let _ = tx.send(Cmd::Rename {
                                id: id.clone(),
                                name,
                            });
                        }
                        self.renaming = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.renaming = None;
                    }
                });
            });
        if !open {
            self.renaming = None;
        }
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
                        if ui
                            .selectable_label(!self.use_custom && self.theme == t, t.name())
                            .clicked()
                        {
                            self.theme = t;
                            self.use_custom = false;
                            self.theme_dirty = true;
                        }
                    }
                    if let Some(c) = &self.custom_theme {
                        if ui
                            .selectable_label(self.use_custom, c.name.as_str())
                            .clicked()
                        {
                            self.use_custom = true;
                            self.theme_dirty = true;
                        }
                    }
                });
                if ui.button("Import theme (JSON)…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("JSON", &["json"])
                        .pick_file()
                    {
                        match std::fs::read_to_string(&p)
                            .ok()
                            .and_then(|s| crate::theme::parse_custom(&s))
                        {
                            Some(c) => {
                                self.custom_theme = Some(c);
                                self.use_custom = true;
                                self.theme_dirty = true;
                            }
                            None => self.toasts.push(Toast {
                                msg: "Couldn't parse that theme file.".to_string(),
                                kind: ToastKind::Error,
                                at: Instant::now(),
                            }),
                        }
                    }
                }

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
                ui.heading("Storage");
                let mut comp = self.compression;
                if ui
                    .checkbox(&mut comp, "Compress stored saves (LZMA2)")
                    .changed()
                {
                    let _ = tx.send(Cmd::SetCompression(comp));
                }
                ui.horizontal(|ui| {
                    if ui.button("Calculate usage").clicked() {
                        let _ = tx.send(Cmd::FetchStorage);
                    }
                    if ui.button("Verify integrity").clicked() {
                        let _ = tx.send(Cmd::Verify);
                    }
                });
                if let Some(st) = &self.storage {
                    ui.label(format!(
                        "{} objects · {} on disk{}",
                        st.total_objects,
                        human_size(st.total_bytes),
                        if st.compressed { " (compressed)" } else { "" }
                    ));
                }

                ui.separator();
                ui.heading("Game detection");
                ui.label(format!(
                    "{} games in the detection database",
                    self.known_games
                ));
                if ui.button("Update game list").clicked() {
                    let _ = tx.send(Cmd::UpdateList);
                }

                ui.separator();
                ui.heading("Encryption");
                if self.encrypted {
                    ui.label("Enabled (zero-knowledge).");
                } else {
                    ui.label(
                        RichText::new(
                            "Encrypt all stored saves. Only possible on an empty store; you'll \
                             get a one-time recovery key.",
                        )
                        .weak(),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.enc_pass)
                                .password(true)
                                .hint_text("Passphrase (min 8 chars)")
                                .desired_width(240.0),
                        );
                        if ui.button("Enable encryption").clicked() && self.enc_pass.len() >= 8 {
                            let _ = tx.send(Cmd::EnableEncryption(self.enc_pass.clone()));
                            self.enc_pass.clear();
                        }
                    });
                }

                ui.separator();
                ui.heading("LAN");
                ui.horizontal(|ui| {
                    if ui.button("Find LAN hosts").clicked() {
                        let _ = tx.send(Cmd::DiscoverLan);
                    }
                    if !self.lan_hosts.is_empty() {
                        ui.label(RichText::new(format!("{} found", self.lan_hosts.len())).weak());
                    }
                });
                for (name, endpoint) in &self.lan_hosts {
                    ui.horizontal(|ui| {
                        if ui.small_button("Use").clicked() {
                            self.remote_buf = format!("lan:@{endpoint}");
                        }
                        ui.label(format!("{name} — {endpoint}"));
                    });
                }

                ui.separator();
                ui.heading("Updates");
                if ui.button("Check for updates").clicked() {
                    let _ = tx.send(Cmd::CheckUpdate);
                }
                if let Some(s) = &self.update_status {
                    ui.label(s.as_str());
                }
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
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.add_path = p.display().to_string();
                        }
                    }
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
        if !open {
            self.show_add = false;
        }
    }

    fn render_game_settings(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        let Some(id) = self.gs_game.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Game settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(500.0)
            .show(ctx, |ui| {
                ui.heading("Extra backup folders");
                ui.label(
                    RichText::new("Backed up and restored together with the save folder.").weak(),
                );
                let mut remove_idx = None;
                for (i, r) in self.gs_extra.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("✕").clicked() {
                            remove_idx = Some(i);
                        }
                        ui.label(r.as_str());
                    });
                }
                if let Some(i) = remove_idx {
                    self.gs_extra.remove(i);
                }
                ui.horizontal(|ui| {
                    if ui.button("Add folder…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.gs_extra.push(p.display().to_string());
                        }
                    }
                    if ui.button("Save extra folders").clicked() {
                        let roots: Vec<PathBuf> = self
                            .gs_extra
                            .iter()
                            .map(|s| PathBuf::from(s.as_str()))
                            .collect();
                        let _ = tx.send(Cmd::SetExtraRoots {
                            id: id.clone(),
                            roots,
                        });
                    }
                });

                ui.separator();
                ui.heading("Close-detection location");
                ui.label(
                    RichText::new(
                        "The game's install folder; GameSync watches it to back up automatically \
                         a few seconds after the game exits.",
                    )
                    .weak(),
                );
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.gs_exe).desired_width(300.0));
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.gs_exe = p.display().to_string();
                        }
                    }
                });
                if ui.button("Save location").clicked() {
                    let path = if self.gs_exe.trim().is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(self.gs_exe.trim()))
                    };
                    let _ = tx.send(Cmd::SetGameExe {
                        id: id.clone(),
                        path,
                    });
                }

                ui.separator();
                ui.heading("Redirect save folder");
                ui.label(
                    RichText::new(
                        "Move this game's saves into a synced folder (e.g. OneDrive) and leave a \
                         link behind, so they sync even when GameSync isn't running.",
                    )
                    .weak(),
                );
                if ui.button("Choose target & redirect…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        let _ = tx.send(Cmd::Redirect {
                            id: id.clone(),
                            target: p,
                        });
                    }
                }
            });
        if !open {
            self.gs_game = None;
        }
    }

    fn render_files(&mut self, ctx: &egui::Context) {
        let Some(game) = self.files_game.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Files")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(580.0)
            .show(ctx, |ui| {
                if self.files_for.as_deref() != Some(game.as_str()) {
                    ui.label("Loading…");
                } else if self.files.is_empty() {
                    ui.label("No files in the save folder.");
                } else {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .max_height(380.0)
                        .show(ui, |ui| {
                            for f in &self.files {
                                ui.horizontal(|ui| {
                                    if ui.small_button("Reveal").clicked() {
                                        reveal(&f.abs_path);
                                    }
                                    ui.label(human_size(f.size));
                                    ui.monospace(f.rel_path.as_str());
                                });
                            }
                        });
                }
            });
        if !open {
            self.files_game = None;
        }
    }

    fn render_diff(&mut self, ctx: &egui::Context) {
        if !self.show_diff {
            return;
        }
        let mut open = self.show_diff;
        egui::Window::new("Changes")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(500.0)
            .show(ctx, |ui| {
                if let Some(d) = &self.diff_result {
                    if d.is_empty() {
                        ui.label(format!("No changes ({} files identical).", d.unchanged));
                    } else {
                        ui.label(format!(
                            "{} changed · {} unchanged",
                            d.changed_count(),
                            d.unchanged
                        ));
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .auto_shrink(false)
                            .max_height(380.0)
                            .show(ui, |ui| {
                                for p in &d.added {
                                    ui.colored_label(
                                        Color32::from_rgb(120, 200, 120),
                                        format!("+ {p}"),
                                    );
                                }
                                for p in &d.modified {
                                    ui.colored_label(
                                        Color32::from_rgb(210, 180, 90),
                                        format!("~ {p}"),
                                    );
                                }
                                for p in &d.removed {
                                    ui.colored_label(
                                        Color32::from_rgb(225, 110, 110),
                                        format!("- {p}"),
                                    );
                                }
                            });
                    }
                }
            });
        self.show_diff = open;
    }

    fn render_plugins(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        if !self.show_plugins {
            return;
        }
        let mut open = self.show_plugins;
        egui::Window::new("Plugins")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                if let Some(pl) = &self.plugins {
                    ui.label(RichText::new(format!("Folder: {}", pl.dir)).weak());
                    let mut allowed = pl.commands_allowed;
                    if ui
                        .checkbox(
                            &mut allowed,
                            "Allow plugins to run shell commands (hooks & viewers)",
                        )
                        .changed()
                    {
                        let _ = tx.send(Cmd::SetCommandsAllowed(allowed));
                    }
                    ui.separator();
                    if pl.plugins.is_empty() {
                        ui.label("No plugins installed.");
                    }
                    for p in &pl.plugins {
                        ui.horizontal(|ui| {
                            let mut en = p.enabled;
                            if ui.checkbox(&mut en, "").changed() {
                                let _ = tx.send(Cmd::SetPluginEnabled {
                                    id: p.id.clone(),
                                    enabled: en,
                                });
                            }
                            ui.strong(p.name.as_str());
                            ui.label(
                                RichText::new(format!(
                                    "{} games · {} emulators · {} hooks · {} viewers",
                                    p.games, p.emulators, p.hooks, p.viewers
                                ))
                                .weak(),
                            );
                        });
                    }
                    for (id, err) in &pl.errors {
                        ui.colored_label(Color32::from_rgb(225, 110, 110), format!("{id}: {err}"));
                    }
                } else {
                    ui.label("Loading…");
                }
            });
        self.show_plugins = open;
    }

    fn render_recovery(&mut self, ctx: &egui::Context) {
        let Some(key) = self.recovery_key.clone() else {
            return;
        };
        let mut close = false;
        egui::Window::new("Recovery key — write this down")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    "Encryption is enabled. This recovery key can unlock your saves if you \
                     forget the passphrase. It is shown only once:",
                );
                ui.add_space(8.0);
                ui.monospace(key.as_str());
                ui.add_space(8.0);
                if ui.button("I've written it down").clicked() {
                    close = true;
                }
            });
        if close {
            self.recovery_key = None;
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

    fn render_wizard(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                ui.heading("Welcome to GameSync");
                ui.add_space(8.0);
                ui.label("Back up and sync your game saves safely. Let's find your games.");
                ui.add_space(16.0);
                if ui.button("Scan for installed games").clicked() {
                    let _ = tx.send(Cmd::Scan);
                }
                ui.add_space(4.0);
                ui.label(RichText::new("You can also add games manually with Add game.").weak());
                ui.add_space(20.0);
                if ui.button("Get started").clicked() {
                    let _ = tx.send(Cmd::MarkSetupComplete);
                    self.wizard_dismissed = true;
                }
            });
        });
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

/// Decode the bundled brand PNG into an egui texture for the topbar logo.
fn load_brand(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let bytes = include_bytes!("../assets/brand.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let pixels = img.into_raw();
    let color = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    Some(ctx.load_texture("brand", color, egui::TextureOptions::LINEAR))
}

/// Open the OS file manager at `path` (revealing the file where supported).
fn reveal(path: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.replace('/', "\\")))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

/// A filled accent button with white text.
fn primary_button(ui: &mut egui::Ui, text: &str, accent: Color32) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(accent))
}

/// A tonal button: accent text, faint accent fill, thin accent border. Greyed
/// out when `enabled` is false (for not-yet-applicable actions).
fn tonal_button(ui: &mut egui::Ui, text: &str, accent: Color32, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(RichText::new(text).color(accent))
        .fill(accent.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.45)));
    ui.add_enabled(enabled, btn)
}

/// A small readable pill badge with the first letter capitalized.
fn badge(ui: &mut egui::Ui, text: &str) {
    let mut label = String::new();
    let mut chars = text.chars();
    if let Some(first) = chars.next() {
        label.extend(first.to_uppercase());
        label.push_str(chars.as_str());
    }
    ui.label(
        RichText::new(format!(" {label} "))
            .small()
            .strong()
            .color(Color32::from_rgb(0xd6, 0xd2, 0xe6))
            .background_color(Color32::from_rgb(0x39, 0x34, 0x4a)),
    );
}

/// A sliding on/off switch. Returns a response whose `.changed()` reflects toggles.
fn toggle_switch(ui: &mut egui::Ui, on: &mut bool, accent: Color32) -> egui::Response {
    let size = egui::vec2(34.0, 18.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let r = rect.height() * 0.5;
    let track = if *on { accent } else { Color32::from_gray(90) };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(r as u8), track);
    let cx = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    ui.painter()
        .circle_filled(egui::pos2(cx, rect.center().y), r * 0.72, Color32::WHITE);
    resp
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.toasts
            .retain(|t| t.at.elapsed() < Duration::from_secs(6));

        let tx = self.handle.tx();

        // Tray menu actions.
        if let Some(rx) = &self.tray_rx {
            while let Ok(action) = rx.try_recv() {
                match action {
                    TrayAction::Open => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    TrayAction::SyncAll => {
                        let _ = tx.send(Cmd::SyncAll);
                    }
                    TrayAction::Quit => {
                        self.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        if self._tray.is_some() && !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        if self.locked && !self.opened {
            self.render_unlock(ctx, &tx);
            self.render_toasts(ctx);
            return;
        }

        if self.opened && !self.setup_complete && !self.wizard_dismissed {
            self.render_wizard(ctx, &tx);
            self.render_toasts(ctx);
            return;
        }

        self.render_topbar(ctx, &tx);
        self.render_remote_bar(ctx, &tx);
        self.render_status(ctx);
        self.render_library(ctx, &tx);
        self.render_settings(ctx, &tx);
        self.render_add(ctx, &tx);
        self.render_game_settings(ctx, &tx);
        self.render_files(ctx);
        self.render_diff(ctx);
        self.render_history(ctx, &tx);
        self.render_rename(ctx, &tx);
        self.render_plugins(ctx, &tx);
        self.render_recovery(ctx);
        self.render_confirm_remove(ctx, &tx);
        self.render_toasts(ctx);

        if self.theme_dirty {
            let visuals = match (self.use_custom, &self.custom_theme) {
                (true, Some(c)) => c.visuals(),
                _ => self.theme.visuals(),
            };
            ctx.set_visuals(visuals);
            crate::theme::save(
                &self.data_dir,
                self.theme,
                &self.custom_theme,
                self.use_custom,
            );
            self.theme_dirty = false;
        }
    }
}
