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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Recent,
    Versions,
    Platform,
}

impl SortKey {
    const ALL: [SortKey; 4] = [
        SortKey::Name,
        SortKey::Recent,
        SortKey::Versions,
        SortKey::Platform,
    ];

    fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name (A–Z)",
            SortKey::Recent => "Recently backed up",
            SortKey::Versions => "Most versions",
            SortKey::Platform => "Platform",
        }
    }
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
    sort_key: SortKey,
    /// Whether the library controls row (count + sort + filter) is shown.
    show_filters: bool,
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

    /// Enable-encryption modal: open flag + passphrase/confirm buffers.
    show_encryption: bool,
    /// Disable-encryption confirmation modal.
    show_disable_encryption: bool,
    enc_pass: String,
    enc_confirm: String,
    recovery_key: Option<String>,

    show_plugins: bool,
    plugins: Option<PluginList>,

    lan_hosts: Vec<(String, String)>,
    /// The connect spec (`lan:token@ip:port`) when this device is hosting.
    lan_hosting: Option<String>,
    update_status: Option<String>,

    custom_theme: Option<Custom>,
    use_custom: bool,
    /// Follow the OS light/dark setting (resolves `theme` to Light/Midnight).
    use_auto: bool,
    /// Whether the "all themes" gallery modal is open.
    show_themes: bool,
    /// Whether the paste-JSON theme importer is showing, and its buffer.
    theme_import_open: bool,
    theme_import_text: String,

    _tray: Option<TrayIcon>,
    tray_rx: Option<Receiver<TrayAction>>,
    quitting: bool,

    brand: Option<egui::TextureHandle>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let data_dir = Engine::default_data_dir();
        let loaded = crate::theme::load(&data_dir);
        // When following the OS, resolve the concrete theme up front.
        let theme = if loaded.use_auto && !loaded.use_custom {
            resolve_auto(&cc.egui_ctx)
        } else {
            loaded.theme
        };
        let visuals = match (loaded.use_custom, &loaded.custom) {
            (true, Some(c)) => c.visuals(),
            _ => theme.visuals(),
        };
        cc.egui_ctx.set_visuals(visuals);
        crate::theme::apply_fonts(&cc.egui_ctx);
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
            theme,
            theme_dirty: false,
            search: String::new(),
            sort_key: SortKey::Recent,
            show_filters: false,
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
            show_encryption: false,
            show_disable_encryption: false,
            enc_pass: String::new(),
            enc_confirm: String::new(),
            recovery_key: None,
            show_plugins: false,
            plugins: None,
            lan_hosts: Vec::new(),
            lan_hosting: None,
            update_status: None,
            custom_theme: loaded.custom,
            use_custom: loaded.use_custom,
            use_auto: loaded.use_auto,
            show_themes: false,
            theme_import_open: false,
            theme_import_text: String::new(),
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

    /// Whether the active theme paints a gradient/bubbles background.
    fn fancy_theme(&self) -> bool {
        self.use_custom && self.custom_theme.as_ref().is_some_and(|c| c.is_fancy())
    }

    /// Frame for the top/remote bars: always a solid, opaque fill so the
    /// gradient/bubbles background does not show through them. Fancy themes make
    /// `panel_fill` transparent, so use the opaque panel color (`window_fill`)
    /// for them; other themes already have an opaque `panel_fill`.
    fn bar_frame(&self, ctx: &egui::Context, v: i8) -> egui::Frame {
        let fill = if self.fancy_theme() {
            ctx.style().visuals.window_fill
        } else {
            ctx.style().visuals.panel_fill
        };
        egui::Frame::default()
            .fill(fill)
            .inner_margin(egui::Margin {
                left: 22,
                right: 22,
                top: v,
                bottom: v,
            })
    }

    /// The active theme's primary-button glow color, if it enables `effects.glow`.
    fn glow(&self) -> Option<Color32> {
        if self.use_custom {
            self.custom_theme.as_ref().and_then(|c| c.glow_color())
        } else {
            None
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
                Evt::LanHosting(spec) => self.lan_hosting = spec,
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
        let glow = self.glow();
        egui::TopBottomPanel::top("top")
            .frame(self.bar_frame(ctx, 13))
            .show(ctx, |ui| {
                translucent_button_fills(ui);
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
                            // Refresh storage usage so it's shown without a click.
                            let _ = tx.send(Cmd::FetchStorage);
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
                        let enabled = self.remote.is_some();
                        // Reserve halo slots before the button so they sit behind it.
                        let slots: Vec<egui::layers::ShapeIdx> = if enabled && glow.is_some() {
                            (0..GLOW_LAYERS.len())
                                .map(|_| ui.painter().add(egui::Shape::Noop))
                                .collect()
                        } else {
                            Vec::new()
                        };
                        let sync = ui.add_enabled(
                            enabled,
                            egui::Button::new(RichText::new("Sync all").color(Color32::WHITE))
                                .fill(accent),
                        );
                        if let Some(g) = glow {
                            if enabled {
                                fill_glow(ui.painter(), &slots, sync.rect, g);
                            }
                        }
                        if sync.clicked() {
                            let _ = tx.send(Cmd::SyncAll);
                        }
                        // Toggle the library controls row (game count + sort + filter).
                        ui.toggle_value(&mut self.show_filters, "Filters");
                    });
                });
            });
    }

    fn render_remote_bar(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        let accent = self.accent();
        let glow = self.glow();
        egui::TopBottomPanel::top("remote")
            .frame(self.bar_frame(ctx, 11))
            .show(ctx, |ui| {
                // Match the library bar's control height so the row has a known
                // height; otherwise the short "REMOTE"/"not set" labels are
                // placed before the taller buttons and get pinned to the top
                // instead of vertically centering.
                ui.spacing_mut().interact_size.y = 32.0;
                ui.spacing_mut().button_padding.y = 7.0;
                translucent_button_fills(ui);
                ui.horizontal(|ui| {
                    ui.set_min_height(32.0);
                    ui.label(RichText::new("REMOTE").small().weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.set_min_height(32.0);
                        if self.remote.is_some() {
                            // Green "set" pill, matching the web build's --ok
                            // (#3fb950 on dark themes, #1a7f37 on light).
                            let ok = if ui.visuals().dark_mode {
                                Color32::from_rgb(0x3f, 0xb9, 0x50)
                            } else {
                                Color32::from_rgb(0x1a, 0x7f, 0x37)
                            };
                            pill_badge(ui, "set", ok);
                        } else {
                            pill_badge(ui, "not set", Color32::from_rgb(0x9a, 0xa6, 0xb2));
                        }
                        if primary_button(ui, "Save", accent, glow).clicked() {
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
                                // Taller inner margin so the field matches the
                                // 30px buttons and fills the bar vertically.
                                .margin(egui::Margin::symmetric(8, 7))
                                .vertical_align(egui::Align::Center)
                                .hint_text(
                                    "Path to a shared/cloud folder (e.g. ~/Dropbox/GameSync)",
                                ),
                        );
                    });
                });
            });
    }

    fn render_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(panel_frame(ctx, 7))
            .show(ctx, |ui| {
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
                            RichText::new(format!("{enc} · {} games · {rem}", self.games.len()))
                                .weak(),
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
            });
    }

    fn render_library(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        let accent = self.accent();
        // Sorted order over the games (matches the web build's sort options).
        let mut order: Vec<usize> = (0..self.games.len()).collect();
        let sort_key = self.sort_key;
        order.sort_by(|&a, &b| {
            let ga = &self.games[a];
            let gb = &self.games[b];
            match sort_key {
                SortKey::Recent => {
                    let la = self
                        .summaries
                        .get(&ga.id)
                        .and_then(|s| s.1)
                        .unwrap_or(i64::MIN);
                    let lb = self
                        .summaries
                        .get(&gb.id)
                        .and_then(|s| s.1)
                        .unwrap_or(i64::MIN);
                    lb.cmp(&la)
                }
                SortKey::Versions => {
                    let va = self.summaries.get(&ga.id).map(|s| s.0).unwrap_or(0);
                    let vb = self.summaries.get(&gb.id).map(|s| s.0).unwrap_or(0);
                    vb.cmp(&va)
                }
                SortKey::Platform => ga
                    .platform
                    .as_str()
                    .cmp(gb.platform.as_str())
                    .then_with(|| ga.name.to_lowercase().cmp(&gb.name.to_lowercase())),
                SortKey::Name => ga.name.to_lowercase().cmp(&gb.name.to_lowercase()),
            }
        });
        egui::CentralPanel::default()
            .frame(panel_frame(ctx, 20))
            .show(ctx, |ui| {
                // The game count + sort + filter live behind the topbar "Filters"
                // toggle, so the library view is uncluttered by default.
                if self.show_filters {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Games ({})", self.games.len())).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Force the dropdown + filter to the same height. The
                            // combo button height is driven by button_padding, so
                            // cap that here and size the text field to match.
                            ui.spacing_mut().interact_size.y = 32.0;
                            ui.spacing_mut().button_padding.y = 7.0;
                            ui.add_sized(
                                egui::vec2(176.0, 32.0),
                                egui::TextEdit::singleline(&mut self.search)
                                    .hint_text("Filter…")
                                    .vertical_align(egui::Align::Center),
                            );
                            egui::ComboBox::from_id_salt("sort")
                                .selected_text(self.sort_key.label())
                                .width(176.0)
                                .show_ui(ui, |ui| {
                                    // Roomier, full-width rows for a cleaner popup.
                                    ui.spacing_mut().item_spacing.y = 3.0;
                                    ui.spacing_mut().button_padding = egui::vec2(10.0, 7.0);
                                    ui.set_min_width(176.0);
                                    for key in SortKey::ALL {
                                        ui.selectable_value(&mut self.sort_key, key, key.label());
                                    }
                                });
                            ui.label(RichText::new("Sort").small().weak());
                        });
                    });
                    ui.add_space(4.0);
                }
                let q = self.search.trim().to_lowercase();
                // Under a "fancy" theme (gradient/bubbles painted on the
                // background), make the cards translucent so the effect shows
                // through them instead of being hidden by an opaque fill.
                let fancy =
                    self.use_custom && self.custom_theme.as_ref().is_some_and(|c| c.is_fancy());
                let card_fill = {
                    let c = ui.visuals().faint_bg_color;
                    let a = if fancy { 140 } else { c.a() };
                    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
                };
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
                        for &gi in &order {
                            let g = &self.games[gi];
                            if !q.is_empty() && !g.name.to_lowercase().contains(&q) {
                                continue;
                            }
                            let (count, last) =
                                self.summaries.get(&g.id).copied().unwrap_or((0, None));
                            let card = egui::Frame::group(ui.style())
                                .fill(card_fill)
                                .corner_radius(egui::CornerRadius::same(15))
                                .inner_margin(egui::Margin::same(16))
                                .show(ui, |ui| {
                                    // Split the row into a left column + a fixed-width right
                                    // column, accounting for the horizontal item spacing so the
                                    // total never exceeds the available width (which would feed
                                    // back and push the buttons further right each frame).
                                    let gap = ui.spacing().item_spacing.x;
                                    let full = ui.available_width();
                                    let right_w = 392.0_f32.min((full - gap - 120.0).max(120.0));
                                    let left_w = full - right_w - gap;
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Min),
                                        |ui| {
                                            // Left column: name/badge, path, status, links.
                                            let left_col = ui.allocate_ui_with_layout(
                                                egui::vec2(left_w, 0.0),
                                                egui::Layout::top_down(egui::Align::Min),
                                                |ui| {
                                                    ui.set_width(left_w);
                                                    // Tighten the line spacing to match the
                                                    // web build's compact card (path/meta
                                                    // lines are 4–5px apart, not egui's 6px).
                                                    ui.spacing_mut().item_spacing.y = 4.0;
                                                    title_with_badge(
                                                        ui,
                                                        g.name.as_str(),
                                                        g.platform.as_str(),
                                                    );
                                                    // A touch more breathing room
                                                    // between the title and the path.
                                                    ui.add_space(2.0);
                                                    ui.label(
                                                        RichText::new(
                                                            g.save_root.display().to_string(),
                                                        )
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
                                                                let _ = tx
                                                                    .send(Cmd::Fork(g.id.clone()));
                                                                self.conflicts.remove(&g.id);
                                                            }
                                                        });
                                                    }
                                                    ui.add_space(1.0);
                                                    ui.horizontal(|ui| {
                                                        if ui.link("Rename").clicked() {
                                                            self.renaming = Some(g.id.clone());
                                                            self.rename_buf = g.name.clone();
                                                        }
                                                        ui.label(RichText::new("·").weak());
                                                        if ui.link("Settings").clicked() {
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
                                                        .link("Redirect to synced folder")
                                                        .on_hover_text(
                                                            "Move this save folder into a synced \
                                                             folder (e.g. OneDrive) and leave a \
                                                             symlink",
                                                        )
                                                        .clicked()
                                                    {
                                                        if let Some(p) = rfd::FileDialog::new()
                                                            .set_title(
                                                                "Choose a synced folder (e.g. \
                                                                 OneDrive or Google Drive)",
                                                            )
                                                            .pick_folder()
                                                        {
                                                            let _ = tx.send(Cmd::Redirect {
                                                                id: g.id.clone(),
                                                                target: p,
                                                            });
                                                        }
                                                    }
                                                        ui.label(RichText::new("·").weak());
                                                        if ui
                                                            .add(
                                                                egui::Label::new(
                                                                    RichText::new("Remove").color(
                                                                        Color32::from_rgb(
                                                                            0xe0, 0x6c, 0x6c,
                                                                        ),
                                                                    ),
                                                                )
                                                                .sense(egui::Sense::click()),
                                                            )
                                                            .on_hover_cursor(
                                                                egui::CursorIcon::PointingHand,
                                                            )
                                                            .clicked()
                                                        {
                                                            self.confirm_remove =
                                                                Some(g.id.clone());
                                                        }
                                                    });
                                                },
                                            );
                                            // Right column: toggle + action buttons, right-aligned
                                            // and centered against the *full* card height so they
                                            // sit at the card's vertical middle.
                                            let row_h = left_col.response.rect.height();
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(right_w, row_h),
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_height(row_h);
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
                                                    if tonal_button(
                                                        ui,
                                                        "History",
                                                        accent,
                                                        count > 0,
                                                    )
                                                    .clicked()
                                                    {
                                                        self.show_history = Some(g.id.clone());
                                                        let _ =
                                                            tx.send(Cmd::Versions(g.id.clone()));
                                                    }
                                                    if tonal_button(ui, "Files", accent, true)
                                                        .clicked()
                                                    {
                                                        self.files_game = Some(g.id.clone());
                                                        self.files.clear();
                                                        self.files_for = None;
                                                        let _ =
                                                            tx.send(Cmd::ListFiles(g.id.clone()));
                                                    }
                                                    if tonal_button(ui, "Back up", accent, true)
                                                        .clicked()
                                                    {
                                                        let _ = tx.send(Cmd::Backup(g.id.clone()));
                                                    }
                                                    ui.label(RichText::new("Sync").weak());
                                                    let mut sync = g.sync_enabled;
                                                    if toggle_switch(ui, &mut sync, accent)
                                                        .changed()
                                                    {
                                                        let _ = tx.send(Cmd::ToggleSync {
                                                            id: g.id.clone(),
                                                            enabled: sync,
                                                        });
                                                    }
                                                },
                                            );
                                        },
                                    );
                                });
                            // Hover affordance: an accent border that draws itself
                            // around the card's outer edge on hover (and retracts
                            // on mouse-out). No fill — just the edge animates.
                            let card_rect = card.response.rect;
                            let anim_id = egui::Id::new(("card_hover", g.id.as_str()));
                            let hovered = ui.rect_contains_pointer(card_rect);
                            let t = ui.ctx().animate_bool_with_time(anim_id, hovered, 0.25);
                            paint_border_trace(ui.painter(), card_rect, accent, t);
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
        let glow = self.glow();
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
                                    if primary_button(ui, "Restore", accent, glow).clicked() {
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
        // Size the modal relative to the window so it grows/shrinks with it;
        // the inner ScrollArea keeps a tall settings list reachable when space
        // is tight (and the centered title bar stays visible).
        let sr = ctx.screen_rect();
        let w = (sr.width() * 0.6).clamp(380.0, 680.0);
        let h = (sr.height() * 0.82).clamp(300.0, (sr.height() - 80.0).max(300.0));
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .fixed_size([w, h])
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(modal_frame(ctx))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.heading("Appearance");
                        ui.add_space(4.0);
                        // Top-align so every swatch cell shares one baseline (a
                        // plain/centered row lets them drift down to the left).
                        ui.horizontal_top(|ui| {
                            // Auto: follow the OS light/dark setting.
                            let auto_accent = resolve_auto(ui.ctx()).accent();
                            if let ThemePick::Select = theme_choice(
                                ui,
                                "Auto",
                                None,
                                auto_accent,
                                None,
                                self.use_auto,
                                false,
                            ) {
                                self.use_auto = true;
                                self.use_custom = false;
                                self.theme_dirty = true;
                            }
                            for t in Theme::ALL {
                                let selected =
                                    !self.use_auto && !self.use_custom && self.theme == t;
                                if let ThemePick::Select = theme_choice(
                                    ui,
                                    t.name(),
                                    Some(t.bg()),
                                    t.accent(),
                                    None,
                                    selected,
                                    false,
                                ) {
                                    self.theme = t;
                                    self.use_auto = false;
                                    self.use_custom = false;
                                    self.theme_dirty = true;
                                }
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("Show all themes…").clicked() {
                                self.show_themes = true;
                            }
                            ui.label(
                                RichText::new("Custom imports & the full gallery")
                                    .weak()
                                    .small(),
                            );
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
                        ui.heading("Storage");
                        let mut comp = self.compression;
                        if ui
                            .checkbox(&mut comp, "Compress stored saves (LZMA2)")
                            .changed()
                        {
                            let _ = tx.send(Cmd::SetCompression(comp));
                        }
                        match &self.storage {
                            Some(st) => {
                                ui.label(format!(
                                    "{} objects · {} on disk{}",
                                    st.total_objects,
                                    human_size(st.total_bytes),
                                    if st.compressed { " (compressed)" } else { "" }
                                ));
                            }
                            None => {
                                ui.label(RichText::new("Calculating usage…").weak());
                            }
                        }
                        if ui.button("Verify integrity").clicked() {
                            let _ = tx.send(Cmd::Verify);
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
                            if ui.button("Disable encryption…").clicked() {
                                self.show_disable_encryption = true;
                            }
                        } else {
                            ui.label(
                        RichText::new(
                            "Encrypt all stored saves. Only possible on an empty store; you'll \
                             get a one-time recovery key.",
                        )
                        .weak(),
                    );
                            if ui.button("Enable encryption…").clicked() {
                                self.enc_pass.clear();
                                self.enc_confirm.clear();
                                self.show_encryption = true;
                            }
                        }

                        ui.separator();
                        ui.heading("LAN");
                        // Host: share this device's saves so other devices can sync to it.
                        if let Some(spec) = self.lan_hosting.clone() {
                            ui.label(
                                RichText::new("Hosting on this network. On another device, paste:")
                                    .weak(),
                            );
                            ui.horizontal(|ui| {
                                // Editable view of a per-frame clone — selectable to
                                // copy, but edits are discarded (reset each frame).
                                let mut display = spec.clone();
                                ui.add(
                                    egui::TextEdit::singleline(&mut display)
                                        .desired_width(260.0)
                                        .font(egui::TextStyle::Monospace),
                                );
                                if ui.button("Copy").clicked() {
                                    ui.ctx().copy_text(spec.clone());
                                }
                            });
                            if ui.button("Stop hosting").clicked() {
                                let _ = tx.send(Cmd::StopLanHost);
                            }
                        } else if ui.button("Host on this network").clicked() {
                            let _ = tx.send(Cmd::StartLanHost);
                        }
                        ui.add_space(4.0);
                        // Find: discover other devices already hosting.
                        ui.horizontal(|ui| {
                            if ui.button("Find LAN hosts").clicked() {
                                let _ = tx.send(Cmd::DiscoverLan);
                            }
                            if !self.lan_hosts.is_empty() {
                                ui.label(
                                    RichText::new(format!("{} found", self.lan_hosts.len())).weak(),
                                );
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
            });
        self.show_settings = open;
    }

    /// The "all themes" gallery: every built-in + the imported theme (with its
    /// delete ✕), plus the JSON importer. Opened from Settings → Show all themes.
    fn render_themes_modal(&mut self, ctx: &egui::Context) {
        if !self.show_themes {
            return;
        }
        let accent = self.accent();
        let glow = self.glow();
        let mut open = self.show_themes;
        egui::Window::new("Themes")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(modal_frame(ctx))
            .show(ctx, |ui| {
                ui.label(RichText::new("Pick a theme, or import your own.").weak());
                ui.add_space(6.0);
                // Pull the imported theme's swatch data out first so the closure
                // below can still mutate `self` when it's clicked/deleted.
                let custom_info = self.custom_theme.as_ref().map(|c| {
                    let g = if c.has_effects() {
                        Some(c.glow_color().unwrap_or_else(|| c.accent_color()))
                    } else {
                        None
                    };
                    (c.name.clone(), c.bg_color(), c.accent_color(), g)
                });
                // Lay the themes out a fixed 4 per row (built-ins first, then any
                // imported theme), rather than packing as many as fit.
                enum Entry {
                    Auto,
                    Builtin(Theme),
                    Custom {
                        name: String,
                        bg: Color32,
                        accent: Color32,
                        glow: Option<Color32>,
                    },
                }
                let mut entries: Vec<Entry> = vec![Entry::Auto];
                entries.extend(Theme::ALL.into_iter().map(Entry::Builtin));
                if let Some((name, bg, accent_c, g)) = custom_info {
                    entries.push(Entry::Custom {
                        name,
                        bg,
                        accent: accent_c,
                        glow: g,
                    });
                }
                for row in entries.chunks(4) {
                    ui.horizontal_top(|ui| {
                        for entry in row {
                            match entry {
                                Entry::Auto => {
                                    let auto_accent = resolve_auto(ui.ctx()).accent();
                                    if let ThemePick::Select = theme_choice(
                                        ui,
                                        "Auto",
                                        None,
                                        auto_accent,
                                        None,
                                        self.use_auto,
                                        false,
                                    ) {
                                        self.use_auto = true;
                                        self.use_custom = false;
                                        self.theme_dirty = true;
                                    }
                                }
                                Entry::Builtin(t) => {
                                    let selected =
                                        !self.use_auto && !self.use_custom && self.theme == *t;
                                    if let ThemePick::Select = theme_choice(
                                        ui,
                                        t.name(),
                                        Some(t.bg()),
                                        t.accent(),
                                        None,
                                        selected,
                                        false,
                                    ) {
                                        self.theme = *t;
                                        self.use_auto = false;
                                        self.use_custom = false;
                                        self.theme_dirty = true;
                                    }
                                }
                                Entry::Custom {
                                    name,
                                    bg,
                                    accent,
                                    glow,
                                } => {
                                    match theme_choice(
                                        ui,
                                        name,
                                        Some(*bg),
                                        *accent,
                                        *glow,
                                        self.use_custom,
                                        true,
                                    ) {
                                        ThemePick::Select => {
                                            self.use_custom = true;
                                            self.use_auto = false;
                                            self.theme_dirty = true;
                                        }
                                        ThemePick::Delete => {
                                            self.custom_theme = None;
                                            // Fall back to the last built-in theme.
                                            self.use_custom = false;
                                            self.theme_dirty = true;
                                            self.toasts.push(Toast {
                                                msg: format!("Removed imported theme \"{name}\"."),
                                                kind: ToastKind::Info,
                                                at: Instant::now(),
                                            });
                                        }
                                        ThemePick::None => {}
                                    }
                                }
                            }
                        }
                    });
                }

                ui.add_space(6.0);
                ui.separator();
                ui.heading("Import a theme");
                ui.horizontal(|ui| {
                    if ui.button("Paste JSON…").clicked() {
                        self.theme_import_open = !self.theme_import_open;
                    }
                    // Keep the file picker available as a secondary option.
                    if ui.button("From file…").clicked() {
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
                                    self.use_auto = false;
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
                });
                if self.theme_import_open {
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.theme_import_text)
                            .desired_rows(7)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .hint_text(
                                "Paste theme JSON here, e.g.\n{ \"name\": \"Ocean\", \"colors\": \
                                 { \"bg\": \"#0a0e1a\", \"accent\": \"#22d3ee\" },\n  \"effects\": \
                                 { \"glow\": \"#22d3ee\" } }",
                            ),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, "Import", accent, glow).clicked() {
                            match crate::theme::parse_custom(&self.theme_import_text) {
                                Some(c) => {
                                    let name = c.name.clone();
                                    self.custom_theme = Some(c);
                                    self.use_custom = true;
                                    self.use_auto = false;
                                    self.theme_dirty = true;
                                    self.theme_import_open = false;
                                    self.theme_import_text.clear();
                                    self.toasts.push(Toast {
                                        msg: format!("Imported theme \"{name}\"."),
                                        kind: ToastKind::Info,
                                        at: Instant::now(),
                                    });
                                }
                                None => self.toasts.push(Toast {
                                    msg: "Couldn't parse that theme JSON.".to_string(),
                                    kind: ToastKind::Error,
                                    at: Instant::now(),
                                }),
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.theme_import_open = false;
                        }
                    });
                }
            });
        self.show_themes = open;
    }

    /// The "Enable encryption" modal: passphrase + confirm, with validation.
    /// On enable it sends the command; the one-time recovery key then pops up
    /// via the existing recovery modal.
    fn render_encryption_modal(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        if !self.show_encryption {
            return;
        }
        let accent = self.accent();
        let mut open = self.show_encryption;
        egui::Window::new("Enable encryption")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(modal_frame(ctx))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Encrypts all saved data at rest with a passphrase (zero-knowledge). \
                         Only available on a store with no data yet.",
                    )
                    .weak(),
                );
                ui.add_space(10.0);
                ui.label("Passphrase (min 8 chars)");
                ui.add(
                    egui::TextEdit::singleline(&mut self.enc_pass)
                        .password(true)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(6.0);
                ui.label("Confirm passphrase");
                ui.add(
                    egui::TextEdit::singleline(&mut self.enc_confirm)
                        .password(true)
                        .desired_width(f32::INFINITY),
                );

                let long_enough = self.enc_pass.len() >= 8;
                let matches = self.enc_pass == self.enc_confirm;
                if !self.enc_confirm.is_empty() && !matches {
                    ui.add_space(6.0);
                    ui.colored_label(
                        Color32::from_rgb(0xe0, 0x6c, 0x6c),
                        "Passphrases don't match.",
                    );
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let valid = long_enough && matches;
                    let enable = ui.add_enabled(
                        valid,
                        egui::Button::new(RichText::new("Enable").color(Color32::WHITE))
                            .fill(accent),
                    );
                    if enable.clicked() {
                        let _ = tx.send(Cmd::EnableEncryption(self.enc_pass.clone()));
                        self.enc_pass.clear();
                        self.enc_confirm.clear();
                        self.show_encryption = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.enc_pass.clear();
                        self.enc_confirm.clear();
                        self.show_encryption = false;
                    }
                });
            });
        if !open {
            // Closed via the window ✕ — drop any typed passphrase.
            self.enc_pass.clear();
            self.enc_confirm.clear();
            self.show_encryption = false;
        }
    }

    /// Confirmation modal for turning encryption back off.
    fn render_disable_encryption_modal(&mut self, ctx: &egui::Context, tx: &Sender<Cmd>) {
        if !self.show_disable_encryption {
            return;
        }
        let mut open = self.show_disable_encryption;
        egui::Window::new("Disable encryption")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(modal_frame(ctx))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "This decrypts all stored saves back to plaintext on disk and removes \
                         the recovery key. Your saves stay intact — they just won't be encrypted \
                         at rest anymore.",
                    )
                    .weak(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Disable encryption").color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(0xc0, 0x55, 0x55)),
                        )
                        .clicked()
                    {
                        let _ = tx.send(Cmd::DisableEncryption);
                        self.show_disable_encryption = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_disable_encryption = false;
                    }
                });
            });
        if !open {
            self.show_disable_encryption = false;
        }
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

