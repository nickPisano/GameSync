//! gamesync-gui — a native, webview-free GameSync desktop UI on egui/eframe.
//!
//! Experiment (branch `experiment/native-gui`): replace the Tauri/WebView2 shell
//! with a pure-Rust native GUI so the shipped binary embeds no browser engine.
//! It calls `gamesync-core`'s `Engine` directly — there is no IPC bridge — and
//! renders with egui's glow (OpenGL) backend.
//!
//! This is intentionally a minimal-but-real subset (scan / list / back up /
//! restore / verify / add) so we can build it on every platform and compare its
//! VirusTotal behavior report against the WebView2 build before porting the rest.

// Don't pop a console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use gamesync_core::{BackupOptions, Engine, Game, Snapshot, SnapshotKind};

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 620.0])
            .with_min_inner_size([680.0, 440.0])
            .with_title("GameSync"),
        ..Default::default()
    };
    eframe::run_native(
        "GameSync",
        native_options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(GameSyncApp::new()))
        }),
    )
}

/// Work requested by a click during rendering, applied after the frame's panels
/// are drawn. This keeps the render closures borrowing only individual fields
/// (egui's disjoint closure captures), never calling `&mut self` methods mid-draw.
enum Action {
    None,
    Scan,
    Refresh,
    Verify,
    Select(String),
    Backup,
    Restore(String),
    Add,
    ToggleSync(String, bool),
}

struct GameSyncApp {
    engine: Option<Engine>,
    data_dir: PathBuf,

    // Unlock screen (shown when the store is encrypted and not yet open).
    needs_unlock: bool,
    passphrase: String,

    games: Vec<Game>,
    selected: Option<String>,
    versions: Vec<Snapshot>,

    add_name: String,
    add_path: String,

    status: String,
    error: Option<String>,
}

impl GameSyncApp {
    fn new() -> Self {
        let data_dir = Engine::default_data_dir();
        let mut app = Self {
            engine: None,
            data_dir: data_dir.clone(),
            needs_unlock: false,
            passphrase: String::new(),
            games: Vec::new(),
            selected: None,
            versions: Vec::new(),
            add_name: String::new(),
            add_path: String::new(),
            status: String::new(),
            error: None,
        };
        if Engine::is_encrypted(&data_dir) {
            app.needs_unlock = true;
        } else {
            match Engine::open(data_dir) {
                Ok(eng) => {
                    app.engine = Some(eng);
                    app.refresh();
                }
                Err(e) => app.error = Some(format!("Couldn't open data store: {e}")),
            }
        }
        app
    }

    fn unlock(&mut self) {
        match Engine::unlock(self.data_dir.clone(), &self.passphrase) {
            Ok(eng) => {
                self.engine = Some(eng);
                self.needs_unlock = false;
                self.passphrase.clear();
                self.error = None;
                self.refresh();
            }
            Err(e) => self.error = Some(format!("Unlock failed: {e}")),
        }
    }

    fn refresh(&mut self) {
        let listed = self.engine.as_ref().map(|e| e.list_games());
        match listed {
            Some(Ok(g)) => {
                self.games = g;
                self.error = None;
            }
            Some(Err(e)) => self.error = Some(e.to_string()),
            None => {}
        }
        self.reload_versions();
    }

    fn reload_versions(&mut self) {
        self.versions.clear();
        let id = match &self.selected {
            Some(i) => i.clone(),
            None => return,
        };
        let r = self.engine.as_ref().map(|e| e.versions(&id));
        match r {
            Some(Ok(v)) => self.versions = v,
            Some(Err(e)) => self.error = Some(e.to_string()),
            None => {}
        }
    }

    fn scan(&mut self) {
        let r = self.engine.as_ref().map(|e| e.scan_all());
        match r {
            Some(Ok(found)) => {
                self.status = format!("Scan complete — {} game(s) detected.", found.len());
                self.error = None;
                self.refresh();
            }
            Some(Err(e)) => self.error = Some(e.to_string()),
            None => {}
        }
    }

    fn verify(&mut self) {
        let r = self.engine.as_ref().map(|e| e.verify());
        match r {
            Some(Ok(rep)) => {
                self.status = if rep.ok() {
                    format!("Integrity OK — {} object(s) checked.", rep.objects_checked)
                } else {
                    format!("Integrity check found {} problem(s).", rep.problems.len())
                };
                self.error = None;
            }
            Some(Err(e)) => self.error = Some(e.to_string()),
            None => {}
        }
    }

    fn backup_selected(&mut self) {
        let id = match &self.selected {
            Some(i) => i.clone(),
            None => return,
        };
        let r = self.engine.as_ref().map(|e| {
            e.backup(
                &id,
                BackupOptions {
                    force: false,
                    wait_quiescent: false,
                    kind: SnapshotKind::Manual,
                    label: None,
                },
            )
        });
        match r {
            Some(Ok(snap)) => {
                self.status = format!(
                    "Backed up {} file(s) → version {}.",
                    snap.file_count(),
                    short(&snap.version_id)
                );
                self.error = None;
                self.reload_versions();
            }
            Some(Err(e)) => self.error = Some(format!("Backup failed: {e}")),
            None => {}
        }
    }

