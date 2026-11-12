//! Copy mode (Ctrl+Shift+X), as in WezTerm: a cursor moved over the screen
//! and scrollback with vim keys, v / V / Ctrl+v to select characters, lines
//! or a block, y (or Enter) to copy, Esc or q to leave. Keys go nowhere else
//! meanwhile.

use crate::app::{SelMode, Selection};
use crate::term::Term;
use egui::{Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CopyMode {
    /// (line, column) of the copy cursor.
    pub cur: (u64, usize),
    /// `g` was pressed: a second `g` goes to the top.
    pub g: bool,
}

/// Lines that can be visited (lo..=hi), the ones on screen, and the width.
pub struct Bounds {
    pub lo: u64,
    pub hi: u64,
    pub view: (u64, u64),
    pub cols: usize,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Stay,
    Copy,
    Exit,
}

/// A line's characters with their columns (wide characters once).
fn chars(t: &Term, abs: u64) -> Vec<(char, usize)> {
    t.visible_row(abs).map(|r| r.chars_with_cols()).unwrap_or_default()
}

/// Index of the character covering `col` (or the last one before it).
fn index_at(cs: &[(char, usize)], col: usize) -> usize {
    cs.iter().rposition(|&(_, c)| c <= col).unwrap_or(0)
}

fn blank(c: char) -> bool {
    c.is_whitespace()
}

/// Positions after (a, i) in reading order, across lines up to `hi`; an
/// empty line yields one position (index 0) so it can stop a motion.
fn forward(t: &Term, hi: u64, a: u64, i: usize) -> impl Iterator<Item = (u64, usize, char)> + '_ {
    let mut line = a;
    let mut cs = chars(t, a);
    let mut idx = i + 1;
    std::iter::from_fn(move || loop {
        if idx < cs.len() {
            idx += 1;
            return Some((line, cs[idx - 1].1, cs[idx - 1].0));
        }
        if line >= hi {
            return None;
        }
        line += 1;
        cs = chars(t, line);
        idx = 0;
        if cs.iter().all(|c| blank(c.0)) {
            idx = cs.len();
            return Some((line, 0, '\n'));
        }
    })
}

/// Positions before (a, i), backwards, down to `lo`.
fn backward(t: &Term, lo: u64, a: u64, i: usize) -> impl Iterator<Item = (u64, usize, char)> + '_ {
    let mut line = a;
    let mut cs = chars(t, a);
    let mut idx = i;
    std::iter::from_fn(move || loop {
        if idx > 0 {
            idx -= 1;
            return Some((line, cs[idx].1, cs[idx].0));
        }
        if line <= lo {
            return None;
        }
        line -= 1;
        cs = chars(t, line);
        idx = cs.len();
        if cs.iter().all(|c| blank(c.0)) {
            idx = 0;
            return Some((line, 0, '\n'));
        }
    })
}

/// vim W: the start of the next run of non-blank characters.
fn word_start_next(t: &Term, b: &Bounds, (a, col): (u64, usize)) -> (u64, usize) {
    let cs = chars(t, a);
    let i = index_at(&cs, col);
    let mut seen_blank = cs.get(i).map(|c| blank(c.0)).unwrap_or(true);
    for (l, c, ch) in forward(t, b.hi, a, i) {
        if ch == '\n' || blank(ch) {
            seen_blank = true;
            if ch == '\n' {
                return (l, c);
            }
        } else if seen_blank {
            return (l, c);
        }
    }
    (a, col)
}

/// vim B: the start of this word, or of the one before.
fn word_start_prev(t: &Term, b: &Bounds, (a, col): (u64, usize)) -> (u64, usize) {
    let cs = chars(t, a);
    let i = index_at(&cs, col);
    let mut found: Option<(u64, usize)> = None;
    for (l, c, ch) in backward(t, b.lo, a, i) {
        match (found, ch == '\n' || blank(ch)) {
            (None, true) => {
                if ch == '\n' {
                    return (l, c);
                }
            }
            (None, false) => found = Some((l, c)),
            (Some(_), false) => found = Some((l, c)),
            (Some(f), true) => return f,
        }
    }
    found.unwrap_or((a, col))
}

