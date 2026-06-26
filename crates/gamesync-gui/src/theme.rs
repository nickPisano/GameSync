//! Built-in + imported custom themes, and their persistence.
//!
//! Palettes are the exact values from the web build's `styles.css`
//! (`[data-theme]`) / `theme.ts`. Imported custom themes use the web build's
//! JSON shape (`{ name, colors: {...}, effects: { gradient, bubbles, ... } }`);
//! the gradient + bubble effects are painted by the app (the glass/neo/skeuo
//! surface blurs are CSS-only and have no egui equivalent). The selection is
//! persisted next to the engine's data dir as `gui-prefs.json`.

use std::path::Path;

use eframe::egui::{self, Color32, Visuals};
use serde::{Deserialize, Serialize};

/// Global spacing/padding tweaks (not part of `Visuals`, so they survive theme
/// switches). Call once at startup.
pub fn apply_style(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};
    let mut style = (*ctx.style()).clone();
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

fn c32(a: [u8; 3]) -> Color32 {
    Color32::from_rgb(a[0], a[1], a[2])
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

/// An imported theme: a full palette plus optional gradient + bubble effects.
#[derive(Clone, Serialize, Deserialize)]
pub struct Custom {
    pub name: String,
    pub accent: [u8; 3],
    pub bg: [u8; 3],
    #[serde(default)]
    pub light: bool,
    #[serde(default)]
    pub panel: Option<[u8; 3]>,
    #[serde(default)]
    pub card: Option<[u8; 3]>,
    #[serde(default)]
    pub border: Option<[u8; 3]>,
    #[serde(default)]
    pub text: Option<[u8; 3]>,
    /// 2–4 gradient stops (top→bottom) painted behind the UI. Empty = none.
    #[serde(default)]
    pub gradient: Vec<[u8; 3]>,
    #[serde(default)]
    pub bubbles: bool,
    #[serde(default)]
    pub bubble_color: Option<[u8; 3]>,
}

impl Custom {
    pub fn accent_color(&self) -> Color32 {
        c32(self.accent)
    }

    pub fn bg_color(&self) -> Color32 {
        c32(self.bg)
    }

    /// Whether this theme paints a gradient and/or bubbles behind the UI.
    pub fn is_fancy(&self) -> bool {
        self.gradient.len() >= 2 || self.bubbles
    }

    pub fn gradient_colors(&self) -> Vec<Color32> {
        self.gradient.iter().map(|&c| c32(c)).collect()
    }

    pub fn bubble_color32(&self) -> Color32 {
        self.bubble_color
            .map(c32)
            .unwrap_or_else(|| c32(self.accent))
    }

    fn palette(&self) -> Palette {
        let bg = c32(self.bg);
        Palette {
            bg,
            panel: self
                .panel
                .map(c32)
                .unwrap_or_else(|| bg.gamma_multiply(1.35)),
            card: self.card.map(c32).unwrap_or_else(|| bg.gamma_multiply(1.7)),
            border: self
                .border
                .map(c32)
                .unwrap_or_else(|| bg.gamma_multiply(2.2)),
            text: self.text.map(c32).unwrap_or(if self.light {
                rgb(0x1b2027)
            } else {
                rgb(0xe6e9ee)
            }),
            accent: c32(self.accent),
            light: self.light,
        }
    }

    pub fn visuals(&self) -> Visuals {
        let mut v = themed_visuals(&self.palette());
        // Let the painted gradient/bubbles show through the panels.
        if self.is_fancy() {
            v.panel_fill = Color32::TRANSPARENT;
        }
        v
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

fn is_light(c: [u8; 3]) -> bool {
    0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32 > 150.0
}

// ---- import (web build's JSON shape) -------------------------------------

#[derive(Deserialize, Default)]
struct ImportColors {
    #[serde(default)]
    bg: Option<String>,
    #[serde(default)]
    panel: Option<String>,
    #[serde(default, rename = "panel-2")]
    panel2: Option<String>,
    #[serde(default)]
    border: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    accent: Option<String>,
}

#[derive(Deserialize, Default)]
struct ImportEffects {
    #[serde(default)]
    gradient: Option<Vec<String>>,
    #[serde(default)]
    bubbles: Option<bool>,
    #[serde(default, rename = "bubbleColor")]
    bubble_color: Option<String>,
    #[serde(default)]
    highlight: Option<String>,
}

#[derive(Deserialize)]
struct Import {
    name: String,
    #[serde(default)]
    colors: Option<ImportColors>,
    #[serde(default)]
    effects: Option<ImportEffects>,
    // Fallback simple shape.
    #[serde(default)]
    accent: Option<String>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    light: Option<bool>,
}

/// Parse an imported theme: the web build's `{name, colors, effects}` shape, or
/// the simpler `{name, accent, background, light}`.
pub fn parse_custom(json: &str) -> Option<Custom> {
    let imp: Import = serde_json::from_str(json).ok()?;
    let colors = imp.colors.as_ref();
    let accent = colors
        .and_then(|c| c.accent.as_deref())
        .or(imp.accent.as_deref())
        .and_then(hex3)?;
    let bg_opt = colors
        .and_then(|c| c.bg.as_deref())
        .or(imp.background.as_deref())
        .and_then(hex3);
    let light = imp
        .light
        .unwrap_or_else(|| bg_opt.map(is_light).unwrap_or(false));
    let bg = bg_opt.unwrap_or(if light { [246, 247, 249] } else { [15, 18, 22] });

    let mut gradient = Vec::new();
    let mut bubbles = false;
    let mut bubble_color = None;
    if let Some(fx) = &imp.effects {
        if let Some(g) = &fx.gradient {
            gradient = g.iter().filter_map(|s| hex3(s)).collect();
            if gradient.len() < 2 {
                gradient.clear();
            }
        }
        bubbles = fx.bubbles.unwrap_or(false);
        bubble_color = fx
            .bubble_color
            .as_deref()
            .and_then(hex3)
            .or(fx.highlight.as_deref().and_then(hex3));
    }

    Some(Custom {
        name: imp.name,
        accent,
        bg,
        light,
        panel: colors.and_then(|c| c.panel.as_deref()).and_then(hex3),
        card: colors.and_then(|c| c.panel2.as_deref()).and_then(hex3),
        border: colors.and_then(|c| c.border.as_deref()).and_then(hex3),
        text: colors.and_then(|c| c.text.as_deref()).and_then(hex3),
        gradient,
        bubbles,
        bubble_color,
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
    custom: Option<Custom>,
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
            let use_custom = p.theme == "custom" && p.custom.is_some();
            Loaded {
                theme: Theme::from_name(&p.theme),
                custom: p.custom,
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
        custom: custom.clone(),
    };
    if let Ok(s) = serde_json::to_string_pretty(&prefs) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(prefs_path(data_dir), s);
    }
}
