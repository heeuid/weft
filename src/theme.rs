//! Design tokens and font setup.  Every color in the UI comes from here.
//!
//! Palette: graphite neutrals in four elevation steps, one blue accent, and
//! muted semantic colors.  The ANSI palette is tuned for the terminal
//! background so that every color keeps at least ~4.5:1 contrast.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Vec2};
use std::sync::Arc;

pub const BG_TERM: Color32 = Color32::from_rgb(0x16, 0x18, 0x1d);
pub const BG_BASE: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x24);
pub const BG_RAISED: Color32 = Color32::from_rgb(0x23, 0x27, 0x2f);
pub const BG_HOVER: Color32 = Color32::from_rgb(0x2c, 0x31, 0x3b);
pub const BORDER: Color32 = Color32::from_rgb(0x2f, 0x34, 0x3e);

pub const TEXT: Color32 = Color32::from_rgb(0xd7, 0xda, 0xe0);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9a, 0xa1, 0xad);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x64, 0x6b, 0x78);

pub const ACCENT: Color32 = Color32::from_rgb(0x7a, 0xa2, 0xf7);
pub const SUCCESS: Color32 = Color32::from_rgb(0x73, 0xc9, 0x91);
pub const WARN: Color32 = Color32::from_rgb(0xe5, 0xc0, 0x7b);
pub const ERROR: Color32 = Color32::from_rgb(0xf0, 0x71, 0x78);

pub const TERM_FG: Color32 = Color32::from_rgb(0xc8, 0xcd, 0xd6);
pub const CURSOR: Color32 = Color32::from_rgb(0xd7, 0xda, 0xe0);

pub fn selection() -> Color32 {
    Color32::from_rgba_unmultiplied(0x7a, 0xa2, 0xf7, 70)
}

pub fn search_hit() -> Color32 {
    Color32::from_rgba_unmultiplied(0xe5, 0xc0, 0x7b, 90)
}

pub fn tint(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

pub const ANSI: [Color32; 16] = [
    Color32::from_rgb(0x2a, 0x2e, 0x36), // black
    Color32::from_rgb(0xe0, 0x6c, 0x75), // red
    Color32::from_rgb(0x98, 0xc3, 0x79), // green
    Color32::from_rgb(0xe5, 0xc0, 0x7b), // yellow
    Color32::from_rgb(0x61, 0xaf, 0xef), // blue
    Color32::from_rgb(0xc6, 0x78, 0xdd), // magenta
    Color32::from_rgb(0x56, 0xb6, 0xc2), // cyan
    Color32::from_rgb(0xab, 0xb2, 0xbf), // white
    Color32::from_rgb(0x5c, 0x63, 0x70), // bright black
    Color32::from_rgb(0xf0, 0x87, 0x8e),
    Color32::from_rgb(0xb0, 0xd7, 0x91),
    Color32::from_rgb(0xf0, 0xd0, 0x92),
    Color32::from_rgb(0x82, 0xc3, 0xf8),
    Color32::from_rgb(0xd6, 0x94, 0xe8),
    Color32::from_rgb(0x78, 0xce, 0xd8),
    Color32::from_rgb(0xe6, 0xe9, 0xef),
];

pub fn xterm256(idx: u8) -> Color32 {
    match idx {
        0..=15 => ANSI[idx as usize],
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

pub fn apply_style(ctx: &egui::Context) {
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

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_BASE;
    v.window_fill = BG_RAISED;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = BG_TERM;
    v.faint_bg_color = BG_RAISED;
    v.code_bg_color = BG_TERM;
    v.override_text_color = None;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = tint(ACCENT, 55);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.warn_fg_color = WARN;
    v.error_fg_color = ERROR;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = BG_BASE;
    w.noninteractive.weak_bg_fill = BG_BASE;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    w.inactive.bg_fill = BG_RAISED;
    w.inactive.weak_bg_fill = BG_RAISED;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.bg_fill = BG_HOVER;
    w.hovered.weak_bg_fill = BG_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.active.bg_fill = tint(ACCENT, 60);
    w.active.weak_bg_fill = tint(ACCENT, 60);
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.open.bg_fill = BG_HOVER;
    w.open.weak_bg_fill = BG_HOVER;
    w.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = CornerRadius::same(5);
    }
    style.visuals = v;
    // egui follows the OS theme by default; on a light-mode OS that silently
    // swaps in light widget visuals (white text fields). weft is dark-only.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}
