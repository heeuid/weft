//! Design tokens and font setup.  Every color in the UI comes from here.
//!
//! Selectable themes with one token structure: background elevation steps,
//! text levels, one accent, semantic colors and a terminal ANSI set tuned for
//! that background.  Catppuccin flavors (https://catppuccin.com, MIT) use the
//! official colors and terminal mapping; Graphite is weft's own pair.  Colors
//! are read through functions so the active theme can change at runtime.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Vec2};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Palette {
    id: &'static str,
    name: &'static str,
    light: bool,
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

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// The Catppuccin colors weft uses, by their official names.
struct Flavor {
    base: u32,
    mantle: u32,
    surface0: u32,
    surface1: u32,
    surface2: u32,
    overlay0: u32,
    overlay1: u32,
    subtext0: u32,
    subtext1: u32,
    text: u32,
    rosewater: u32,
    pink: u32,
    red: u32,
    peach: u32,
    yellow: u32,
    green: u32,
    teal: u32,
    blue: u32,
}

const fn catppuccin(id: &'static str, name: &'static str, light: bool, f: Flavor) -> Palette {
    // ANSI per Catppuccin's terminal ports: black/white come from the surface
    // and subtext steps (swapped for Latte), colors are shared by normal/bright.
    let (black, white, bright_black, bright_white) =
        if light { (f.subtext1, f.surface2, f.subtext0, f.surface1) } else { (f.surface1, f.subtext1, f.surface2, f.subtext0) };
    Palette {
        id,
        name,
        light,
        bg_term: rgb(f.base),
        bg_base: rgb(f.mantle),
        bg_raised: rgb(f.surface0),
        bg_hover: rgb(f.surface1),
        border: rgb(f.surface1),
        text: rgb(f.text),
        text_dim: rgb(f.subtext0),
        text_faint: if light { rgb(f.overlay1) } else { rgb(f.overlay0) },
        accent: rgb(f.blue),
        success: rgb(f.green),
        // Latte's yellow is too faint for text on a light background.
        warn: if light { rgb(f.peach) } else { rgb(f.yellow) },
        error: rgb(f.red),
        term_fg: rgb(f.text),
        cursor: rgb(f.rosewater),
        ansi: [
            rgb(black),
            rgb(f.red),
            rgb(f.green),
            rgb(f.yellow),
            rgb(f.blue),
            rgb(f.pink),
            rgb(f.teal),
            rgb(white),
            rgb(bright_black),
            rgb(f.red),
            rgb(f.green),
            rgb(f.yellow),
            rgb(f.blue),
            rgb(f.pink),
            rgb(f.teal),
            rgb(bright_white),
        ],
    }
}

