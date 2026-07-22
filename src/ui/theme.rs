//! Find a UI theme color table and expose it. That's the whole job.
//!
//! When Herdr's config is available we mirror Herdr's own theme tables (ported
//! from `herdr/src/app/state.rs` + `src/config/theme.rs`, v0.7.4):
//!
//!   1. read `[theme]` from Herdr's `config.toml`,
//!   2. look up the named built-in palette,
//!   3. apply `[theme.custom]` overrides.
//!
//! When that config is missing (standalone mode on a machine without Herdr),
//! fall back to the built-in `terminal` palette so Corral still runs.
//!
//! Colors are [`ratatui::style::Color`]. Serialization uses ratatui's serde impl.
//!
//! # Usage
//!
//! ```no_run
//! use corral::ui::Palette;
//! let p = Palette::resolve();     // Herdr config if present, else terminal
//! let accent = p.accent;          // ratatui::style::Color, ready for any TUI
//! ```

use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Herdr's 16 semantic UI theme tokens (mirrors `state::Palette`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Palette {
    pub name: &'static str,
    pub accent: Color,
    pub panel_bg: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub surface_dim: Color,
    pub overlay0: Color,
    pub overlay1: Color,
    pub text: Color,
    pub subtext0: Color,
    pub mauve: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
    pub blue: Color,
    pub teal: Color,
    pub peach: Color,
}

impl Palette {
    /// Resolve the active theme.
    ///
    /// - Herdr config present → same resolution as Herdr (name + custom overrides)
    /// - no Herdr config (standalone / missing install) → built-in `terminal` palette
    pub fn resolve() -> Palette {
        match ThemeConfig::try_read() {
            Some(cfg) => {
                // auto_switch needs host light/dark detection; without it we default
                // to the dark name (Herdr's own default appearance).
                let name = if cfg.auto_switch {
                    cfg.dark_name
                        .clone()
                        .unwrap_or_else(|| cfg.effective_manual())
                } else {
                    cfg.effective_manual()
                };
                let mut palette = from_name(&name).unwrap_or_else(catppuccin);
                cfg.apply_overrides(&mut palette);
                palette
            }
            // Standalone / no Herdr install: ANSI-named tokens follow the host terminal.
            None => terminal(),
        }
    }

    /// Resolve a built-in theme by name, ignoring config (previews/tests).
    pub fn named(name: &str) -> Option<Palette> {
        from_name(name)
    }

    /// Token value by name (`"accent"`, `"red"`, ...).
    pub fn token(&self, name: &str) -> Option<Color> {
        Some(match name {
            "accent" => self.accent,
            "panel_bg" => self.panel_bg,
            "surface0" => self.surface0,
            "surface1" => self.surface1,
            "surface_dim" => self.surface_dim,
            "overlay0" => self.overlay0,
            "overlay1" => self.overlay1,
            "text" => self.text,
            "subtext0" => self.subtext0,
            "mauve" => self.mauve,
            "green" => self.green,
            "yellow" => self.yellow,
            "red" => self.red,
            "blue" => self.blue,
            "teal" => self.teal,
            "peach" => self.peach,
            _ => return None,
        })
    }
}

/// Token names in declaration order, for iteration.
pub const TOKEN_NAMES: [&str; 16] = [
    "accent",
    "panel_bg",
    "surface0",
    "surface1",
    "surface_dim",
    "overlay0",
    "overlay1",
    "text",
    "subtext0",
    "mauve",
    "green",
    "yellow",
    "red",
    "blue",
    "teal",
    "peach",
];

// --- config -----------------------------------------------------------------
//
// Deserialized with serde/toml, mirroring Herdr's own `config::ThemeConfig` /
// `CustomThemeColors`. Only the `[theme]` table is read; every other section in
// Herdr's config is ignored (serde skips unknown fields by default).

