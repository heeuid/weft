//! Terminal panes: the cell grid plus the GUI layers woven into it
//! (block gutter/badges/toolbars, inline widgets, composer, search).

use crate::app::*;
use crate::icons::{self, Icon};
use crate::term::{self, BlockState, Color, Row, Term, WIDE_LEAD, WIDE_TAIL};
use crate::theme;
use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, Event, EventFilter, FontId, Id, ImeEvent, Key, Modifiers,
    Painter, PointerButton, Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder,
};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VRow {
    Line(u64),
    /// A folded block output: (block id, hidden line count, first hidden line).
    Fold(u64, u64, u64),
    /// Alternate screen row.
    Screen(usize),
}

impl VRow {
    fn abs(&self, t: &Term) -> Option<u64> {
        match *self {
            VRow::Line(a) => Some(a),
            VRow::Screen(y) => Some(t.screen_base() + y as u64),
            VRow::Fold(..) => None,
        }
    }
}

/// Every displayable row of the primary buffer, honoring folded blocks.
pub fn visual_rows(t: &Term) -> Vec<VRow> {
    let mut folds: Vec<(u64, u64, u64)> = t
        .blocks
        .iter()
        .filter(|b| b.folded)
        .filter_map(|b| match (b.out_line, b.end_line) {
            (Some(o), Some(e)) if e > o => Some((o, e, b.id)),
            _ => None,
        })
        .collect();
    folds.sort();
    let mut v = Vec::with_capacity((t.end_line() - t.first_line()) as usize);
    let mut abs = t.first_line();
    let end = t.end_line();
    let mut fi = 0;
    while abs < end {
        while fi < folds.len() && folds[fi].1 <= abs {
            fi += 1;
        }
        if fi < folds.len() && folds[fi].0 <= abs {
            let (_, e, id) = folds[fi];
            v.push(VRow::Fold(id, e - abs, abs));
            abs = e;
            fi += 1;
            continue;
        }
        v.push(VRow::Line(abs));
        abs += 1;
    }
    v
}