#[rustfmt::skip]
const THEMES: [Palette; 6] = [
    catppuccin("catppuccin-mocha", "Catppuccin Mocha", false, Flavor {
        base: 0x1e1e2e, mantle: 0x181825, surface0: 0x313244, surface1: 0x45475a, surface2: 0x585b70,
        overlay0: 0x6c7086, overlay1: 0x7f849c, subtext0: 0xa6adc8, subtext1: 0xbac2de, text: 0xcdd6f4,
        rosewater: 0xf5e0dc, pink: 0xf5c2e7, red: 0xf38ba8, peach: 0xfab387, yellow: 0xf9e2af,
        green: 0xa6e3a1, teal: 0x94e2d5, blue: 0x89b4fa,
    }),
    catppuccin("catppuccin-macchiato", "Catppuccin Macchiato", false, Flavor {
        base: 0x24273a, mantle: 0x1e2030, surface0: 0x363a4f, surface1: 0x494d64, surface2: 0x5b6078,
        overlay0: 0x6e738d, overlay1: 0x8087a2, subtext0: 0xa5adcb, subtext1: 0xb8c0e0, text: 0xcad3f5,
        rosewater: 0xf4dbd6, pink: 0xf5bde6, red: 0xed8796, peach: 0xf5a97f, yellow: 0xeed49f,
        green: 0xa6da95, teal: 0x8bd5ca, blue: 0x8aadf4,
    }),
    catppuccin("catppuccin-frappe", "Catppuccin Frappé", false, Flavor {
        base: 0x303446, mantle: 0x292c3c, surface0: 0x414559, surface1: 0x51576d, surface2: 0x626880,
        overlay0: 0x737994, overlay1: 0x838ba7, subtext0: 0xa5adce, subtext1: 0xb5bfe2, text: 0xc6d0f5,
        rosewater: 0xf2d5cf, pink: 0xf4b8e4, red: 0xe78284, peach: 0xef9f76, yellow: 0xe5c890,
        green: 0xa6d189, teal: 0x81c8be, blue: 0x8caaee,
    }),
    catppuccin("catppuccin-latte", "Catppuccin Latte", true, Flavor {
        base: 0xeff1f5, mantle: 0xe6e9ef, surface0: 0xccd0da, surface1: 0xbcc0cc, surface2: 0xacb0be,
        overlay0: 0x9ca0b0, overlay1: 0x8c8fa1, subtext0: 0x6c6f85, subtext1: 0x5c5f77, text: 0x4c4f69,
        rosewater: 0xdc8a78, pink: 0xea76cb, red: 0xd20f39, peach: 0xfe640b, yellow: 0xdf8e1d,
        green: 0x40a02b, teal: 0x179299, blue: 0x1e66f5,
    }),
    Palette {
        id: "graphite-dark", name: "Graphite Dark", light: false,
        bg_term: rgb(0x16181d), bg_base: rgb(0x1b1e24), bg_raised: rgb(0x23272f), bg_hover: rgb(0x2c313b), border: rgb(0x2f343e),
        text: rgb(0xd7dae0), text_dim: rgb(0x9aa1ad), text_faint: rgb(0x646b78),
        accent: rgb(0x7aa2f7), success: rgb(0x73c991), warn: rgb(0xe5c07b), error: rgb(0xf07178),
        term_fg: rgb(0xc8cdd6), cursor: rgb(0xd7dae0),
        ansi: [
            rgb(0x2a2e36), rgb(0xe06c75), rgb(0x98c379), rgb(0xe5c07b), rgb(0x61afef), rgb(0xc678dd), rgb(0x56b6c2), rgb(0xabb2bf),
            rgb(0x5c6370), rgb(0xf0878e), rgb(0xb0d791), rgb(0xf0d092), rgb(0x82c3f8), rgb(0xd694e8), rgb(0x78ced8), rgb(0xe6e9ef),
        ],
    },
    Palette {
        id: "graphite-light", name: "Graphite Light", light: true,
        bg_term: rgb(0xfbfbfc), bg_base: rgb(0xf3f4f6), bg_raised: rgb(0xffffff), bg_hover: rgb(0xe7e9ee), border: rgb(0xd5d9e0),
        text: rgb(0x1f2328), text_dim: rgb(0x555e6b), text_faint: rgb(0x8b939f),
        accent: rgb(0x2f6feb), success: rgb(0x1a7f37), warn: rgb(0x9a6700), error: rgb(0xcf222e),
        term_fg: rgb(0x24292f), cursor: rgb(0x24292f),
        ansi: [
            rgb(0x24292f), rgb(0xcf222e), rgb(0x116329), rgb(0x8a5a00), rgb(0x0969da), rgb(0x8250df), rgb(0x1b7c83), rgb(0x6e7781),
            rgb(0x57606a), rgb(0xa40e26), rgb(0x1a7f37), rgb(0x633c01), rgb(0x218bff), rgb(0xa475f9), rgb(0x3192aa), rgb(0x8c959f),
        ],
    },
];

pub const DEFAULT_DARK: &str = "catppuccin-mocha";
pub const DEFAULT_LIGHT: &str = "catppuccin-latte";

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn p() -> &'static Palette {
    &THEMES[ACTIVE.load(Ordering::Relaxed).min(THEMES.len() - 1)]
}

