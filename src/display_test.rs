//! Display test: a window of test screens, to see with one's own eyes how
//! weft draws with the font that is chosen: block characters and a logo
//! built of them, colours and attributes, columns, box drawing. Each page
//! says what to look for.
//!
//! The pages are terminal output (escape sequences) fed to a terminal of
//! their own and drawn by the terminal's row painter: what shows here is
//! what a program printing the same would show in a pane. The tests look
//! at the same pages in pixels (regress.rs).

use crate::app::App;
use crate::term::Term;
use crate::theme;
use egui::{vec2, CornerRadius, RichText, ScrollArea, Sense};

/// How wide the test terminal is.
const COLS: usize = 76;

const ESC: char = '\x1b';

fn sgr(codes: &str, text: &str) -> String {
    format!("{ESC}[{codes}m{text}{ESC}[0m")
}

fn chars(range: impl Iterator<Item = u32>) -> String {
    range.filter_map(char::from_u32).collect()
}

/// A logo as Claude Code prints its own (orange, the row of its eyes on a
/// black background), and every kind of block character.
pub fn blocks() -> String {
    let orange = "38;2;215;119;87";
    let o = |t: &str| sgr(orange, t);
    let eyes = sgr(&format!("{orange};48;2;0;0;0"), "\u{259b}\u{2588}\u{2588}\u{2588}\u{259c}");
    [
        String::new(),
        format!(" {}{eyes}{}   a logo made of blocks", o("\u{2590}"), o("\u{258c}")),
        format!("{}  its eye row has a black background", o("\u{259d}\u{259c}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{259b}\u{2598}")),
        format!("{}    only the two eyes may be black", o("  \u{2598}\u{2598} \u{259d}\u{259d}")),
        String::new(),
        format!("eighths, rising   {}|", chars(0x2581..0x2589)),
        format!("eighths, widening {}|", chars((0x2588..0x2590).rev())),
        format!("shades            {}|", "\u{2591}\u{2591}\u{2592}\u{2592}\u{2593}\u{2593}\u{2588}\u{2588}"),
        format!("halves, quarters  {}|", "\u{2580}\u{2584}\u{258c}\u{2590}\u{2596}\u{2597}\u{2598}\u{259d}"),
        format!("full, on red      {}|", sgr("48;2;200;0;0", "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}")),
        format!("two rows of full  {}|", "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}"),
        format!("blocks: no line   {}|", "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}"),
    ]
    .join("\n")
}

pub fn colours() -> String {
    let mut lines = vec![
        format!("plain, {}, {}, {}, {}, {}, {}", sgr("1", "bold"), sgr("4", "underline"), sgr("7", "inverse"), sgr("2", "dim"), sgr("9", "strike"), sgr("1;4", "bold underline")),
        String::new(),
        (0..8).map(|i| sgr(&format!("{}", 30 + i), &format!("colour{i}"))).collect::<Vec<_>>().join(" "),
        (0..8).map(|i| sgr(&format!("{}", 90 + i), &format!("bright{i}"))).collect::<Vec<_>>().join(" "),
        (0..8).map(|i| sgr(&format!("{}", 40 + i), &format!("  bg{i}   "))).collect::<Vec<_>>().join(""),
        (0..8).map(|i| sgr(&format!("{}", 100 + i), &format!(" bright{i} "))).collect::<Vec<_>>().join(""),
        String::new(),
        "256 colours:".to_string(),
    ];
    for start in [16, 88, 160] {
        lines.push((start..start + 72).map(|i| sgr(&format!("48;5;{i}"), " ")).collect());
    }
    lines.push((232..256).map(|i| sgr(&format!("48;5;{i}"), "  ")).collect());
    lines.push(String::new());
    lines.push("true colour, a smooth ramp:".to_string());
    lines.push((0..64).map(|i| sgr(&format!("48;2;{};{};{}", i * 4, 80, 255 - i * 4), " ")).collect());
    lines.push((0..64).map(|i| sgr(&format!("38;2;{};{};{}", 255 - i * 4, i * 4, 120), "\u{2588}")).collect());
    lines.join("\n")
}

pub fn columns() -> String {
    // Each line filled up to forty cells (a wide character is two), then a bar.
    let bar_at_40 = |text: &str, shown: &str| {
        let cells: usize = text.chars().map(|c| if c as u32 >= 0x1100 { 2 } else { 1 }).sum();
        format!("{shown}{}|", " ".repeat(40usize.saturating_sub(cells)))
    };
    let plain = |text: &str| bar_at_40(text, text);
    [
        "Every line ends with a bar, all in the same column:".to_string(),
        String::new(),
        plain("0123456789012345678901234567890123456789"),
        plain("MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM"),
        plain("iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii"),
        plain("WiWiWiWiWiWiWiWiWiWiWiWiWiWiWiWiWiWiWiWi"),
        plain("v2.1.312  a.b,c;d:e!? (x) [y] {z} <=> !="),
        plain(&"\u{d55c}\u{ae00}".repeat(10)),
        plain(&"\u{65e5}\u{672c}\u{8a9e}\u{4e2d}\u{6587}".repeat(4)),
        plain("\u{d55c}\u{ae00} mixed ascii and \u{6f22}\u{5b57}, then more"),
        plain("short"),
        bar_at_40("bold bold bold bold bold bold bold bold", &sgr("1", "bold bold bold bold bold bold bold bold")),
        bar_at_40("underlined text underlined text under", &sgr("4", "underlined text underlined text under")),
        bar_at_40("inverse, and dim, and struck through", &format!("{}, and {}, and {}", sgr("7", "inverse"), sgr("2", "dim"), sgr("9", "struck through"))),
    ]
    .join("\n")
}

