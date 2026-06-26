//! gamesync-gui — a native, webview-free GameSync desktop UI on egui/eframe.
//!
//! Replaces the Tauri/WebView2 shell with a pure-Rust native GUI: no embedded
//! browser engine. The UI talks to `gamesync-core`'s `Engine` through a
//! background [`worker`] thread (no IPC bridge) and renders with egui's glow
//! (OpenGL) backend.

// Don't pop a console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod theme;
mod util;
mod worker;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 680.0])
            .with_min_inner_size([720.0, 460.0])
            .with_title("GameSync"),
        ..Default::default()
    };
    eframe::run_native(
        "GameSync",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