pub fn active() -> usize {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn is_light() -> bool {
    p().light
}

/// (index, id, display name, is light) of every selectable theme.
pub fn themes() -> impl Iterator<Item = (usize, &'static str, &'static str, bool)> {
    THEMES.iter().enumerate().map(|(i, t)| (i, t.id, t.name, t.light))
}

/// Background, accent and text color of a theme, for pickers.
pub fn swatch(i: usize) -> [Color32; 3] {
    let t = &THEMES[i.min(THEMES.len() - 1)];
    [t.bg_term, t.accent, t.text]
}

pub fn index_of(id: &str) -> Option<usize> {
    THEMES.iter().position(|t| t.id == id)
}

/// The theme to show for a settings choice: a theme id, or "system" (the OS
/// light/dark setting picks `light` / `dark`).  "dark"/"light" from older
/// settings files map to the chosen dark/light theme.
pub fn resolve(choice: &str, dark: &str, light: &str, ctx: &egui::Context) -> usize {
    let pick = |id: &str, fallback: &str| index_of(id).or_else(|| index_of(fallback)).unwrap_or(0);
    match choice {
        "system" => {
            if ctx.system_theme() == Some(egui::Theme::Light) {
                pick(light, DEFAULT_LIGHT)
            } else {
                pick(dark, DEFAULT_DARK)
            }
        }
        "light" => pick(light, DEFAULT_LIGHT),
        "dark" => pick(dark, DEFAULT_DARK),
        id => index_of(id).unwrap_or_else(|| pick(dark, DEFAULT_DARK)),
    }
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

/// Fonts built into weft: Nerd Font builds of well-known coding fonts, so
/// icons (nvim plugins, prompts, file trees) show on any machine. Their
/// licenses are next to them in fonts/.
pub const BUILTIN_FONTS: &[(&str, &str, &[u8])] = &[
    ("builtin:jetbrains-mono", "JetBrains Mono Nerd Font", include_bytes!("../fonts/JetBrainsMonoNerdFontMono-Regular.ttf")),
    ("builtin:caskaydia-cove", "CaskaydiaCove Nerd Font (Cascadia)", include_bytes!("../fonts/CaskaydiaCoveNerdFontMono-Regular.ttf")),
    ("builtin:hack", "Hack Nerd Font", include_bytes!("../fonts/HackNerdFontMono-Regular.ttf")),
];

/// Default terminal font and icon fallback (settings "term_font", "icon_font").
pub const DEFAULT_FONT: &str = "builtin:jetbrains-mono";

/// A font setting's display name: a built-in font's name, else the file name.
pub fn font_label(spec: &str) -> String {
    if let Some(b) = BUILTIN_FONTS.iter().find(|b| b.0 == spec) {
        return b.1.to_string();
    }
    std::path::Path::new(spec).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| spec.to_string())
}

/// A font setting's data: "builtin:…" or a .ttf/.otf/.ttc path.
fn font_data(spec: &str) -> Result<FontData, String> {
    if let Some(b) = BUILTIN_FONTS.iter().find(|b| b.0 == spec) {
        return Ok(FontData::from_static(b.2));
    }
    let bytes = std::fs::read(spec).map_err(|e| format!("{spec}: {e}"))?;
    // egui panics later on data that isn't a font; check it now.
    if bytes.len() < 4 || !matches!(&bytes[..4], [0, 1, 0, 0] | b"OTTO" | b"ttcf" | b"true") {
        return Err(format!("{}: not a TrueType/OpenType font", font_label(spec)));
    }
    Ok(FontData::from_owned(bytes))
}

/// Font files a user can pick: data/fonts (fonts you downloaded) first, then
/// the system's. Bold, italic and other weights are left out; a path can
/// still be entered for those.
pub fn font_files(own: &std::path::Path) -> Vec<(String, String, bool)> {
    let mut dirs: Vec<(std::path::PathBuf, bool)> = vec![(own.to_path_buf(), true)];
    if let Ok(w) = std::env::var("WINDIR") {
        dirs.push((format!("{w}\\Fonts").into(), false));
    }
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        dirs.push((format!("{l}\\Microsoft\\Windows\\Fonts").into(), false));
    }
    if let Ok(h) = std::env::var("HOME") {
        dirs.push((format!("{h}/.local/share/fonts").into(), false));
        dirs.push((format!("{h}/Library/Fonts").into(), false));
    }
    dirs.push(("/usr/share/fonts".into(), false));
    dirs.push(("/usr/local/share/fonts".into(), false));
    dirs.push(("/System/Library/Fonts".into(), false));
    let mut out: Vec<(String, String, bool)> = Vec::new();
    // Linux keeps fonts a folder or two down.
    let mut stack: Vec<(std::path::PathBuf, bool, u8)> = dirs.into_iter().rev().map(|(d, own)| (d, own, 0)).collect();
    while let Some((d, own, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut here: Vec<(String, String, bool)> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 2 {
                    stack.push((p, own, depth + 1));
                }
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if is_regular_face(&name) {
                here.push((font_label(&name), p.display().to_string().replace('\\', "/"), own));
            }
        }
        here.sort_by_key(|f| f.0.to_lowercase());
        out.extend(here);
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|f| seen.insert(f.0.to_lowercase()));
    out
}