    fn restore_version(&mut self, version_id: &str) {
        let id = match &self.selected {
            Some(i) => i.clone(),
            None => return,
        };
        let r = self
            .engine
            .as_ref()
            .map(|e| e.restore(&id, version_id, false));
        match r {
            Some(Ok(snap)) => {
                self.status = format!(
                    "Restored version {} ({} files). A pre-restore safety backup was taken first.",
                    short(&snap.version_id),
                    snap.file_count()
                );
                self.error = None;
                self.reload_versions();
            }
            Some(Err(e)) => self.error = Some(format!("Restore failed: {e}")),
            None => {}
        }
    }

    fn add_manual(&mut self) {
        if self.add_name.trim().is_empty() || self.add_path.trim().is_empty() {
            self.error = Some("Enter both a name and a save-folder path.".to_string());
            return;
        }
        let name = self.add_name.trim().to_string();
        let path = PathBuf::from(self.add_path.trim());
        let r = self.engine.as_ref().map(|e| e.add_manual_game(&name, path));
        match r {
            Some(Ok(game)) => {
                self.status = format!("Added {}.", game.name);
                self.selected = Some(game.id.clone());
                self.add_name.clear();
                self.add_path.clear();
                self.error = None;
                self.refresh();
            }
            Some(Err(e)) => self.error = Some(format!("Add failed: {e}")),
            None => {}
        }
    }

    fn toggle_sync(&mut self, id: &str, enabled: bool) {
        let r = self
            .engine
            .as_ref()
            .map(|e| e.set_sync_enabled(id, enabled));
        if let Some(Err(e)) = r {
            self.error = Some(e.to_string());
        }
        self.refresh();
    }

    fn unlock_screen(&mut self, ctx: &egui::Context) {
        let mut do_unlock = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(90.0);
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
                let entered = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                if ui.button("Unlock").clicked() || entered {
                    do_unlock = true;
                }
                if let Some(e) = &self.error {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), e.as_str());
                }
            });
        });
        if do_unlock {
            self.unlock();
        }
    }
}

impl eframe::App for GameSyncApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.needs_unlock {
            self.unlock_screen(ctx);
            return;
        }

        let mut action = Action::None;

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("GameSync");
                ui.label(egui::RichText::new("native · webview-free build").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Verify").clicked() {
                        action = Action::Verify;
                    }
                    if ui.button("Scan").clicked() {
                        action = Action::Scan;
                    }
                    if ui.button("Refresh").clicked() {
                        action = Action::Refresh;
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(2.0);
            if let Some(e) = &self.error {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("⚠ {e}"));
            } else if !self.status.is_empty() {
                ui.colored_label(egui::Color32::from_rgb(120, 200, 120), self.status.as_str());
            } else {
                ui.label(egui::RichText::new(format!("Data: {}", self.data_dir.display())).weak());
            }
            ui.add_space(2.0);
        });

        egui::SidePanel::left("games")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.strong("Games");
                    ui.label(format!("({})", self.games.len()));
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.games.is_empty() {
                        ui.label("No games tracked yet.");
                        ui.label("Click Scan, or add one on the right.");
                    }
                    for g in &self.games {
                        let is_sel = self.selected.as_deref() == Some(g.id.as_str());
                        let mark = if g.sync_enabled { " ⟳" } else { "" };
                        let text = format!("{}  [{}]{}", g.name, g.platform.as_str(), mark);
                        if ui.selectable_label(is_sel, text).clicked() {
                            action = Action::Select(g.id.clone());
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let selected_game = self
                .games
                .iter()
                .find(|g| self.selected.as_deref() == Some(g.id.as_str()));
            match selected_game {
                Some(g) => {
                    ui.heading(g.name.as_str());
                    ui.label(format!("Platform: {}", g.platform.as_str()));
                    ui.label(format!("Saves: {}", g.save_root.display()));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Back up now").clicked() {
                            action = Action::Backup;
                        }
                        let mut sync = g.sync_enabled;
                        if ui.checkbox(&mut sync, "Sync enabled").changed() {
                            action = Action::ToggleSync(g.id.clone(), sync);
                        }
                    });
                    ui.separator();
                    ui.label(format!(
                        "History — {} version(s), newest first",
                        self.versions.len()
                    ));
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
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
                                    action = Action::Restore(v.version_id.clone());
                                }
                            });
                        }
                    });
                }
                None => {
                    ui.heading("No game selected");
                    ui.label("Pick a game on the left, or add one manually:");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.add_name);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Save folder:");
                        ui.text_edit_singleline(&mut self.add_path);
                    });
                    ui.add_space(6.0);
                    if ui.button("Add game").clicked() {
                        action = Action::Add;
                    }
                }
            }
        });

        match action {
            Action::None => {}
            Action::Scan => self.scan(),
            Action::Refresh => self.refresh(),
            Action::Verify => self.verify(),
            Action::Select(id) => {
                self.selected = Some(id);
                self.reload_versions();
            }
            Action::Backup => self.backup_selected(),
            Action::Restore(v) => self.restore_version(&v),
            Action::Add => self.add_manual(),
            Action::ToggleSync(id, en) => self.toggle_sync(&id, en),
        }
    }
}

// ---- tiny formatting helpers (mirrors the CLI) --------------------------

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn humanize_ago(ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let delta = (now - ms).max(0) / 1000;
    match delta {
        0..=59 => format!("{delta}s ago"),
        60..=3599 => format!("{}m ago", delta / 60),
        3600..=86399 => format!("{}h ago", delta / 3600),
        _ => format!("{}d ago", delta / 86400),
    }
}
