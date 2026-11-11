//! Inline GUI widgets drawn over the rows their text fallback occupies.
//! Row height equals the terminal line height, so a widget reads as an
//! "upgraded" version of the text underneath rather than a floating window.

use crate::app::{Action, InlineWidget, WidgetKind};
use crate::host::sh_quote;
use crate::icons::{self, Icon};
use crate::theme;
use egui::{pos2, vec2, Align2, CornerRadius, FontId, Id, Rect, Sense, Stroke, Ui};
use std::cmp::Ordering;

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
    let col = if resp.hovered() || sorted.is_some() { theme::TEXT } else { theme::TEXT_DIM };
    let font = FontId::proportional(11.5);
    let (pos, align) = if right { (pos2(r.right() - 14.0, r.center().y), Align2::RIGHT_CENTER) } else { (pos2(r.left() + 2.0, r.center().y), Align2::LEFT_CENTER) };
    let tr = ui.painter().text(pos, align, label, font, col);
    if let Some(asc) = sorted {
        let ir = Rect::from_min_size(pos2(if right { r.right() - 13.0 } else { tr.right() + 2.0 }, r.center().y - 5.0), vec2(10.0, 10.0));
        icons::paint(ui.painter(), ir, if asc { Icon::Up } else { Icon::ChevronDown }, theme::ACCENT);
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
    painter.rect_filled(r, CornerRadius::same(4), theme::BG_BASE);
    let row_rect = |i: usize| Rect::from_min_size(pos2(r.left(), r.top() + i as f32 * ch), vec2(r.width(), ch));
    let wid = Id::new(("w", pane, w.id.clone()));

    match &mut w.kind {
        WidgetKind::Progress { pct, label } => {
            let rr = row_rect(0).shrink2(vec2(4.0, 3.0));
            let bar = Rect::from_min_size(rr.min, vec2((rr.width() * 0.45).min(420.0), rr.height()));
            painter.rect_filled(bar, CornerRadius::same(3), theme::BG_RAISED);
            let done = *pct >= 100.0;
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * *pct / 100.0, bar.height()));
            painter.rect_filled(fill, CornerRadius::same(3), if done { theme::SUCCESS } else { theme::ACCENT });
            painter.text(
                pos2(bar.right() + 10.0, rr.center().y),
                Align2::LEFT_CENTER,
                format!("{:>3.0}%  {}", pct, label),
                FontId::proportional(12.0),
                theme::TEXT,
            );
        }

        WidgetKind::Choose { options } => {
            let hr = row_rect(0);
            let msg = if w.chosen.is_some() { "chosen" } else { "choose one — click, or type its number and Enter" };
            painter.text(pos2(hr.left() + 6.0, hr.center().y), Align2::LEFT_CENTER, msg, FontId::proportional(12.0), theme::TEXT_DIM);
            for (i, opt) in options.iter().enumerate() {
                let rr = row_rect(i + 1).shrink2(vec2(2.0, 1.0));
                let resp = ui.interact(rr, wid.with(i), Sense::click());
                let chosen = w.chosen == Some(i);
                let live = w.chosen.is_none();
                let bg = if chosen {
                    theme::tint(theme::ACCENT, 50)
                } else if live && resp.hovered() {
                    theme::BG_HOVER
                } else {
                    theme::BG_BASE
                };
                painter.rect_filled(rr, CornerRadius::same(4), bg);
                let badge = Rect::from_min_size(pos2(rr.left() + 4.0, rr.top() + 2.0), vec2(ch + 4.0, rr.height() - 4.0));
                painter.rect_filled(badge, CornerRadius::same(3), theme::BG_RAISED);
                painter.text(badge.center(), Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(11.0), theme::TEXT_DIM);
                let col = if live || chosen { theme::TEXT } else { theme::TEXT_FAINT };
                painter.text(pos2(badge.right() + 8.0, rr.center().y), Align2::LEFT_CENTER, opt, font.clone(), col);
                if chosen {
                    icons::paint(&painter, Rect::from_min_size(pos2(rr.right() - ch - 4.0, rr.top() + 1.0), vec2(ch - 2.0, ch - 2.0)), Icon::Check, theme::ACCENT);
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
            painter.hline(hr.x_range(), hr.bottom() - 0.5, Stroke::new(1.0, theme::BORDER));
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
                    painter.rect_filled(rr, CornerRadius::same(3), theme::BG_HOVER);
                }
                let ir = Rect::from_min_size(pos2(rr.left() + 4.0, rr.top() + 2.0), vec2(ch - 4.0, ch - 4.0));
                icons::paint(&painter, ir, if e.dir { Icon::Folder } else { Icon::File }, if e.dir { theme::ANSI[4] } else { theme::TEXT_FAINT });
                let ncol = if e.dir { theme::ANSI[12] } else { theme::TEXT };
                painter.text(pos2(name_r.left(), rr.center().y), Align2::LEFT_CENTER, clip_text(&e.name, name_chars), font.clone(), ncol);
                let small = FontId::proportional(11.5);
                if !e.dir {
                    painter.text(pos2(size_r.right() - 14.0, rr.center().y), Align2::RIGHT_CENTER, human_size(e.size), small.clone(), theme::TEXT_DIM);
                }
                painter.text(pos2(time_r.right() - 14.0, rr.center().y), Align2::RIGHT_CENTER, ago(e.mtime), small.clone(), theme::TEXT_DIM);
                painter.text(pos2(mode_r.right() - 14.0, rr.center().y), Align2::RIGHT_CENTER, &e.mode, font.clone(), theme::TEXT_FAINT);
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
            painter.hline(hr.x_range(), hr.bottom() - 0.5, Stroke::new(1.0, theme::BORDER));
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
                    painter.rect_filled(rr, CornerRadius::same(3), theme::BG_HOVER);
                } else if row % 2 == 1 {
                    painter.rect_filled(rr, CornerRadius::ZERO, theme::tint(theme::TEXT, 4));
                }
                for c in 0..ncol {
                    let cell = data[di].get(c).map(|s| s.as_str()).unwrap_or("");
                    let text = clip_text(cell, widths[c]);
                    let col = if c == 0 { theme::TEXT } else { theme::TEXT_DIM };
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