/// vim E: the end of this word, or of the next.
fn word_end_next(t: &Term, b: &Bounds, (a, col): (u64, usize)) -> (u64, usize) {
    let cs = chars(t, a);
    let i = index_at(&cs, col);
    let mut found: Option<(u64, usize)> = None;
    for (l, c, ch) in forward(t, b.hi, a, i) {
        match (found, ch == '\n' || blank(ch)) {
            (None, true) => {}
            (None, false) | (Some(_), false) => found = Some((l, c)),
            (Some(f), true) => return f,
        }
    }
    found.unwrap_or((a, col))
}

fn line_end(t: &Term, a: u64) -> usize {
    chars(t, a).iter().rev().find(|c| !blank(c.0)).map(|c| c.1).unwrap_or(0)
}

fn line_first(t: &Term, a: u64) -> usize {
    chars(t, a).iter().find(|c| !blank(c.0)).map(|c| c.1).unwrap_or(0)
}

/// Turn a selection kind on, switch to it, or (the same again) off.
fn toggle(sel: &mut Option<Selection>, cur: (u64, usize), mode: SelMode) {
    *sel = match *sel {
        Some(s) if s.mode == mode => None,
        Some(s) => Some(Selection { mode, ..s }),
        None => Some(Selection { a: cur, b: cur, mode }),
    };
}