#[derive(Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    theme: ThemeConfig,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ThemeConfig {
    name: Option<String>,
    auto_switch: bool,
    dark_name: Option<String>,
    light_name: Option<String>,
    custom: Option<CustomColors>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CustomColors {
    accent: Option<String>,
    panel_bg: Option<String>,
    surface0: Option<String>,
    surface1: Option<String>,
    surface_dim: Option<String>,
    overlay0: Option<String>,
    overlay1: Option<String>,
    text: Option<String>,
    subtext0: Option<String>,
    mauve: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
    red: Option<String>,
    blue: Option<String>,
    teal: Option<String>,
    peach: Option<String>,
}

impl ThemeConfig {
    fn effective_manual(&self) -> String {
        self.name.clone().unwrap_or_else(|| "catppuccin".into())
    }

    /// Read Herdr's theme config when the file exists and parses.
    /// `None` means no Herdr config available (standalone fallback path).
    fn try_read() -> Option<ThemeConfig> {
        let path = herdr_config_path()?;
        let text = std::fs::read_to_string(path).ok()?;
        // File exists but parse fails → treat as empty theme section, not standalone.
        Some(
            toml::from_str::<RootConfig>(&text)
                .map(|c| c.theme)
                .unwrap_or_default(),
        )
    }

    fn apply_overrides(&self, palette: &mut Palette) {
        let Some(c) = &self.custom else { return };
        let set = |slot: &mut Color, raw: &Option<String>| {
            if let Some(s) = raw {
                *slot = parse_color(s);
            }
        };
        set(&mut palette.accent, &c.accent);
        set(&mut palette.panel_bg, &c.panel_bg);
        set(&mut palette.surface0, &c.surface0);
        set(&mut palette.surface1, &c.surface1);
        set(&mut palette.surface_dim, &c.surface_dim);
        set(&mut palette.overlay0, &c.overlay0);
        set(&mut palette.overlay1, &c.overlay1);
        set(&mut palette.text, &c.text);
        set(&mut palette.subtext0, &c.subtext0);
        set(&mut palette.mauve, &c.mauve);
        set(&mut palette.green, &c.green);
        set(&mut palette.yellow, &c.yellow);
        set(&mut palette.red, &c.red);
        set(&mut palette.blue, &c.blue);
        set(&mut palette.teal, &c.teal);
        set(&mut palette.peach, &c.peach);
    }
}

/// Port of Herdr's `config::theme::parse_color` (hex / rgb() / named / reset).
pub fn parse_color(s: &str) -> Color {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "reset" | "default" | "none" | "transparent" => return Color::Reset,
        _ => {}
    }
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return Color::Rgb(r, g, b);
            }
        } else if hex.len() == 3 {
            let c: Vec<u8> = hex
                .chars()
                .filter_map(|c| u8::from_str_radix(&c.to_string(), 16).ok())
                .collect();
            if c.len() == 3 {
                return Color::Rgb(c[0] * 17, c[1] * 17, c[2] * 17);
            }
        }
    }
    if let Some(inner) = s.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let p: Vec<&str> = inner.split(',').collect();
        if p.len() == 3 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                p[0].trim().parse::<u8>(),
                p[1].trim().parse::<u8>(),
                p[2].trim().parse::<u8>(),
            ) {
                return Color::Rgb(r, g, b);
            }
        }
    }
    match s.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        _ => Color::Cyan, // Herdr falls back to cyan
    }
}