pub fn boxes() -> String {
    [
        "\u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{252c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}  \u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2566}\u{2550}\u{2550}\u{2550}\u{2550}\u{2557}  \u{256d}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256e}",
        "\u{2502} name     \u{2502} size   \u{2502}  \u{2551} left \u{2551} up \u{2551}  \u{2502} soft \u{2502}",
        "\u{251c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{253c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2524}  \u{2560}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{256c}\u{2550}\u{2550}\u{2550}\u{2550}\u{2563}  \u{2502} edge \u{2502}",
        "\u{2502} weft.exe \u{2502}  12 MB \u{2502}  \u{2551} down \u{2551} ok \u{2551}  \u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256f}",
        "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2534}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}  \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2569}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}",
        "",
        "a tree:",
        "\u{251c}\u{2500}\u{2500} src",
        "\u{2502}   \u{251c}\u{2500}\u{2500} main.rs",
        "\u{2502}   \u{2514}\u{2500}\u{2500} ui",
        "\u{2502}       \u{2514}\u{2500}\u{2500} view.rs",
        "\u{2514}\u{2500}\u{2500} Cargo.toml",
        "",
        "heavy and dashed: \u{250f}\u{2501}\u{2501}\u{2501}\u{2513} \u{2504}\u{2504}\u{2504}\u{2504} \u{2508}\u{2508}\u{2508}\u{2508}   arrows: \u{2190} \u{2191} \u{2192} \u{2193}   powerline: \u{e0b0} \u{e0b2}",
    ]
    .join("\n")
}

/// A page: its name, what to look for, and what a program would print.
type Page = (&'static str, &'static str, fn() -> String);

pub const PAGES: [Page; 4] = [
    ("Blocks", "The creature is orange with two small black eyes and nothing else black. The stairs rise and widen evenly, the shades go from light to solid, no red shows around the full blocks, and the two rows of full blocks at the bottom are one solid block with no line between them.", blocks),
    ("Colours", "Each word looks as it is named (bold is heavier, dim is fainter, inverse has its colours swapped). The colour rows are eight different colours each, and the ramps change smoothly without bands or gaps.", colours),
    ("Columns", "Every line's last | stands in the same column, straight under the others: letters, Korean, Japanese and Chinese (two cells each), bold and underlined text all keep to the grid.", columns),
    ("Boxes", "Lines meet at the corners and crossings with no gaps or overhang; the tree's branches join; the frames are closed.", boxes),
];

impl App {
    pub fn ui_display_test(&mut self, ctx: &egui::Context) {
        let Some(mut page) = self.display_test else { return };
        page = page.min(PAGES.len() - 1);
        let mut open = true;
        let font = theme::term_font(self.settings.font_size);
        let which = self.settings.term_font.rsplit(['/', '\\']).next().unwrap_or("").to_string();
        crate::widgets::dialog(ctx, "Display test", vec2(660.0, 500.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, (name, _, _)) in PAGES.iter().enumerate() {
                    if ui.selectable_label(i == page, *name).clicked() {
                        page = i;
                    }
                }
                ui.label(RichText::new(format!("font: {which}, {} pt", self.settings.font_size)).size(11.5).color(theme::text_faint()));
            });
            ui.label(RichText::new(PAGES[page].1).size(12.5).color(theme::text_dim()));
            ui.add_space(4.0);
            // As a pane would show it: the page is fed to a terminal, and
            // its rows are drawn by the terminal's own painter.
            let text = (PAGES[page].2)();
            let rows = text.matches('\n').count() + 1;
            let mut term = Term::new(COLS, rows);
            term.feed(&mut vte::Parser::new(), text.replace('\n', "\r\n").as_bytes());
            let (cw, ch) = ui.fonts(|f| (theme::cell_width(f, &font), f.row_height(&font).ceil()));
            ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(COLS as f32 * cw + 16.0, rows as f32 * ch + 16.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(4), theme::bg_term());
                let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                let (x, y) = ((rect.left() + 8.0).round(), (rect.top() + 8.0).round());
                for (i, row) in term.screen.iter().enumerate() {
                    crate::ui_term::paint_row(&painter, row, x, y + i as f32 * ch, cw, ch, &font, COLS);
                }
            });
        });
        self.display_test = open.then_some(page);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every page fits the test terminal: no line is cut off or wrapped.
    #[test]
    fn the_pages_fit_their_terminal() {
        for (name, look_for, page) in PAGES {
            assert!(!look_for.is_empty());
            let text = page();
            let rows = text.matches('\n').count() + 1;
            let mut term = Term::new(COLS, rows);
            term.feed(&mut vte::Parser::new(), text.replace('\n', "\r\n").as_bytes());
            assert!(term.history.is_empty(), "{name}: nothing scrolled away");
            for (i, row) in term.screen.iter().enumerate() {
                assert!(!row.wrapped, "{name}: line {i} is wider than {COLS} cells");
            }
        }
    }
}
