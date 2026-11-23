//! Inline GUI widgets drawn over the rows their text fallback occupies.
//! Row height equals the terminal line height, so a widget reads as an
//! "upgraded" version of the text underneath rather than a floating window.

use crate::app::{Action, InlineWidget, WidgetKind};
use crate::host::sh_quote;
use crate::icons::{self, Icon};
use crate::theme;
use egui::{pos2, vec2, Align2, CornerRadius, FontId, Id, Rect, Sense, Stroke, Ui};
use std::cmp::Ordering;

/// Where a double-clicked word ends: whitespace (so never past a line end),
/// brackets, quotes and `<>|;,`. Letters of any script, digits and the
/// characters of paths and addresses (`. / - _ : ~ @ = ? & # %`) are part
/// of a word, so `src/main.rs`, `user@host` or a URL is taken whole (the
/// same rule as WezTerm's and kitty's).
pub fn word_break(c: char) -> bool {
    c.is_whitespace() || "\"'`<>()[]{}|;,".contains(c)
}

/// The word around char index `at` of `text` (char indices, end exclusive);
/// on a break character, that character alone.
pub fn word_at(text: &str, at: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    // A click past a line's end lands on its newline: the word before it.
    let at = if at >= chars.len() || (chars[at] == '\n' && at > 0 && !word_break(chars[at - 1])) { at.saturating_sub(1).min(chars.len() - 1) } else { at };
    if word_break(chars[at]) {
        return (at, at + 1);
    }
    let (mut s, mut e) = (at, at + 1);
    while s > 0 && !word_break(chars[s - 1]) {
        s -= 1;
    }
    while e < chars.len() && !word_break(chars[e]) {
        e += 1;
    }
    (s, e)
}

/// Where a text field's cursor is before it is drawn this frame: on a
/// double-click, where its first click put it (see below).
pub fn cursor_before(ui: &Ui, id: Id) -> Option<usize> {
    egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range()).map(|r| r.primary.index)
}

/// A double-click in a text field selects the word by `word_at`, not by
/// egui's rule (ASCII letters only: on Hangul it ran over spaces and line
/// ends up to the next Latin letter). The word is the one the first click
/// landed on (`before`): in rendered markdown that click shows the
/// paragraph's markup, moving the text under the second one.
pub fn select_word_on_double_click(ui: &Ui, out: &egui::text_edit::TextEditOutput, text: &str, before: Option<usize>) {
    if !out.response.double_clicked() {
        return;
    }
    let at = match before {
        Some(i) => i,
        None => {
            let Some(p) = ui.input(|i| i.pointer.interact_pos()) else { return };
            out.galley.cursor_from_pos(p - out.galley_pos).ccursor.index
        }
    };
    let (s, e) = word_at(text, at);
    let mut state = out.state.clone();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(s), egui::text::CCursor::new(e))));
    state.store(ui.ctx(), out.response.id);
}

/// A dialog window that fits the screen: at most `size`, never larger than
/// the window it opens in (help and the like ran off a small window's edge).
pub fn dialog(ctx: &egui::Context, title: &str, size: egui::Vec2) -> egui::Window<'static> {
    let screen = ctx.screen_rect();
    let room = (screen.size() - vec2(24.0, 24.0)).max(vec2(160.0, 120.0));
    egui::Window::new(title.to_owned())
        .collapsible(false)
        .default_size(size.min(room))
        .max_size(room)
        .default_pos(screen.center())
        .pivot(Align2::CENTER_CENTER)
        .constrain_to(screen)
}

/// Text at `pos` (left edge, vertically centred) cut with … to `max_w`,
/// so it never runs into whatever sits to its right.
///
/// Drawn as `weft_core::shown`: a name from a Mac shows its Hangul as
/// syllables, not letter by letter.
pub fn text_fit(p: &egui::Painter, pos: egui::Pos2, text: &str, font: FontId, color: egui::Color32, max_w: f32) {
    weft_draw::fallback::want(p.ctx(), &font, text);
    let mut job = egui::text::LayoutJob::simple_singleline(weft_core::shown(text).into_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let g = p.layout_job(job);
    p.galley(pos2(pos.x, pos.y - g.size().y / 2.0), g, color);
}

/// Width of a single line of text.
pub fn text_width(p: &egui::Painter, text: &str, font: FontId) -> f32 {
    p.layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE).size().x
}

