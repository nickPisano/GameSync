//! Built-in + imported custom themes, and their persistence.
//!
//! Palettes are the exact values from the web build's `styles.css`
//! (`[data-theme]`) / `theme.ts`. egui is themed through [`egui::Visuals`]; the
//! chosen theme (and any imported custom one) is persisted next to the engine's
//! data dir as `gui-prefs.json`.

use std::path::Path;

use eframe::egui::{self, Color32, Visuals};
use serde::{Deserialize, Serialize};

/// Global spacing/padding tweaks (not part of `Visuals`, so they survive theme
/// switches). Call once at startup.
pub fn apply_style(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};
    let mut style = (*ctx.style()).clone();
    // Match the web build's type scale (body 14px, meta/small 12px, buttons 13px).
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(18.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    ctx.set_style(style);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Midnight,
    Light,
    Forest,
    Grape,
}

/// One theme's colors (matches the web build's CSS variables).
struct Palette {
    bg: Color32,
    panel: Color32,
    card: Color32,
    border: Color32,
    text: Color32,
    accent: Color32,
    light: bool,
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
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

    pub fn accent(self) -> Color32 {
        self.palette().accent
    }

    fn palette(self) -> Palette {
        match self {
            Theme::Midnight => Palette {
                bg: rgb(0x0f1216),
                panel: rgb(0x171b21),
                card: rgb(0x1e242c),
                border: rgb(0x2a313b),
                text: rgb(0xe6e9ee),
                accent: rgb(0x4f8cff),
                light: false,
            },
            Theme::Light => Palette {
                bg: rgb(0xf6f7f9),
                panel: rgb(0xffffff),
                card: rgb(0xeef1f5),
                border: rgb(0xd7dce3),
                text: rgb(0x1b2027),
                accent: rgb(0x2f6ae0),
                light: true,
            },
            Theme::Forest => Palette {
                bg: rgb(0x0e1512),
                panel: rgb(0x14201a),
                card: rgb(0x1b2b22),
                border: rgb(0x26392e),
                text: rgb(0xe3efe8),
                accent: rgb(0x3fb37f),
                light: false,
            },
            Theme::Grape => Palette {
                bg: rgb(0x15121d),
                panel: rgb(0x1e1830),
                card: rgb(0x281f3d),
                border: rgb(0x342a4e),
                text: rgb(0xece8f5),
                accent: rgb(0xb57bff),
                light: false,
            },
        }
    }

    pub fn visuals(self) -> Visuals {
        themed_visuals(&self.palette())
    }
}

/// An imported theme: accent + background; panels/text are derived.
#[derive(Clone)]
pub struct Custom {
    pub name: String,
    pub accent: [u8; 3],
    pub bg: [u8; 3],
    pub light: bool,
}

impl Custom {
    pub fn accent_color(&self) -> Color32 {
        Color32::from_rgb(self.accent[0], self.accent[1], self.accent[2])
    }

    fn palette(&self) -> Palette {
        let bg = Color32::from_rgb(self.bg[0], self.bg[1], self.bg[2]);
        Palette {
            bg,
            panel: bg.gamma_multiply(1.35),
            card: bg.gamma_multiply(1.7),
            border: bg.gamma_multiply(2.2),
            text: if self.light {
                rgb(0x1b2027)
            } else {
                rgb(0xe6e9ee)
            },
            accent: self.accent_color(),
            light: self.light,
        }
    }

    pub fn visuals(&self) -> Visuals {
        themed_visuals(&self.palette())
    }
}

fn themed_visuals(p: &Palette) -> Visuals {
    let mut v = if p.light {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.faint_bg_color = p.card;
    v.extreme_bg_color = p.card;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent.gamma_multiply(0.40);
    v.selection.stroke.color = p.accent;
    v.widgets.noninteractive.bg_stroke.color = p.border;
    v.widgets.hovered.bg_stroke.color = p.accent;
    v.widgets.active.bg_stroke.color = p.accent;
    let cr = egui::CornerRadius::same(6);
    v.widgets.noninteractive.corner_radius = cr;
    v.widgets.inactive.corner_radius = cr;
    v.widgets.hovered.corner_radius = cr;
    v.widgets.active.corner_radius = cr;
    v.widgets.open.corner_radius = cr;
    v.window_corner_radius = egui::CornerRadius::same(10);
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
            [246, 247, 249]
        } else {
            [15, 18, 22]
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
