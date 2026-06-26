//! Built-in themes and their persistence.
//!
//! egui is themed through [`egui::Visuals`]; each theme maps to a base
//! light/dark visuals plus an accent color tinting selection, links, and
//! interactive widget strokes. The chosen theme is persisted next to the
//! engine's data dir as `gui-prefs.json`.

use std::path::Path;

use eframe::egui::{Color32, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Midnight,
    Light,
    Forest,
    Grape,
}

impl Theme {
    pub const ALL: [Theme; 4] = [Theme::Midnight, Theme::Light, Theme::Forest, Theme::Grape];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Midnight => "Midnight",
            Theme::Light => "Light",
            Theme::Forest => "Forest",
            Theme::Grape => "Grape",
        }
    }

    fn from_name(s: &str) -> Theme {
        match s {
            "Light" => Theme::Light,
            "Forest" => Theme::Forest,
            "Grape" => Theme::Grape,
            _ => Theme::Midnight,
        }
    }

    fn accent(self) -> Color32 {
        match self {
            Theme::Midnight => Color32::from_rgb(0x4d, 0x8d, 0xff),
            Theme::Light => Color32::from_rgb(0x25, 0x63, 0xeb),
            Theme::Forest => Color32::from_rgb(0x4c, 0xc0, 0x5a),
            Theme::Grape => Color32::from_rgb(0xa5, 0x6b, 0xff),
        }
    }

    pub fn visuals(self) -> Visuals {
        let accent = self.accent();
        let mut v = if matches!(self, Theme::Light) {
            Visuals::light()
        } else {
            Visuals::dark()
        };
        v.hyperlink_color = accent;
        v.selection.bg_fill = accent.gamma_multiply(0.45);
        v.selection.stroke.color = accent;
        v.widgets.hovered.bg_stroke.color = accent;
        v.widgets.active.bg_stroke.color = accent;
        match self {
            Theme::Midnight => {
                v.panel_fill = Color32::from_rgb(0x12, 0x16, 0x1f);
                v.window_fill = Color32::from_rgb(0x12, 0x16, 0x1f);
            }
            Theme::Forest => {
                v.panel_fill = Color32::from_rgb(0x0f, 0x1a, 0x13);
                v.window_fill = Color32::from_rgb(0x0f, 0x1a, 0x13);
            }
            Theme::Grape => {
                v.panel_fill = Color32::from_rgb(0x18, 0x12, 0x1f);
                v.window_fill = Color32::from_rgb(0x18, 0x12, 0x1f);
            }
            Theme::Light => {}
        }
        v
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Prefs {
    theme: String,
}

fn prefs_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("gui-prefs.json")
}

pub fn load(data_dir: &Path) -> Theme {
    std::fs::read_to_string(prefs_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str::<Prefs>(&s).ok())
        .map(|p| Theme::from_name(&p.theme))
        .unwrap_or(Theme::Midnight)
}

pub fn save(data_dir: &Path, theme: Theme) {
    let prefs = Prefs {
        theme: theme.name().to_string(),
    };
    if let Ok(s) = serde_json::to_string_pretty(&prefs) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(prefs_path(data_dir), s);
    }
}
