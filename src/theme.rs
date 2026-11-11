//! Design tokens and font setup.  Every color in the UI comes from here.
//!
//! Two palettes with the same structure: graphite neutrals in four elevation
//! steps, one blue accent, muted semantic colors, and an ANSI set tuned for
//! its own terminal background (light: "white" is a mid gray so text printed
//! in white stays readable).  Colors are read through functions so the
//! active palette can change at runtime.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// What the user picked in settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    /// Follow the OS light/dark setting.
    System,
}

struct Palette {
    bg_term: Color32,
    bg_base: Color32,
    bg_raised: Color32,
    bg_hover: Color32,
    border: Color32,
    text: Color32,
    text_dim: Color32,
    text_faint: Color32,
    accent: Color32,
    success: Color32,
    warn: Color32,
    error: Color32,
    term_fg: Color32,
    cursor: Color32,
    ansi: [Color32; 16],
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const DARK: Palette = Palette {
    bg_term: rgb(0x16, 0x18, 0x1d),
    bg_base: rgb(0x1b, 0x1e, 0x24),
    bg_raised: rgb(0x23, 0x27, 0x2f),
    bg_hover: rgb(0x2c, 0x31, 0x3b),
    border: rgb(0x2f, 0x34, 0x3e),
    text: rgb(0xd7, 0xda, 0xe0),
    text_dim: rgb(0x9a, 0xa1, 0xad),
    text_faint: rgb(0x64, 0x6b, 0x78),
    accent: rgb(0x7a, 0xa2, 0xf7),
    success: rgb(0x73, 0xc9, 0x91),
    warn: rgb(0xe5, 0xc0, 0x7b),
    error: rgb(0xf0, 0x71, 0x78),
    term_fg: rgb(0xc8, 0xcd, 0xd6),
    cursor: rgb(0xd7, 0xda, 0xe0),
    ansi: [
        rgb(0x2a, 0x2e, 0x36), // black
        rgb(0xe0, 0x6c, 0x75), // red
        rgb(0x98, 0xc3, 0x79), // green
        rgb(0xe5, 0xc0, 0x7b), // yellow
        rgb(0x61, 0xaf, 0xef), // blue
        rgb(0xc6, 0x78, 0xdd), // magenta
        rgb(0x56, 0xb6, 0xc2), // cyan
        rgb(0xab, 0xb2, 0xbf), // white
        rgb(0x5c, 0x63, 0x70), // bright black
        rgb(0xf0, 0x87, 0x8e),
        rgb(0xb0, 0xd7, 0x91),
        rgb(0xf0, 0xd0, 0x92),
        rgb(0x82, 0xc3, 0xf8),
        rgb(0xd6, 0x94, 0xe8),
        rgb(0x78, 0xce, 0xd8),
        rgb(0xe6, 0xe9, 0xef),
    ],
};

const LIGHT: Palette = Palette {
    bg_term: rgb(0xfb, 0xfb, 0xfc),
    bg_base: rgb(0xf3, 0xf4, 0xf6),
    bg_raised: rgb(0xff, 0xff, 0xff),
    bg_hover: rgb(0xe7, 0xe9, 0xee),
    border: rgb(0xd5, 0xd9, 0xe0),
    text: rgb(0x1f, 0x23, 0x28),
    text_dim: rgb(0x55, 0x5e, 0x6b),
    text_faint: rgb(0x8b, 0x93, 0x9f),
    accent: rgb(0x2f, 0x6f, 0xeb),
    success: rgb(0x1a, 0x7f, 0x37),
    warn: rgb(0x9a, 0x67, 0x00),
    error: rgb(0xcf, 0x22, 0x2e),
    term_fg: rgb(0x24, 0x29, 0x2f),
    cursor: rgb(0x24, 0x29, 0x2f),
    ansi: [
        rgb(0x24, 0x29, 0x2f), // black
        rgb(0xcf, 0x22, 0x2e), // red
        rgb(0x11, 0x63, 0x29), // green
        rgb(0x8a, 0x5a, 0x00), // yellow (dark enough to read on white)
        rgb(0x09, 0x69, 0xda), // blue
        rgb(0x82, 0x50, 0xdf), // magenta
        rgb(0x1b, 0x7c, 0x83), // cyan
        rgb(0x6e, 0x77, 0x81), // white (mid gray on a light background)
        rgb(0x57, 0x60, 0x6a), // bright black
        rgb(0xa4, 0x0e, 0x26),
        rgb(0x1a, 0x7f, 0x37),
        rgb(0x63, 0x3c, 0x01),
        rgb(0x21, 0x8b, 0xff),
        rgb(0xa4, 0x75, 0xf9),
        rgb(0x31, 0x92, 0xaa),
        rgb(0x8c, 0x95, 0x9f),
    ],
};

static LIGHT_ACTIVE: AtomicBool = AtomicBool::new(false);

fn p() -> &'static Palette {
    if is_light() {
        &LIGHT
    } else {
        &DARK
    }
}

