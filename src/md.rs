//! Markdown that is edited in place: the text stays the raw markdown (the
//! editor edits it as usual), but the layout styles it like the rendered
//! document.  Markup characters are made invisible except on the cursor line,
//! where they show faintly so they can be edited.  Things text styling can't
//! express (bullets, checkboxes, quote bars, code block backgrounds, rules)
//! are returned as `Deco`s anchored to character indices; the editor paints
//! them at the galley positions of those characters.

use crate::theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Stroke};

pub const BOLD_FONT: &str = "bold";

#[derive(Clone, Debug, PartialEq)]
pub enum Deco {
    /// "-", "*" or "+" list marker at this char index: draw a bullet there.
    Bullet { at: usize },
    /// "[ ]" / "[x]" at char index `at`; `byte` is the byte of the inner char (toggle target).
    Check { at: usize, byte: usize, checked: bool },
    /// Block quote line spanning chars [start, end].
    Quote { start: usize, end: usize },
    /// Fenced code block spanning chars [start, end].
    Code { start: usize, end: usize },
    /// Horizontal rule on the line starting at this char.
    Rule { at: usize },
    /// Heading underline below the line ending at this char (H1/H2).
    Underline { at: usize },
    /// ```mermaid block over chars [start, end]. `collapsed`: the code is hidden
    /// and the block's height reserved for the rendered diagram.
    Diagram { start: usize, end: usize, source: String, collapsed: bool },
    /// Rendered table: char index of each row start (header first, separator
    /// row excluded), char index of its end, and the x of each column border
    /// relative to the text's left edge.
    Table { rows: Vec<usize>, end: usize, cols: Vec<f32> },
}

pub struct MdLayout {
    pub job: LayoutJob,
    pub decos: Vec<Deco>,
}

struct Ctx<'a> {
    job: &'a mut LayoutJob,
    cursor_line: bool,
}

#[derive(Clone, Copy)]
struct Style {
    family: Fam,
    size: f32,
    color: Color32,
    italics: bool,
    strike: bool,
    underline: bool,
    background: Color32,
}

#[derive(Clone, Copy, PartialEq)]
enum Fam {
    Body,
    Bold,
    Mono,
}

impl Style {
    fn body(size: f32) -> Style {
        Style { family: Fam::Body, size, color: theme::text(), italics: false, strike: false, underline: false, background: Color32::TRANSPARENT }
    }

    fn format(&self) -> TextFormat {
        let family = match self.family {
            Fam::Body => FontFamily::Proportional,
            Fam::Bold => FontFamily::Name(BOLD_FONT.into()),
            Fam::Mono => FontFamily::Monospace,
        };
        TextFormat {
            font_id: FontId::new(self.size, family),
            color: self.color,
            italics: self.italics,
            strikethrough: if self.strike { Stroke::new(1.0_f32, self.color) } else { Stroke::NONE },
            underline: if self.underline { Stroke::new(1.0_f32, self.color) } else { Stroke::NONE },
            background: self.background,
            ..Default::default()
        }
    }
}