/// A panel frame with the web build's 22px horizontal padding (and `v` vertical).
fn panel_frame(ctx: &egui::Context, v: i8) -> egui::Frame {
    egui::Frame::default()
        .fill(ctx.style().visuals.panel_fill)
        .inner_margin(egui::Margin {
            left: 22,
            right: 22,
            top: v,
            bottom: v,
        })
}

/// Window frame for the Settings/Themes modals: the default window styling with
/// generous, balanced inner padding so content isn't cramped against the edges.
fn modal_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::symmetric(22, 16))
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn sample_stops(stops: &[Color32], t: f32) -> Color32 {
    match stops.len() {
        0 => Color32::TRANSPARENT,
        1 => stops[0],
        n => {
            let seg = t.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (seg.floor() as usize).min(n - 2);
            lerp_color(stops[i], stops[i + 1], seg - i as f32)
        }
    }
}

/// Paint a vertical multi-stop gradient across `rect` as fine horizontal strips.
fn paint_gradient(painter: &egui::Painter, rect: egui::Rect, stops: &[Color32]) {
    let strips = 48;
    for i in 0..strips {
        let t0 = i as f32 / strips as f32;
        let t1 = (i + 1) as f32 / strips as f32;
        let c = sample_stops(stops, (t0 + t1) * 0.5);
        let r = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + rect.height() * t0),
            egui::pos2(rect.right(), rect.top() + rect.height() * t1),
        );
        painter.rect_filled(r, egui::CornerRadius::same(0), c);
    }
}