pub fn is_light() -> bool {
    LIGHT_ACTIVE.load(Ordering::Relaxed)
}

pub fn bg_term() -> Color32 {
    p().bg_term
}
pub fn bg_base() -> Color32 {
    p().bg_base
}
pub fn bg_raised() -> Color32 {
    p().bg_raised
}
pub fn bg_hover() -> Color32 {
    p().bg_hover
}
pub fn border() -> Color32 {
    p().border
}
pub fn text() -> Color32 {
    p().text
}
pub fn text_dim() -> Color32 {
    p().text_dim
}
pub fn text_faint() -> Color32 {
    p().text_faint
}
pub fn accent() -> Color32 {
    p().accent
}
pub fn success() -> Color32 {
    p().success
}
pub fn warn() -> Color32 {
    p().warn
}
pub fn error() -> Color32 {
    p().error
}
pub fn term_fg() -> Color32 {
    p().term_fg
}
pub fn cursor() -> Color32 {
    p().cursor
}
pub fn ansi(i: usize) -> Color32 {
    p().ansi[i & 15]
}

pub fn selection() -> Color32 {
    tint(accent(), if is_light() { 60 } else { 70 })
}

pub fn search_hit() -> Color32 {
    tint(warn(), if is_light() { 70 } else { 90 })
}