/// A word that changes (saving…, saved, unsaved) in room as wide as the
/// widest it can be: what follows it stays where it is.
pub fn steady_label(ui: &mut Ui, text: &str, color: egui::Color32, size: f32, all: &[&str]) -> egui::Response {
    let font = FontId::proportional(size);
    let width = |t: &str| ui.fonts(|f| f.layout_no_wrap(t.to_string(), font.clone(), color).size().x);
    let w = all.iter().copied().chain([text]).map(width).fold(0.0, f32::max);
    let h = ui.fonts(|f| f.row_height(&font)).max(ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    if !text.is_empty() {
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, text, font, color);
    }
    resp
}

/// A button whose word changes for a moment (Copy, ✓ copied) as wide as
/// the widest of them.
pub fn steady_button(ui: &mut Ui, text: egui::RichText, size: f32, all: &[&str]) -> egui::Response {
    let font = FontId::proportional(size);
    let w = all.iter().map(|t| ui.fonts(|f| f.layout_no_wrap(t.to_string(), font.clone(), egui::Color32::WHITE).size().x)).fold(0.0, f32::max);
    let pad = ui.spacing().button_padding.x * 2.0;
    ui.add(egui::Button::new(text).min_size(vec2(w + pad, 0.0)))
}

pub fn human_size(n: u64) -> String {
    let mut v = n as f64;
    for u in ["B", "K", "M", "G", "T"] {
        if v < 1024.0 {
            return if u == "B" { format!("{n}B") } else { format!("{v:.1}{u}") };
        }
        v /= 1024.0;
    }
    format!("{v:.1}P")
}

pub fn ago(ts: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let d = (now - ts).max(0);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", d / 60),
        3600..=86_399 => format!("{}h ago", d / 3600),
        86_400..=2_591_999 => format!("{}d ago", d / 86_400),
        2_592_000..=31_535_999 => format!("{}mo ago", d / 2_592_000),
        _ => format!("{}y ago", d / 31_536_000),
    }
}