/// One key in copy mode.
pub fn key(cm: &mut CopyMode, sel: &mut Option<Selection>, t: &Term, b: &Bounds, key: Key, m: Modifiers) -> Outcome {
    let g = std::mem::take(&mut cm.g);
    let (mut a, mut col) = cm.cur;
    let ctrl = m.ctrl || m.command;
    let page = (b.view.1.saturating_sub(b.view.0) + 1).max(2);
    let cs = chars(t, a);
    let i = index_at(&cs, col);
    match (key, ctrl, m.shift) {
        (Key::Escape, ..) | (Key::Q, false, false) => return Outcome::Exit,
        (Key::Y, false, false) | (Key::Enter, false, _) | (Key::C, true, _) => return Outcome::Copy,
        (Key::H, false, false) | (Key::ArrowLeft, false, _) | (Key::Backspace, false, _) => col = cs.get(i.saturating_sub(1)).map(|c| c.1).unwrap_or(0),
        (Key::L, false, false) | (Key::ArrowRight, false, _) | (Key::Space, false, _) => {
            col = cs.get(i + 1).map(|c| c.1).unwrap_or(col).min(b.cols.saturating_sub(1));
        }
        (Key::K, false, false) | (Key::ArrowUp, false, _) => a = a.saturating_sub(1),
        (Key::J, false, false) | (Key::ArrowDown, false, _) => a += 1,
        (Key::Num0, false, false) | (Key::Home, false, _) => col = 0,
        // ^ and $ are Shift+6 and Shift+4 on most layouts.
        (Key::Num6, false, true) => col = line_first(t, a),
        (Key::Num4, false, true) | (Key::End, false, _) => col = line_end(t, a),
        (Key::W, false, _) => (a, col) = word_start_next(t, b, (a, col)),
        (Key::B, false, _) => (a, col) = word_start_prev(t, b, (a, col)),
        (Key::E, false, _) => (a, col) = word_end_next(t, b, (a, col)),
        (Key::G, false, false) => {
            if !g {
                cm.g = true;
                return Outcome::Stay;
            }
            (a, col) = (b.lo, 0);
        }
        (Key::G, false, true) => (a, col) = (b.hi, 0),
        (Key::H, false, true) => a = b.view.0,
        (Key::M, false, true) => a = b.view.0 + (b.view.1.saturating_sub(b.view.0)) / 2,
        (Key::L, false, true) => a = b.view.1,
        (Key::U, true, _) => a = a.saturating_sub(page / 2),
        (Key::D, true, _) => a += page / 2,
        (Key::B, true, _) | (Key::PageUp, false, _) => a = a.saturating_sub(page),
        (Key::F, true, _) | (Key::PageDown, false, _) => a += page,
        (Key::V, false, false) => toggle(sel, cm.cur, SelMode::Char),
        (Key::V, false, true) => toggle(sel, cm.cur, SelMode::Line),
        (Key::V, true, _) => toggle(sel, cm.cur, SelMode::Block),
        (Key::O, false, _) => {
            if let Some(s) = sel.as_mut() {
                (a, col) = s.a;
                s.a = s.b;
            }
        }
        _ => return Outcome::Stay,
    }
    let a = a.clamp(b.lo, b.hi);
    cm.cur = (a, col.min(b.cols.saturating_sub(1)));
    if let Some(s) = sel.as_mut() {
        s.b = cm.cur;
    }
    Outcome::Stay
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(text: &str) -> Term {
        let mut t = Term::new(20, 6);
        let mut p = vte::Parser::new();
        t.feed(&mut p, text.replace('\n', "\r\n").as_bytes());
        t
    }

    fn press(cm: &mut CopyMode, sel: &mut Option<Selection>, t: &Term, keys: &[(Key, Modifiers)]) -> Outcome {
        let b = Bounds { lo: t.first_line(), hi: t.end_line() - 1, view: (t.first_line(), t.end_line() - 1), cols: t.cols };
        let mut out = Outcome::Stay;
        for &(k, m) in keys {
            out = key(cm, sel, t, &b, k, m);
        }
        out
    }

    const N: Modifiers = Modifiers::NONE;
    const S: Modifiers = Modifiers::SHIFT;

    #[test]
    fn moves_like_vim() {
        let t = term("ls -l src/main.rs\nfoo  bar\n\nbaz");
        let mut cm = CopyMode { cur: (0, 0), g: false };
        let mut sel = None;
        press(&mut cm, &mut sel, &t, &[(Key::W, N)]);
        assert_eq!(cm.cur, (0, 3), "w: next word");
        press(&mut cm, &mut sel, &t, &[(Key::W, N), (Key::W, N)]);
        assert_eq!(cm.cur, (1, 0), "w crosses to the next line");
        press(&mut cm, &mut sel, &t, &[(Key::E, N)]);
        assert_eq!(cm.cur, (1, 2), "e: end of word");
        press(&mut cm, &mut sel, &t, &[(Key::W, N), (Key::W, N)]);
        assert_eq!(cm.cur, (2, 0), "an empty line stops w");
        press(&mut cm, &mut sel, &t, &[(Key::B, N)]);
        assert_eq!(cm.cur, (1, 5), "b: back to a word start");
        press(&mut cm, &mut sel, &t, &[(Key::Num4, S)]);
        assert_eq!(cm.cur, (1, 7), "$: last character");
        press(&mut cm, &mut sel, &t, &[(Key::Num0, N), (Key::L, N), (Key::L, N), (Key::H, N)]);
        assert_eq!(cm.cur, (1, 1));
        press(&mut cm, &mut sel, &t, &[(Key::G, N), (Key::G, N)]);
        assert_eq!(cm.cur, (0, 0), "gg: top");
        press(&mut cm, &mut sel, &t, &[(Key::K, N)]);
        assert_eq!(cm.cur, (0, 0), "stays inside");
    }

    #[test]
    fn select_and_copy() {
        let t = term("ls -l src/main.rs\nfoo  bar\nbaz");
        let mut cm = CopyMode { cur: (0, 6), g: false };
        let mut sel = None;
        press(&mut cm, &mut sel, &t, &[(Key::V, N), (Key::E, N)]);
        assert_eq!(sel, Some(Selection { a: (0, 6), b: (0, 16), mode: SelMode::Char }));
        // V switches to lines, keeping the start; j extends.
        press(&mut cm, &mut sel, &t, &[(Key::V, S), (Key::J, N)]);
        assert_eq!(sel.map(|s| (s.mode, s.a, s.b)), Some((SelMode::Line, (0, 6), (1, 16))), "j keeps the column, as in vim");
        // o: to the other end.
        press(&mut cm, &mut sel, &t, &[(Key::O, N)]);
        assert_eq!(cm.cur, (0, 6));
        // The same kind again: off.
        press(&mut cm, &mut sel, &t, &[(Key::V, S)]);
        assert_eq!(sel, None);
        assert_eq!(press(&mut cm, &mut sel, &t, &[(Key::Y, N)]), Outcome::Copy);
        assert_eq!(press(&mut cm, &mut sel, &t, &[(Key::Escape, N)]), Outcome::Exit);
    }
}