/// Paint soft bubbles floating up `rect`, animated by `time` (seconds).
fn paint_bubbles(painter: &egui::Painter, rect: egui::Rect, color: Color32, time: f64) {
    let span = rect.height() + 80.0;
    let col = color.gamma_multiply(0.10);
    for i in 0..16u32 {
        let s = i as f32;
        let x = rect.left() + rect.width() * fract(s * 0.618_03 + 0.13);
        let size = 10.0 + 26.0 * fract(s * 0.317 + 0.5);
        let speed = 14.0 + 26.0 * fract(s * 0.523 + 0.2);
        let phase = fract(s * 0.911) * span;
        let y = rect.bottom() + 40.0 - ((time as f32 * speed + phase) % span);
        painter.circle_filled(egui::pos2(x, y), size, col);
    }
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

/// A filled accent button with white text. When `glow` is set (a theme's
/// `effects.glow`), a soft halo is painted behind it.
fn primary_button(
    ui: &mut egui::Ui,
    text: &str,
    accent: Color32,
    glow: Option<Color32>,
) -> egui::Response {
    // Reserve halo shape slots *before* the button so they paint underneath it.
    let slots: Vec<egui::layers::ShapeIdx> = if glow.is_some() {
        (0..GLOW_LAYERS.len())
            .map(|_| ui.painter().add(egui::Shape::Noop))
            .collect()
    } else {
        Vec::new()
    };
    let resp = ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(accent));
    if let Some(g) = glow {
        fill_glow(ui.painter(), &slots, resp.rect, g);
    }
    resp
}