/// A .ttf/.otf/.ttc file that isn't a bold, italic or other non-regular face.
fn is_regular_face(file: &str) -> bool {
    let n = file.to_lowercase();
    if !(n.ends_with(".ttf") || n.ends_with(".otf") || n.ends_with(".ttc")) {
        return false;
    }
    let stem = n.rsplit_once('.').map(|s| s.0).unwrap_or(&n);
    !["bold", "italic", "light", "thin", "medium", "semibold", "black", "heavy", "oblique", "condensed"].iter().any(|w| stem.contains(w))
        // Windows' short names for those faces: arialbd, consolai, seguisb, …
        && !(stem.ends_with("bd") || stem.ends_with("bi") || stem.ends_with("z") && stem.len() > 4 || stem.ends_with("sb") || stem.ends_with("sl"))
}

/// Font chain with explicit fallbacks so no glyph renders as tofu:
/// mono: terminal font -> Malgun Gothic (Hangul) -> icon font (Nerd Font) -> Segoe UI Symbol -> egui defaults (emoji)
/// ui:   Segoe UI -> Malgun Gothic -> icon font -> Segoe UI Symbol -> egui defaults
///
/// `term` and `icons` are font settings ("builtin:…" or a file path). One that
/// can't be read falls back to the default; the problem is returned.
pub fn install_fonts(ctx: &egui::Context, term: &str, icons: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut fonts = FontDefinitions::default();
    let mut pick = |spec: &str| match font_data(spec) {
        Ok(d) => d,
        Err(e) => {
            problems.push(e);
            font_data(DEFAULT_FONT).unwrap()
        }
    };
    let (term_data, icon_data) = (pick(term), pick(icons));
    fonts.font_data.insert("mono".into(), Arc::new(term_data));
    fonts.font_data.insert("icons".into(), Arc::new(icon_data));
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
    let ui = load("ui", &["segoeui.ttf", "DejaVuSans.ttf"]);
    let kr = load("kr", &["malgun.ttf", "NanumGothic.ttf", "AppleSDGothicNeo.ttc"]);
    let sym = load("sym", &["seguisym.ttf"]);
    let ui_bold = load("ui_bold", &["segoeuib.ttf", "DejaVuSans-Bold.ttf"]);
    let kr_bold = load("kr_bold", &["malgunbd.ttf", "NanumGothicBold.ttf"]);

    let egui_mono = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let egui_prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let chain = |first: Option<String>, rest: &[String]| -> Vec<String> {
        let mut v: Vec<String> = first.into_iter().collect();
        v.extend(kr.clone());
        // Before Segoe UI Symbol, which has a few private-use glyphs of its own.
        v.push("icons".into());
        v.extend(sym.clone());
        for r in rest {
            if !v.contains(r) {
                v.push(r.clone());
            }
        }
        v
    };
    let term_chain = chain(Some("mono".into()), &egui_mono);
    fonts.families.insert(FontFamily::Monospace, term_chain.clone());
    fonts.families.insert(FontFamily::Name(TERM_FONT.into()), term_chain);
    let prop_chain = chain(ui, &egui_prop);
    // Bold family for rendered markdown: bold Latin and Hangul faces first,
    // then the regular chain so nothing renders as tofu.
    let mut bold: Vec<String> = ui_bold.into_iter().chain(kr_bold).collect();
    for f in &prop_chain {
        if !bold.contains(f) {
            bold.push(f.clone());
        }
    }
    fonts.families.insert(FontFamily::Name(crate::md::BOLD_FONT.into()), bold);
    fonts.families.insert(FontFamily::Proportional, prop_chain);
    ctx.set_fonts(fonts);
    problems
}

