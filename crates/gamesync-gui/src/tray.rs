//! System tray icon + menu (Open / Sync all / Quit).
//!
//! tray-icon delivers menu clicks on a global channel independent of the winit
//! event loop, so a listener thread forwards them to the app and wakes egui via
//! `request_repaint` — important while the window is hidden in the tray.

use std::sync::mpsc::{channel, Receiver};

use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

pub enum TrayAction {
    Open,
    SyncAll,
    Quit,
}

/// A small 32×32 accent-blue tile with a darker border.
fn make_icon() -> Option<tray_icon::Icon> {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let border = x < 2 || y < 2 || x >= size - 2 || y >= size - 2;
            let (r, g, b) = if border {
                (0x20, 0x40, 0x80)
            } else {
                (0x4d, 0x8d, 0xff)
            };
            rgba.extend_from_slice(&[r, g, b, 0xff]);
        }
    }
    tray_icon::Icon::from_rgba(rgba, size, size).ok()
}

/// Build the tray and a receiver of menu actions. Returns `None` if the platform
/// couldn't create a tray. The returned `TrayIcon` must be kept alive.
pub fn setup(ctx: egui::Context) -> Option<(TrayIcon, Receiver<TrayAction>)> {
    let menu = Menu::new();
    let open = MenuItem::new("Open GameSync", true, None);
    let sync = MenuItem::new("Sync all", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let (open_id, sync_id, quit_id) = (open.id().clone(), sync.id().clone(), quit.id().clone());
    menu.append(&open).ok()?;
    menu.append(&sync).ok()?;
    menu.append(&quit).ok()?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("GameSync")
        .with_icon(make_icon()?)
        .build()
        .ok()?;

    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let events = MenuEvent::receiver();
        while let Ok(ev) = events.recv() {
            let action = if ev.id == open_id {
                TrayAction::Open
            } else if ev.id == sync_id {
                TrayAction::SyncAll
            } else if ev.id == quit_id {
                TrayAction::Quit
            } else {
                continue;
            };
            if tx.send(action).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    Some((tray, rx))
}
