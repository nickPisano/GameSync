//! Built-in + imported custom themes, and their persistence.
//!
//! egui is themed through [`egui::Visuals`]; each theme maps to a base
//! light/dark visuals plus an accent color tinting selection, links, and
//! interactive widget strokes, and an optional background fill. The selection
//! (and any imported custom theme) is persisted next to the engine's data dir
//! as `gui-prefs.json`.

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

    fn background(self) -> Option<Color32> {
        match self {
            Theme::Midnight => Some(Color32::from_rgb(0x12, 0x16, 0x1f)),
            Theme::Forest => Some(Color32::from_rgb(0x0f, 0x1a, 0x13)),
            Theme::Grape => Some(Color32::from_rgb(0x18, 0x12, 0x1f)),
            Theme::Light => None,
        }
    }

    pub fn visuals(self) -> Visuals {
        themed_visuals(
            matches!(self, Theme::Light),
            self.accent(),
            self.background(),
        )
    }
}

/// An imported theme: just colors (egui can't do the web build's CSS effects).
#[derive(Clone)]
pub struct Custom {
    pub name: String,
    pub accent: [u8; 3],
    pub bg: [u8; 3],
    pub light: bool,
}

impl Custom {
    pub fn visuals(&self) -> Visuals {
        let accent = Color32::from_rgb(self.accent[0], self.accent[1], self.accent[2]);
        let bg = Color32::from_rgb(self.bg[0], self.bg[1], self.bg[2]);
        themed_visuals(self.light, accent, Some(bg))
    }
}

fn themed_visuals(light: bool, accent: Color32, bg: Option<Color32>) -> Visuals {
    let mut v = if light {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    v.hyperlink_color = accent;
    v.selection.bg_fill = accent.gamma_multiply(0.45);
    v.selection.stroke.color = accent;
    v.widgets.hovered.bg_stroke.color = accent;
    v.widgets.active.bg_stroke.color = accent;
    if let Some(bg) = bg {
        v.panel_fill = bg;
        v.window_fill = bg;
    }
    v
}

fn hex3(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ])
}

#[derive(Deserialize)]
struct CustomJson {
    name: String,
    accent: String,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    light: bool,
}

/// Parse an imported theme JSON: `{"name","accent":"#rrggbb","background":"#rrggbb","light":false}`.
pub fn parse_custom(json: &str) -> Option<Custom> {
    let c: CustomJson = serde_json::from_str(json).ok()?;
    let accent = hex3(&c.accent)?;
    let bg = c
        .background
        .as_deref()
        .and_then(hex3)
        .unwrap_or(if c.light {
            [245, 245, 247]
        } else {
            [18, 22, 31]
        });
    Some(Custom {
        name: c.name,
        accent,
        bg,
        light: c.light,
    })
}

pub struct Loaded {
    pub theme: Theme,
    pub custom: Option<Custom>,
    pub use_custom: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Prefs {
    theme: String,
    #[serde(default)]
    custom: Option<CustomStored>,
}

#[derive(Serialize, Deserialize, Clone)]
struct CustomStored {
    name: String,
    accent: [u8; 3],
    bg: [u8; 3],
    light: bool,
}

fn prefs_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("gui-prefs.json")
}

pub fn load(data_dir: &Path) -> Loaded {
    let prefs: Option<Prefs> = std::fs::read_to_string(prefs_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    match prefs {
        Some(p) => {
            let custom = p.custom.map(|c| Custom {
                name: c.name,
                accent: c.accent,
                bg: c.bg,
                light: c.light,
            });
            let use_custom = p.theme == "custom" && custom.is_some();
            Loaded {
                theme: Theme::from_name(&p.theme),
                custom,
                use_custom,
            }
        }
        None => Loaded {
            theme: Theme::Midnight,
            custom: None,
            use_custom: false,
        },
    }
}

pub fn save(data_dir: &Path, theme: Theme, custom: &Option<Custom>, use_custom: bool) {
    let prefs = Prefs {
        theme: if use_custom {
            "custom".to_string()
        } else {
            theme.name().to_string()
        },
        custom: custom.as_ref().map(|c| CustomStored {
            name: c.name.clone(),
            accent: c.accent,
            bg: c.bg,
            light: c.light,
        }),
    };
    if let Ok(s) = serde_json::to_string_pretty(&prefs) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(prefs_path(data_dir), s);
    }
}