impl Ctx<'_> {
    fn text(&mut self, s: &str, st: Style) {
        if !s.is_empty() {
            self.job.append(s, 0.0, st.format());
        }
    }

    /// Markup: faint on the cursor line, otherwise invisible and (almost) zero-width.
    fn marker(&mut self, s: &str, st: Style) {
        if s.is_empty() {
            return;
        }
        if self.cursor_line {
            self.text(s, Style { color: theme::text_faint(), strike: false, underline: false, background: Color32::TRANSPARENT, ..st });
        } else {
            let f = TextFormat { font_id: FontId::new(1.0, FontFamily::Proportional), color: Color32::TRANSPARENT, ..Default::default() };
            self.job.append(s, 0.0, f);
        }
    }

    /// Invisible but keeps its normal width (room for a painted bullet/checkbox).
    fn blank(&mut self, s: &str, st: Style) {
        self.text(s, Style { color: Color32::TRANSPARENT, background: Color32::TRANSPARENT, strike: false, underline: false, ..st });
    }

    /// Inline markdown: code spans, bold, italic, strikethrough, links, images, autolinks.
    fn inline(&mut self, s: &str, st: Style) {
        let b = s.as_bytes();
        let mut plain_start = 0;
        let mut i = 0;
        while i < b.len() {
            let rest = &s[i..];
            let mut handled: Option<usize> = None; // bytes consumed by a span
            if rest.starts_with('`') {
                if let Some(e) = rest[1..].find('`') {
                    self.text(&s[plain_start..i], st);
                    self.marker("`", st);
                    let code = Style { family: Fam::Mono, size: st.size * 0.92, background: theme::tint(theme::text_faint(), 45), ..st };
                    self.text(&rest[1..1 + e], code);
                    self.marker("`", st);
                    handled = Some(e + 2);
                }
            } else if rest.starts_with("**") || rest.starts_with("__") || rest.starts_with("~~") {
                let m = &rest[..2];
                if let Some(e) = rest[2..].find(m).filter(|&e| e > 0) {
                    self.text(&s[plain_start..i], st);
                    self.marker(m, st);
                    let inner = if m == "~~" { Style { strike: true, color: theme::text_dim(), ..st } } else { Style { family: Fam::Bold, ..st } };
                    self.inline(&rest[2..2 + e], inner);
                    self.marker(m, st);
                    handled = Some(e + 4);
                }
            } else if rest.starts_with('*') || (rest.starts_with('_') && (i == 0 || !b[i - 1].is_ascii_alphanumeric())) {
                let m = &rest[..1];
                let close = rest[1..].find(m).filter(|&e| {
                    e > 0 && !rest[1..].starts_with(' ') && (m == "*" || rest.as_bytes().get(e + 2).map(|c| !c.is_ascii_alphanumeric()).unwrap_or(true))
                });
                if let Some(e) = close {
                    self.text(&s[plain_start..i], st);
                    self.marker(m, st);
                    self.inline(&rest[1..1 + e], Style { italics: true, ..st });
                    self.marker(m, st);
                    handled = Some(e + 2);
                }
            } else if rest.starts_with("![") || rest.starts_with('[') {
                let img = rest.starts_with('!');
                let open = if img { 2 } else { 1 };
                if let Some(close) = rest[open..].find("](") {
                    let label_end = open + close;
                    if let Some(paren) = rest[label_end + 2..].find(')') {
                        let end = label_end + 2 + paren + 1;
                        self.text(&s[plain_start..i], st);
                        self.marker(&rest[..open], st);
                        let label = &rest[open..label_end];
                        if img {
                            self.text(label, Style { italics: true, color: theme::text_dim(), ..st });
                        } else {
                            self.inline(label, Style { color: theme::accent(), underline: true, ..st });
                        }
                        self.marker(&rest[label_end..end], st);
                        handled = Some(end);
                    }
                }
            } else if rest.starts_with("<http") {
                if let Some(e) = rest.find('>') {
                    self.text(&s[plain_start..i], st);
                    self.marker("<", st);
                    self.text(&rest[1..e], Style { color: theme::accent(), underline: true, ..st });
                    self.marker(">", st);
                    handled = Some(e + 1);
                }
            }
            match handled {
                Some(n) => {
                    i += n;
                    plain_start = i;
                }
                None => i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1),
            }
        }
        self.text(&s[plain_start..], st);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Center,
    Right,
}

/// A table being laid out (cursor outside it): where its columns start.
struct Table {
    header: usize,
    end: usize,
    aligns: Vec<Align>,
    widths: Vec<f32>,
    col_x: Vec<f32>,
    cols: Vec<f32>,
    rows: Vec<usize>,
}

/// Cells of a `| a | b |` row (outer pipes optional).
fn table_cells(line: &str) -> Vec<&str> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').collect()
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.contains('|') && t.chars().all(|c| "|-: ".contains(c))
}

/// Width of the sections from `from` on, as laid out without wrapping.
fn sections_width(job: &LayoutJob, from: usize, measure: &mut dyn FnMut(&str, &FontId) -> f32) -> f32 {
    job.sections[from..].iter().map(|s| s.leading_space + measure(&job.text[s.byte_range.clone()], &s.format.font_id)).sum()
}