/// (outward growth, alpha) for each ring of the primary-button / swatch glow.
const GLOW_LAYERS: [(f32, u8); 4] = [(10.0, 16), (7.0, 26), (4.0, 40), (2.0, 60)];

/// Draw a theme preview swatch — the bg color with a small accent dot, plus an
/// Paint a theme preview swatch (bg + accent dot, optional glow) into `rect`.
/// `auto` draws a split dark/light swatch to signal the "follow OS" theme.
fn paint_swatch(
    painter: &egui::Painter,
    rect: egui::Rect,
    bg: Color32,
    accent: Color32,
    glow: Option<Color32>,
    auto: bool,
) {
    if let Some(g) = glow {
        paint_glow(painter, rect, g);
    }
    let cr = egui::CornerRadius::same(7);
    if auto {
        // Left half dark (Midnight), right half light — "follows the system".
        painter.rect_filled(rect, cr, Theme::Midnight.bg());
        let right = egui::Rect::from_min_max(
            egui::pos2(rect.center().x, rect.top() + 1.0),
            egui::pos2(rect.right() - 1.0, rect.bottom() - 1.0),
        );
        painter.rect_filled(
            right,
            egui::CornerRadius {
                nw: 0,
                ne: 6,
                sw: 0,
                se: 6,
            },
            Theme::Light.bg(),
        );
    } else {
        painter.rect_filled(rect, cr, bg);
    }
    painter.rect_stroke(
        rect,
        cr,
        egui::Stroke::new(1.0, Color32::from_gray(127).gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(
        egui::pos2(rect.right() - 9.0, rect.bottom() - 9.0),
        5.0,
        accent,
    );
}

/// Result of clicking a theme cell.
enum ThemePick {
    None,
    Select,
    Delete,
}

/// A clickable theme cell: a preview swatch + the theme name, with an accent
/// ring when selected and a ✕ to delete imported themes. Drawn at a *fixed*
/// size so a row of cells is perfectly level (no layout drift between cells).
fn theme_choice(
    ui: &mut egui::Ui,
    name: &str,
    // `None` = the "Auto" cell (a split dark/light swatch).
    bg: Option<Color32>,
    accent: Color32,
    glow: Option<Color32>,
    selected: bool,
    deletable: bool,
) -> ThemePick {
    let auto = bg.is_none();
    let bg = bg.unwrap_or(Color32::TRANSPARENT);
    const CELL_H: f32 = 40.0;
    const SWATCH_W: f32 = 44.0;
    const SWATCH_H: f32 = 28.0;
    const PAD: f32 = 10.0;
    const GAP: f32 = 8.0;

    let text_color = ui.visuals().text_color();
    let border_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .ctx()
        .fonts(|f| f.layout_no_wrap(name.to_owned(), font, text_color));

    let cell_w = PAD + SWATCH_W + GAP + galley.size().x + PAD;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(cell_w, CELL_H), egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let cr = egui::CornerRadius::same(10);
    let (border, border_w) = if selected {
        (accent, 2.0)
    } else {
        (border_color, 1.0)
    };
    ui.painter().rect_stroke(
        rect,
        cr,
        egui::Stroke::new(border_w, border),
        egui::StrokeKind::Inside,
    );

    // Swatch + label, both vertically centered in the fixed-height cell.
    let sw = egui::Rect::from_min_size(
        egui::pos2(rect.left() + PAD, rect.center().y - SWATCH_H / 2.0),
        egui::vec2(SWATCH_W, SWATCH_H),
    );
    paint_swatch(ui.painter(), sw, bg, accent, glow, auto);
    let text_pos = egui::pos2(sw.right() + GAP, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, text_color);

    // A small ✕ in the top-right corner removes an imported theme.
    let mut deleted = false;
    if deletable {
        let center = egui::pos2(rect.right() - 9.0, rect.top() + 9.0);
        let hit = egui::Rect::from_center_size(center, egui::vec2(16.0, 16.0));
        let del = ui
            .interact(hit, resp.id.with("del"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Remove this imported theme");
        let col = if del.hovered() {
            Color32::from_rgb(0xe0, 0x6c, 0x6c)
        } else {
            Color32::from_gray(160)
        };
        let p = ui.painter();
        p.circle_filled(center, 7.0, Color32::from_black_alpha(140));
        let s = 3.0;
        let stroke = egui::Stroke::new(1.5, col);
        p.line_segment(
            [
                egui::pos2(center.x - s, center.y - s),
                egui::pos2(center.x + s, center.y + s),
            ],
            stroke,
        );
        p.line_segment(
            [
                egui::pos2(center.x - s, center.y + s),
                egui::pos2(center.x + s, center.y - s),
            ],
            stroke,
        );
        deleted = del.clicked();
    }

    if deleted {
        ThemePick::Delete
    } else if resp.clicked() {
        ThemePick::Select
    } else {
        ThemePick::None
    }
}

/// Draw an accent border around `rect`, revealed left→right by a vertical sweep
/// at `x = left + width * t`: every part of the rounded outline to the left of
/// the sweep is drawn, so on hover the border fills in from the left edge across
/// to the right (and retracts as `t` falls back to 0).
fn paint_border_trace(painter: &egui::Painter, rect: egui::Rect, color: Color32, t: f32) {
    if t <= 0.0 {
        return;
    }
    const R: f32 = 15.0;
    fn arc(pts: &mut Vec<egui::Pos2>, cx: f32, cy: f32, a0: f32, a1: f32) {
        for i in 0..=5 {
            let a = (a0 + (a1 - a0) * (i as f32 / 5.0)).to_radians();
            pts.push(egui::pos2(cx + R * a.cos(), cy + R * a.sin()));
        }
    }
    let (l, r, top, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let mut pts = vec![egui::pos2(l + R, top), egui::pos2(r - R, top)];
    arc(&mut pts, r - R, top + R, -90.0, 0.0); // top-right
    pts.push(egui::pos2(r, b - R));
    arc(&mut pts, r - R, b - R, 0.0, 90.0); // bottom-right
    pts.push(egui::pos2(l + R, b));
    arc(&mut pts, l + R, b - R, 90.0, 180.0); // bottom-left
    pts.push(egui::pos2(l, top + R));
    arc(&mut pts, l + R, top + R, 180.0, 270.0); // top-left
    pts.push(egui::pos2(l + R, top)); // close

    let xr = rect.left() + rect.width() * t.clamp(0.0, 1.0);
    let stroke = egui::Stroke::new(2.0, color);
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        match (a.x <= xr, b.x <= xr) {
            (true, true) => {
                painter.line_segment([a, b], stroke);
            }
            (false, false) => {}
            // Straddles the sweep line: draw only the part left of it.
            _ => {
                let f = (xr - a.x) / (b.x - a.x);
                let mid = a + (b - a) * f;
                let inside = if a.x <= xr { a } else { b };
                painter.line_segment([inside, mid], stroke);
            }
        }
    }
}

/// Paint a soft accent halo behind `rect` (mimics the web build's
/// `box-shadow: 0 0 14px` glow), drawing largest/faintest ring first.
fn paint_glow(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    for (grow, alpha) in GLOW_LAYERS {
        let c = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        let r = egui::CornerRadius::same((8.0 + grow) as u8);
        painter.rect_filled(rect.expand(grow), r, c);
    }
}

/// Like [`paint_glow`] but into pre-reserved shape slots (so the halo lands
/// behind a widget that was added after the slots were reserved).
fn fill_glow(
    painter: &egui::Painter,
    slots: &[egui::layers::ShapeIdx],
    rect: egui::Rect,
    color: Color32,
) {
    for (idx, (grow, alpha)) in slots.iter().zip(GLOW_LAYERS) {
        let c = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        let r = egui::CornerRadius::same((8.0 + grow) as u8);
        painter.set(*idx, egui::Shape::rect_filled(rect.expand(grow), r, c));
    }
}

/// A small green "✓ set" remote-status indicator. The checkmark is hand-painted
/// (egui's default font has no check glyph) and the whole thing is sized to the
/// 30px row height so it vertically centers with the buttons beside it.
/// A small rounded pill badge: `text` in `color` on a faint tint of it (the
/// same style as the platform badges). Vertically centered in a 32px row.
fn pill_badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let font = egui::FontId::new(12.0, egui::FontFamily::Proportional);
    let galley = ui
        .ctx()
        .fonts(|f| f.layout_no_wrap(text.to_owned(), font, color));
    let (pad_x, pad_y) = (9.0, 3.0);
    let pill = egui::vec2(galley.size().x + pad_x * 2.0, galley.size().y + pad_y * 2.0);
    // Reserve the row height so the pill centers with the buttons beside it.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(pill.x, 32.0), egui::Sense::hover());
    let brect = egui::Rect::from_center_size(rect.center(), pill);
    let radius = egui::CornerRadius::same((pill.y * 0.5) as u8);
    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38);
    let border = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 72);
    let painter = ui.painter();
    painter.rect_filled(brect, radius, fill);
    painter.rect_stroke(
        brect,
        radius,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        egui::pos2(
            brect.left() + pad_x,
            brect.center().y - galley.size().y * 0.5,
        ),
        galley,
        color,
    );
}

/// A tonal button: accent text, faint accent fill, thin accent border. Greyed
/// out when `enabled` is false (for not-yet-applicable actions).
/// Lower the alpha of the default (grey) button fills so the plain buttons in
/// the top/remote bars read as semi-translucent. Accent-filled buttons (Sync
/// all, Save) set their own `.fill()` and are left fully opaque.
fn translucent_button_fills(ui: &mut egui::Ui) {
    let w = &mut ui.visuals_mut().widgets;
    for st in [&mut w.inactive, &mut w.hovered, &mut w.active] {
        let c = st.weak_bg_fill;
        st.weak_bg_fill = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 150);
    }
}