/// Width of a terminal cell. Taken from a laid-out run, not a single glyph:
/// egui rounds each glyph's advance to whole pixels inside a run, and ASCII
/// is drawn in runs, so a single glyph's width let text drift off the grid
/// (up to more than a cell across a line: the cursor hid the last letter).
pub fn cell_width(fonts: &egui::text::Fonts, font: &FontId) -> f32 {
    const N: usize = 64;
    fonts.layout_no_wrap("M".repeat(N), font.clone(), Color32::WHITE).size().x / N as f32
}

pub fn term_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(TERM_FONT.into()))
}

/// Switch the active theme and egui's widget visuals together.
pub fn apply(ctx: &egui::Context, index: usize) {
    ACTIVE.store(index.min(THEMES.len() - 1), Ordering::Relaxed);
    let light = is_light();
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
    v.extreme_bg_color = bg_term();
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

    #[test]
    fn tui_symbols_never_show_as_boxes() {
        // Symbols TUIs put in key hints and status lines (gitui, lazygit,
        // btop, nvim plugins). Not every font has them all (Hack has no ␣ or
        // ⏎); the fallback chain must, whichever built-in font is chosen.
        let symbols = "␣⏎↵⇧⌘⌥⎋⌦⌫⇥⇤↑↓←→⇐⇒▲▼◀▶●○◆◇■□▪✓✔✗✘⚠…│─┼╭╮╯╰█▏▕░▒▓⣿\u{f1050}\u{e0b0}\u{f418}";
        for (spec, name, _) in BUILTIN_FONTS {
            let ctx = egui::Context::default();
            install_fonts(&ctx, spec, DEFAULT_FONT);
            let _ = ctx.run(Default::default(), |_| {});
            let missing: String = symbols.chars().filter(|c| !ctx.fonts(|f| f.has_glyph(&term_font(14.0), *c))).collect();
            assert!(missing.is_empty(), "{name}: no glyph for {missing}");
        }
    }

    #[test]
    fn text_runs_stay_on_the_cell_grid() {
        // ASCII runs are drawn as one string; their glyphs must land on the
        // columns the grid (cursor, backgrounds) uses, at any size and scale.
        let line: String = "nvim ~/.config/nvim/init.lua && cargo build --release 2>&1 | tee log.txt; ".repeat(2).chars().take(120).collect();
        for (spec, name, _) in BUILTIN_FONTS {
            for ppp in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
                let ctx = egui::Context::default();
                install_fonts(&ctx, spec, spec);
                ctx.set_pixels_per_point(ppp);
                let _ = ctx.run(Default::default(), |_| {});
                for size in [11.0f32, 13.0, 14.0, 15.0, 17.5, 20.0] {
                    let f = term_font(size);
                    let (cw, run) = ctx.fonts(|fo| (cell_width(fo, &f), fo.layout_no_wrap(line.clone(), f.clone(), Color32::WHITE).size().x));
                    let drift = run - cw * 120.0;
                    assert!(drift.abs() < 1.0, "{name} ×{ppp} {size}pt: 120 columns off by {drift:.2}px");
                }
            }
        }
    }

    #[test]
    fn builtin_fonts_have_nerd_icons() {
        let ctx = egui::Context::default();
        for (spec, name, _) in BUILTIN_FONTS {
            assert!(install_fonts(&ctx, spec, spec).is_empty());
            let _ = ctx.run(Default::default(), |_| {});
            // nvim which-key's space key, powerline arrow, folder, git branch.
            for c in ['\u{f1050}', '\u{e0b0}', '\u{f07c}', '\u{e725}', 'a', '0'] {
                assert!(ctx.fonts(|f| f.has_glyph(&term_font(14.0), c)), "{name} lacks U+{:X}", c as u32);
            }
        }
    }

    #[test]
    fn unreadable_fonts_fall_back_to_the_default() {
        let ctx = egui::Context::default();
        let junk = std::env::temp_dir().join("weft-not-a-font.ttf");
        std::fs::write(&junk, b"hello").unwrap();
        let problems = install_fonts(&ctx, &junk.display().to_string(), "C:/no/such/font.ttf");
        assert_eq!(problems.len(), 2, "{problems:?}");
        let _ = ctx.run(Default::default(), |_| {});
        assert!(ctx.fonts(|f| f.has_glyph(&term_font(14.0), '\u{f1050}')));
        let _ = std::fs::remove_file(junk);
    }

    #[test]
    fn only_regular_faces_are_offered() {
        for f in ["JetBrainsMonoNerdFont-Regular.ttf", "consola.ttf", "D2Coding.ttc", "Hack-NF.ttf", "malgun.ttf"] {
            assert!(is_regular_face(f), "{f}");
        }
        for f in ["HackNerdFont-Bold.ttf", "consolab.ttf.txt", "arialbd.ttf", "consolaz.ttf", "x-SemiLight.otf", "readme.txt"] {
            assert!(!is_regular_face(f), "{f}");
        }
        assert_eq!(font_label("builtin:hack"), "Hack Nerd Font");
        assert_eq!(font_label("C:/Fonts/My Font.ttf"), "My Font");
    }

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
    fn themes_keep_body_text_readable() {
        for t in THEMES.iter() {
            assert!(contrast(t.text, t.bg_term) >= 6.5, "{} text {}", t.id, contrast(t.text, t.bg_term));
            assert!(contrast(t.term_fg, t.bg_term) >= 6.5, "{} term fg", t.id);
            assert!(contrast(t.text_dim, t.bg_base) >= 3.5, "{} dim text {}", t.id, contrast(t.text_dim, t.bg_base));
            // ANSI colors are each theme's own; Catppuccin Latte's yellow is
            // intentionally soft, so only rule out near-invisible ones.
            for i in (1..=6).chain(9..=14) {
                assert!(contrast(t.ansi[i], t.bg_term) >= 2.0, "{} ansi {i} {}", t.id, contrast(t.ansi[i], t.bg_term));
            }
        }
    }

    #[test]
    fn resolve_choices() {
        let ctx = egui::Context::default();
        assert_eq!(THEMES[resolve("catppuccin-frappe", DEFAULT_DARK, DEFAULT_LIGHT, &ctx)].id, "catppuccin-frappe");
        assert_eq!(THEMES[resolve("light", DEFAULT_DARK, "graphite-light", &ctx)].id, "graphite-light");
        assert_eq!(THEMES[resolve("dark", "catppuccin-macchiato", DEFAULT_LIGHT, &ctx)].id, "catppuccin-macchiato");
        assert_eq!(THEMES[resolve("no-such-theme", DEFAULT_DARK, DEFAULT_LIGHT, &ctx)].id, DEFAULT_DARK);
        assert!(index_of(DEFAULT_DARK).is_some() && index_of(DEFAULT_LIGHT).is_some());
    }
}