fn heading_level(line: &str) -> Option<(usize, usize)> {
    let hashes = line.bytes().take_while(|&c| c == b'#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes).map(|&c| c == b' ').unwrap_or(line.len() == hashes) {
        Some((hashes, (hashes + 1).min(line.len())))
    } else {
        None
    }
}

fn is_rule(t: &str) -> bool {
    let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}

/// An open fenced block: closing marker, first char, and for a mermaid
/// block its source and the height reserved for the diagram (if collapsed).
struct Fence {
    marker: String,
    start: usize,
    mermaid: Option<(String, Option<f32>)>,
}

/// Lay out `text` as in-place-rendered markdown. `cursor_line` (0-based) shows
/// its markup. `diagram` gives the height to reserve for a rendered mermaid
/// source (None: show it as code).
///
/// `measure` gives the unwrapped width of a text in a font (aligns table columns).
pub fn layout(text: &str, cursor_line: Option<usize>, size: f32, diagram: &mut dyn FnMut(&str) -> Option<f32>, measure: &mut dyn FnMut(&str, &FontId) -> f32) -> MdLayout {
    let mut job = LayoutJob::default();
    let mut decos = Vec::new();
    let mut char_pos = 0usize; // char index of the current line start
    let mut byte_pos = 0usize;
    let mut fence: Option<Fence> = None;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let soft = soft_breaks(&lines, cursor_line);
    let mut table: Option<Table> = None;
    for (ln, raw_line) in lines.iter().copied().enumerate() {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let newline = &raw_line[line.len()..];
        let line_chars = line.chars().count();
        let mut cx = Ctx { job: &mut job, cursor_line: cursor_line == Some(ln) };
        let body = Style::body(size);
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        let char_at = |byte: usize| char_pos + line[..byte].chars().count();
        let mut tiny_newline = false;

        if fence.is_none() && table.is_none() && trimmed.starts_with('|') && lines.get(ln + 1).map(|l| is_table_separator(l)).unwrap_or(false) {
            let end = (ln + 2..lines.len()).find(|&i| !lines[i].trim_start().starts_with('|')).unwrap_or(lines.len());
            if !cursor_line.map(|c| (ln..end).contains(&c)).unwrap_or(false) {
                let aligns: Vec<Align> = table_cells(lines[ln + 1])
                    .iter()
                    .map(|c| {
                        let c = c.trim();
                        match (c.starts_with(':'), c.ends_with(':')) {
                            (true, true) => Align::Center,
                            (false, true) => Align::Right,
                            _ => Align::Left,
                        }
                    })
                    .collect();
                let mut widths = vec![0.0_f32; aligns.len()];
                for (i, l) in lines[ln..end].iter().enumerate() {
                    if i == 1 {
                        continue;
                    }
                    let st = if i == 0 { Style { family: Fam::Bold, ..body } } else { body };
                    for (c, cell) in table_cells(l).iter().enumerate().take(widths.len()) {
                        let mut scratch = LayoutJob::default();
                        Ctx { job: &mut scratch, cursor_line: false }.inline(cell.trim(), st);
                        widths[c] = widths[c].max(sections_width(&scratch, 0, measure));
                    }
                }
                let pad = size * 0.6;
                let (mut cols, mut col_x) = (vec![0.0_f32], Vec::new());
                for w in &widths {
                    let left = *cols.last().unwrap();
                    col_x.push(left + pad);
                    cols.push(left + w + 2.0 * pad);
                }
                table = Some(Table { header: ln, end, aligns, widths, col_x, cols, rows: Vec::new() });
            }
        }

        if let Some(f) = fence.as_ref() {
            let collapsed = f.mermaid.as_ref().and_then(|m| m.1);
            if trimmed.starts_with(&f.marker) {
                let end = char_pos + line_chars;
                match collapsed {
                    Some(h) => {
                        // One invisible glyph as tall as the diagram reserves its space.
                        let first = line.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                        let tall = TextFormat { font_id: FontId::new((h / 1.3).max(1.0), FontFamily::Proportional), color: Color32::TRANSPARENT, ..Default::default() };
                        cx.job.append(&line[..first], 0.0, tall);
                        cx.marker(&line[first..], body);
                    }
                    None => {
                        cx.marker(line, Style { family: Fam::Mono, ..body });
                        decos.push(Deco::Code { start: f.start, end });
                    }
                }
                if let Some((source, _)) = &f.mermaid {
                    decos.push(Deco::Diagram { start: f.start, end, source: source.clone(), collapsed: collapsed.is_some() });
                }
                fence = None;
            } else if collapsed.is_some() {
                cx.marker(line, body);
            } else {
                cx.text(line, Style { family: Fam::Mono, size: size * 0.92, ..body });
            }
        } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = trimmed[..3].to_string();
            let lang = trimmed[3..].trim();
            let mut mermaid = None;
            if lang.eq_ignore_ascii_case("mermaid") {
                let close = lines.iter().enumerate().skip(ln + 1).find(|(_, l)| l.trim_start().starts_with(&marker)).map(|(i, _)| i);
                if let Some(c) = close {
                    let source: String = lines[ln + 1..c].concat();
                    let editing = cursor_line.map(|cl| (ln..=c).contains(&cl)).unwrap_or(false);
                    let h = if editing { None } else { diagram(&source) };
                    mermaid = Some((source, h));
                }
            }
            fence = Some(Fence { marker, start: char_pos, mermaid });
            cx.marker(line, Style { family: Fam::Mono, ..body });
        } else if let Some((level, text_start)) = heading_level(line) {
            let scale = [1.8, 1.5, 1.3, 1.15, 1.05, 1.0][level - 1];
            let st = Style { family: Fam::Bold, size: size * scale, ..body };
            cx.marker(&line[..text_start], st);
            cx.inline(&line[text_start..], st);
            if level <= 2 && !cx.cursor_line {
                decos.push(Deco::Underline { at: char_pos + line_chars });
            }
        } else if is_rule(line) && !line.is_empty() {
            if cx.cursor_line {
                cx.marker(line, body);
            } else {
                cx.blank(line, body);
                decos.push(Deco::Rule { at: char_pos });
            }
        } else if trimmed.starts_with('>') {
            let n = trimmed.bytes().take_while(|&c| c == b'>' || c == b' ').count();
            cx.text(indent, body);
            if cx.cursor_line {
                cx.marker(&trimmed[..n], body);
            } else {
                // Keep the width: the quote bar is painted there.
                cx.blank(&trimmed[..n], body);
            }
            cx.inline(&trimmed[n..], Style { color: theme::text_dim(), italics: true, ..body });
            decos.push(Deco::Quote { start: char_pos, end: char_pos + line_chars });
        } else if (trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ")) && !is_rule(trimmed) {
            let marker_byte = indent.len();
            cx.text(indent, body);
            if cx.cursor_line {
                cx.marker(&trimmed[..2], body);
            } else {
                cx.blank(&trimmed[..2], body);
                decos.push(Deco::Bullet { at: char_at(marker_byte) });
            }
            let item = &trimmed[2..];
            let task = item.len() >= 3 && item.starts_with('[') && item.as_bytes()[2] == b']' && matches!(item.as_bytes()[1], b' ' | b'x' | b'X');
            if task {
                let checked = item.as_bytes()[1] != b' ';
                let box_byte = marker_byte + 2;
                if cx.cursor_line {
                    cx.marker(&item[..3], body);
                } else {
                    cx.blank(&item[..3], body);
                }
                decos.push(Deco::Check { at: char_at(box_byte), byte: byte_pos + box_byte + 1, checked });
                let st = if checked { Style { strike: true, color: theme::text_faint(), ..body } } else { body };
                cx.inline(&item[3..], st);
            } else {
                cx.inline(item, body);
            }
        } else if let Some(t) = table.as_mut() {
            if ln == t.header + 1 {
                // The separator row: hidden and (almost) no height.
                cx.marker(line, body);
                tiny_newline = true;
            } else {
                t.rows.push(char_pos);
                let st = if ln == t.header { Style { family: Fam::Bold, ..body } } else { body };
                let row_first = cx.job.sections.len();
                let lead = line.len() - line.trim_start().len();
                let inner = line.trim();
                let has_open = inner.starts_with('|');
                cx.marker(&line[..lead], body);
                let mut rest = inner;
                if has_open {
                    cx.marker("|", body);
                    rest = &rest[1..];
                }
                let has_close = rest.ends_with('|');
                if has_close {
                    rest = &rest[..rest.len() - 1];
                }
                let cells = table_cells(line).len();
                for (c, part) in rest.splitn(cells, '|').enumerate() {
                    if c > 0 {
                        cx.marker("|", body);
                    }
                    let content = part.trim();
                    let before = part.len() - part.trim_start().len();
                    cx.marker(&part[..before], body);
                    let x_now = sections_width(cx.job, row_first, measure);
                    let s0 = cx.job.sections.len();
                    cx.inline(content, st);
                    if c < t.col_x.len() && cx.job.sections.len() > s0 {
                        let slack = (t.widths[c] - sections_width(cx.job, s0, measure)).max(0.0);
                        let off = match t.aligns[c] {
                            Align::Left => 0.0,
                            Align::Center => slack / 2.0,
                            Align::Right => slack,
                        };
                        cx.job.sections[s0].leading_space = (t.col_x[c] + off - x_now).max(0.0);
                    }
                    cx.marker(&part[before + content.len()..], body);
                }
                if has_close {
                    cx.marker("|", body);
                }
                cx.marker(&line[lead + inner.len()..], body);
            }
            if ln + 1 == t.end {
                let t = table.take().unwrap();
                decos.push(Deco::Table { rows: t.rows, end: char_pos + line_chars, cols: t.cols });
            }
        } else if trimmed.starts_with('|') {
            // Tables: monospace so columns line up; pipes and the separator row faint.
            let mono = Style { family: Fam::Mono, size: size * 0.92, ..body };
            let sep = trimmed.chars().all(|c| "|-: ".contains(c));
            for (k, part) in line.split('|').enumerate() {
                if k > 0 {
                    cx.text("|", Style { color: theme::text_faint(), ..mono });
                }
                if sep {
                    cx.text(part, Style { color: theme::text_faint(), ..mono });
                } else {
                    cx.inline(part, mono);
                }
            }
        } else {
            let ordered = trimmed.bytes().take_while(|c| c.is_ascii_digit()).count();
            if ordered > 0 && trimmed[ordered..].starts_with(". ") {
                cx.text(indent, body);
                cx.text(&trimmed[..ordered + 2], Style { color: theme::text_dim(), ..body });
                cx.inline(&trimmed[ordered + 2..], body);
            } else {
                cx.inline(line, body);
            }
        }
        if !newline.is_empty() {
            // Keep the line's own height for the newline (headings stay tall).
            let collapsed = fence.as_ref().and_then(|f| f.mermaid.as_ref()).and_then(|m| m.1).is_some();
            let st = if fence.is_some() && !collapsed { Style { family: Fam::Mono, ..Style::body(size) } } else { Style::body(size) };
            if tiny_newline {
                cx.marker(newline, st);
            } else if soft[ln] {
                // A single newline inside a paragraph is a space, as in CommonMark
                // (same char count, so cursor positions still match the source).
                cx.text(&newline.replace('\n', " "), Style { color: Color32::TRANSPARENT, ..st });
            } else {
                cx.text(newline, Style { color: Color32::TRANSPARENT, ..st });
            }
        }
        char_pos += line_chars + newline.chars().count();
        byte_pos += raw_line.len();
    }
    if let Some(f) = fence {
        decos.push(Deco::Code { start: f.start, end: char_pos });
    }
    MdLayout { job, decos }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Para,
    Item,
    Quote,
    Other,
}