fn herdr_config_path() -> Option<PathBuf> {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return Some(PathBuf::from(x).join("herdr/config.toml"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config/herdr/config.toml"))
}

// --- built-in palettes (ported verbatim from herdr v0.7.4) ------------------

/// Resolve a built-in theme by name, mirroring Herdr's `Palette::from_name`.
pub fn from_name(name: &str) -> Option<Palette> {
    let norm = name.to_lowercase().replace([' ', '_'], "-");
    Some(match norm.as_str() {
        "catppuccin" | "catppuccin-mocha" => catppuccin(),
        "catppuccin-latte" | "latte" | "light" => catppuccin_latte(),
        "terminal" => terminal(),
        "tokyo-night" | "tokyonight" => tokyo_night(),
        "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => tokyo_night_day(),
        "dracula" => dracula(),
        "nord" => nord(),
        "gruvbox" | "gruvbox-dark" => gruvbox(),
        "gruvbox-light" => gruvbox_light(),
        "one-dark" | "onedark" => one_dark(),
        "one-light" | "onelight" => one_light(),
        "solarized" | "solarized-dark" => solarized(),
        "solarized-light" => solarized_light(),
        "kanagawa" => kanagawa(),
        "kanagawa-lotus" | "lotus" => kanagawa_lotus(),
        "rose-pine" | "rosepine" => rose_pine(),
        "rose-pine-dawn" | "rosepine-dawn" | "dawn" => rose_pine_dawn(),
        "vesper" => vesper(),
        _ => return None,
    })
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

pub fn catppuccin() -> Palette {
    Palette {
        name: "catppuccin",
        accent: rgb(137, 180, 250),
        panel_bg: rgb(24, 24, 37),
        surface0: rgb(49, 50, 68),
        surface1: rgb(69, 71, 90),
        surface_dim: rgb(30, 30, 46),
        overlay0: rgb(108, 112, 134),
        overlay1: rgb(127, 132, 156),
        text: rgb(205, 214, 244),
        subtext0: rgb(166, 173, 200),
        mauve: rgb(203, 166, 247),
        green: rgb(166, 227, 161),
        yellow: rgb(249, 226, 175),
        red: rgb(243, 139, 168),
        blue: rgb(137, 180, 250),
        teal: rgb(148, 226, 213),
        peach: rgb(250, 179, 135),
    }
}

pub fn catppuccin_latte() -> Palette {
    Palette {
        name: "catppuccin-latte",
        accent: rgb(30, 102, 245),
        panel_bg: rgb(239, 241, 245),
        surface0: rgb(204, 208, 218),
        surface1: rgb(188, 192, 204),
        surface_dim: rgb(230, 233, 239),
        overlay0: rgb(156, 160, 176),
        overlay1: rgb(140, 143, 161),
        text: rgb(76, 79, 105),
        subtext0: rgb(108, 111, 133),
        mauve: rgb(136, 57, 239),
        green: rgb(64, 160, 43),
        yellow: rgb(223, 142, 29),
        red: rgb(210, 15, 57),
        blue: rgb(30, 102, 245),
        teal: rgb(23, 146, 153),
        peach: rgb(254, 100, 11),
    }
}

/// The `terminal` theme: tokens are ANSI names, so it follows the terminal's
/// own palette.
pub fn terminal() -> Palette {
    Palette {
        name: "terminal",
        accent: Color::Blue,
        panel_bg: Color::Reset,
        surface0: Color::Reset,
        surface1: Color::DarkGray,
        surface_dim: Color::DarkGray,
        overlay0: Color::Gray,
        overlay1: Color::White,
        text: Color::Reset,
        subtext0: Color::Gray,
        mauve: Color::Gray,
        green: Color::Green,
        yellow: Color::Yellow,
        red: Color::LightRed,
        blue: Color::Blue,
        teal: Color::Cyan,
        peach: Color::Yellow,
    }
}

pub fn tokyo_night() -> Palette {
    Palette {
        name: "tokyo-night",
        accent: rgb(122, 162, 247),
        panel_bg: rgb(26, 27, 38),
        surface0: rgb(36, 40, 59),
        surface1: rgb(65, 72, 104),
        surface_dim: rgb(26, 27, 38),
        overlay0: rgb(86, 95, 137),
        overlay1: rgb(105, 113, 150),
        text: rgb(192, 202, 245),
        subtext0: rgb(169, 177, 214),
        mauve: rgb(187, 154, 247),
        green: rgb(158, 206, 106),
        yellow: rgb(224, 175, 104),
        red: rgb(247, 118, 142),
        blue: rgb(122, 162, 247),
        teal: rgb(125, 207, 255),
        peach: rgb(255, 158, 100),
    }
}

pub fn tokyo_night_day() -> Palette {
    Palette {
        name: "tokyo-night-day",
        accent: rgb(46, 125, 233),
        panel_bg: rgb(225, 226, 231),
        surface0: rgb(196, 200, 218),
        surface1: rgb(168, 174, 203),
        surface_dim: rgb(210, 211, 218),
        overlay0: rgb(137, 144, 179),
        overlay1: rgb(104, 112, 154),
        text: rgb(55, 96, 191),
        subtext0: rgb(97, 114, 176),
        mauve: rgb(120, 71, 189),
        green: rgb(88, 117, 57),
        yellow: rgb(140, 108, 62),
        red: rgb(245, 42, 101),
        blue: rgb(46, 125, 233),
        teal: rgb(17, 140, 116),
        peach: rgb(177, 92, 0),
    }
}

pub fn dracula() -> Palette {
    Palette {
        name: "dracula",
        accent: rgb(189, 147, 249),
        panel_bg: rgb(40, 42, 54),
        surface0: rgb(68, 71, 90),
        surface1: rgb(98, 114, 164),
        surface_dim: rgb(40, 42, 54),
        overlay0: rgb(98, 114, 164),
        overlay1: rgb(130, 140, 180),
        text: rgb(248, 248, 242),
        subtext0: rgb(210, 210, 220),
        mauve: rgb(255, 121, 198),
        green: rgb(80, 250, 123),
        yellow: rgb(241, 250, 140),
        red: rgb(255, 85, 85),
        blue: rgb(139, 233, 253),
        teal: rgb(139, 233, 253),
        peach: rgb(255, 184, 108),
    }
}

pub fn nord() -> Palette {
    Palette {
        name: "nord",
        accent: rgb(136, 192, 208),
        panel_bg: rgb(46, 52, 64),
        surface0: rgb(59, 66, 82),
        surface1: rgb(67, 76, 94),
        surface_dim: rgb(46, 52, 64),
        overlay0: rgb(76, 86, 106),
        overlay1: rgb(100, 110, 130),
        text: rgb(236, 239, 244),
        subtext0: rgb(216, 222, 233),
        mauve: rgb(180, 142, 173),
        green: rgb(163, 190, 140),
        yellow: rgb(235, 203, 139),
        red: rgb(191, 97, 106),
        blue: rgb(129, 161, 193),
        teal: rgb(143, 188, 187),
        peach: rgb(208, 135, 112),
    }
}

pub fn gruvbox() -> Palette {
    Palette {
        name: "gruvbox",
        accent: rgb(215, 153, 33),
        panel_bg: rgb(40, 40, 40),
        surface0: rgb(60, 56, 54),
        surface1: rgb(80, 73, 69),
        surface_dim: rgb(40, 40, 40),
        overlay0: rgb(146, 131, 116),
        overlay1: rgb(168, 153, 132),
        text: rgb(235, 219, 178),
        subtext0: rgb(213, 196, 161),
        mauve: rgb(211, 134, 155),
        green: rgb(184, 187, 38),
        yellow: rgb(250, 189, 47),
        red: rgb(251, 73, 52),
        blue: rgb(131, 165, 152),
        teal: rgb(142, 192, 124),
        peach: rgb(254, 128, 25),
    }
}

pub fn gruvbox_light() -> Palette {
    Palette {
        name: "gruvbox-light",
        accent: rgb(7, 102, 120),
        panel_bg: rgb(251, 241, 199),
        surface0: rgb(235, 219, 178),
        surface1: rgb(213, 196, 161),
        surface_dim: rgb(242, 229, 188),
        overlay0: rgb(146, 131, 116),
        overlay1: rgb(124, 111, 100),
        text: rgb(60, 56, 54),
        subtext0: rgb(80, 73, 69),
        mauve: rgb(143, 63, 113),
        green: rgb(121, 116, 14),
        yellow: rgb(181, 118, 20),
        red: rgb(157, 0, 6),
        blue: rgb(7, 102, 120),
        teal: rgb(66, 123, 88),
        peach: rgb(175, 58, 3),
    }
}

pub fn one_dark() -> Palette {
    Palette {
        name: "one-dark",
        accent: rgb(97, 175, 239),
        panel_bg: rgb(40, 44, 52),
        surface0: rgb(44, 49, 58),
        surface1: rgb(62, 68, 81),
        surface_dim: rgb(40, 44, 52),
        overlay0: rgb(92, 99, 112),
        overlay1: rgb(115, 122, 135),
        text: rgb(171, 178, 191),
        subtext0: rgb(150, 156, 168),
        mauve: rgb(198, 120, 221),
        green: rgb(152, 195, 121),
        yellow: rgb(229, 192, 123),
        red: rgb(224, 108, 117),
        blue: rgb(97, 175, 239),
        teal: rgb(86, 182, 194),
        peach: rgb(209, 154, 102),
    }
}

pub fn one_light() -> Palette {
    Palette {
        name: "one-light",
        accent: rgb(64, 120, 242),
        panel_bg: rgb(250, 250, 250),
        surface0: rgb(240, 240, 241),
        surface1: rgb(229, 229, 230),
        surface_dim: rgb(245, 245, 246),
        overlay0: rgb(160, 161, 167),
        overlay1: rgb(104, 107, 119),
        text: rgb(56, 58, 66),
        subtext0: rgb(104, 107, 119),
        mauve: rgb(166, 38, 164),
        green: rgb(80, 161, 79),
        yellow: rgb(193, 132, 1),
        red: rgb(228, 86, 73),
        blue: rgb(64, 120, 242),
        teal: rgb(1, 132, 188),
        peach: rgb(152, 104, 1),
    }
}

pub fn solarized() -> Palette {
    Palette {
        name: "solarized",
        accent: rgb(38, 139, 210),
        panel_bg: rgb(0, 43, 54),
        surface0: rgb(7, 54, 66),
        surface1: rgb(88, 110, 117),
        surface_dim: rgb(0, 43, 54),
        overlay0: rgb(88, 110, 117),
        overlay1: rgb(101, 123, 131),
        text: rgb(147, 161, 161),
        subtext0: rgb(131, 148, 150),
        mauve: rgb(211, 54, 130),
        green: rgb(133, 153, 0),
        yellow: rgb(181, 137, 0),
        red: rgb(220, 50, 47),
        blue: rgb(38, 139, 210),
        teal: rgb(42, 161, 152),
        peach: rgb(203, 75, 22),
    }
}

pub fn solarized_light() -> Palette {
    Palette {
        name: "solarized-light",
        accent: rgb(38, 139, 210),
        panel_bg: rgb(253, 246, 227),
        surface0: rgb(238, 232, 213),
        surface1: rgb(147, 161, 161),
        surface_dim: rgb(238, 232, 213),
        overlay0: rgb(147, 161, 161),
        overlay1: rgb(88, 110, 117),
        text: rgb(101, 123, 131),
        subtext0: rgb(131, 148, 150),
        mauve: rgb(211, 54, 130),
        green: rgb(133, 153, 0),
        yellow: rgb(181, 137, 0),
        red: rgb(220, 50, 47),
        blue: rgb(38, 139, 210),
        teal: rgb(42, 161, 152),
        peach: rgb(203, 75, 22),
    }
}

pub fn kanagawa() -> Palette {
    Palette {
        name: "kanagawa",
        accent: rgb(126, 156, 216),
        panel_bg: rgb(31, 31, 40),
        surface0: rgb(42, 42, 55),
        surface1: rgb(54, 54, 70),
        surface_dim: rgb(31, 31, 40),
        overlay0: rgb(114, 113, 105),
        overlay1: rgb(135, 134, 125),
        text: rgb(220, 215, 186),
        subtext0: rgb(200, 195, 170),
        mauve: rgb(149, 127, 184),
        green: rgb(118, 148, 106),
        yellow: rgb(192, 163, 110),
        red: rgb(195, 64, 67),
        blue: rgb(126, 156, 216),
        teal: rgb(127, 180, 202),
        peach: rgb(255, 160, 102),
    }
}

pub fn kanagawa_lotus() -> Palette {
    Palette {
        name: "kanagawa-lotus",
        accent: rgb(77, 105, 155),
        panel_bg: rgb(242, 236, 188),
        surface0: rgb(220, 213, 172),
        surface1: rgb(201, 203, 209),
        surface_dim: rgb(213, 206, 163),
        overlay0: rgb(160, 156, 172),
        overlay1: rgb(138, 137, 128),
        text: rgb(84, 84, 100),
        subtext0: rgb(67, 67, 108),
        mauve: rgb(98, 76, 131),
        green: rgb(111, 137, 78),
        yellow: rgb(119, 113, 63),
        red: rgb(200, 64, 83),
        blue: rgb(77, 105, 155),
        teal: rgb(78, 140, 162),
        peach: rgb(204, 109, 0),
    }
}

pub fn rose_pine() -> Palette {
    Palette {
        name: "rose-pine",
        accent: rgb(196, 167, 231),
        panel_bg: rgb(25, 23, 36),
        surface0: rgb(31, 29, 46),
        surface1: rgb(38, 35, 58),
        surface_dim: rgb(25, 23, 36),
        overlay0: rgb(110, 106, 134),
        overlay1: rgb(144, 140, 170),
        text: rgb(224, 222, 244),
        subtext0: rgb(200, 197, 220),
        mauve: rgb(196, 167, 231),
        green: rgb(49, 116, 143),
        yellow: rgb(246, 193, 119),
        red: rgb(235, 111, 146),
        blue: rgb(49, 116, 143),
        teal: rgb(156, 207, 216),
        peach: rgb(234, 154, 151),
    }
}

pub fn rose_pine_dawn() -> Palette {
    Palette {
        name: "rose-pine-dawn",
        accent: rgb(144, 122, 169),
        panel_bg: rgb(250, 244, 237),
        surface0: rgb(242, 233, 225),
        surface1: rgb(255, 250, 243),
        surface_dim: rgb(242, 233, 225),
        overlay0: rgb(152, 147, 165),
        overlay1: rgb(121, 117, 147),
        text: rgb(70, 66, 97),
        subtext0: rgb(121, 117, 147),
        mauve: rgb(144, 122, 169),
        green: rgb(40, 105, 131),
        yellow: rgb(234, 157, 52),
        red: rgb(180, 99, 122),
        blue: rgb(40, 105, 131),
        teal: rgb(86, 148, 159),
        peach: rgb(215, 130, 126),
    }
}

pub fn vesper() -> Palette {
    Palette {
        name: "vesper",
        accent: rgb(255, 199, 153),
        panel_bg: rgb(26, 26, 26),
        surface0: rgb(35, 35, 35),
        surface1: rgb(40, 40, 40),
        surface_dim: rgb(16, 16, 16),
        overlay0: rgb(92, 92, 92),
        overlay1: rgb(126, 126, 126),
        text: rgb(255, 255, 255),
        subtext0: rgb(160, 160, 160),
        mauve: rgb(255, 209, 168),
        green: rgb(153, 255, 228),
        yellow: rgb(255, 199, 153),
        red: rgb(255, 128, 128),
        blue: rgb(176, 176, 176),
        teal: rgb(102, 221, 204),
        peach: rgb(255, 199, 153),
    }
}