pub fn tint(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

pub fn xterm256(idx: u8) -> Color32 {
    match idx {
        0..=15 => ansi(idx as usize),
        16..=231 => {
            let i = idx - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (idx - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

pub const TERM_FONT: &str = "term";

/// Font chain with explicit fallbacks so no glyph renders as tofu:
/// mono: Cascadia Mono -> Malgun Gothic (Hangul) -> Segoe UI Symbol -> egui defaults (emoji)
/// ui:   Segoe UI -> Malgun Gothic -> Segoe UI Symbol -> egui defaults
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let mut load = |key: &str, files: &[&str]| -> Option<String> {
        for f in files {
            for base in [dir.as_str(), "/usr/share/fonts/truetype", "/System/Library/Fonts"] {
                if let Ok(bytes) = std::fs::read(format!("{base}/{f}")) {
                    fonts.font_data.insert(key.to_string(), Arc::new(FontData::from_owned(bytes)));
                    return Some(key.to_string());
                }
            }
        }
        None
    };
    let mono = load("mono", &["CascadiaMono.ttf", "consola.ttf", "DejaVuSansMono.ttf", "Menlo.ttc"]);
    let ui = load("ui", &["segoeui.ttf", "DejaVuSans.ttf"]);
    let kr = load("kr", &["malgun.ttf", "NanumGothic.ttf", "AppleSDGothicNeo.ttc"]);
    let sym = load("sym", &["seguisym.ttf"]);

    let egui_mono = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let egui_prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let chain = |first: Option<String>, rest: &[String]| -> Vec<String> {
        let mut v: Vec<String> = first.into_iter().collect();
        v.extend(kr.clone());
        v.extend(sym.clone());
        for r in rest {
            if !v.contains(r) {
                v.push(r.clone());
            }
        }
        v
    };
    let term_chain = chain(mono.clone(), &egui_mono);
    fonts.families.insert(FontFamily::Monospace, term_chain.clone());
    fonts.families.insert(FontFamily::Name(TERM_FONT.into()), term_chain);
    fonts.families.insert(FontFamily::Proportional, chain(ui, &egui_prop));
    ctx.set_fonts(fonts);
}

pub fn term_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(TERM_FONT.into()))
}

/// Light or dark for a settings choice (`System` asks the OS through egui).
pub fn wants_light(mode: ThemeMode, ctx: &egui::Context) -> bool {
    match mode {
        ThemeMode::Dark => false,
        ThemeMode::Light => true,
        ThemeMode::System => ctx.system_theme() == Some(egui::Theme::Light),
    }
}

/// Switch the active palette and egui's widget visuals together.
pub fn apply(ctx: &egui::Context, light: bool) {
    LIGHT_ACTIVE.store(light, Ordering::Relaxed);
    let mut style = (*ctx.style()).clone();
    use egui::TextStyle::*;
    style.text_styles.insert(Body, FontId::proportional(13.5));
    style.text_styles.insert(Button, FontId::proportional(13.0));
    style.text_styles.insert(Small, FontId::proportional(11.5));
    style.text_styles.insert(Heading, FontId::proportional(16.0));
    style.text_styles.insert(Monospace, FontId::monospace(13.0));
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    style.spacing.button_padding = Vec2::new(8.0, 3.0);
    style.spacing.interact_size.y = 22.0;

    let mut v = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = bg_base();
    v.window_fill = bg_raised();
    v.window_stroke = Stroke::new(1.0_f32, border());
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = if light { bg_raised() } else { bg_term() };
    v.faint_bg_color = bg_raised();
    v.code_bg_color = bg_term();
    v.override_text_color = None;
    v.hyperlink_color = accent();
    v.selection.bg_fill = tint(accent(), 55);
    v.selection.stroke = Stroke::new(1.0_f32, accent());
    v.warn_fg_color = warn();
    v.error_fg_color = error();

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = bg_base();
    w.noninteractive.weak_bg_fill = bg_base();
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, border());
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, text_dim());
    w.inactive.bg_fill = if light { bg_hover() } else { bg_raised() };
    w.inactive.weak_bg_fill = if light { bg_hover() } else { bg_raised() };
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, text());
    w.hovered.bg_fill = bg_hover();
    w.hovered.weak_bg_fill = bg_hover();
    w.hovered.bg_stroke = Stroke::new(1.0_f32, border());
    w.hovered.fg_stroke = Stroke::new(1.0_f32, text());
    w.active.bg_fill = tint(accent(), 60);
    w.active.weak_bg_fill = tint(accent(), 60);
    w.active.bg_stroke = Stroke::new(1.0_f32, accent());
    w.active.fg_stroke = Stroke::new(1.0_f32, text());
    w.open.bg_fill = bg_hover();
    w.open.weak_bg_fill = bg_hover();
    w.open.fg_stroke = Stroke::new(1.0_f32, text());
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = CornerRadius::same(5);
    }
    style.visuals = v;
    // Pin egui to the palette weft chose; otherwise egui follows the OS theme
    // on its own and mixes light widgets into a dark UI (or the reverse).
    let theme = if light { egui::Theme::Light } else { egui::Theme::Dark };
    ctx.set_theme(if light { egui::ThemePreference::Light } else { egui::ThemePreference::Dark });
    ctx.set_style_of(theme, style);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative-luminance contrast ratio.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let ch = |v: u8| {
                let s = v as f32 / 255.0;
                if s <= 0.03928 {
                    s / 12.92
                } else {
                    ((s + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn palettes_keep_text_readable() {
        for (name, pal) in [("dark", &DARK), ("light", &LIGHT)] {
            assert!(contrast(pal.text, pal.bg_term) >= 7.0, "{name} text");
            assert!(contrast(pal.term_fg, pal.bg_term) >= 7.0, "{name} term fg");
            assert!(contrast(pal.text_dim, pal.bg_base) >= 4.5, "{name} dim text");
            // ANSI colors 1..=6 and 9..=14 carry meaning (errors, diffs): keep them legible.
            for i in (1..=6).chain(9..=14) {
                assert!(contrast(pal.ansi[i], pal.bg_term) >= 3.0, "{name} ansi {i}");
            }
        }
    }
}