/// Per line: does its newline join the next line into the same paragraph?
/// Not for hard breaks (two trailing spaces or a backslash), nor anywhere in
/// the paragraph holding the cursor, which shows its lines as written.
fn soft_breaks(lines: &[&str], cursor_line: Option<usize>) -> Vec<bool> {
    let mut in_fence: Option<String> = None;
    let kinds: Vec<Kind> = lines
        .iter()
        .map(|raw| {
            let line = raw.trim_end_matches(['\n', '\r']);
            let t = line.trim_start();
            if let Some(m) = &in_fence {
                if t.starts_with(m.as_str()) {
                    in_fence = None;
                }
                return Kind::Other;
            }
            if t.starts_with("```") || t.starts_with("~~~") {
                in_fence = Some(t[..3].to_string());
                return Kind::Other;
            }
            let ordered = t.bytes().take_while(|c| c.is_ascii_digit()).count();
            if t.is_empty() || heading_level(line).is_some() || is_rule(line) || t.starts_with('|') || line.starts_with("    ") || line.starts_with('\t') {
                Kind::Other
            } else if t.starts_with('>') {
                Kind::Quote
            } else if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") || (ordered > 0 && t[ordered..].starts_with(". ")) {
                Kind::Item
            } else {
                Kind::Para
            }
        })
        .collect();
    let mut soft: Vec<bool> = (0..lines.len())
        .map(|i| {
            let Some(&next) = kinds.get(i + 1) else { return false };
            let line = lines[i].trim_end_matches(['\n', '\r']);
            let hard = line.ends_with("  ") || line.ends_with('\\');
            !hard && kinds[i] != Kind::Other && (next == Kind::Para || (kinds[i] == Kind::Quote && next == Kind::Quote))
        })
        .collect();
    if let Some(c) = cursor_line.filter(|&c| c < lines.len()) {
        let mut a = c;
        while a > 0 && soft[a - 1] {
            a -= 1;
        }
        let mut b = c;
        while b < lines.len() && soft[b] {
            b += 1;
        }
        for v in &mut soft[a..b.min(lines.len())] {
            *v = false;
        }
    }
    soft
}

