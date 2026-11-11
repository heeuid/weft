//! Vector icons painted with strokes, so they never depend on a font having
//! the right glyph (no tofu boxes, no emoji fallback surprises).

use crate::theme;
use egui::{pos2, vec2, Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Folder,
    File,
    ChevronRight,
    ChevronDown,
    Check,
    Cross,
    Rerun,
    Copy,
    Fold,
    Unfold,
    Open,
    Plus,
    SplitRight,
    SplitDown,
    Search,
    Up,
    Refresh,
    Sidebar,
    Editor,
    Dot,
    Plug,
    Terminal,
}

pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 16.0; // design grid: 16x16
    let at = |x: f32, y: f32| pos2(c.x + (x - 8.0) * u, c.y + (y - 8.0) * u);
    let st = Stroke::new((1.4 * u).max(1.0), color);
    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), st));
    };
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    match icon {
        Icon::Folder => {
            let pts = vec![at(2.0, 4.0), at(6.5, 4.0), at(8.0, 5.5), at(14.0, 5.5), at(14.0, 12.5), at(2.0, 12.5)];
            p.add(Shape::convex_polygon(pts.clone(), theme::tint(color, 50), Stroke::NONE));
            p.add(Shape::closed_line(pts, st));
        }
        Icon::File => {
            let pts = vec![at(4.0, 2.0), at(9.5, 2.0), at(12.5, 5.0), at(12.5, 14.0), at(4.0, 14.0)];
            p.add(Shape::closed_line(pts, st));
            line(&[(9.5, 2.0), (9.5, 5.0), (12.5, 5.0)]);
        }
        Icon::ChevronRight => line(&[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]),
        Icon::ChevronDown => line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
        Icon::Up => {
            line(&[(4.0, 7.0), (8.0, 3.0), (12.0, 7.0)]);
            line(&[(8.0, 3.0), (8.0, 13.0)]);
        }
        Icon::Check => line(&[(3.0, 8.5), (6.5, 12.0), (13.0, 4.5)]),
        Icon::Cross => {
            line(&[(4.0, 4.0), (12.0, 12.0)]);
            line(&[(12.0, 4.0), (4.0, 12.0)]);
        }
        Icon::Plus => {
            line(&[(8.0, 3.0), (8.0, 13.0)]);
            line(&[(3.0, 8.0), (13.0, 8.0)]);
        }
        Icon::Rerun | Icon::Refresh => {
            let n = 14;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = -0.6 + (i as f32 / n as f32) * 5.0;
                    at(8.0 + 5.0 * a.cos(), 8.0 + 5.0 * a.sin())
                })
                .collect();
            p.add(Shape::line(pts, st));
            let a: f32 = -0.6;
            let tip = (8.0 + 5.0 * a.cos(), 8.0 + 5.0 * a.sin());
            line(&[(tip.0 - 3.2, tip.1 - 0.4), tip, (tip.0 + 0.2, tip.1 - 3.4)]);
        }
        Icon::Copy => {
            p.rect_stroke(r(5.5, 5.5, 13.0, 13.0), CornerRadius::same(1), st, StrokeKind::Middle);
            line(&[(3.0, 10.5), (3.0, 3.0), (10.5, 3.0)]);
        }
        Icon::Fold => {
            line(&[(4.0, 3.0), (8.0, 7.0), (12.0, 3.0)]);
            line(&[(4.0, 13.0), (8.0, 9.0), (12.0, 13.0)]);
        }
        Icon::Unfold => {
            line(&[(4.0, 6.0), (8.0, 2.0), (12.0, 6.0)]);
            line(&[(4.0, 10.0), (8.0, 14.0), (12.0, 10.0)]);
        }
        Icon::Open => {
            line(&[(9.0, 3.0), (13.0, 3.0), (13.0, 7.0)]);
            line(&[(13.0, 3.0), (7.5, 8.5)]);
            line(&[(11.0, 9.5), (11.0, 13.0), (3.0, 13.0), (3.0, 5.0), (6.5, 5.0)]);
        }
        Icon::SplitRight | Icon::SplitDown | Icon::Sidebar | Icon::Editor => {
            p.rect_stroke(r(2.0, 3.0, 14.0, 13.0), CornerRadius::same(2), st, StrokeKind::Middle);
            match icon {
                Icon::SplitRight => line(&[(8.0, 3.0), (8.0, 13.0)]),
                Icon::SplitDown => line(&[(2.0, 8.0), (14.0, 8.0)]),
                Icon::Sidebar => {
                    p.rect_filled(r(2.0, 3.0, 6.0, 13.0), CornerRadius::same(1), theme::tint(color, 110));
                }
                _ => {
                    p.rect_filled(r(10.0, 3.0, 14.0, 13.0), CornerRadius::same(1), theme::tint(color, 110));
                }
            }
        }
        Icon::Search => {
            p.circle_stroke(at(7.0, 7.0), 4.0 * u, st);
            line(&[(10.0, 10.0), (13.5, 13.5)]);
        }
        Icon::Dot => {
            p.circle_filled(c, 3.2 * u, color);
        }
        Icon::Plug => {
            line(&[(6.0, 2.5), (6.0, 5.5)]);
            line(&[(10.0, 2.5), (10.0, 5.5)]);
            line(&[(4.0, 5.5), (12.0, 5.5), (12.0, 8.0), (10.0, 10.5), (6.0, 10.5), (4.0, 8.0), (4.0, 5.5)]);
            line(&[(8.0, 10.5), (8.0, 14.0)]);
        }
        Icon::Terminal => {
            p.rect_stroke(r(2.0, 3.0, 14.0, 13.0), CornerRadius::same(2), st, StrokeKind::Middle);
            line(&[(4.5, 6.0), (7.0, 8.0), (4.5, 10.0)]);
            line(&[(8.5, 10.5), (11.5, 10.5)]);
        }
    }
}

/// Small square icon button with hover background and a tooltip.
pub fn button(ui: &mut Ui, icon: Icon, tip: &str) -> Response {
    button_colored(ui, icon, tip, theme::TEXT_DIM)
}

pub fn button_colored(ui: &mut Ui, icon: Icon, tip: &str, color: Color32) -> Response {
    let size = vec2(22.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(5), theme::BG_HOVER);
        }
        let col = if hovered { theme::TEXT } else { color };
        paint(ui.painter(), rect.shrink(4.0), icon, col);
    }
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}

/// Icon button drawn at an absolute rect (for overlays on the terminal grid).
pub fn overlay_button(ui: &mut Ui, rect: Rect, id: egui::Id, icon: Icon, tip: &str) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    let bg = if resp.hovered() { theme::BG_HOVER } else { theme::BG_RAISED };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    let col = if resp.hovered() { theme::TEXT } else { theme::TEXT_DIM };
    paint(ui.painter(), rect.shrink(3.5), icon, col);
    resp.on_hover_text(tip)
}