fn cmp_cells(a: &str, b: &str) -> Ordering {
    let num = |s: &str| s.trim().trim_end_matches('%').parse::<f64>().ok();
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

fn clip_text(s: &str, max_chars: usize) -> String {
    let s = weft_core::shown(s);
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Header cell; returns true when clicked (sort toggle).
fn header_cell(ui: &mut Ui, r: Rect, id: Id, label: &str, sorted: Option<bool>, right: bool) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    let col = if resp.hovered() || sorted.is_some() { theme::text() } else { theme::text_dim() };
    let font = FontId::proportional(11.5);
    let (pos, align) = if right { (pos2(r.right() - 14.0, r.center().y), Align2::RIGHT_CENTER) } else { (pos2(r.left() + 2.0, r.center().y), Align2::LEFT_CENTER) };
    let tr = ui.painter().text(pos, align, label, font, col);
    if let Some(asc) = sorted {
        let ir = Rect::from_min_size(pos2(if right { r.right() - 13.0 } else { tr.right() + 2.0 }, r.center().y - 5.0), vec2(10.0, 10.0));
        icons::paint(ui.painter(), ir, if asc { Icon::Up } else { Icon::ChevronDown }, theme::accent());
    }
    resp.clicked()
}

fn toggle_sort(sort: &mut Option<(usize, bool)>, col: usize) {
    *sort = match *sort {
        Some((c, asc)) if c == col => Some((c, !asc)),
        _ => Some((col, true)),
    };
}

#[allow(clippy::too_many_arguments)]
pub fn ui_widget(
    ui: &mut Ui,
    w: &mut InlineWidget,
    r: Rect,
    cw: f32,
    ch: f32,
    font: &FontId,
    pane: usize,
    host: &str,
    at_prompt: bool,
    acts: &mut Vec<Action>,
) {
    let painter = ui.painter().clone();
    painter.rect_filled(r, CornerRadius::same(4), theme::bg_base());
    let row_rect = |i: usize| Rect::from_min_size(pos2(r.left(), r.top() + i as f32 * ch), vec2(r.width(), ch));
    let wid = Id::new(("w", pane, w.id.clone()));

    match &mut w.kind {
        WidgetKind::Progress { pct, label } => {
            let rr = row_rect(0).shrink2(vec2(4.0, 3.0));
            let bar = Rect::from_min_size(rr.min, vec2((rr.width() * 0.45).min(420.0), rr.height()));
            painter.rect_filled(bar, CornerRadius::same(3), theme::bg_raised());
            let done = *pct >= 100.0;
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * *pct / 100.0, bar.height()));
            painter.rect_filled(fill, CornerRadius::same(3), if done { theme::success() } else { theme::accent() });
            painter.text(
                pos2(bar.right() + 10.0, rr.center().y),
                Align2::LEFT_CENTER,
                format!("{:>3.0}%  {}", pct, label),
                FontId::proportional(12.0),
                theme::text(),
            );
        }

        WidgetKind::Choose { options } => {
            let hr = row_rect(0);
            let msg = if w.chosen.is_some() { "chosen" } else { "choose one — click, or type its number and Enter" };
            painter.text(pos2(hr.left() + 6.0, hr.center().y), Align2::LEFT_CENTER, msg, FontId::proportional(12.0), theme::text_dim());
            for (i, opt) in options.iter().enumerate() {
                let rr = row_rect(i + 1).shrink2(vec2(2.0, 1.0));
                let resp = ui.interact(rr, wid.with(i), Sense::click());
                let chosen = w.chosen == Some(i);
                let live = w.chosen.is_none();
                let bg = if chosen {
                    theme::tint(theme::accent(), 50)
                } else if live && resp.hovered() {
                    theme::bg_hover()
                } else {
                    theme::bg_base()
                };
                painter.rect_filled(rr, CornerRadius::same(4), bg);
                let badge = Rect::from_min_size(pos2(rr.left() + 4.0, rr.top() + 2.0), vec2(ch + 4.0, rr.height() - 4.0));
                painter.rect_filled(badge, CornerRadius::same(3), theme::bg_raised());
                painter.text(badge.center(), Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(11.0), theme::text_dim());
                let col = if live || chosen { theme::text() } else { theme::text_faint() };
                painter.text(pos2(badge.right() + 8.0, rr.center().y), Align2::LEFT_CENTER, opt, font.clone(), col);
                if chosen {
                    icons::paint(&painter, Rect::from_min_size(pos2(rr.right() - ch - 4.0, rr.top() + 1.0), vec2(ch - 2.0, ch - 2.0)), Icon::Check, theme::accent());
                }
                if live && resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if live && resp.clicked() {
                    w.chosen = Some(i);
                    acts.push(Action::Send(pane, format!("{}\r", i + 1).into_bytes()));
                }
            }
        }

        WidgetKind::Files { dir, entries } => {
            let _ = dir;
            let size_w = 9.0 * cw;
            let time_w = 10.0 * cw;
            let mode_w = 11.0 * cw;
            let hr = row_rect(0);
            let name_r = Rect::from_min_max(pos2(hr.left() + ch + 6.0, hr.top()), pos2(hr.right() - size_w - time_w - mode_w, hr.bottom()));
            let size_r = Rect::from_min_size(pos2(name_r.right(), hr.top()), vec2(size_w, ch));
            let time_r = Rect::from_min_size(pos2(size_r.right(), hr.top()), vec2(time_w, ch));
            let mode_r = Rect::from_min_size(pos2(time_r.right(), hr.top()), vec2(mode_w, ch));
            let s = w.sort;
            let st = |c: usize| s.filter(|x| x.0 == c).map(|x| x.1);
            painter.hline(hr.x_range(), hr.bottom() - 0.5, Stroke::new(1.0_f32, theme::border()));
            if header_cell(ui, name_r, wid.with("h0"), "Name", st(0), false) {
                toggle_sort(&mut w.sort, 0);
            }
            if header_cell(ui, size_r, wid.with("h1"), "Size", st(1), true) {
                toggle_sort(&mut w.sort, 1);
            }
            if header_cell(ui, time_r, wid.with("h2"), "Modified", st(2), true) {
                toggle_sort(&mut w.sort, 2);
            }
            if header_cell(ui, mode_r, wid.with("h3"), "Mode", st(3), true) {
                toggle_sort(&mut w.sort, 3);
            }
            let mut order: Vec<usize> = (0..entries.len()).collect();
            if let Some((c, asc)) = w.sort {
                order.sort_by(|&a, &b| {
                    let (x, y) = (&entries[a], &entries[b]);
                    let o = match c {
                        0 => y.dir.cmp(&x.dir).then(x.name.to_lowercase().cmp(&y.name.to_lowercase())),
                        1 => x.size.cmp(&y.size),
                        2 => x.mtime.cmp(&y.mtime),
                        _ => x.mode.cmp(&y.mode),
                    };
                    if asc {
                        o
                    } else {
                        o.reverse()
                    }
                });
            }
            let name_chars = ((name_r.width() / cw) as usize).max(4);
            for (row, &ei) in order.iter().enumerate() {
                let e = &entries[ei];
                let rr = row_rect(row + 1);
                if !ui.clip_rect().intersects(rr) {
                    continue;
                }
                let resp = ui.interact(rr, wid.with(("row", ei)), Sense::click());
                if resp.hovered() {
                    painter.rect_filled(rr, CornerRadius::same(3), theme::bg_hover());
                }
                let ir = Rect::from_min_size(pos2(rr.left() + 4.0, rr.top() + 2.0), vec2(ch - 4.0, ch - 4.0));
                icons::paint(&painter, ir, if e.dir { Icon::Folder } else { Icon::File }, if e.dir { theme::ansi(4) } else { theme::text_faint() });
                let ncol = if e.dir { theme::ansi(12) } else { theme::text() };
                painter.text(pos2(name_r.left(), rr.center().y), Align2::LEFT_CENTER, clip_text(&e.name, name_chars), font.clone(), ncol);
                let small = FontId::proportional(11.5);
                if !e.dir {
                    painter.text(pos2(size_r.right() - 14.0, rr.center().y), Align2::RIGHT_CENTER, human_size(e.size), small.clone(), theme::text_dim());
                }
                painter.text(pos2(time_r.right() - 14.0, rr.center().y), Align2::RIGHT_CENTER, ago(e.mtime), small.clone(), theme::text_dim());
                painter.text(pos2(mode_r.right() - 14.0, rr.center().y), Align2::RIGHT_CENTER, &e.mode, font.clone(), theme::text_faint());
                if resp.double_clicked() {
                    if e.dir {
                        if at_prompt {
                            acts.push(Action::Run(pane, format!("cd -- {}", sh_quote(&e.path))));
                        } else {
                            acts.push(Action::SetRoot(host.to_string(), e.path.clone()));
                        }
                    } else {
                        acts.push(Action::OpenFile(host.to_string(), e.path.clone(), 1));
                    }
                }
                let resp = resp.on_hover_text(if e.dir { "double-click: cd into it" } else { "double-click: open in editor" });
                let (path, isdir) = (e.path.clone(), e.dir);
                resp.context_menu(|ui| {
                    if !isdir && ui.button("Open in editor").clicked() {
                        acts.push(Action::OpenFile(host.to_string(), path.clone(), 1));
                        ui.close_menu();
                    }
                    if isdir && ui.add_enabled(at_prompt, egui::Button::new("cd here")).clicked() {
                        acts.push(Action::Run(pane, format!("cd -- {}", sh_quote(&path))));
                        ui.close_menu();
                    }
                    if ui.button("Show in Files panel").clicked() {
                        let target = if isdir { path.clone() } else { crate::app::parent(&path) };
                        acts.push(Action::SetRoot(host.to_string(), target));
                        ui.close_menu();
                    }
                    if ui.button("Insert path at prompt").clicked() {
                        acts.push(Action::Send(pane, format!("{} ", sh_quote(&path)).into_bytes()));
                        ui.close_menu();
                    }
                    if ui.button("Copy path").clicked() {
                        acts.push(Action::Copy(path.clone()));
                        ui.close_menu();
                    }
                });
            }
        }

        WidgetKind::Table { columns, data } => {
            let ncol = columns.len().max(1);
            let mut widths: Vec<usize> = columns.iter().map(|c| c.chars().count()).collect();
            for row in data.iter().take(500) {
                for (i, cell) in row.iter().enumerate().take(ncol) {
                    widths[i] = widths[i].max(cell.chars().count());
                }
            }
            for wv in widths.iter_mut() {
                *wv = (*wv).clamp(3, 48);
            }
            // The last column takes the remaining width.
            let total_cols = (r.width() / cw) as usize;
            let used: usize = widths[..ncol - 1].iter().map(|w| w + 2).sum();
            widths[ncol - 1] = total_cols.saturating_sub(used + 1).max(4);
            let xs: Vec<f32> = widths
                .iter()
                .scan(r.left() + 4.0, |x, w| {
                    let cur = *x;
                    *x += (*w + 2) as f32 * cw;
                    Some(cur)
                })
                .collect();
            let numeric: Vec<bool> = (0..ncol)
                .map(|c| {
                    let vals: Vec<&String> = data.iter().filter_map(|r| r.get(c)).take(50).collect();
                    !vals.is_empty() && vals.iter().all(|v| v.trim().trim_end_matches('%').parse::<f64>().is_ok())
                })
                .collect();
            let hr = row_rect(0);
            painter.hline(hr.x_range(), hr.bottom() - 0.5, Stroke::new(1.0_f32, theme::border()));
            for c in 0..ncol {
                let cr = Rect::from_min_size(pos2(xs[c], hr.top()), vec2((widths[c] + 1) as f32 * cw, ch));
                let sorted = w.sort.filter(|s| s.0 == c).map(|s| s.1);
                if header_cell(ui, cr, wid.with(("h", c)), &columns[c], sorted, numeric[c]) {
                    toggle_sort(&mut w.sort, c);
                }
            }
            let mut order: Vec<usize> = (0..data.len()).collect();
            if let Some((c, asc)) = w.sort {
                order.sort_by(|&a, &b| {
                    let o = cmp_cells(data[a].get(c).map(|s| s.as_str()).unwrap_or(""), data[b].get(c).map(|s| s.as_str()).unwrap_or(""));
                    if asc {
                        o
                    } else {
                        o.reverse()
                    }
                });
            }
            for (row, &di) in order.iter().enumerate() {
                let rr = row_rect(row + 1);
                if !ui.clip_rect().intersects(rr) {
                    continue;
                }
                let resp = ui.interact(rr, wid.with(("r", di)), Sense::click());
                if resp.hovered() {
                    painter.rect_filled(rr, CornerRadius::same(3), theme::bg_hover());
                } else if row % 2 == 1 {
                    painter.rect_filled(rr, CornerRadius::ZERO, theme::tint(theme::text(), 4));
                }
                for c in 0..ncol {
                    let cell = data[di].get(c).map(|s| s.as_str()).unwrap_or("");
                    let text = clip_text(cell, widths[c]);
                    let col = if c == 0 { theme::text() } else { theme::text_dim() };
                    if numeric[c] {
                        painter.text(pos2(xs[c] + (widths[c] + 1) as f32 * cw - 14.0, rr.center().y), Align2::RIGHT_CENTER, text, font.clone(), col);
                    } else {
                        painter.text(pos2(xs[c] + 2.0, rr.center().y), Align2::LEFT_CENTER, text, font.clone(), col);
                    }
                }
                let row_tsv = data[di].join("\t");
                let cols_snapshot: Vec<String> = data.iter().map(|r| r.first().cloned().unwrap_or_default()).collect();
                resp.context_menu(|ui| {
                    if ui.button("Copy row").clicked() {
                        acts.push(Action::Copy(row_tsv.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Copy first column").clicked() {
                        acts.push(Action::Copy(cols_snapshot.join("\n")));
                        ui.close_menu();
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod word_tests {
    use super::word_at;

    fn word(text: &str, needle: &str, off: usize) -> String {
        let at = text[..text.find(needle).unwrap()].chars().count() + off;
        let (s, e) = word_at(text, at);
        text.chars().skip(s).take(e - s).collect()
    }

    #[test]
    fn double_click_words() {
        // Hangul is a word, and the line end stops it (egui's rule ran on).
        let t = "설정 파일 없음\n기본값 사용";
        assert_eq!(word(t, "없음", 1), "없음");
        assert_eq!(word(t, "기본값", 0), "기본값");
        // Paths and addresses whole; brackets and quotes stop them.
        let t = "see (src/main.rs) or \"user@host:~/x\", ok";
        assert_eq!(word(t, "main", 0), "src/main.rs");
        assert_eq!(word(t, "host", 0), "user@host:~/x");
        // A one-letter word at a line end: not the newline with it.
        assert_eq!(word("a\nfoo", "a", 0), "a");
        // Past the line end: the word before it.
        assert_eq!(word("abc\ndef", "\n", 0), "abc");
        // On a space: the space alone.
        assert_eq!(word("a b", " ", 0), " ");
    }

    /// Two clicks in two frames on a Hangul word of a real egui text field,
    /// the text moving between them (as rendered markdown does when the
    /// first click shows the paragraph's markup): that word, nothing more.
    #[test]
    fn double_click_in_a_text_field() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        let mut text = String::from("설정 파일 없음\n기본값 사용 text");
        let id = egui::Id::new("field");
        let mut indent = 0.0;
        let mut target = egui::Pos2::ZERO;
        let mut frame = |events: Vec<egui::Event>, indent: f32, target: &mut egui::Pos2| {
            let mut range = None;
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(indent);
                    let before = super::cursor_before(ui, id);
                    let out = egui::TextEdit::multiline(&mut text).id(id).show(ui);
                    super::select_word_on_double_click(ui, &out, out.galley.job.text.as_str(), before);
                    *target = out.galley.pos_from_ccursor(egui::text::CCursor::new(7)).center() + out.galley_pos.to_vec2();
                    range = egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range()).map(|r| (r.primary.index.min(r.secondary.index), r.primary.index.max(r.secondary.index)));
                });
            });
            range
        };
        frame(vec![], indent, &mut target);
        let at = target;
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(vec![egui::Event::PointerMoved(at), click(true), click(false)], indent, &mut target);
        // The text moves down a line before the second click lands.
        indent = 20.0;
        let r = frame(vec![click(true), click(false)], indent, &mut target);
        assert_eq!(r, Some((6, 8)), "\"없음\" only (egui alone took more)");
    }
}