/// Byte offset of `line` (0-based) and the 0-based line of a char index.
pub fn line_of_char(text: &str, char_index: usize) -> usize {
    text.chars().take(char_index).filter(|&c| c == '\n').count()
}

pub fn is_markdown(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".md") || p.ends_with(".markdown") || p.ends_with(".mdx")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono(t: &str, f: &FontId) -> f32 {
        t.chars().count() as f32 * f.size * 0.5
    }

    fn visible(l: &MdLayout) -> String {
        // Concatenate the text of sections that are not invisible markup.
        l.job
            .sections
            .iter()
            .filter(|s| s.format.color != Color32::TRANSPARENT)
            .map(|s| &l.job.text[s.byte_range.clone()])
            .collect()
    }

    #[test]
    fn layout_keeps_every_byte_of_the_source() {
        let src = "# 제목\n\n**굵게** 와 *기울임*, `code`, [링크](http://x)\n- [ ] 할 일\n- [x] 끝\n> 인용\n```rs\nfn main() {}\n```\n---\n| a | b |\n";
        let l = layout(src, None, 14.0, &mut |_| None, &mut mono);
        assert_eq!(l.job.text, src, "the editor needs the exact source text");
        assert_eq!(layout("a\nb", None, 14.0, &mut |_| None, &mut mono).job.text, "a b");
        let covered: usize = l.job.sections.iter().map(|s| s.byte_range.len()).sum();
        assert_eq!(covered, src.len());
    }

    #[test]
    fn tables_hide_pipes_and_align_columns() {
        let src = "| name | n |\n|:--|--:|\n| a | 10 |\n| longer | 2 |\n";
        let l = layout(src, None, 10.0, &mut |_| None, &mut mono);
        assert_eq!(l.job.text, src);
        let Some(Deco::Table { rows, cols, .. }) = l.decos.iter().find(|d| matches!(d, Deco::Table { .. })) else { panic!("no table") };
        assert_eq!(rows.len(), 3, "header + 2 rows, separator excluded");
        assert_eq!(cols.len(), 3);
        // Cell text starts at the same x in every row of a left-aligned column.
        let x_of = |needle: &str| {
            let at = src.find(needle).unwrap();
            let row = src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let first = l.job.sections.iter().position(|s| s.byte_range.start >= row).unwrap();
            let me = l.job.sections.iter().position(|s| s.byte_range.start == at).unwrap();
            sections_width(&l.job, first, &mut mono) - sections_width(&l.job, me, &mut mono)
        };
        assert!((x_of("a ") - x_of("longer")).abs() < 0.01);
        assert!((x_of("a ") - x_of("name")).abs() < 0.01);
        // Visible text has no pipes.
        let shown: String = l.job.sections.iter().filter(|s| s.format.color != Color32::TRANSPARENT).map(|s| &l.job.text[s.byte_range.clone()]).collect();
        assert!(!shown.contains('|'), "{shown}");
        // With the cursor inside, the table is raw.
        let raw = layout(src, Some(2), 10.0, &mut |_| None, &mut mono);
        assert!(!raw.decos.iter().any(|d| matches!(d, Deco::Table { .. })));
    }

    #[test]
    fn single_newlines_join_a_paragraph() {
        let src = "one\ntwo  \nthree\n\nfour\n- item\nmore\n# h\nx\n```\na\nb\n```\n";
        let l = layout(src, None, 14.0, &mut |_| None, &mut mono);
        assert_eq!(l.job.text.chars().count(), src.chars().count(), "cursor positions must match the source");
        let lines: Vec<&str> = l.job.text.split('\n').collect();
        assert_eq!(&lines[..6], ["one two  ", "three", "", "four", "- item more", "# h"]);
        assert_eq!(&lines[6..10], ["x", "```", "a", "b"]);
        // The paragraph holding the cursor keeps its lines.
        let l = layout(src, Some(1), 14.0, &mut |_| None, &mut mono);
        assert!(l.job.text.starts_with("one\ntwo  \nthree\n"));
    }

    #[test]
    fn markup_hidden_except_on_cursor_line() {
        let src = "## Title **bold**\nnext";
        let away = visible(&layout(src, Some(1), 14.0, &mut |_| None, &mut mono));
        assert!(!away.contains('#') && !away.contains("**"), "{away}");
        assert!(away.contains("Title") && away.contains("bold"));
        let here = visible(&layout(src, Some(0), 14.0, &mut |_| None, &mut mono));
        assert!(here.contains("## ") && here.contains("**bold**"), "{here}");
    }

    #[test]
    fn decorations_for_blocks() {
        let src = "- a\n- [x] done\n> q\n```\ncode\n```\n---\n# H\n";
        let l = layout(src, None, 14.0, &mut |_| None, &mut mono);
        assert!(l.decos.contains(&Deco::Bullet { at: 0 }));
        let check = l.decos.iter().find_map(|d| match d {
            Deco::Check { byte, checked, .. } => Some((*byte, *checked)),
            _ => None,
        });
        assert_eq!(check, Some((7, true)));
        assert_eq!(&src[7..8], "x");
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Quote { .. })));
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Code { .. })));
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Rule { .. })));
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Underline { .. })));
    }

    #[test]
    fn mermaid_block_collapses_into_reserved_space() {
        let src = "text\n```mermaid\ngraph TD; A-->B\n```\nafter\n";
        let mut asked = Vec::new();
        let l = layout(src, Some(0), 14.0, &mut |s: &str| {
            asked.push(s.to_string());
            Some(120.0)
        }, &mut mono);
        assert_eq!(asked, vec!["graph TD; A-->B\n".to_string()]);
        assert_eq!(l.job.text, src);
        assert!(!visible(&l).contains("graph"), "code hidden when the diagram is shown");
        assert!(l.job.sections.iter().any(|s| s.format.font_id.size > 80.0), "space reserved");
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Diagram { collapsed: true, .. })));
        // Cursor inside the block: edit the source, no diagram requested.
        let mut asked = 0;
        let l = layout(src, Some(2), 14.0, &mut |_| {
            asked += 1;
            Some(120.0)
        }, &mut mono);
        assert_eq!(asked, 0);
        assert!(visible(&l).contains("graph"));
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Diagram { collapsed: false, .. })));
    }

    #[test]
    fn snake_case_is_not_italic() {
        let l = layout("call my_func_name now", None, 14.0, &mut |_| None, &mut mono);
        assert!(l.job.sections.iter().all(|s| !s.format.italics));
    }
}