fn tonal_button(ui: &mut egui::Ui, text: &str, accent: Color32, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(RichText::new(text).color(accent))
        .fill(accent.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.45)));
    ui.add_enabled(enabled, btn)
}

/// Resolve the "Auto" theme to a concrete built-in from the OS light/dark
/// setting (dark or unknown → Midnight, light → Light).
fn resolve_auto(ctx: &egui::Context) -> Theme {
    match ctx.system_theme() {
        Some(egui::Theme::Light) => Theme::Light,
        _ => Theme::Midnight,
    }
}

/// Per-platform badge accent, matching the web build's `.badge-*` colors.
fn platform_color(platform: &str) -> Color32 {
    match platform.to_lowercase().as_str() {
        "steam" => Color32::from_rgb(0x66, 0xc0, 0xf4),
        "emulator" | "gog" => Color32::from_rgb(0xc0, 0x8c, 0xff),
        "standalone" => Color32::from_rgb(0x5d, 0xcc, 0xa5),
        "epic" => Color32::from_rgb(0xcf, 0xcf, 0xcf),
        _ => Color32::from_rgb(0x9a, 0xa6, 0xb2), // manual / unknown
    }
}

/// Render the game title and its platform badge on one row, both centered on a
/// single vertical line so they read as vertically aligned (egui's default
/// per-widget centering drifts when the title and the taller badge differ in
/// height). The badge is a rounded pill tinted with the platform color.
fn title_with_badge(ui: &mut egui::Ui, name: &str, platform: &str) {
    let strong = ui.visuals().strong_text_color();
    let title = ui
        .ctx()
        .fonts(|f| f.layout_no_wrap(name.to_owned(), egui::FontId::proportional(16.0), strong));
    let (title_w, title_h) = (title.size().x, title.size().y);

    // Capitalize the platform name for the badge label.
    let mut label = String::new();
    let mut chars = platform.chars();
    if let Some(first) = chars.next() {
        label.extend(first.to_uppercase());
        label.push_str(chars.as_str());
    }
    let color = platform_color(platform);
    let bg = ui
        .ctx()
        .fonts(|f| f.layout_no_wrap(label, egui::FontId::proportional(12.0), color));
    let bg_h = bg.size().y;
    let (pad_x, pad_y) = (10.0, 4.0);
    let badge_w = bg.size().x + pad_x * 2.0;
    let badge_h = bg_h + pad_y * 2.0;

    let gap = 8.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(title_w + gap + badge_w, title_h.max(badge_h)),
        egui::Sense::hover(),
    );
    let cy = rect.center().y;

    // Title, centered on cy.
    ui.painter()
        .galley(egui::pos2(rect.left(), cy - title_h / 2.0), title, strong);

    // Badge pill + label, centered on the same cy.
    let brect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + title_w + gap, cy - badge_h / 2.0),
        egui::vec2(badge_w, badge_h),
    );
    let radius = egui::CornerRadius::same((badge_h * 0.5) as u8);
    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38);
    let border = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 72);
    ui.painter().rect_filled(brect, radius, fill);
    ui.painter().rect_stroke(
        brect,
        radius,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(egui::pos2(brect.left() + pad_x, cy - bg_h / 2.0), bg, color);
}