fn resolve(c: Color, default: Color32) -> Color32 {
    match c {
        Color::Default => default,
        Color::Idx(i) => theme::xterm256(i),
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

/// Final (fg, bg) colors for a cell; bg is None for the default background.
fn cell_colors(a: term::Attr) -> (Color32, Option<Color32>) {
    let mut fg_c = a.fg;
    if a.flags & term::F_BOLD != 0 {
        if let Color::Idx(i) = fg_c {
            if i < 8 {
                fg_c = Color::Idx(i + 8);
            }
        }
    }
    let mut fg = resolve(fg_c, theme::term_fg());
    let mut bg = match a.bg {
        Color::Default => None,
        c => Some(resolve(c, theme::bg_term())),
    };
    if a.flags & term::F_INVERSE != 0 {
        let new_bg = fg;
        fg = bg.unwrap_or(theme::bg_term());
        bg = Some(new_bg);
    }
    if a.flags & term::F_DIM != 0 {
        fg = fg.gamma_multiply(0.6);
    }
    if a.flags & term::F_HIDDEN != 0 {
        fg = bg.unwrap_or(theme::bg_term());
    }
    (fg, bg)
}

// Where and how big a row is, eight plain numbers and references: a struct
// for them would only be filled in at the two calls.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_row(p: &Painter, row: &Row, x0: f32, y: f32, cw: f32, ch: f32, font: &FontId, cols: usize) {
    let n = row.cells.len().min(cols);
    // Backgrounds, merged into runs.
    let mut x = 0;
    while x < n {
        let (_, bg) = cell_colors(row.cells[x].attr);
        let start = x;
        while x < n && cell_colors(row.cells[x].attr).1 == bg {
            x += 1;
        }
        if let Some(bg) = bg {
            let r = Rect::from_min_size(pos2(x0 + start as f32 * cw, y), vec2((x - start) as f32 * cw, ch));
            p.rect_filled(r, CornerRadius::ZERO, bg);
        }
    }
    // Text: ASCII runs as one string (the mono font keeps them on the grid),
    // everything else per cell so fallback fonts can't push columns around.
    let mut x = 0;
    while x < n {
        let cell = row.cells[x];
        if cell.wide == WIDE_TAIL {
            x += 1;
            continue;
        }
        let (fg, _) = cell_colors(cell.attr);
        let flags = cell.attr.flags;
        if cell.ch.is_ascii() && cell.wide == 0 {
            let start = x;
            let mut s = String::new();
            while x < n {
                let c = row.cells[x];
                if !(c.ch.is_ascii() && c.wide == 0 && c.attr == cell.attr) {
                    break;
                }
                s.push(c.ch);
                x += 1;
            }
            let px = x0 + start as f32 * cw;
            if !s.trim().is_empty() {
                let bold = flags & term::F_BOLD != 0;
                let galley = p.layout_no_wrap(s.clone(), font.clone(), fg);
                if (galley.size().x - (x - start) as f32 * cw).abs() <= 0.5 {
                    if bold {
                        p.galley(pos2(px + 0.6, y), galley.clone(), fg);
                    }
                    p.galley(pos2(px, y), galley, fg);
                } else {
                    // A font that isn't fixed-width (or joins letters): the
                    // run would leave the grid, so each letter goes in its
                    // own cell, centred.
                    for (k, c) in s.chars().enumerate().filter(|(_, c)| *c != ' ') {
                        let g = p.layout_no_wrap(c.to_string(), font.clone(), fg);
                        let gx = px + k as f32 * cw + ((cw - g.size().x) / 2.0).max(0.0);
                        if bold {
                            p.galley(pos2(gx + 0.6, y), g.clone(), fg);
                        }
                        p.galley(pos2(gx, y), g, fg);
                    }
                }
            }
            decorate(p, flags, px, (x - start) as f32 * cw, y, ch, fg);
        } else if let Some((arms, round)) = box_arms(cell.ch) {
            // Box drawing is drawn too, for the same reason as the blocks
            // below: a font's lines stop short of the row above and below,
            // and frames and trees came out broken.
            let px = x0 + x as f32 * cw;
            paint_box(p, arms, round, px, y, cw, ch, fg);
            decorate(p, flags, px, cw, y, ch, fg);
            x += 1;
        } else if let Some((rects, count, alpha)) = block_rects(cell.ch) {
            // Block elements are drawn, not set in type: a font's blocks
            // rarely fill the row, and the cell's background showed through
            // (a black band across a logo built of them). Edges go to whole
            // pixels, the same ones for neighbours, so blocks join.
            let ppp = p.ctx().pixels_per_point();
            let snap = |v: f32| (v * ppp).round() / ppp;
            let px = x0 + x as f32 * cw;
            let color = if alpha < 1.0 { fg.gamma_multiply(alpha) } else { fg };
            for r in &rects[..count] {
                let r = Rect::from_min_max(pos2(snap(px + r[0] * cw), snap(y + r[1] * ch)), pos2(snap(px + r[2] * cw), snap(y + r[3] * ch)));
                p.rect_filled(r, CornerRadius::ZERO, color);
            }
            decorate(p, flags, px, cw, y, ch, fg);
            x += 1;
        } else {
            let w = if cell.wide == WIDE_LEAD { 2.0 } else { 1.0 };
            let px = x0 + x as f32 * cw;
            let galley = p.layout_no_wrap(cell.ch.to_string(), font.clone(), fg);
            let gx = px + ((w * cw - galley.size().x) / 2.0).max(0.0);
            let gy = y + ((ch - galley.size().y) / 2.0);
            p.galley(pos2(gx, gy), galley, fg);
            decorate(p, flags, px, w * cw, y, ch, fg);
            x += 1;
        }
    }
}

/// A box drawing character (U+2500–257F) as the lines leaving the middle of
/// its cell: left, up, right, down, each none (0), light (1), heavy (2) or
/// double (3); and whether its corner is rounded. None for the dashed and
/// diagonal ones, which stay the font's.
fn box_arms(c: char) -> Option<([u8; 4], bool)> {
    // Left, up, right, down for U+2500 … U+257F; "----": not drawn here.
    const ARMS: &str = "\
1010 2020 0101 0202 ---- ---- ---- ---- ---- ---- ---- ---- 0011 0021 0012 0022 \
1001 2001 1002 2002 0110 0120 0210 0220 1100 2100 1200 2200 0111 0121 0211 0112 \
0212 0221 0122 0222 1101 2101 1201 1102 1202 2201 2102 2202 1011 2011 1021 2021 \
1012 2012 1022 2022 1110 2110 1120 2120 1210 2210 1220 2220 1111 2111 1121 2121 \
1211 1112 1212 2211 1221 2112 1122 2221 2122 2212 1222 2222 ---- ---- ---- ---- \
3030 0303 0031 0013 0033 3001 1003 3003 0130 0310 0330 3100 1300 3300 0131 0313 \
0333 3101 1303 3303 3031 1013 3033 3130 1310 3330 3131 1313 3333 0011 1001 1100 \
0110 ---- ---- ---- 1000 0100 0010 0001 2000 0200 0020 0002 1020 0102 2010 0201";
    let cp = c as u32;
    if !(0x2500..=0x257F).contains(&cp) {
        return None;
    }
    let code = ARMS.split(' ').nth((cp - 0x2500) as usize)?;
    if code == "----" {
        return None;
    }
    let b = code.as_bytes();
    Some(([b[0] - b'0', b[1] - b'0', b[2] - b'0', b[3] - b'0'], (0x256D..=0x2570).contains(&cp)))
}

/// Draw a box drawing character's lines in its cell, to the cell's very
/// edges (so they join the next cell's), on whole pixels.
#[allow(clippy::too_many_arguments)]
fn paint_box(p: &Painter, arms: [u8; 4], round: bool, x: f32, y: f32, cw: f32, ch: f32, color: Color32) {
    let ppp = p.ctx().pixels_per_point();
    let px = |v: f32| (v * ppp).round() as i32;
    let (x0, x1, y0, y1) = (px(x), px(x + cw), px(y), px(y + ch));
    let (cx, cy) = ((x0 + x1) / 2, (y0 + y1) / 2);
    // A light line's thickness, a heavy one's, and how far a double line's
    // two lines are from the middle; in pixels.
    let t = ((ch * ppp) / 15.0).round().max(1.0) as i32;
    let heavy = (2 * t).max(t + 1);
    let d = t + t / 2;
    let thick = |w: u8| if w == 2 { heavy } else { t };
    // The pixels a line of thickness `th` around `c` takes: [start, end).
    let band = |c: i32, th: i32| (c - th / 2, c - th / 2 + th);
    let fill = |a: i32, b: i32, c: i32, e: i32| {
        if b > a && e > c {
            let r = Rect::from_min_max(pos2(a as f32 / ppp, c as f32 / ppp), pos2(b as f32 / ppp, e as f32 / ppp));
            p.rect_filled(r, CornerRadius::ZERO, color);
        }
    };
    let [l, u, r, dn] = arms;

    if round {
        // A quarter circle between the two arms, then straight to the edges.
        let w = t as f32 / ppp;
        let (mx, my) = ((band(cx, t).0 as f32 + t as f32 / 2.0) / ppp, (band(cy, t).0 as f32 + t as f32 / 2.0) / ppp);
        let rad = ((x1 - x0).min(y1 - y0) as f32 / 2.0 / ppp).min(cw / 2.0);
        let (sx, sy) = (if r != 0 { 1.0 } else { -1.0 }, if dn != 0 { 1.0 } else { -1.0 });
        let centre = pos2(mx + sx * rad, my + sy * rad);
        let mut pts = vec![pos2(mx, if dn != 0 { y1 as f32 / ppp } else { y0 as f32 / ppp })];
        for k in 0..=8 {
            let a = std::f32::consts::FRAC_PI_2 * k as f32 / 8.0;
            pts.push(pos2(centre.x - sx * rad * a.cos(), centre.y - sy * rad * a.sin()));
        }
        pts.push(pos2(if r != 0 { x1 as f32 / ppp } else { x0 as f32 / ppp }, my));
        p.add(egui::Shape::line(pts, Stroke::new(w, color)));
        return;
    }

    let double = |w: u8| w == 3;
    // Light and heavy arms: from the cell's edge to the middle, where they
    // cover the crossing line's width (a square corner, a clean T). Beside
    // a double line they stop at it.
    let any_h = |w: u8| w == 1 || w == 2;
    // The wider of two arms that are light or heavy (light if neither is).
    let wider = |a: u8, b: u8| if any_h(a) || any_h(b) { a.max(b).min(2) } else { 1 };
    let (hb, vb) = (band(cy, thick(wider(l, r))), band(cx, thick(wider(u, dn))));
    // Horizontal arms.
    for (w, right) in [(l, false), (r, true)] {
        if !any_h(w) {
            continue;
        }
        let (b0, b1) = band(cy, thick(w));
        let other = if right { l } else { r };
        // Where it starts near the middle.
        let near = if double(u) || double(dn) {
            if any_h(other) {
                cx // both sides: through the double line
            } else if double(u) && double(dn) {
                if right { band(cx + d, t).0 } else { band(cx - d, t).1 }
            } else if right {
                band(cx - d, t).0
            } else {
                band(cx + d, t).1
            }
        } else if right {
            vb.0
        } else {
            vb.1
        };
        if right {
            fill(near, x1, b0, b1);
        } else {
            fill(x0, near, b0, b1);
        }
    }
    // Vertical arms.
    for (w, down) in [(u, false), (dn, true)] {
        if !any_h(w) {
            continue;
        }
        let (b0, b1) = band(cx, thick(w));
        let other = if down { u } else { dn };
        let near = if double(l) || double(r) {
            if any_h(other) {
                cy
            } else if double(l) && double(r) {
                if down { band(cy + d, t).0 } else { band(cy - d, t).1 }
            } else if down {
                band(cy - d, t).0
            } else {
                band(cy + d, t).1
            }
        } else if down {
            hb.0
        } else {
            hb.1
        };
        if down {
            fill(b0, b1, near, y1);
        } else {
            fill(b0, b1, y0, near);
        }
    }
    // Double arms: two lines each. A line on the side where a double arm
    // turns off stops at that arm's nearer line (the inner corner); on the
    // other side it runs on to the farther one (the outer corner).
    let light_half = band(cx, t);
    // Horizontal double arms; `side` is the vertical arm on the line's side.
    for (w, right) in [(l, false), (r, true)] {
        if !double(w) {
            continue;
        }
        let other = if right { l } else { r };
        for (off, side, far) in [(-d, u, dn), (d, dn, u)] {
            let (b0, b1) = band(cy + off, t);
            let near = if double(side) {
                if right { band(cx + d, t).0 } else { band(cx - d, t).1 }
            } else if double(far) || other != 0 {
                if right { band(cx - d, t).0 } else { band(cx + d, t).1 }
            } else if right {
                light_half.0
            } else {
                light_half.1
            };
            if right {
                fill(near, x1, b0, b1);
            } else {
                fill(x0, near, b0, b1);
            }
        }
    }
    let light_half = band(cy, t);
    for (w, down) in [(u, false), (dn, true)] {
        if !double(w) {
            continue;
        }
        let other = if down { u } else { dn };
        for (off, side, far) in [(-d, l, r), (d, r, l)] {
            let (b0, b1) = band(cx + off, t);
            let near = if double(side) {
                if down { band(cy + d, t).0 } else { band(cy - d, t).1 }
            } else if double(far) || other != 0 {
                if down { band(cy - d, t).0 } else { band(cy + d, t).1 }
            } else if down {
                light_half.0
            } else {
                light_half.1
            };
            if down {
                fill(b0, b1, near, y1);
            } else {
                fill(b0, b1, y0, near);
            }
        }
    }
}

/// A block element (U+2580–259F) as parts of its cell: up to four rects
/// `[left, top, right, bottom]` in cell units, how many, and how solid
/// (the shades ░▒▓ are the whole cell, lighter).
fn block_rects(c: char) -> Option<([[f32; 4]; 4], usize, f32)> {
    let cp = c as u32;
    let one = |r: [f32; 4], alpha: f32| Some(([r, [0.0; 4], [0.0; 4], [0.0; 4]], 1, alpha));
    match cp {
        0x2580 => one([0.0, 0.0, 1.0, 0.5], 1.0),
        // ▁ … █: the lower eighths.
        0x2581..=0x2588 => one([0.0, 1.0 - (cp - 0x2580) as f32 / 8.0, 1.0, 1.0], 1.0),
        // ▉ … ▏: the left seven eighths down to one.
        0x2589..=0x258F => one([0.0, 0.0, (0x2590 - cp) as f32 / 8.0, 1.0], 1.0),
        0x2590 => one([0.5, 0.0, 1.0, 1.0], 1.0),
        0x2591..=0x2593 => one([0.0, 0.0, 1.0, 1.0], (cp - 0x2590) as f32 * 0.25),
        0x2594 => one([0.0, 0.0, 1.0, 0.125], 1.0),
        0x2595 => one([0.875, 0.0, 1.0, 1.0], 1.0),
        0x2596..=0x259F => {
            // Quadrants: upper left 1, upper right 2, lower left 4, lower right 8.
            const QUADS: [u8; 10] = [4, 8, 1, 1 | 4 | 8, 1 | 8, 1 | 2 | 4, 1 | 2 | 8, 2, 2 | 4, 2 | 4 | 8];
            let bits = QUADS[(cp - 0x2596) as usize];
            let mut rects = [[0.0; 4]; 4];
            let mut n = 0;
            for (bit, r) in [(1, [0.0, 0.0, 0.5, 0.5]), (2, [0.5, 0.0, 1.0, 0.5]), (4, [0.0, 0.5, 0.5, 1.0]), (8, [0.5, 0.5, 1.0, 1.0])] {
                if bits & bit != 0 {
                    rects[n] = r;
                    n += 1;
                }
            }
            Some((rects, n, 1.0))
        }
        _ => None,
    }
}

fn decorate(p: &Painter, flags: u8, x: f32, w: f32, y: f32, ch: f32, fg: Color32) {
    if flags & term::F_UNDER != 0 {
        p.hline(x..=x + w, y + ch - 1.5, Stroke::new(1.0_f32, fg));
    }
    if flags & term::F_STRIKE != 0 {
        p.hline(x..=x + w, y + ch * 0.55, Stroke::new(1.0_f32, fg));
    }
}

pub fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 1.0 {
        format!("{}ms", (s * 1000.0) as u64)
    } else if s < 60.0 {
        format!("{s:.1}s")
    } else if s < 3600.0 {
        format!("{}m{:02}s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else {
        format!("{}h{:02}m", (s / 3600.0) as u64, ((s % 3600.0) / 60.0) as u64)
    }
}

pub fn key_bytes(key: Key, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let modn = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8;
    let cursor = |c: char| -> Vec<u8> {
        if modn > 1 {
            format!("\x1b[1;{modn}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modn > 1 {
            format!("\x1b[{n};{modn}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let ss3 = |c: char| -> Vec<u8> {
        if modn > 1 {
            format!("\x1b[1;{modn}{c}").into_bytes()
        } else {
            format!("\x1bO{c}").into_bytes()
        }
    };
    let alt = |mut v: Vec<u8>| -> Vec<u8> {
        if m.alt {
            v.insert(0, 0x1b);
        }
        v
    };
    Some(match key {
        Key::ArrowUp => cursor('A'),
        Key::ArrowDown => cursor('B'),
        Key::ArrowRight => cursor('C'),
        Key::ArrowLeft => cursor('D'),
        Key::Home => cursor('H'),
        Key::End => cursor('F'),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::F1 => ss3('P'),
        Key::F2 => ss3('Q'),
        Key::F3 => ss3('R'),
        Key::F4 => ss3('S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Enter => alt(b"\r".to_vec()),
        Key::Tab => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        Key::Backspace => alt(if m.ctrl { vec![0x08] } else { vec![0x7f] }),
        Key::Escape => vec![0x1b],
        _ => {
            let name = key.name();
            if m.ctrl {
                let b = match key {
                    Key::Space | Key::Num2 => 0x00,
                    Key::OpenBracket => 0x1b,
                    Key::Backslash => 0x1c,
                    Key::CloseBracket => 0x1d,
                    Key::Num6 => 0x1e,
                    Key::Minus | Key::Slash => 0x1f,
                    _ if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() => {
                        name.as_bytes()[0].to_ascii_lowercase() & 0x1f
                    }
                    _ => return None,
                };
                alt(vec![b])
            } else if m.alt && name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() {
                let c = name.as_bytes()[0];
                vec![0x1b, if m.shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }]
            } else {
                return None;
            }
        }
    })
}

/// Bytes for the PTY from this frame's text, IME and key events.
///
/// IME composition (Hangul etc.) needs care. While a syllable is being
/// composed, keys belong to the IME:
/// * Backspace edits the syllable; passing it on deleted the committed text
///   too ("거야" + Backspace left only "ㅇ");
/// * Enter, Tab, arrows... make the IME commit the syllable and then deliver
///   the key again; passing the first copy on sent Enter before the syllable
///   ("거" submitted, then "야" submitted separately).
///
/// So keys pressed while composing are dropped (as egui's TextEdit does), and
/// the rare text event that arrives mid-composition is held until the commit.
pub fn typed_bytes(events: &[Event], preedit: &mut String, mods: Modifiers, app_cursor: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut held = Vec::new();
    // The preedit was just emptied by the IME (last jamo deleted): the
    // Backspace that did it may arrive after the update in the same frame.
    let mut emptied_by_ime = false;
    for ev in events {
        let composing = !preedit.is_empty();
        match ev {
            Event::Ime(ImeEvent::Preedit(s)) => {
                emptied_by_ime = composing && s.is_empty();
                *preedit = s.clone();
            }
            Event::Ime(ImeEvent::Commit(s)) => {
                out.extend_from_slice(s.as_bytes());
                preedit.clear();
                emptied_by_ime = false;
                out.append(&mut held);
            }
            Event::Ime(ImeEvent::Disabled) => {
                preedit.clear();
                out.append(&mut held);
            }
            Event::Text(s) => {
                if mods.ctrl == mods.alt {
                    if composing {
                        held.extend_from_slice(s.as_bytes());
                    } else {
                        out.extend_from_slice(s.as_bytes());
                    }
                }
            }
            Event::Key { key, pressed: true, modifiers, .. } => {
                if composing || (emptied_by_ime && matches!(key, Key::Backspace | Key::Delete)) {
                    continue;
                }
                emptied_by_ime = false;
                if let Some(b) = key_bytes(*key, *modifiers, app_cursor) {
                    out.extend_from_slice(&b);
                }
            }
            _ => {}
        }
    }
    out.append(&mut held);
    out
}

/// Where to tell the IME the caret is: frozen while composing, otherwise the
/// cursor cell, else the last known spot, else `fallback`. Never "nowhere".
pub fn ime_anchor(composing: bool, last: Option<Rect>, cursor: Option<Rect>, fallback: Rect) -> Rect {
    if composing {
        last.or(cursor).unwrap_or(fallback)
    } else {
        cursor.or(last).unwrap_or(fallback)
    }
}

fn mouse_seq(sgr: bool, code: u8, col: usize, row: usize, pressed: bool) -> Vec<u8> {
    if sgr {
        format!("\x1b[<{};{};{}{}", code, col + 1, row + 1, if pressed { 'M' } else { 'm' }).into_bytes()
    } else {
        let c = if pressed { code } else { 3 + (code & !3) };
        vec![0x1b, b'[', b'M', 32 + c, (33 + col.min(222)) as u8, (33 + row.min(222)) as u8]
    }
}

/// Remove a matching key press from this frame's events (before a TextEdit sees it).
pub fn take_key(ui: &Ui, key: Key, pred: impl Fn(Modifiers) -> bool) -> bool {
    ui.input_mut(|i| {
        let pos = i.events.iter().position(|e| matches!(e, Event::Key { key: k, pressed: true, modifiers, .. } if *k == key && pred(*modifiers)));
        match pos {
            Some(p) => {
                i.events.remove(p);
                true
            }
            None => false,
        }
    })
}

/// A row of what is on screen: the alternate screen while a full-screen app
/// runs, else the primary buffer (scrollback + screen).
fn row_at(t: &Term, abs: u64) -> Option<&term::Row> {
    if t.alt_active {
        t.screen.get(abs.checked_sub(t.screen_base())? as usize)
    } else {
        t.line(abs)
    }
}

/// The word around a cell as columns [start, end): a run of characters
/// between spaces and brackets/quotes; a separator alone is its own word.
fn word_span(t: &Term, abs: u64, col: usize) -> Option<(usize, usize)> {
    let chars = row_at(t, abs)?.chars_with_cols();
    let idx = chars.iter().rposition(|&(_, c)| c <= col)?;
    let delim = crate::widgets::word_break;
    let end_of = |i: usize| chars.get(i + 1).map(|n| n.1).unwrap_or(chars[i].1 + 1);
    if delim(chars[idx].0) {
        return Some((chars[idx].1, end_of(idx)));
    }
    let (mut s, mut e) = (idx, idx);
    while s > 0 && !delim(chars[s - 1].0) {
        s -= 1;
    }
    while e + 1 < chars.len() && !delim(chars[e + 1].0) {
        e += 1;
    }
    Some((chars[s].1, end_of(e)))
}

/// Ordered ends (inclusive), widened to whole words or lines by the mode.
fn sel_ends(t: &Term, s: Selection) -> ((u64, usize), (u64, usize)) {
    let (mut a, mut b) = if s.a <= s.b { (s.a, s.b) } else { (s.b, s.a) };
    match s.mode {
        SelMode::Char => {}
        SelMode::Word => {
            if let Some((st, _)) = word_span(t, a.0, a.1) {
                a.1 = st;
            }
            if let Some((_, e)) = word_span(t, b.0, b.1) {
                b.1 = e.saturating_sub(1);
            }
        }
        SelMode::Line => {
            a.1 = 0;
            // The full width: rows in scrollback are stored without trailing blanks.
            b.1 = t.cols.saturating_sub(1);
        }
        SelMode::Block => {
            a.1 = s.a.1.min(s.b.1);
            b.1 = s.a.1.max(s.b.1);
        }
    }
    (a, b)
}

/// The selection's columns [c0, c1) on row `abs`, or None when off it.
fn sel_cols(t: &Term, s: Selection, abs: u64, cols: usize) -> Option<(usize, usize)> {
    let (a, b) = sel_ends(t, s);
    if abs < a.0 || abs > b.0 {
        return None;
    }
    if s.mode == SelMode::Block {
        return Some((a.1, (b.1 + 1).min(cols)));
    }
    Some((if abs == a.0 { a.1 } else { 0 }, if abs == b.0 { (b.1 + 1).min(cols) } else { cols }))
}

fn selection_text(t: &Term, s: Selection) -> String {
    let (a, b) = sel_ends(t, s);
    // A block is its columns of each row, one row per line.
    if s.mode == SelMode::Block {
        return (a.0..=b.0).filter_map(|abs| row_at(t, abs).map(|r| r.text_range(a.1, b.1 + 1))).collect::<Vec<_>>().join("\n");
    }
    if !t.alt_active {
        return t.text_between(a, (b.0, b.1 + 1));
    }
    let mut out = String::new();
    for abs in a.0..=b.0 {
        let Some(row) = row_at(t, abs) else { continue };
        let from = if abs == a.0 { a.1 } else { 0 };
        let to = if abs == b.0 { b.1 + 1 } else { row.cells.len() };
        out.push_str(&row.text_range(from, to));
        if abs < b.0 {
            out.push('\n');
        }
    }
    out
}

/// Path or URL under the pointer, resolved to (kind, target, line).
fn classify_token(tok: &str, cwd: Option<&str>, home: Option<&str>) -> Option<(bool, String, usize)> {
    if tok.starts_with("http://") || tok.starts_with("https://") {
        return Some((true, tok.to_string(), 0));
    }
    let mut path = tok.to_string();
    let mut nums = Vec::new();
    for _ in 0..2 {
        match path.rsplit_once(':') {
            Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) && !head.is_empty() => {
                nums.push(tail.parse::<usize>().unwrap_or(1));
                path = head.to_string();
            }
            _ => break,
        }
    }
    let line = nums.last().copied().unwrap_or(1);
    let path = path.strip_prefix("file://").map(String::from).unwrap_or(path);
    if !path.contains('/') && !path.contains('.') {
        return None;
    }
    let abs = if path.starts_with('/') || (path.len() > 2 && path.as_bytes()[1] == b':') {
        path
    } else if let Some(rest) = path.strip_prefix("~/") {
        join(home?, rest)
    } else {
        join(cwd?, path.trim_start_matches("./"))
    };
    Some((false, abs, line))
}

/// How long a cursor the program has hidden stays on screen (see where
/// it is drawn): longer than a redraw takes to arrive, too short to see.
pub const CURSOR_HOLD: Duration = Duration::from_millis(150);

type SplitHandle = (Vec<bool>, Rect, bool, Rect);

/// Pane rects and splitter handles for a tab. Returns true when zoomed: only
/// the active pane is laid out, over the whole tab (the split tree is kept
/// untouched so un-zooming restores it exactly).
pub fn tab_layout(tab: &Tab, rect: Rect, panes: &mut Vec<(usize, Rect)>, splits: &mut Vec<SplitHandle>) -> bool {
    tab.root.layout(rect, &mut Vec::new(), panes, splits);
    let zoomed = tab.zoomed && panes.len() > 1;
    if zoomed {
        panes.clear();
        panes.push((tab.active, rect));
        splits.clear();
    }
    zoomed
}

/// Points where splitters meet (indices into `splits`): three panes around
/// one, or four (a +).
pub fn split_junctions(splits: &[SplitHandle]) -> Vec<crate::app::Junction> {
    let near = |a: f32, b: f32| (a - b).abs() <= 6.0;
    let mut out = Vec::new();
    for (vi, (_, v, vertical, _)) in splits.iter().enumerate() {
        if !vertical {
            continue;
        }
        for (hi, (_, h, hvert, _)) in splits.iter().enumerate() {
            if *hvert {
                continue;
            }
            // Horizontal splitter ends at the vertical one (a | b, b split top/bottom) ...
            let h_touches_v = (near(h.left(), v.right()) || near(h.right(), v.left())) && v.y_range().contains(h.center().y);
            // ... or the vertical splitter ends at the horizontal one (top/bottom, one side split left/right).
            let v_touches_h = (near(v.top(), h.bottom()) || near(v.bottom(), h.top())) && h.x_range().contains(v.center().x);
            if h_touches_v || v_touches_h {
                out.push((vi, hi, egui::pos2(v.center().x, h.center().y)));
            }
        }
    }
    crate::app::group_junctions(out)
}

impl App {
    pub fn ui_panes(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let Some(tab) = self.tabs.get(self.active_tab) else { return };
        let mut panes = Vec::new();
        let mut splits = Vec::new();
        let active = tab.active;
        let zoomed = tab_layout(tab, rect, &mut panes, &mut splits);
        self.pane_rects = panes.clone();
        let multi = panes.len() > 1 || zoomed;
        // Over a junction the splitters under it stand aside (see the panel
        // borders): right on the point the thin splitter used to win.
        const JUNCTION: f32 = 20.0;
        let junctions = split_junctions(&splits);
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let near = |i: usize| pointer.is_some_and(|p| junctions.iter().any(|j| (j.0.contains(&i) || j.1.contains(&i)) && Rect::from_center_size(j.2, vec2(JUNCTION, JUNCTION)).contains(p)));
        for (i, (path, handle, vertical, parent)) in splits.iter().cloned().enumerate() {
            let id = Id::new(("split", self.active_tab, path.clone()));
            if near(i) && !ui.ctx().is_being_dragged(id) {
                ui.painter().rect_filled(handle, CornerRadius::ZERO, theme::bg_base());
                continue;
            }
            let resp = ui.interact(handle.expand(2.0), id, Sense::drag());
            let hot = resp.hovered() || resp.dragged();
            if hot {
                ui.ctx().set_cursor_icon(if vertical { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
            }
            ui.painter().rect_filled(handle, CornerRadius::ZERO, if hot { theme::tint(theme::accent(), 140) } else { theme::bg_base() });
            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let r = if vertical {
                        (pos.x - parent.left()) / parent.width()
                    } else {
                        (pos.y - parent.top()) / parent.height()
                    };
                    if let Some(ratio) = self.tabs[self.active_tab].root.ratio_at(&path) {
                        *ratio = r.clamp(0.1, 0.9);
                    }
                }
            }
        }
        // Where a vertical and a horizontal splitter meet (three panes touch),
        // a handle drags both at once.
        for (vs, hs, at) in junctions {
            let paths: Vec<Vec<bool>> = vs.iter().chain(&hs).map(|&i| splits[i].0.clone()).collect();
            let zone = Rect::from_center_size(at, vec2(JUNCTION, JUNCTION));
            let resp = ui.interact(zone, Id::new(("junction", self.active_tab, paths)), Sense::drag());
            let hot = resp.hovered() || resp.dragged();
            if hot {
                ui.ctx().set_cursor_icon(CursorIcon::Move);
                ui.painter().circle_filled(at, 4.5, theme::accent());
            }
            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    // Every splitter there: verticals follow x, horizontals y.
                    let root = &mut self.tabs[self.active_tab].root;
                    for &i in &vs {
                        let (path, _, _, parent) = &splits[i];
                        if let Some(r) = root.ratio_at(path) {
                            *r = ((pos.x - parent.left()) / parent.width()).clamp(0.1, 0.9);
                        }
                    }
                    for &i in &hs {
                        let (path, _, _, parent) = &splits[i];
                        if let Some(r) = root.ratio_at(path) {
                            *r = ((pos.y - parent.top()) / parent.height()).clamp(0.1, 0.9);
                        }
                    }
                }
            }
        }
        for (id, r) in panes {
            if self.panes.contains_key(&id) {
                self.ui_pane(ui, id, r, id == active, multi);
            }
        }
        self.pane_drop(ui);
        if zoomed {
            // "ZOOM" badge left of the pane buttons; clicking it restores the split.
            let label = "ZOOM · click to restore";
            let g = ui.painter().layout_no_wrap(label.into(), FontId::proportional(11.0), theme::accent());
            let pill = Rect::from_min_size(pos2(rect.right() - 90.0 - g.size().x - 16.0, rect.top() + 4.0), vec2(g.size().x + 16.0, 16.0));
            let resp = ui.interact(pill, Id::new(("zoom-badge", self.active_tab)), Sense::click());
            let alpha = if resp.hovered() { 70 } else { 40 };
            ui.painter().rect_filled(pill, CornerRadius::same(8), theme::tint(theme::accent(), alpha));
            ui.painter().galley(pos2(pill.left() + 8.0, pill.center().y - g.size().y / 2.0), g, theme::accent());
            if resp.on_hover_text("restore split layout (Ctrl+Shift+Z)").clicked() {
                self.actions.push(Action::ToggleZoom);
            }
        }
    }

    /// While a pane is dragged by its header: show where it would go — a
    /// half of the pane under the pointer (split there) or its middle (swap)
    /// — and move it there on release.
    fn pane_drop(&mut self, ui: &mut Ui) {
        let Some(src) = self.pane_drag else { return };
        let (pos, released) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.any_released() || !i.pointer.primary_down()));
        let target = pos.and_then(|p| self.pane_rects.iter().find(|(id, r)| *id != src && r.contains(p)).map(|&(id, r)| (id, r, p)));
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("pane-drop")));
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        let drop = target.map(|(id, r, p)| {
            let (fx, fy) = ((p.x - r.left()) / r.width(), (p.y - r.top()) / r.height());
            // The middle third (both ways) swaps; elsewhere the nearest edge.
            let side = if (0.33..=0.67).contains(&fx) && (0.33..=0.67).contains(&fy) {
                None
            } else {
                [(Dock::Left, fx), (Dock::Right, 1.0 - fx), (Dock::Top, fy), (Dock::Bottom, 1.0 - fy)].into_iter().min_by(|a, b| a.1.total_cmp(&b.1)).map(|t| t.0)
            };
            let zone = match side {
                None => r.shrink2(vec2(r.width() * 0.2, r.height() * 0.2)),
                Some(Dock::Left) => Rect::from_min_max(r.min, pos2(r.center().x, r.bottom())),
                Some(Dock::Right) => Rect::from_min_max(pos2(r.center().x, r.top()), r.max),
                Some(Dock::Top) => Rect::from_min_max(r.min, pos2(r.right(), r.center().y)),
                Some(Dock::Bottom) => Rect::from_min_max(pos2(r.left(), r.center().y), r.max),
            };
            (id, side, zone)
        });
        match drop {
            Some((_, side, zone)) => {
                painter.rect(zone.shrink(4.0), CornerRadius::same(8), theme::tint(theme::accent(), 40), Stroke::new(2.0_f32, theme::accent()), StrokeKind::Inside);
                let text = match side {
                    None => "swap the two panes".to_string(),
                    Some(d) => format!("move the pane to the {}", d.label()),
                };
                painter.text(zone.center(), Align2::CENTER_CENTER, text, FontId::proportional(14.0), theme::text());
            }
            None => {
                if let Some(p) = pos {
                    painter.text(p + vec2(16.0, 16.0), Align2::LEFT_TOP, "drop on another pane of this tab", FontId::proportional(12.0), theme::text_dim());
                }
            }
        }
        if released {
            self.pane_drag = None;
            if let (Some((target, side, _)), Some(tab)) = (drop, self.tabs.get_mut(self.active_tab)) {
                let root = std::mem::replace(&mut tab.root, crate::app::Node::Leaf(src));
                tab.root = root.moved(src, target, side);
                tab.active = src;
                tab.zoomed = false;
            }
        }
    }

    fn ui_pane(&mut self, ui: &mut Ui, id: usize, rect: Rect, active: bool, multi: bool) {
        let font = theme::term_font(self.settings.font_size);
        let (cw, ch) = ui.fonts(|f| (theme::cell_width(f, &font), f.row_height(&font).ceil()));
        let header_h = if multi { 24.0 } else { 0.0 };
        let gutter = 14.0;
        let body = Rect::from_min_max(pos2(rect.left(), rect.top() + header_h), rect.max);
        let inner = Rect::from_min_max(pos2(body.left() + gutter, body.top() + 4.0), pos2(body.right() - 8.0, body.bottom() - 4.0));
        let cols = ((inner.width() / cw).floor() as usize).max(10);
        let rows = ((inner.height() / ch).floor() as usize).max(2);
        let term_id = Id::new(("pane", id));
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, theme::bg_term());
        let mut acts: Vec<Action> = Vec::new();

        // 1. geometry
        {
            // Apply a new size only once it has been stable for a moment: while a
            // splitter is dragged, every intermediate width would reflow the
            // buffer and send the shell a SIGWINCH (each redraws the prompt).
            let p = self.panes.get_mut(&id).unwrap();
            let want = (cols, rows);
            if (p.term.cols, p.term.rows) != want {
                let settle = Duration::from_millis(120);
                let ready = match p.pending_size {
                    _ if !p.sized => true,
                    Some((s, at)) if s == want => at.elapsed() >= settle,
                    _ => {
                        p.pending_size = Some((want, std::time::Instant::now()));
                        false
                    }
                };
                if ready {
                    p.pending_size = None;
                    p.sized = true;
                    p.term.resize(cols, rows);
                    if let Some(pty) = p.pty.as_ref() {
                        pty.resize(cols as u16, rows as u16);
                    }
                    p.sel = None;
                    if let Some((base, new_abs)) = p.term.take_line_remap() {
                        for w in p.widgets.iter_mut() {
                            if let Some(&n) = w.anchor.checked_sub(base).and_then(|i| new_abs.get(i as usize)) {
                                w.anchor = n;
                            }
                        }
                    }
                } else {
                    ui.ctx().request_repaint_after(settle);
                }
            } else {
                p.pending_size = None;
                p.sized = true;
            }
        }

        // 2. visual rows / window
        let (all, top, window) = {
            let t = &self.panes[&id].term;
            if t.alt_active {
                let w: Vec<VRow> = (0..t.rows).map(VRow::Screen).collect();
                (Vec::new(), 0, w)
            } else {
                let all = visual_rows(t);
                let n = all.len();
                let p = self.panes.get_mut(&id).unwrap();
                p.scroll = p.scroll.min(n.saturating_sub(rows));
                // Copy mode: scroll so its cursor stays on screen.
                if let Some(cm) = p.copy {
                    if let Some(idx) = all.iter().position(|v| matches!(v, VRow::Line(a) if *a == cm.cur.0)) {
                        let bottom = n - p.scroll;
                        if idx < bottom.saturating_sub(rows) {
                            p.scroll = n - (idx + rows).min(n);
                        } else if idx >= bottom {
                            p.scroll = n - idx - 1;
                        }
                    }
                }
                let bottom = n - p.scroll;
                let top = bottom.saturating_sub(rows);
                let w = all[top..bottom].to_vec();
                (all, top, w)
            }
        };
        // The program has ended: its notice takes the bottom of the pane,
        // and the text is drawn that much higher, its last line just above
        // the notice (the notice covered it). Only as far as the text
        // reaches down there.
        let banner = Rect::from_min_size(pos2(inner.left(), inner.bottom() - ch - 10.0), vec2(inner.width(), ch + 8.0));
        let lift = if self.panes[&id].exited { (inner.top() + window.len() as f32 * ch - (banner.top() - 2.0)).max(0.0) } else { 0.0 };
        let row_y = |i: usize| inner.top() - lift + i as f32 * ch;

        // 3. interaction
        let resp = ui.interact(body, term_id, Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
            resp.request_focus();
            if !active {
                acts.push(Action::Focus(id));
            }
        }
        if active && self.focus_term && self.panes[&id].composer.is_none() && self.panes[&id].search.as_ref().map(|s| !s.focus).unwrap_or(true) {
            resp.request_focus();
            self.focus_term = false;
        }
        let focused = ui.memory(|m| m.has_focus(term_id));
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(term_id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
            });
        }
        let host = self.panes[&id].host.clone();
        let home = self.conns.get(&host).and_then(|c| c.home());
        let mods = ui.input(|i| i.modifiers);
        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| inner.contains(*p) && resp.hovered());
        let hit = |t: &Term, pos: egui::Pos2| -> Option<(u64, usize)> {
            let i = ((pos.y - inner.top() + lift) / ch).floor().clamp(0.0, (window.len().max(1) - 1) as f32) as usize;
            let col = ((pos.x - inner.left()) / cw).floor().clamp(0.0, (cols - 1) as f32) as usize;
            window.get(i).and_then(|v| v.abs(t)).map(|a| (a, col))
        };

        {
            let p = self.panes.get_mut(&id).unwrap();
            let t_alt = p.term.alt_active;
            let report = p.term.mouse_mode != 0 && !mods.shift && (t_alt || p.scroll == 0);
            let events = ui.input(|i| i.events.clone());
            let mut out: Vec<u8> = Vec::new();
            let mut mouse_out: Vec<u8> = Vec::new();

            // Keyboard (only when this pane owns focus).
            if focused && p.copy.is_some() {
                let mut cm = p.copy.unwrap();
                let t = &p.term;
                let view = (window.first().and_then(|v| v.abs(t)).unwrap_or(0), window.last().and_then(|v| v.abs(t)).unwrap_or(0));
                let (lo, hi) = if t.alt_active { (t.screen_base(), t.screen_base() + t.rows as u64 - 1) } else { (t.first_line(), t.end_line() - 1) };
                let b = crate::copymode::Bounds { lo, hi, view, cols };
                // Ctrl+C / Ctrl+V may come as Copy / Paste events instead of keys.
                let ctrl_key = |k: Key| events.iter().any(|e| matches!(e, Event::Key { key, pressed: true, modifiers, .. } if *key == k && modifiers.ctrl));
                let (has_c, has_v) = (ctrl_key(Key::C), ctrl_key(Key::V));
                let mut outcome = crate::copymode::Outcome::Stay;
                for ev in &events {
                    let (k, m) = match ev {
                        Event::Key { key, pressed: true, modifiers, .. } => (*key, *modifiers),
                        Event::Copy if !has_c => (Key::C, Modifiers::CTRL),
                        Event::Paste(_) if !has_v => (Key::V, Modifiers::CTRL),
                        _ => continue,
                    };
                    outcome = crate::copymode::key(&mut cm, &mut p.sel, &p.term, &b, k, m);
                    if outcome != crate::copymode::Outcome::Stay {
                        break;
                    }
                }
                match outcome {
                    crate::copymode::Outcome::Stay => p.copy = Some(cm),
                    crate::copymode::Outcome::Copy => {
                        // Nothing selected: the cursor's line.
                        let text = match p.sel {
                            Some(s) => selection_text(&p.term, s),
                            None => p.term.visible_row(cm.cur.0).map(|r| r.text()).unwrap_or_default(),
                        };
                        acts.push(Action::Copy(text));
                        p.sel = None;
                        p.copy = None;
                    }
                    crate::copymode::Outcome::Exit => {
                        p.sel = None;
                        p.copy = None;
                    }
                }
            } else if focused {
                if p.exited && events.iter().any(|e| matches!(e, Event::Key { key: Key::Enter, pressed: true, .. })) {
                    acts.push(Action::Restart(id));
                } else {
                    out.extend(typed_bytes(&events, &mut p.preedit, mods, p.term.app_cursor));
                }
                for ev in &events {
                    match ev {
                        Event::Paste(s) => acts.push(Action::Paste(id, s.clone())),
                        Event::Copy => match p.sel {
                            Some(sel) => {
                                acts.push(Action::Copy(selection_text(&p.term, sel)));
                                p.sel = None;
                            }
                            None if !mods.shift => out.push(0x03),
                            None => {}
                        },
                        Event::Cut => out.push(0x18),
                        _ => {}
                    }
                }
            }

            // Mouse.
            if report && resp.hovered() {
                let to_cell = |pos: egui::Pos2| {
                    let col = ((pos.x - inner.left()) / cw).floor().clamp(0.0, (cols - 1) as f32) as usize;
                    let row = ((pos.y - inner.top()) / ch).floor().clamp(0.0, (rows - 1) as f32) as usize;
                    (col, row)
                };
                let held = ui.input(|i| {
                    if i.pointer.primary_down() {
                        Some(0u8)
                    } else if i.pointer.middle_down() {
                        Some(1)
                    } else if i.pointer.secondary_down() {
                        Some(2)
                    } else {
                        None
                    }
                });
                for ev in &events {
                    match ev {
                        Event::PointerButton { pos, button, pressed, .. } if inner.contains(*pos) => {
                            let code = match button {
                                PointerButton::Primary => 0,
                                PointerButton::Middle => 1,
                                PointerButton::Secondary => 2,
                                _ => continue,
                            };
                            let (c, r) = to_cell(*pos);
                            mouse_out.extend(mouse_seq(p.term.mouse_sgr, code, c, r, *pressed));
                        }
                        Event::PointerMoved(pos) if inner.contains(*pos) => {
                            let motion = (p.term.mouse_mode == 1002 && held.is_some()) || p.term.mouse_mode == 1003;
                            if motion {
                                let (c, r) = to_cell(*pos);
                                mouse_out.extend(mouse_seq(p.term.mouse_sgr, 32 + held.unwrap_or(3), c, r, true));
                            }
                        }
                        Event::MouseWheel { delta, .. } => {
                            if let Some(pos) = hover {
                                let (c, r) = to_cell(pos);
                                let code = if delta.y > 0.0 { 64 } else { 65 };
                                if delta.y != 0.0 {
                                    mouse_out.extend(mouse_seq(p.term.mouse_sgr, code, c, r, true));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            } else if resp.hovered() {
                let dy = ui.input(|i| i.raw_scroll_delta.y);
                if dy != 0.0 {
                    let n = ((dy.abs() / ch).ceil() as usize).max(1);
                    if t_alt {
                        let seq = if dy > 0.0 { key_bytes(Key::ArrowUp, Modifiers::NONE, p.term.app_cursor) } else { key_bytes(Key::ArrowDown, Modifiers::NONE, p.term.app_cursor) };
                        for _ in 0..n {
                            out.extend(seq.clone().unwrap_or_default());
                        }
                    } else if dy > 0.0 {
                        p.scroll += n;
                    } else {
                        p.scroll = p.scroll.saturating_sub(n);
                    }
                }
                // Selection, as in WezTerm: drag = characters, double-click
                // (and drag) = words, triple-click (and drag) = lines,
                // Alt+drag = a block of columns.
                let clicks_id = term_id.with("clicks");
                let mut clicks: u8 = ui.data(|d| d.get_temp::<(f64, u8, egui::Pos2)>(clicks_id)).map(|c| c.1).unwrap_or(1);
                for ev in &events {
                    if let Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers } = ev {
                        if !inner.contains(*pos) {
                            continue;
                        }
                        let now = ui.input(|i| i.time);
                        let last = ui.data(|d| d.get_temp::<(f64, u8, egui::Pos2)>(clicks_id));
                        clicks = match last {
                            Some((t0, n, p0)) if now - t0 < 0.45 && p0.distance(*pos) < 6.0 => n % 3 + 1,
                            _ => 1,
                        };
                        ui.data_mut(|d| d.insert_temp(clicks_id, (now, clicks, *pos)));
                        if clicks >= 2 {
                            if let Some(a) = hit(&p.term, *pos) {
                                let mode = if clicks == 2 { SelMode::Word } else { SelMode::Line };
                                p.sel = Some(Selection { a, b: a, mode });
                                p.selecting = true;
                            }
                        } else if modifiers.alt {
                            p.sel = None;
                        }
                    }
                }
                if resp.drag_started_by(PointerButton::Primary) && clicks == 1 {
                    if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                        if let Some(a) = hit(&p.term, origin) {
                            let mode = if mods.alt { SelMode::Block } else { SelMode::Char };
                            p.sel = Some(Selection { a, b: a, mode });
                            p.selecting = true;
                        }
                    }
                }
                if p.selecting && ui.input(|i| i.pointer.primary_down()) {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                        if pos.y < inner.top() && !t_alt {
                            p.scroll += 1;
                        } else if pos.y > inner.bottom() {
                            p.scroll = p.scroll.saturating_sub(1);
                        }
                        if let (Some(b), Some(sel)) = (hit(&p.term, pos), p.sel.as_mut()) {
                            sel.b = b;
                        }
                    }
                }
                if p.selecting && !ui.input(|i| i.pointer.primary_down()) {
                    p.selecting = false;
                    // A plain click leaves no selection; words and lines stay.
                    if p.sel.map(|s| s.a == s.b && matches!(s.mode, SelMode::Char | SelMode::Block)).unwrap_or(false) {
                        p.sel = None;
                    }
                }
                if resp.clicked() && clicks == 1 {
                    p.sel = None;
                    if mods.ctrl {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            if let Some((abs, col)) = hit(&p.term, pos) {
                                if let Some((tok, _, _)) = p.term.token_at(abs, col) {
                                    let cwd = p.term.cwd.clone().or(p.initial_cwd.clone());
                                    match classify_token(&tok, cwd.as_deref(), home.as_deref()) {
                                        Some((true, url, _)) => acts.push(Action::OpenUrl(url)),
                                        Some((false, path, line)) => acts.push(Action::OpenFile(host.clone(), path, line)),
                                        None => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Typing clears the selection; mouse reports to the app don't (an
            // app tracking every move would wipe a Shift+drag selection).
            if !out.is_empty() {
                if focused {
                    p.sel = None;
                }
                p.send(&out);
            }
            if !mouse_out.is_empty() {
                p.send(&mouse_out);
            }
        }

        // 4. draw grid + overlays (read-only)
        // (Raised, the text stays inside the terminal: its top line is cut
        // off rather than drawn over the pane's header.)
        let whole = painter.clone();
        let painter = if lift > 0.0 { painter.with_clip_rect(painter.clip_rect().intersect(inner.expand2(vec2(2.0, 0.0)))) } else { painter };
        let p = &self.panes[&id];
        let t = &p.term;
        for (i, vr) in window.iter().enumerate() {
            let y = row_y(i);
            match *vr {
                VRow::Line(abs) => {
                    if let Some(row) = t.line(abs) {
                        paint_row(&painter, row, inner.left(), y, cw, ch, &font, cols);
                    }
                }
                VRow::Screen(sy) => {
                    if let Some(row) = t.screen.get(sy) {
                        paint_row(&painter, row, inner.left(), y, cw, ch, &font, cols);
                    }
                }
                VRow::Fold(bid, count, _) => {
                    let r = Rect::from_min_size(pos2(inner.left(), y + 1.0), vec2(inner.width().min(46.0 * cw), ch - 2.0));
                    let fr = ui.interact(r, Id::new(("fold", id, bid)), Sense::click());
                    let bg = if fr.hovered() { theme::bg_hover() } else { theme::bg_raised() };
                    painter.rect_filled(r, CornerRadius::same(4), bg);
                    icons::paint(&painter, Rect::from_min_size(r.min + vec2(4.0, 1.0), vec2(ch - 4.0, ch - 4.0)), Icon::Unfold, theme::text_dim());
                    painter.text(
                        pos2(r.left() + ch + 4.0, y + ch / 2.0),
                        Align2::LEFT_CENTER,
                        format!("{count} lines folded — click to expand"),
                        FontId::proportional(12.0),
                        theme::text_dim(),
                    );
                    if fr.clicked() {
                        acts.push(Action::ToggleFold(id, bid));
                    }
                }
            }
        }
        let vis_index = |abs: u64| -> Option<usize> { window.iter().position(|v| v.abs(t) == Some(abs)) };

        // Search hits and selection.
        if let Some(s) = p.search.as_ref() {
            for (k, &(abs, c0, c1)) in s.hits.iter().enumerate() {
                if let Some(i) = vis_index(abs) {
                    let r = Rect::from_min_size(pos2(inner.left() + c0 as f32 * cw, row_y(i)), vec2((c1 - c0) as f32 * cw, ch));
                    let col = if k == s.cur { theme::tint(theme::warn(), 150) } else { theme::search_hit() };
                    painter.rect_filled(r, CornerRadius::same(2), col);
                }
            }
        }
        if let Some(sel) = p.sel {
            for (i, vr) in window.iter().enumerate() {
                let Some(abs) = vr.abs(t) else { continue };
                let Some((c0, c1)) = sel_cols(t, sel, abs, cols) else { continue };
                let r = Rect::from_min_size(pos2(inner.left() + c0 as f32 * cw, row_y(i)), vec2((c1.saturating_sub(c0)) as f32 * cw, ch));
                painter.rect_filled(r, CornerRadius::ZERO, theme::selection());
            }
        }
        if let Some(cm) = p.copy {
            if let Some(i) = vis_index(cm.cur.0) {
                let r = Rect::from_min_size(pos2(inner.left() + cm.cur.1 as f32 * cw, row_y(i)), vec2(cw, ch));
                painter.rect_filled(r, CornerRadius::ZERO, theme::tint(theme::accent(), 90));
                painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5_f32, theme::accent()), StrokeKind::Inside);
            }
            let label = "COPY · hjkl w b e 0 $ gg G · v V Ctrl+v select · y copy · Esc";
            let g = painter.layout_no_wrap(label.into(), FontId::proportional(11.0), theme::accent());
            let pill = Rect::from_min_size(pos2(inner.right() - g.size().x - 16.0, inner.top()), vec2(g.size().x + 16.0, 18.0));
            painter.rect_filled(pill, CornerRadius::same(9), theme::tint(theme::accent(), 45));
            painter.galley(pos2(pill.left() + 8.0, pill.center().y - g.size().y / 2.0), g, theme::accent());
        }

        // Ctrl-hover: underline clickable paths/URLs.
        if mods.ctrl && !t.alt_active {
            if let Some(pos) = hover {
                if let Some((abs, col)) = hit(t, pos) {
                    if let Some((tok, s, e)) = t.token_at(abs, col) {
                        let cwd = t.cwd.as_deref().or(p.initial_cwd.as_deref());
                        if classify_token(&tok, cwd, home.as_deref()).is_some() {
                            if let Some(i) = vis_index(abs) {
                                let y = row_y(i) + ch - 1.5;
                                painter.hline(inner.left() + s as f32 * cw..=inner.left() + e as f32 * cw, y, Stroke::new(1.5_f32, theme::accent()));
                                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                            }
                        }
                    }
                }
            }
        }

        // Block chrome: gutter bars, status badges, hover toolbars.
        let mut widget_slots: Vec<(usize, Rect)> = Vec::new();
        if !t.alt_active && !window.is_empty() {
            let first_abs = window.iter().find_map(|v| v.abs(t)).unwrap_or(0);
            let last_abs = window.iter().rev().find_map(|v| v.abs(t)).unwrap_or(0);
            for b in &t.blocks {
                let end = b.end_line.unwrap_or(t.cursor_abs() + 1);
                if end <= first_abs || b.prompt_line > last_abs {
                    continue;
                }
                let idxs: Vec<usize> = window
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| match **v {
                        VRow::Line(a) => a >= b.prompt_line && a < end,
                        VRow::Fold(bid, ..) => bid == b.id,
                        VRow::Screen(_) => false,
                    })
                    .map(|(i, _)| i)
                    .collect();
                let (Some(&i0), Some(&i1)) = (idxs.first(), idxs.last()) else { continue };
                let y0 = row_y(i0);
                let y1 = row_y(i1) + ch;
                let state = b.state();
                let color = match state {
                    BlockState::Prompt => None,
                    BlockState::Running => Some(theme::accent()),
                    BlockState::Done => Some(match b.exit {
                        Some(0) => theme::tint(theme::success(), 150),
                        Some(_) => theme::error(),
                        // Guessed (or the shell gives no code): neutral.
                        None => theme::tint(theme::text_faint(), 150),
                    }),
                };
                let gut = Rect::from_min_max(pos2(body.left() + 4.0, y0 + 1.0), pos2(body.left() + 7.0, y1 - 1.0));
                if let Some(c) = color {
                    painter.rect_filled(gut, CornerRadius::same(2), c);
                    // Clicking the bar selects the block's output.
                    let gr = ui.interact(Rect::from_min_max(pos2(body.left(), y0), pos2(body.left() + gutter, y1)), Id::new(("gutter", id, b.id)), Sense::click());
                    if gr.hovered() {
                        painter.rect_filled(gut.expand(1.0), CornerRadius::same(2), c);
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                    if gr.clicked() {
                        if let Some(o) = b.out_line {
                            acts.push(Action::Copy(t.block_output(b)));
                            let _ = o;
                        }
                    }
                    gr.on_hover_text("click: copy this command's output");
                }
                let in_block = hover.map(|h| h.y >= y0 && h.y < y1).unwrap_or(false)
                    || ui.input(|i| i.pointer.hover_pos()).map(|h| h.y >= y0 && h.y < y1 && h.x > inner.right() - 160.0 && body.contains(h)).unwrap_or(false);
                if in_block && state != BlockState::Prompt {
                    painter.rect_filled(Rect::from_min_max(pos2(body.left() + gutter - 4.0, y0), pos2(inner.right(), y1)), CornerRadius::same(3), theme::tint(theme::text(), 6));
                }
                // Badge / toolbar on the prompt line.
                let Some(ip) = vis_index(b.prompt_line) else { continue };
                let yp = row_y(ip);
                let text_len = t.line(b.prompt_line).map(|r| r.text().chars().count()).unwrap_or(0) as f32 * cw;
                if in_block && state != BlockState::Prompt {
                    let bs = ch.max(18.0);
                    let tools: Vec<(Icon, &str)> = vec![
                        (Icon::Copy, "copy command"),
                        (Icon::File, "copy output"),
                        (Icon::Rerun, "run again"),
                        (Icon::Terminal, "edit in composer"),
                        (if b.folded { Icon::Unfold } else { Icon::Fold }, if b.folded { "unfold output" } else { "fold output" }),
                        (Icon::Open, "open output in editor"),
                    ];
                    let mut x = inner.right() - tools.len() as f32 * (bs + 2.0);
                    for (k, (icon, tip)) in tools.into_iter().enumerate() {
                        let r = Rect::from_min_size(pos2(x, yp + (ch - bs) / 2.0), vec2(bs, bs));
                        x += bs + 2.0;
                        if icons::overlay_button(ui, r, Id::new(("tool", id, b.id, k)), icon, tip).clicked() {
                            match k {
                                0 => acts.push(Action::Copy(b.command.clone())),
                                1 => acts.push(Action::Copy(t.block_output(b))),
                                2 => acts.push(Action::Run(id, b.command.clone())),
                                3 => acts.push(Action::ComposerOpen(id, b.command.clone())),
                                4 => acts.push(Action::ToggleFold(id, b.id)),
                                _ => acts.push(Action::OpenScratch(format!("output: {}", b.command), t.block_output(b))),
                            }
                        }
                    }
                } else if state != BlockState::Prompt {
                    let dur = b.elapsed().map(fmt_dur).unwrap_or_default();
                    let (icon, col, label) = match (state, b.exit) {
                        (BlockState::Running, _) => (Icon::Dot, theme::accent(), dur),
                        (_, Some(0)) => (Icon::Check, theme::success(), dur),
                        (_, Some(code)) => (Icon::Cross, theme::error(), format!("exit {code} · {dur}")),
                        _ => (Icon::Dot, theme::text_faint(), dur),
                    };
                    let f = FontId::proportional(11.5);
                    let g = painter.layout_no_wrap(label, f, col);
                    let w = g.size().x + ch + 8.0;
                    let r = Rect::from_min_size(pos2(inner.right() - w, yp + 2.0), vec2(w, ch - 4.0));
                    if inner.left() + text_len + 12.0 < r.left() {
                        painter.rect_filled(r, CornerRadius::same(4), theme::tint(col, 22));
                        let ir = Rect::from_min_size(pos2(r.left() + 3.0, r.top() + (r.height() - ch + 6.0) / 2.0), vec2(ch - 6.0, ch - 6.0));
                        let ic = if state == BlockState::Running {
                            let phase = (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
                            theme::tint(col, (120.0 + 135.0 * phase) as u8)
                        } else {
                            col
                        };
                        icons::paint(&painter, ir, icon, ic);
                        painter.galley(pos2(r.left() + ch - 1.0, r.center().y - g.size().y / 2.0), g, col);
                    }
                }
            }
            // Inline widget slots.
            for (wi, w) in p.widgets.iter().enumerate() {
                // Cover `rows` logical lines: soft-wrapped continuations count too.
                let mut span = 0u64;
                let mut logical = 0usize;
                while logical < w.rows && span < 4000 {
                    let wrapped = t.line(w.anchor + span).map(|r| r.wrapped).unwrap_or(false);
                    span += 1;
                    if !wrapped {
                        logical += 1;
                    }
                }
                let wend = w.anchor + span;
                if wend <= first_abs || w.anchor > last_abs {
                    continue;
                }
                let folded = window.iter().any(|v| matches!(v, VRow::Fold(_, n, s) if *s < wend && s + n > w.anchor));
                if folded {
                    continue;
                }
                let y = match vis_index(w.anchor) {
                    Some(i) => row_y(i),
                    None => row_y(0) - (first_abs - w.anchor) as f32 * ch,
                };
                widget_slots.push((wi, Rect::from_min_size(pos2(inner.left() - 2.0, y), vec2(inner.width() + 2.0, span as f32 * ch))));
            }
        }

        // Cursor + IME.
        // What is drawn as the cursor this frame (and the character under
        // it), where, and what to remember for the frames after.
        let mut cursor_now: Option<(Rect, Option<char>)> = None;
        let mut drawn: Option<Rect> = None;
        // Scrolled back, the cursor is drawn as long as its line is in view
        // (`ci` is None when it is not), moving with the text as in other
        // terminals. It used to go at the first line scrolled: near the
        // end a turn of the wheel switched it off and on.
        let show_cursor = t.cursor_visible;
        // Where a cell of the text is on screen now, if it is.
        let place = |alt: bool, line: u64, col: usize, wide: bool| -> Option<Rect> {
            let i = if alt { Some(line as usize) } else { vis_index(line) }?;
            Some(Rect::from_min_size(pos2(inner.left() + col as f32 * cw, row_y(i)), vec2(if wide { 2.0 * cw } else { cw }, ch)))
        };
        // The cell at a line and column: wide (two cells), and the
        // character in it (drawn inverted by a block cursor).
        let cell_at = |alt: bool, line: u64, col: usize| {
            let row = if alt { t.screen.get(line as usize) } else { t.line(line) };
            let c = row.and_then(|r| r.cells.get(col));
            (c.is_some_and(|c| c.wide == WIDE_LEAD), c.map(|c| c.ch).filter(|c| *c != ' '))
        };
        let here_line = if t.alt_active { t.cy as u64 } else { t.cursor_abs() };
        let (here_wide, here_under) = cell_at(t.alt_active, here_line, t.cx);
        let cursor_rect = place(t.alt_active, here_line, t.cx, here_wide);
        // A program hides the cursor while it redraws and shows it again
        // when done (through Windows' pseudo console every update comes
        // that way). A frame drawn in between had no cursor: it flickered
        // at the rate the program redrew, a spinner's say. A hidden cursor
        // stays where it was shown for a moment (one still hidden after
        // that goes). Where is a line and column of the text: scrolled
        // since, it is found again and moves with the text (it was a place
        // on screen, dropped at every turn of the wheel, and not looked at
        // when the line the program was drawing on was out of view). The
        // moment is when the program hid it, taken as the output was read
        // (it was the last frame drawn with the cursor shown: on a busy PC
        // a show and the next hide could come between two frames, and the
        // hold had run out).
        let held = t.hidden_from.filter(|h| !show_cursor && h.alt == t.alt_active && h.at.elapsed() < CURSOR_HOLD);
        let was_held = held.is_some();
        if show_cursor {
            cursor_now = cursor_rect.map(|r| (r, here_under));
        } else if let Some(h) = held {
            let (wide, under) = cell_at(h.alt, h.line, h.col);
            cursor_now = place(h.alt, h.line, h.col, wide).map(|r| (r, under));
            ui.ctx().request_repaint_after(CURSOR_HOLD.saturating_sub(h.at.elapsed()) + Duration::from_millis(5));
        }
        {
            if let (Some((r, under)), true) = (cursor_now, p.composer.is_none()) {
                drawn = Some(r);
                if focused {
                    match self.settings.cursor_style {
                        // Thin bar / underline: the character stays fully visible.
                        CursorStyle::Bar => {
                            let bar = Rect::from_min_size(pos2(r.left(), r.top() + 1.0), vec2(2.0, ch - 2.0));
                            painter.rect_filled(bar, CornerRadius::same(1), theme::cursor());
                        }
                        CursorStyle::Underline => {
                            let line = Rect::from_min_size(pos2(r.left(), r.bottom() - 2.5), vec2(r.width(), 2.0));
                            painter.rect_filled(line, CornerRadius::ZERO, theme::cursor());
                        }
                        CursorStyle::Block => {
                            painter.rect_filled(r, CornerRadius::same(1), theme::cursor());
                            if let Some(c) = under {
                                let g = painter.layout_no_wrap(c.to_string(), font.clone(), theme::bg_term());
                                painter.galley(pos2(r.left() + (r.width() - g.size().x) / 2.0, r.top() + (ch - g.size().y) / 2.0), g, theme::bg_term());
                            }
                        }
                    }
                } else {
                    painter.rect_stroke(r.shrink(0.5), CornerRadius::same(1), Stroke::new(1.0_f32, theme::text_dim()), StrokeKind::Inside);
                }
            }
        }
        if let Some(r) = cursor_rect {
            if focused && !p.preedit.is_empty() {
                let g = painter.layout_no_wrap(p.preedit.clone(), font.clone(), theme::text());
                let pr = Rect::from_min_size(r.min, vec2(g.size().x.max(cw), ch));
                painter.rect_filled(pr, CornerRadius::ZERO, theme::bg_raised());
                painter.galley(pr.min, g, theme::text());
                painter.hline(pr.x_range(), pr.bottom() - 1.0, Stroke::new(1.0_f32, theme::accent()));
            }
        }

        // Scroll position indicator.
        if !t.alt_active && p.scroll > 0 && !all.is_empty() {
            let n = all.len() as f32;
            let h = (inner.height() * rows as f32 / n).max(16.0);
            let frac = top as f32 / (n - rows as f32).max(1.0);
            let y = inner.top() + (inner.height() - h) * frac;
            painter.rect_filled(Rect::from_min_size(pos2(body.right() - 5.0, y), vec2(3.0, h)), CornerRadius::same(2), theme::tint(theme::text_dim(), 120));
            let label = format!("{} lines below — scroll or type to return", p.scroll);
            let g = painter.layout_no_wrap(label, FontId::proportional(11.5), theme::text());
            let r = Rect::from_min_size(pos2(inner.right() - g.size().x - 16.0, inner.bottom() - g.size().y - 10.0), g.size() + vec2(12.0, 6.0));
            painter.rect_filled(r, CornerRadius::same(5), theme::bg_raised());
            painter.galley(r.min + vec2(6.0, 3.0), g, theme::text());
        }

        let painter = whole;
        // Exited banner.
        if p.exited {
            let msg = match &p.spawn_error {
                Some(e) => format!("failed to start: {e} — Enter: retry · Ctrl+Shift+W: close"),
                None => "process exited — Enter: restart · Ctrl+Shift+W: close".to_string(),
            };
            let r = banner;
            painter.rect_filled(r, CornerRadius::same(5), theme::tint(theme::warn(), 30));
            painter.text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, msg, FontId::proportional(12.5), theme::warn());
        }

        // Header (split layouts) and focus ring.
        if multi {
            let hr = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + header_h));
            painter.rect_filled(hr, CornerRadius::ZERO, theme::bg_base());
            if active {
                painter.hline(hr.x_range(), hr.bottom() - 1.0, Stroke::new(2.0_f32, theme::accent()));
            }
            icons::paint(&painter, Rect::from_min_size(hr.min + vec2(8.0, 5.0), vec2(14.0, 14.0)), Icon::Terminal, if active { theme::accent() } else { theme::text_faint() });
            let title = if t.title.is_empty() { p.title() } else { format!("{} — {}", p.title(), t.title) };
            // [zoom]   gap   [close]: window-style icons, and close turns red on
            // hover, so restoring a zoomed pane is never confused with closing it.
            let bx = hr.right() - 26.0;
            let zx = bx - 34.0;
            let title_clip = Rect::from_min_max(hr.min, pos2(zx - 6.0, hr.bottom()));
            // The header moves the pane: drag it onto another pane of the tab.
            let grab = ui.interact(title_clip, Id::new(("pane-grab", id)), Sense::click_and_drag());
            if grab.hovered() && self.pane_drag.is_none() {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            }
            if grab.drag_started() {
                self.pane_drag = Some(id);
            }
            if grab.clicked() && !active {
                acts.push(Action::Focus(id));
            }
            grab.on_hover_text("drag onto another pane to move it there (the middle swaps the two)");
            crate::widgets::text_fit(&painter.with_clip_rect(title_clip), pos2(hr.left() + 28.0, hr.center().y), &title, FontId::proportional(12.0), if active { theme::text() } else { theme::text_dim() }, title_clip.right() - hr.left() - 34.0);
            if icons::overlay_button_styled(ui, Rect::from_min_size(pos2(bx, hr.top() + 2.0), vec2(20.0, 20.0)), Id::new(("close", id)), Icon::Cross, "close pane (Ctrl+Shift+W)", true).clicked() {
                acts.push(Action::ClosePane(id));
            }
            let zoomed = self.tabs.get(self.active_tab).map(|t| t.zoomed).unwrap_or(false);
            let (zi, ztip) = if zoomed { (Icon::Restore, "restore split layout (Ctrl+Shift+Z)") } else { (Icon::Maximize, "zoom this pane (Ctrl+Shift+Z)") };
            if icons::overlay_button(ui, Rect::from_min_size(pos2(zx, hr.top() + 2.0), vec2(20.0, 20.0)), Id::new(("zoom", id)), zi, ztip).clicked() {
                acts.push(Action::Focus(id));
                acts.push(Action::ToggleZoom);
            }
        }
        if let Some(at) = p.bell_at {
            let a = 1.0 - at.elapsed().as_secs_f32() / 0.6;
            if a > 0.0 {
                painter.rect_stroke(body.shrink(1.0), CornerRadius::same(2), Stroke::new(2.0_f32, theme::tint(theme::warn(), (a * 200.0) as u8)), StrokeKind::Inside);
                ui.ctx().request_repaint();
            }
        }

        // Context menu. While an app takes the mouse (nvim, htop…) a right-click
        // is the app's; Shift+right-click opens this menu instead, the same
        // way Shift+drag selects here instead of in the app.
        let has_sel = p.sel.is_some();
        let sel_text = p.sel.map(|s| selection_text(t, s));
        let cwd = t.cwd.clone();
        let app_mouse = t.mouse_mode != 0 && (t.alt_active || p.scroll == 0);
        let menu_id = term_id.with("weft-menu");
        let menu_open = ui.data(|d| d.get_temp::<bool>(menu_id).unwrap_or(false));
        let ours = !app_mouse || mods.shift || menu_open;
        let shown = ours.then(|| resp.context_menu(|ui| {
            ui.set_min_width(200.0);
            if ui.add_enabled(has_sel, egui::Button::new("Copy selection")).clicked() {
                if let Some(s) = sel_text.clone() {
                    acts.push(Action::Copy(s));
                }
                ui.close_menu();
            }
            if ui.button("Composer  (Ctrl+Space)").clicked() {
                acts.push(Action::ComposerOpen(id, String::new()));
                ui.close_menu();
            }
            if let Some(cwd) = cwd.clone() {
                if ui.button("Show this directory in Files").clicked() {
                    acts.push(Action::SetRoot(host.clone(), cwd));
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui.button("Split right  (Ctrl+Shift+D)").clicked() {
                acts.push(Action::Focus(id));
                acts.push(Action::Split(true));
                ui.close_menu();
            }
            if ui.button("Split down  (Ctrl+Shift+E)").clicked() {
                acts.push(Action::Focus(id));
                acts.push(Action::Split(false));
                ui.close_menu();
            }
            if multi && ui.button("Zoom / restore pane  (Ctrl+Shift+Z)").clicked() {
                acts.push(Action::Focus(id));
                acts.push(Action::ToggleZoom);
                ui.close_menu();
            }
            if ui.button("Close pane  (Ctrl+Shift+W)").clicked() {
                acts.push(Action::ClosePane(id));
                ui.close_menu();
            }
            if app_mouse {
                ui.separator();
                ui.label(egui::RichText::new("The app here uses the mouse:\nShift+drag selects text for weft,\nShift+right-click opens this menu.").size(11.0).color(theme::text_faint()));
            }
        }));
        let open_now = shown.flatten().is_some();
        if open_now != menu_open {
            ui.data_mut(|d| d.insert_temp(menu_id, open_now));
        }

        // IME position. Every focused frame must report one: a frame without it
        // makes egui disable the IME, and Windows then commits the syllable being
        // composed ("돼" typed as "ㄷ" + "ㅙ"). While composing, the position stays
        // put even if the app redraws and moves the cursor around.
        {
            let pm = self.panes.get_mut(&id).unwrap();
            pm.cursor_drawn = drawn;
            if let Some(tr) = pm.trace.as_mut() {
                let cell = drawn.map(|r| (((r.top() - inner.top()) / ch).round().max(0.0) as usize, ((r.left() - inner.left()) / cw).round().max(0.0) as usize));
                tr.cursor(cell, was_held);
            }
        }
        if focused {
            let pm = self.panes.get_mut(&id).unwrap();
            let fallback = Rect::from_min_size(inner.min, vec2(cw, ch));
            let r = ime_anchor(!pm.preedit.is_empty(), pm.ime_rect, cursor_rect, fallback);
            pm.ime_rect = Some(r);
            ui.ctx().output_mut(|o| o.ime = Some(egui::output::IMEOutput { rect: r, cursor_rect: r }));
        }

        // 5. mutable overlays: widgets, composer, search
        for (wi, r) in widget_slots {
            let clip = r.intersect(inner.expand2(vec2(2.0, 0.0)));
            if clip.height() <= 0.0 {
                continue;
            }
            let at_prompt = self.panes[&id].term.at_prompt();
            let p = self.panes.get_mut(&id).unwrap();
            let w = &mut p.widgets[wi];
            let mut child = ui.new_child(UiBuilder::new().max_rect(r).id_salt(("widget", id, wi)));
            child.set_clip_rect(clip);
            crate::widgets::ui_widget(&mut child, w, r, cw, ch, &font, id, &host, at_prompt, &mut acts);
        }
        self.ui_composer(ui, id, inner, cursor_rect, ch, &font, &mut acts);
        self.ui_search(ui, id, inner, &all, rows, &mut acts);
        self.actions.extend(acts);
    }

    // The pane's layout as the caller has just worked it out.
    #[allow(clippy::too_many_arguments)]
    fn ui_composer(&mut self, ui: &mut Ui, id: usize, inner: Rect, cursor: Option<Rect>, ch: f32, font: &FontId, acts: &mut Vec<Action>) {
        let p = self.panes.get_mut(&id).unwrap();
        let Some(c) = p.composer.as_mut() else { return };
        let hist = self.history.get(&p.host);
        let cid = Id::new(("composer", id));
        let lines = (c.text.matches('\n').count() + 1).clamp(1, 10);
        let cand_rows = if c.cands.len() > 1 { 1 } else { 0 };
        let h = lines as f32 * ch + 16.0 + 20.0 + cand_rows as f32 * 22.0;
        let top = cursor.map(|r| r.top()).unwrap_or(inner.bottom() - h).min(inner.bottom() - h).max(inner.top());
        let rect = Rect::from_min_size(pos2(inner.left() - 4.0, top), vec2(inner.width() + 4.0, h));
        let painter = ui.painter();
        painter.rect_filled(rect.expand(2.0), CornerRadius::same(7), theme::tint(theme::bg_term(), 230));
        painter.rect_filled(rect, CornerRadius::same(6), theme::bg_raised());
        painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0_f32, theme::tint(theme::accent(), 160)), StrokeKind::Inside);

        let has_focus = ui.memory(|m| m.has_focus(cid));
        let suggestion = if c.text.is_empty() || c.text.contains('\n') {
            None
        } else {
            hist.and_then(|h| h.iter().rev().find(|e| e.starts_with(&c.text) && e.len() > c.text.len()).cloned())
        };
        let mut submit = false;
        let mut close = false;
        // egui takes the focus away on Esc before the frame starts: with it
        // gone to nobody, the key is still this field's.
        if (has_focus || ui.memory(|m| m.focused().is_none())) && take_key(ui, Key::Escape, |_| true) {
            close = true;
        }
        if has_focus {
            if take_key(ui, Key::Enter, |m| !m.shift) {
                submit = true;
            }
            if take_key(ui, Key::Tab, |m| !m.shift) {
                acts.push(Action::Complete(id));
            }
            let single = !c.text.contains('\n');
            if single {
                let mut step: i32 = 0;
                if take_key(ui, Key::ArrowUp, |_| true) {
                    step = -1;
                }
                if take_key(ui, Key::ArrowDown, |_| true) {
                    step = 1;
                }
                if step != 0 {
                    if let Some(h) = hist {
                        if c.hist_prefix.is_none() {
                            c.hist_prefix = Some(c.text.clone());
                            c.hist_pos = h.len();
                        }
                        let pre = c.hist_prefix.clone().unwrap_or_default();
                        let mut i = c.hist_pos as i64;
                        loop {
                            i += step as i64;
                            if i < 0 {
                                break;
                            }
                            if i as usize >= h.len() {
                                c.hist_pos = h.len();
                                c.text = pre.clone();
                                break;
                            }
                            if h[i as usize].starts_with(&pre) {
                                c.hist_pos = i as usize;
                                c.text = h[i as usize].clone();
                                break;
                            }
                        }
                        let cc = egui::text::CCursor::new(c.text.chars().count());
                        if let Some(mut st) = TextEdit::load_state(ui.ctx(), cid) {
                            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(cc)));
                            st.store(ui.ctx(), cid);
                        }
                    }
                }
            }
            if let Some(s) = suggestion.as_ref() {
                let at_end = TextEdit::load_state(ui.ctx(), cid)
                    .and_then(|st| st.cursor.char_range())
                    .map(|r| r.primary.index == c.text.chars().count())
                    .unwrap_or(true);
                if at_end && take_key(ui, Key::ArrowRight, |m| m.is_none()) {
                    c.text = s.clone();
                    let cc = egui::text::CCursor::new(c.text.chars().count());
                    if let Some(mut st) = TextEdit::load_state(ui.ctx(), cid) {
                        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(cc)));
                        st.store(ui.ctx(), cid);
                    }
                }
            }
        }

        let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 7.0))).id_salt(("composer-ui", id)));
        let edit = TextEdit::multiline(&mut c.text)
            .id(cid)
            .font(font.clone())
            .frame(false)
            .desired_width(f32::INFINITY)
            .desired_rows(lines)
            .lock_focus(true)
            .text_color(theme::text())
            .hint_text(egui::RichText::new("type a command — runs in the shell when you press Enter").color(theme::text_faint()));
        let r = child.add(edit);
        if c.focus {
            r.request_focus();
            c.focus = false;
        }
        if r.changed() {
            c.cands.clear();
            c.hist_prefix = None;
        }
        if c.cands.len() > 1 {
            child.horizontal_wrapped(|ui| {
                for (k, cand) in c.cands.iter().take(16).enumerate() {
                    let col = if k == c.cand_idx { theme::accent() } else { theme::text_dim() };
                    ui.label(egui::RichText::new(cand).monospace().color(col));
                }
                if c.cands.len() > 16 {
                    ui.label(egui::RichText::new(format!("+{} more", c.cands.len() - 16)).color(theme::text_faint()));
                }
            });
        }
        child.horizontal(|ui| {
            let small = |s: &str| egui::RichText::new(s).size(11.0).color(theme::text_faint());
            ui.label(small("Enter run · Shift+Enter newline · Tab complete · ↑↓ history · Esc close"));
            if c.pending {
                ui.spinner();
            }
            if let Some(s) = suggestion.as_ref() {
                ui.label(egui::RichText::new(format!("→ {s}")).size(11.0).color(theme::text_dim()));
            }
        });
        if submit {
            let text = std::mem::take(&mut c.text);
            p.composer = None;
            if !text.trim().is_empty() {
                acts.push(Action::Paste(id, text));
                acts.push(Action::Send(id, b"\r".to_vec()));
            }
            self.focus_term = true;
        } else if close {
            p.composer = None;
            self.focus_term = true;
        }
    }

    fn ui_search(&mut self, ui: &mut Ui, id: usize, inner: Rect, all: &[VRow], rows: usize, _acts: &mut Vec<Action>) {
        let p = self.panes.get_mut(&id).unwrap();
        let Some(s) = p.search.as_mut() else { return };
        if s.seen_generation != p.term.generation {
            s.hits = p.term.search(&s.query, s.icase);
            s.seen_generation = p.term.generation;
            s.cur = s.cur.min(s.hits.len().saturating_sub(1));
        }
        let rect = Rect::from_min_size(pos2(inner.right() - 360.0, inner.top() + 2.0), vec2(356.0, 32.0));
        ui.painter().rect_filled(rect, CornerRadius::same(6), theme::bg_raised());
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0_f32, theme::border()), StrokeKind::Inside);
        let sid = Id::new(("search", id));
        let mut jump: Option<i32> = None;
        let mut close = false;
        if ui.memory(|m| m.has_focus(sid)) {
            if take_key(ui, Key::Enter, |m| !m.shift) {
                jump = Some(1);
            }
            if take_key(ui, Key::Enter, |m| m.shift) {
                jump = Some(-1);
            }
        }
        // Esc already took the focus (egui, before the frame): with nobody
        // else holding it, the key is still ours.
        if (ui.memory(|m| m.has_focus(sid)) || ui.memory(|m| m.focused().is_none())) && take_key(ui, Key::Escape, |_| true) {
            close = true;
        }
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 5.0))).id_salt(("search-ui", id)));
        child.horizontal(|ui| {
            icons::paint(ui.painter(), Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(16.0, 16.0)), Icon::Search, theme::text_dim());
            ui.add_space(20.0);
            let r = ui.add(TextEdit::singleline(&mut s.query).id(sid).desired_width(170.0).hint_text("find in scrollback"));
            if s.focus {
                r.request_focus();
                s.focus = false;
            }
            if r.changed() {
                // The new hits at once, the last of them the current one:
                // Enter then goes to the first. (Counted from the hits of
                // the text before, the first Enter after the first text
                // typed went to the second hit.)
                s.hits = p.term.search(&s.query, s.icase);
                s.seen_generation = p.term.generation;
                s.cur = s.hits.len().saturating_sub(1);
            }
            let label = if s.hits.is_empty() { "0/0".to_string() } else { format!("{}/{}", s.cur + 1, s.hits.len()) };
            ui.label(egui::RichText::new(label).size(11.5).color(theme::text_dim()));
            if ui.selectable_label(s.icase, "Aa").on_hover_text("ignore case").clicked() {
                s.icase = !s.icase;
                s.seen_generation = u64::MAX;
            }
            if icons::button(ui, Icon::ChevronDown, "next (Enter)").clicked() {
                jump = Some(1);
            }
            if icons::button(ui, Icon::Cross, "close (Esc)").clicked() {
                close = true;
            }
        });
        if let (Some(d), false) = (jump, s.hits.is_empty()) {
            let n = s.hits.len() as i32;
            s.cur = ((s.cur as i32 + d).rem_euclid(n)) as usize;
            let abs = s.hits[s.cur].0;
            if let Some(idx) = all.iter().position(|v| *v == VRow::Line(abs)) {
                p.scroll = all.len().saturating_sub(idx + rows / 2 + 1);
            }
        }
        if close {
            p.search = None;
            self.focus_term = true;
        }
    }

    /// Scroll the active pane to the previous/next command block.
    pub fn jump_block(&mut self, dir: i32) {
        let Some(id) = self.active_pane_id() else { return };
        let p = self.panes.get_mut(&id).unwrap();
        if p.term.alt_active {
            return;
        }
        let all = visual_rows(&p.term);
        let rows = p.term.rows;
        let n = all.len();
        let top_idx = n.saturating_sub(p.scroll + rows);
        let idx_of = |abs: u64| all.iter().position(|v| *v == VRow::Line(abs));
        let starts: Vec<usize> = p.term.blocks.iter().filter(|b| b.state() != BlockState::Prompt).filter_map(|b| idx_of(b.prompt_line)).collect();
        let target = if dir < 0 { starts.iter().rev().find(|&&i| i < top_idx) } else { starts.iter().find(|&&i| i > top_idx) };
        match target {
            Some(&i) => p.scroll = n.saturating_sub(i + rows).min(n.saturating_sub(rows)),
            None if dir > 0 => p.scroll = 0,
            None => {}
        }
        if let Some(b) = p.term.blocks.iter().find(|b| idx_of(b.prompt_line) == target.copied()) {
            p.flash_block = Some((b.id, std::time::Instant::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pane dragged by its header in the real App, frame by frame: the
    /// pointer events egui gets from the mouse.
    #[test]
    fn dragging_a_pane_header_moves_the_pane() {
        let dir = std::env::temp_dir().join(format!("weft-pane-drag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        let mut app = App::new_in(ctx.clone(), crate::host::DataDir::at(&dir));
        app.split(true);
        app.split(false);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput { events, screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 700.0))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.ui_panes(ui));
            });
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        let mut ids = Vec::new();
        app.tabs[0].root.leaves(&mut ids);
        let [first, _, third] = ids[..] else { panic!("{ids:?}") };
        let rect = |app: &App, id| app.pane_rects.iter().find(|p| p.0 == id).unwrap().1;
        let (from, to) = (rect(&app, third).left_top() + vec2(60.0, 12.0), rect(&app, first).left_center() + vec2(30.0, 0.0));
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(from)]);
        frame(&mut app, vec![button(from, true)]);
        for k in 1..=10 {
            frame(&mut app, vec![egui::Event::PointerMoved(from + (to - from) * k as f32 / 10.0)]);
        }
        assert_eq!(app.pane_drag, Some(third), "the header drag started");
        frame(&mut app, vec![button(to, false)]);
        frame(&mut app, vec![]);
        assert_eq!(app.pane_drag, None);
        // The third pane is now left of the first, at the same height.
        let (a, b) = (rect(&app, third), rect(&app, first));
        assert!(a.right() <= b.left() + 1.0 && (a.top() - b.top()).abs() < 1.0, "third {a:?} first {b:?}");
        // A second editor, dragged by its grip to the left of the terminals.
        app.editor.visible = true;
        let first_editor = PanelId::first(Region::Editor);
        app.split_panel(first_editor, crate::app::Dock::Bottom);
        let second = PanelId { region: Region::Editor, n: 1 };
        let layout_frame = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput { events, screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 700.0))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.ui_layout(ui));
                app.ui_dock_drop(ctx);
            });
        };
        layout_frame(&mut app, vec![]);
        layout_frame(&mut app, vec![]);
        let panel = |app: &App, p: PanelId| app.panel_rects[&p];
        assert!(panel(&app, second).top() > panel(&app, first_editor).top(), "the new editor is below the first");
        // Its grip: the first thing in the panel, after the frame's margin.
        let grip = panel(&app, second).left_top() + vec2(8.0 + 7.0, 6.0 + 10.0);
        let to = panel(&app, PanelId::TERMS).left_center() + vec2(20.0, 0.0);
        layout_frame(&mut app, vec![egui::Event::PointerMoved(grip)]);
        layout_frame(&mut app, vec![button(grip, true)]);
        for k in 1..=10 {
            layout_frame(&mut app, vec![egui::Event::PointerMoved(grip + (to - grip) * k as f32 / 10.0)]);
        }
        assert_eq!(app.dock_drag, Some(second), "the grip drag started");
        layout_frame(&mut app, vec![button(to, false)]);
        layout_frame(&mut app, vec![]);
        let (ed, terms) = (panel(&app, second), panel(&app, PanelId::TERMS));
        assert!(ed.right() <= terms.left() + 1.0, "second editor {ed:?} left of the terminals {terms:?}");
        assert!(app.settings.layout.contains(first_editor), "the first editor stays");
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn box_drawing_characters_have_their_arms() {
        // Left, up, right, down.
        for (c, arms) in [('\u{2500}', [1, 0, 1, 0]), ('\u{2502}', [0, 1, 0, 1]), ('\u{250c}', [0, 0, 1, 1]), ('\u{2510}', [1, 0, 0, 1]), ('\u{2514}', [0, 1, 1, 0]), ('\u{2518}', [1, 1, 0, 0]), ('\u{251c}', [0, 1, 1, 1]), ('\u{2524}', [1, 1, 0, 1]), ('\u{252c}', [1, 0, 1, 1]), ('\u{2534}', [1, 1, 1, 0]), ('\u{253c}', [1, 1, 1, 1]), ('\u{2501}', [2, 0, 2, 0]), ('\u{254b}', [2, 2, 2, 2]), ('\u{2550}', [3, 0, 3, 0]), ('\u{2551}', [0, 3, 0, 3]), ('\u{2554}', [0, 0, 3, 3]), ('\u{255d}', [3, 3, 0, 0]), ('\u{256c}', [3, 3, 3, 3]), ('\u{2574}', [1, 0, 0, 0]), ('\u{257f}', [0, 2, 0, 1])] {
            assert_eq!(box_arms(c), Some((arms, false)), "{c}");
        }
        for (c, arms) in [('\u{256d}', [0, 0, 1, 1]), ('\u{256e}', [1, 0, 0, 1]), ('\u{256f}', [1, 1, 0, 0]), ('\u{2570}', [0, 1, 1, 0])] {
            assert_eq!(box_arms(c), Some((arms, true)), "{c}: a rounded corner");
        }
        // Dashed and diagonal ones stay the font's; other characters aren't boxes.
        for c in ['\u{2504}', '\u{254c}', '\u{2571}', '\u{2573}', 'a', '\u{2588}', '\u{24ff}', '\u{2580}'] {
            assert_eq!(box_arms(c), None, "{c}");
        }
        // Every character of the block has an entry.
        assert!((0x2500..=0x257F_u32).all(|cp| char::from_u32(cp).is_some_and(|c| box_arms(c).is_some() || matches!(cp, 0x2504..=0x250B | 0x254C..=0x254F | 0x2571..=0x2573))));
    }

    #[test]
    fn blocks_fill_their_cells() {
        // Which quarters of its cell a block covers.
        let covers = |c: char, x: f32, y: f32| {
            let (rects, n, _) = block_rects(c).unwrap();
            rects[..n].iter().any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])
        };
        let (ul, ur, ll, lr) = ((0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75));
        for (c, want) in [
            ('\u{2588}', [true, true, true, true]),
            ('\u{2580}', [true, true, false, false]),
            ('\u{2584}', [false, false, true, true]),
            ('\u{258c}', [true, false, true, false]),
            ('\u{2590}', [false, true, false, true]),
            ('\u{259b}', [true, true, true, false]),
            ('\u{259c}', [true, true, false, true]),
            ('\u{2598}', [true, false, false, false]),
            ('\u{259d}', [false, true, false, false]),
            ('\u{2596}', [false, false, true, false]),
            ('\u{2597}', [false, false, false, true]),
            ('\u{259a}', [true, false, false, true]),
            ('\u{259e}', [false, true, true, false]),
            ('\u{2599}', [true, false, true, true]),
            ('\u{259f}', [false, true, true, true]),
        ] {
            let got = [ul, ur, ll, lr].map(|(x, y)| covers(c, x, y));
            assert_eq!(got, want, "{c}");
        }
        // Eighths: ▁ is the bottom eighth, ▏ the left one; shades are lighter.
        assert!(covers('\u{2581}', 0.5, 0.95) && !covers('\u{2581}', 0.5, 0.8));
        assert!(covers('\u{258f}', 0.05, 0.5) && !covers('\u{258f}', 0.2, 0.5));
        assert!(covers('\u{2589}', 0.8, 0.5) && !covers('\u{2589}', 0.9, 0.5));
        assert_eq!(block_rects('\u{2592}').unwrap().2, 0.5);
        assert!(block_rects('a').is_none() && block_rects('\u{2500}').is_none());
    }

    fn painted(row: &Row, cw: f32, ch: f32, font: &FontId) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx, theme::DEFAULT_FONT, theme::DEFAULT_ICON_FONT);
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            paint_row(&p, row, 10.0, 20.0, cw, ch, font, row.cells.len());
        });
        out.shapes.into_iter().map(|c| c.shape).collect()
    }

    /// A full block on a black background is one rect over the whole cell,
    /// in the foreground colour, and no glyph: the background can't show
    /// through whatever the font (Claude Code's logo had a black band).
    #[test]
    fn a_full_block_hides_its_background() {
        let orange = Color::Rgb(215, 119, 87);
        let cell = |ch| term::Cell { ch, attr: term::Attr { fg: orange, bg: Color::Rgb(0, 0, 0), flags: 0 }, wide: 0 };
        let row = Row { cells: vec![cell('\u{2588}'), cell('\u{259b}')], wrapped: false };
        let (cw, ch) = (9.0, 19.0);
        let fg = Color32::from_rgb(215, 119, 87);
        let mut rects = Vec::new();
        for s in painted(&row, cw, ch, &theme::term_font(14.0)) {
            match s {
                egui::Shape::Rect(r) if r.fill == fg => rects.push(r.rect),
                egui::Shape::Text(t) => panic!("a block drawn as a glyph: {:?}", t.galley.text()),
                _ => {}
            }
        }
        assert_eq!(rects[0], Rect::from_min_size(pos2(10.0, 20.0), vec2(cw, ch)), "the whole first cell");
        // ▛: the lower right quarter of the second cell stays the background.
        let eye = pos2(10.0 + cw * 1.75, 20.0 + ch * 0.75);
        assert!(!rects.iter().any(|r| r.contains(eye)), "the eye");
        assert_eq!(rects.len(), 4);
    }

    /// Text stays on the grid whatever the font: a fixed-width one is laid
    /// out as a run, any other letter by letter, each inside its own cell
    /// (a proportional font drew "v2.1.312" columns away from its place).
    #[test]
    fn letters_keep_their_cells_in_any_font() {
        let cell = |ch| term::Cell { ch, attr: term::Attr::default(), wide: 0 };
        let row = Row { cells: "illi.,;:il".chars().map(cell).collect(), wrapped: false };
        let texts = |font: &FontId, cw: f32| -> Vec<(String, Rect)> {
            painted(&row, cw, 19.0, font)
                .into_iter()
                .filter_map(|s| match s {
                    egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2()))),
                    _ => None,
                })
                .collect()
        };
        let widths = |font: &FontId| {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx, theme::DEFAULT_FONT, theme::DEFAULT_ICON_FONT);
            let mut cw = 0.0;
            let _ = ctx.run(egui::RawInput::default(), |ctx| cw = ctx.fonts(|f| theme::cell_width(f, font)));
            cw
        };
        // Fixed width: one run.
        let mono = theme::term_font(14.0);
        let got = texts(&mono, widths(&mono));
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].0, "illi.,;:il");
        // Proportional: ten letters, the k-th within the k-th cell.
        let prop = FontId::proportional(14.0);
        let cw = widths(&prop);
        let got = texts(&prop, cw);
        assert_eq!(got.len(), 10, "{got:?}");
        for (k, (t, r)) in got.iter().enumerate() {
            let left = 10.0 + k as f32 * cw;
            assert!(r.left() >= left - 0.01 && r.right() <= left + cw + 0.01, "{t:?} (cell {k}) at {r:?}, its cell starts at {left}, {cw} wide");
        }
    }

    #[test]
    fn selection_modes() {
        let mut t = Term::new(30, 4);
        let mut parser = vte::Parser::new();
        t.feed(&mut parser, b"ls -l src/main.rs\r\nfoo(bar) baz\r\nabcdef\r\n123456");
        let sel = |a, b, mode| Selection { a, b, mode };
        // Characters, either drag direction.
        assert_eq!(selection_text(&t, sel((0, 3), (0, 4), SelMode::Char)), "-l");
        assert_eq!(selection_text(&t, sel((0, 4), (0, 3), SelMode::Char)), "-l");
        // Words: both ends widened; brackets split words.
        assert_eq!(selection_text(&t, sel((0, 8), (0, 8), SelMode::Word)), "src/main.rs");
        assert_eq!(selection_text(&t, sel((1, 5), (1, 10), SelMode::Word)), "bar) baz");
        // Lines: whole rows.
        assert_eq!(selection_text(&t, sel((1, 4), (2, 1), SelMode::Line)), "foo(bar) baz\nabcdef");
        // Block: the same columns on each row, from any corner.
        assert_eq!(selection_text(&t, sel((3, 3), (2, 1), SelMode::Block)), "bcd\n234");
        assert_eq!(sel_cols(&t, sel((3, 3), (2, 1), SelMode::Block), 2, 30), Some((1, 4)));
        assert_eq!(sel_cols(&t, sel((3, 3), (2, 1), SelMode::Block), 1, 30), None);
        assert_eq!(sel_cols(&t, sel((1, 4), (2, 1), SelMode::Line), 1, 30), Some((0, 30)));
    }

    #[test]
    fn keys() {
        assert_eq!(key_bytes(Key::C, Modifiers::CTRL, false), Some(vec![3]));
        assert_eq!(key_bytes(Key::ArrowUp, Modifiers::NONE, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes(Key::ArrowLeft, Modifiers::CTRL, false), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(key_bytes(Key::B, Modifiers::ALT, false), Some(b"\x1bb".to_vec()));
        assert_eq!(key_bytes(Key::A, Modifiers::NONE, false), None);
    }

    fn key(k: Key) -> Event {
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }
    fn pre(s: &str) -> Event {
        Event::Ime(ImeEvent::Preedit(s.into()))
    }
    fn commit(s: &str) -> Event {
        Event::Ime(ImeEvent::Commit(s.into()))
    }

    #[test]
    fn hangul_backspace_edits_the_syllable_not_the_app() {
        // "거" already committed, "야" composing; Backspace -> IME leaves "ㅇ".
        let mut preedit = "야".to_string();
        let out = typed_bytes(&[key(Key::Backspace), pre("ㅇ")], &mut preedit, Modifiers::NONE, false);
        assert!(out.is_empty(), "Backspace leaked to the app: {out:?}");
        assert_eq!(preedit, "ㅇ");
        // The last jamo deleted: preedit becomes empty in the same frame.
        let out = typed_bytes(&[pre(""), key(Key::Backspace)], &mut preedit, Modifiers::NONE, false);
        assert!(out.is_empty());
        // Nothing composing any more: Backspace goes to the app.
        let out = typed_bytes(&[key(Key::Backspace)], &mut preedit, Modifiers::NONE, false);
        assert_eq!(out, vec![0x7f]);
    }

    #[test]
    fn enter_during_composition_is_sent_once_after_the_syllable() {
        // Windows Korean IME: Enter while composing -> commit, then Enter again.
        let mut preedit = "야".to_string();
        let out = typed_bytes(&[key(Key::Enter), commit("야"), key(Key::Enter)], &mut preedit, Modifiers::NONE, false);
        assert_eq!(String::from_utf8(out).unwrap(), "야\r");
        // Same, with the re-delivered Enter in the next frame.
        let mut preedit = "야".to_string();
        let a = typed_bytes(&[key(Key::Enter), commit("야")], &mut preedit, Modifiers::NONE, false);
        let b = typed_bytes(&[key(Key::Enter)], &mut preedit, Modifiers::NONE, false);
        assert_eq!(String::from_utf8([a, b].concat()).unwrap(), "야\r");
    }

    #[test]
    fn ime_position_is_never_missing_and_frozen_while_composing() {
        let at = |x: f32| Rect::from_min_size(pos2(x, 0.0), vec2(8.0, 16.0));
        let fallback = at(0.0);
        // Cursor not found this frame: keep the last spot instead of reporting nothing.
        assert_eq!(ime_anchor(false, Some(at(40.0)), None, fallback), at(40.0));
        assert_eq!(ime_anchor(false, None, None, fallback), fallback);
        // Not composing: follow the cursor.
        assert_eq!(ime_anchor(false, Some(at(40.0)), Some(at(48.0)), fallback), at(48.0));
        // Composing: the app redrew and the cursor jumped; the IME stays put.
        assert_eq!(ime_anchor(true, Some(at(40.0)), Some(at(200.0)), fallback), at(40.0));
    }

    #[test]
    fn text_during_composition_follows_the_commit() {
        let mut preedit = "야".to_string();
        let out = typed_bytes(&[Event::Text(" ".into()), commit("야")], &mut preedit, Modifiers::NONE, false);
        assert_eq!(String::from_utf8(out).unwrap(), "야 ");
        let mut preedit = String::new();
        let out = typed_bytes(&[commit("거"), pre("ㅇ"), pre("야"), commit("야"), Event::Text("!".into())], &mut preedit, Modifiers::NONE, false);
        assert_eq!(String::from_utf8(out).unwrap(), "거야!");
    }

    #[test]
    fn tokens() {
        assert_eq!(classify_token("src/main.rs:12:3", Some("/p"), None), Some((false, "/p/src/main.rs".into(), 12)));
        assert_eq!(classify_token("https://x.y/a", None, None).map(|t| t.0), Some(true));
        assert_eq!(classify_token("hello", Some("/p"), None), None);
        assert_eq!(classify_token("~/a.txt", None, Some("/home/u")), Some((false, "/home/u/a.txt".into(), 1)));
    }

    #[test]
    fn zoom_shows_only_active_pane_full_size() {
        let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let root = Node::Split {
            vertical: true,
            ratio: 0.5,
            a: Box::new(Node::Leaf(1)),
            b: Box::new(Node::Split { vertical: false, ratio: 0.5, a: Box::new(Node::Leaf(2)), b: Box::new(Node::Leaf(3)) }),
        };
        let mut tab = Tab { root, active: 3, zoomed: false };
        let (mut panes, mut splits) = (Vec::new(), Vec::new());
        assert!(!tab_layout(&tab, full, &mut panes, &mut splits));
        assert_eq!(panes.len(), 3);
        assert_eq!(splits.len(), 2);

        tab.zoomed = true;
        let (mut panes, mut splits) = (Vec::new(), Vec::new());
        assert!(tab_layout(&tab, full, &mut panes, &mut splits));
        assert_eq!(panes, vec![(3, full)]);
        assert!(splits.is_empty());

        // A single-pane tab never counts as zoomed.
        let single = Tab { root: Node::Leaf(9), active: 9, zoomed: true };
        let (mut panes, mut splits) = (Vec::new(), Vec::new());
        assert!(!tab_layout(&single, full, &mut panes, &mut splits));
        assert_eq!(panes, vec![(9, full)]);
    }

    #[test]
    fn junction_where_three_panes_meet() {
        let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let layout = |root: Node| {
            let tab = Tab { root, active: 1, zoomed: false };
            let (mut panes, mut splits) = (Vec::new(), Vec::new());
            tab_layout(&tab, full, &mut panes, &mut splits);
            split_junctions(&splits)
        };
        // [1 | [2 / 3]]: right side split top/bottom -> one junction at (400, 300).
        let j = layout(Node::Split {
            vertical: true,
            ratio: 0.5,
            a: Box::new(Node::Leaf(1)),
            b: Box::new(Node::Split { vertical: false, ratio: 0.5, a: Box::new(Node::Leaf(2)), b: Box::new(Node::Leaf(3)) }),
        });
        assert_eq!(j.len(), 1);
        assert!((j[0].2.x - 400.0).abs() < 1.0 && (j[0].2.y - 300.0).abs() < 1.0, "{:?}", j[0].2);
        // [[1 / 2] | [3 / 4]]: four panes in a +, one junction with the
        // vertical splitter and both horizontal ones.
        let half = |a, b| Box::new(Node::Split { vertical: false, ratio: 0.5, a: Box::new(Node::Leaf(a)), b: Box::new(Node::Leaf(b)) });
        let j = layout(Node::Split { vertical: true, ratio: 0.5, a: half(1, 2), b: half(3, 4) });
        assert_eq!(j.len(), 1, "{j:?}");
        assert_eq!((j[0].0.len(), j[0].1.len()), (1, 2));
        // Not at the same height: two separate T junctions.
        let j = layout(Node::Split {
            vertical: true,
            ratio: 0.5,
            a: Box::new(Node::Split { vertical: false, ratio: 0.3, a: Box::new(Node::Leaf(1)), b: Box::new(Node::Leaf(2)) }),
            b: half(3, 4),
        });
        assert_eq!(j.len(), 2);
        // [[1 | 2] / 3]: top split left/right -> junction where the vertical one ends.
        let j = layout(Node::Split {
            vertical: false,
            ratio: 0.5,
            a: Box::new(Node::Split { vertical: true, ratio: 0.5, a: Box::new(Node::Leaf(1)), b: Box::new(Node::Leaf(2)) }),
            b: Box::new(Node::Leaf(3)),
        });
        assert_eq!(j.len(), 1);
        // Two side-by-side panes only: no junction.
        let j = layout(Node::Split { vertical: true, ratio: 0.5, a: Box::new(Node::Leaf(1)), b: Box::new(Node::Leaf(2)) });
        assert!(j.is_empty());
    }

    #[test]
    fn folds_collapse_rows() {
        let mut t = Term::new(20, 5);
        let mut p = vte::Parser::new();
        t.feed(&mut p, b"\x1b]133;A\x07$ \x1b]133;B\x07x\r\n\x1b]133;C\x07a\r\nb\r\nc\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ ");
        let before = visual_rows(&t).len();
        t.blocks[0].folded = true;
        let v = visual_rows(&t);
        assert_eq!(v.len(), before - 2);
        assert!(v.iter().any(|r| matches!(r, VRow::Fold(_, 3, _))));
    }
}