/// A sliding on/off switch. Returns a response whose `.changed()` reflects toggles.
fn toggle_switch(ui: &mut egui::Ui, on: &mut bool, accent: Color32) -> egui::Response {
    let size = egui::vec2(38.0, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let r = rect.height() * 0.5;
    let cr = egui::CornerRadius::same(r as u8);
    let col = if *on { accent } else { Color32::from_gray(120) };
    // Transparent (outline) track: just a colored border + knob, no solid fill.
    ui.painter().rect_stroke(
        rect,
        cr,
        egui::Stroke::new(1.5, col),
        egui::StrokeKind::Inside,
    );
    let cx = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    ui.painter()
        .circle_filled(egui::pos2(cx, rect.center().y), r * 0.62, col);
    resp
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.toasts
            .retain(|t| t.at.elapsed() < Duration::from_secs(6));

        // Auto theme: track the OS light/dark setting and re-theme when it flips.
        if self.use_auto && !self.use_custom {
            let resolved = resolve_auto(ctx);
            if resolved != self.theme {
                self.theme = resolved;
                self.theme_dirty = true;
            }
        }

        // Custom-theme background effects: gradient + animated bubbles, painted
        // behind the (transparent) panels.
        if self.use_custom {
            if let Some(c) = &self.custom_theme {
                if c.is_fancy() {
                    let rect = ctx.screen_rect();
                    let painter = ctx.layer_painter(egui::LayerId::background());
                    let grad = c.gradient_colors();
                    if grad.len() >= 2 {
                        paint_gradient(&painter, rect, &grad);
                    } else {
                        painter.rect_filled(rect, egui::CornerRadius::same(0), c.bg_color());
                    }
                    if c.bubbles {
                        let t = ctx.input(|i| i.time);
                        paint_bubbles(&painter, rect, c.bubble_color32(), t);
                        ctx.request_repaint();
                    }
                }
            }
        }

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
        self.render_themes_modal(ctx);
        self.render_encryption_modal(ctx, &tx);
        self.render_disable_encryption_modal(ctx, &tx);
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
                self.use_auto,
            );
            self.theme_dirty = false;
        }
    }
}
