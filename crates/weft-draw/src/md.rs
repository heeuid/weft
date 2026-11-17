//! Markdown that is edited in place: the text stays the raw markdown (the
//! editor edits it as usual), but the layout styles it like the rendered
//! document.  Markup characters are made invisible except on the cursor line,
//! where they show faintly so they can be edited.  Things text styling can't
//! express (bullets, checkboxes, quote bars, code block backgrounds, rules)
//! are returned as `Deco`s anchored to character indices; the editor paints
//! them at the galley positions of those characters.

use crate::theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Galley, epaint::text::Row};
use std::sync::Arc;
use egui::{Color32, FontFamily, FontId, Stroke};

pub const BOLD_FONT: &str = "bold";

#[derive(Clone, Debug, PartialEq)]
pub enum Deco {
    /// "-", "*" or "+" list marker at this char index: draw a bullet in the
    /// `col` wide column that ends where the marker is, its shape by
    /// `depth` (filled dot, hollow dot, filled square, hollow square, again).
    Bullet { at: usize, depth: usize, col: f32 },
    /// An ordered item's number, written as its depth wants ("3.", "iii.",
    /// "c."), right-aligned just before the marker at this char index.
    Number { at: usize, label: String },
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
    /// Rendered table, drawn over the room its first line reserves (char
    /// `at`): every cell wrapped in its column (see `Grid`; x and y from the
    /// text's left edge and the table's top), and each row's source chars.
    Table { at: usize, grid: Grid, rows: Vec<(usize, usize)> },
}

pub struct MdLayout {
    pub job: LayoutJob,
    pub decos: Vec<Deco>,
    /// Where a list item's paragraph starts (byte) and the x its text
    /// starts at, which its wrapped rows go back to (`lay_out_hanging`).
    pub hangs: Vec<Hang>,
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

    /// A list item's indent and marker, its text then starting `tx` from the
    /// left edge whatever spaces the source has. On the cursor line both show
    /// (faint), the room made up before the marker so the text stays where
    /// it is; elsewhere they take none (the marker is painted in the room).
    fn lead_in(&mut self, indent: &str, marker: &str, tx: f32, st: Style, measure: &mut dyn FnMut(&str, &FontId) -> f32) {
        if self.cursor_line {
            self.text(indent, st);
            let font = st.format().font_id;
            let raw = measure(indent, &font) + measure(marker, &font);
            let faint = Style { color: theme::text_faint(), strike: false, underline: false, background: Color32::TRANSPARENT, ..st };
            self.job.append(marker, (tx - raw).max(0.0), faint.format());
        } else {
            // (Hidden, they still take a hair each: taken off the room, so
            // the text is where it is on the cursor line, to the point.)
            let tiny = TextFormat { font_id: FontId::new(1.0, FontFamily::Proportional), color: Color32::TRANSPARENT, ..Default::default() };
            let hair = measure(indent, &tiny.font_id) + measure(marker, &tiny.font_id);
            if !indent.is_empty() {
                self.job.append(indent, 0.0, tiny.clone());
            }
            self.job.append(marker, (tx - hair).max(0.0), tiny);
        }
    }

    /// A line's indent, hidden, its text then starting `x` from the left edge.
    fn indent_to(&mut self, indent: &str, x: f32, measure: &mut dyn FnMut(&str, &FontId) -> f32) {
        let tiny = TextFormat { font_id: FontId::new(1.0, FontFamily::Proportional), color: Color32::TRANSPARENT, ..Default::default() };
        let hair = measure(indent, &tiny.font_id);
        self.job.append(indent, (x - hair).max(0.0), tiny);
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
            if let Some(after) = rest.strip_prefix('`') {
                if let Some(e) = after.find('`') {
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
            } else if rest.starts_with("<!--") {
                if let Some(e) = rest.find("-->") {
                    self.text(&s[plain_start..i], st);
                    self.marker(&rest[..e + 3], st);
                    handled = Some(e + 3);
                }
            } else if let Some(tag) = html_tag(rest) {
                self.text(&s[plain_start..i], st);
                handled = Some(self.html(rest, &tag, st));
            } else if rest.starts_with('&') && !self.cursor_line {
                if let Some((n, c)) = entity(rest) {
                    // The entity shows as its character: same char count, so
                    // cursor positions still match the source.
                    self.text(&s[plain_start..i], st);
                    self.job.append(c.encode_utf8(&mut [0; 4]), 0.0, st.format());
                    self.marker(&rest[1..n], st);
                    handled = Some(n);
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

/// A table being laid out (cursor outside it): the lines it spans, the
/// height it takes, and its cells (taken into the Deco at its last line).
struct Table {
    header: usize,
    end: usize,
    height: f32,
    grid: Option<Grid>,
    /// Char index of the table's first character, and each row's source.
    at: usize,
    row_chars: Vec<(usize, usize)>,
}

/// Cells by row (each laid out to wrap in its column, and its x), column
/// borders' x, rows' top y (and the bottom), the padding above a row's text.
pub type Grid = (Vec<Vec<(LayoutJob, f32)>>, Vec<f32>, Vec<f32>, f32);

/// Column widths that fit `room`: as wide as their content when all fit;
/// otherwise the columns narrower than an even share keep their width and
/// the rest share what is left (never under `min`).
fn fit_columns(natural: &[f32], room: f32, min: f32) -> Vec<f32> {
    if natural.iter().sum::<f32>() <= room || natural.is_empty() {
        return natural.to_vec();
    }
    let mut order: Vec<usize> = (0..natural.len()).collect();
    order.sort_by(|a, b| natural[*a].total_cmp(&natural[*b]));
    let mut out = natural.to_vec();
    let mut left = room;
    for (k, &c) in order.iter().enumerate() {
        let share = left / (order.len() - k) as f32;
        if natural[c] <= share {
            left -= natural[c];
        } else {
            for &rest in &order[k..] {
                out[rest] = share.max(min);
            }
            break;
        }
    }
    out
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

/// HTML tags rendered inside markdown (others stay text, so `Vec<T>` is safe).
const HTML_TAGS: &[&str] = &[
    "a", "abbr", "article", "b", "big", "blockquote", "br", "center", "cite", "code", "dd", "del", "details", "dfn", "div", "dl", "dt", "em", "font", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "i", "img", "ins", "kbd", "li", "mark", "ol", "p", "picture", "pre", "q", "s", "samp", "section", "small", "source", "span", "strike", "strong", "sub", "summary", "sup", "table", "tbody", "td", "th", "thead", "tr", "tt", "u", "ul", "var", "wbr",
];

struct Tag<'a> {
    len: usize,
    name: String,
    closing: bool,
    attrs: &'a str,
}

/// A known HTML tag at the start of `s`.
fn html_tag(s: &str) -> Option<Tag<'_>> {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') {
        return None;
    }
    let closing = b.get(1) == Some(&b'/');
    let start = if closing { 2 } else { 1 };
    let mut i = start;
    while i < b.len() && b[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == start || !b[start].is_ascii_alphabetic() || !matches!(b.get(i), Some(b'>' | b'/' | b' ' | b'\t')) {
        return None;
    }
    let name = s[start..i].to_ascii_lowercase();
    if !HTML_TAGS.contains(&name.as_str()) {
        return None;
    }
    let end = i + s[i..].find('>')?;
    if s[i..end].contains(['<', '\n']) {
        return None;
    }
    Some(Tag { len: end + 1, name, closing, attrs: &s[i..end] })
}

/// Byte range of attribute `key`'s quoted value inside `attrs`.
fn attr_value(attrs: &str, key: &str) -> Option<(usize, usize)> {
    let lower = attrs.to_ascii_lowercase();
    let at = lower.match_indices(key).map(|(i, _)| i).find(|&i| (i == 0 || attrs.as_bytes()[i - 1].is_ascii_whitespace()) && attrs[i + key.len()..].trim_start().starts_with('='))?;
    let after = &attrs[at + key.len()..];
    let eq = after.find('=')?;
    let val = after[eq + 1..].trim_start();
    let quote = val.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let start = attrs.len() - val.len() + 1;
    let end = start + val[1..].find(quote)?;
    Some((start, end))
}

/// `&name;` / `&#123;` / `&#x1F;`: its length and character.
fn entity(s: &str) -> Option<(usize, char)> {
    let end = s.bytes().take(12).position(|c| c == b';')?;
    let name = &s[1..end];
    if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'#') {
        return None;
    }
    let c = match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "middot" => '·',
        "times" => '×',
        "deg" => '°',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        _ => {
            let n = name.strip_prefix('#')?;
            let v = match n.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(v).filter(|c| !c.is_control())?
        }
    };
    Some((end + 1, c))
}

/// The style an HTML element gives its content.
fn html_style(name: &str, st: Style) -> Style {
    match name {
        "b" | "strong" | "summary" | "th" | "dt" => Style { family: Fam::Bold, ..st },
        "i" | "em" | "cite" | "var" | "dfn" | "q" => Style { italics: true, ..st },
        "u" | "ins" => Style { underline: true, ..st },
        "s" | "del" | "strike" => Style { strike: true, color: theme::text_dim(), ..st },
        "code" | "kbd" | "samp" | "tt" => Style { family: Fam::Mono, size: st.size * 0.92, background: theme::tint(theme::text_faint(), 45), ..st },
        "mark" => Style { background: theme::tint(theme::warn(), 70), ..st },
        "a" => Style { color: theme::accent(), underline: true, ..st },
        "sub" | "sup" | "small" => Style { size: st.size * 0.8, ..st },
        "big" => Style { size: st.size * 1.15, ..st },
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = name.as_bytes()[1] - b'1';
            Style { family: Fam::Bold, size: st.size * [1.8, 1.5, 1.3, 1.15, 1.05, 1.0][level as usize], ..st }
        }
        _ => st,
    }
}

/// A line made only of HTML tags and comments (wrappers like `<div align=center>`).
fn html_only(t: &str) -> bool {
    let mut rest = t.trim();
    let mut any = false;
    while !rest.is_empty() {
        let n = if rest.starts_with("<!--") { rest.find("-->").map(|e| e + 3) } else { html_tag(rest).filter(|g| !matches!(g.name.as_str(), "br" | "img" | "hr")).map(|g| g.len) };
        match n {
            Some(n) => {
                rest = rest[n..].trim_start();
                any = true;
            }
            None => return false,
        }
    }
    any
}

fn is_html_rule(t: &str) -> bool {
    html_tag(t.trim()).map(|g| g.name == "hr" && !g.closing && g.len == t.trim().len()).unwrap_or(false)
}

impl Ctx<'_> {
    /// An HTML tag at the start of `rest`: hide the tag, style its content.
    /// Returns the bytes consumed.
    fn html(&mut self, rest: &str, tag: &Tag, st: Style) -> usize {
        let open = &rest[..tag.len];
        if tag.closing {
            self.marker(open, st);
            return tag.len;
        }
        match tag.name.as_str() {
            "br" => {
                if self.cursor_line {
                    self.marker(open, st);
                } else {
                    // A real line break in place of `<` (same char count).
                    let f = TextFormat { font_id: FontId::new(1.0, FontFamily::Proportional), color: Color32::TRANSPARENT, ..Default::default() };
                    self.job.append("\n", 0.0, f);
                    self.marker(&open[1..], st);
                }
                return tag.len;
            }
            "img" => {
                let off = tag.attrs.as_ptr() as usize - rest.as_ptr() as usize;
                match attr_value(tag.attrs, "alt").filter(|(a, b)| b > a) {
                    Some((a, b)) => {
                        self.marker(&open[..off + a], st);
                        self.text(&open[off + a..off + b], Style { italics: true, color: theme::text_dim(), ..st });
                        self.marker(&open[off + b..], st);
                    }
                    None => self.marker(open, st),
                }
                return tag.len;
            }
            "hr" | "wbr" | "source" => {
                self.marker(open, st);
                return tag.len;
            }
            _ => {}
        }
        let close = format!("</{}>", tag.name);
        if let Some(e) = rest[tag.len..].to_ascii_lowercase().find(&close) {
            let e = tag.len + e;
            self.marker(open, st);
            self.inline(&rest[tag.len..e], html_style(&tag.name, st));
            self.marker(&rest[e..e + close.len()], st);
            return e + close.len();
        }
        self.marker(open, st);
        tag.len
    }
}

/// Fenced code blocks as (opening line, closing line).
///
/// Plain markdown pairs fences from the top, so typing or deleting one fence
/// re-pairs every block below it and flips the rest of the document between
/// code and text. While editing (cursor known) blocks are paired from the
/// cursor outwards instead: from the top down to the cursor line, and from the
/// bottom up below it. A fence being typed or deleted at the cursor can't
/// disturb the blocks around it: unmatched, it stays plain text until its
/// partner is typed. An opener above and a closer below the cursor (editing
/// inside a block) still pair.
pub fn fence_blocks(lines: &[&str], cursor: Option<usize>) -> Vec<(usize, usize)> {
    // (line, marker char, bare: nothing but marker characters)
    let fences: Vec<(usize, char, bool)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim_end_matches(['\n', '\r']).trim();
            let c = if t.starts_with("```") { '`' } else if t.starts_with("~~~") { '~' } else { return None };
            Some((i, c, t.chars().all(|x| x == c)))
        })
        .collect();
    // Top down: a bare fence of the same kind closes; anything else inside is content.
    let top_down = |fs: &[(usize, char, bool)]| {
        let mut pairs = Vec::new();
        let mut open: Option<(usize, char)> = None;
        for &(i, c, bare) in fs {
            match open {
                Some((o, oc)) if c == oc && bare => {
                    pairs.push((o, i));
                    open = None;
                }
                Some(_) => {}
                None => open = Some((i, c)),
            }
        }
        (pairs, open)
    };
    let Some(cursor) = cursor else { return top_down(&fences).0 };
    let split = fences.iter().position(|f| f.0 > cursor).unwrap_or(fences.len());
    let (mut pairs, open_above) = top_down(&fences[..split]);
    // Bottom up: a bare fence waits for its opener; a fence with a language
    // and no closer below is plain text.
    let mut closer: Option<(usize, char)> = None;
    for &(i, c, bare) in fences[split..].iter().rev() {
        match closer {
            Some((cl, cc)) if c == cc => {
                pairs.push((i, cl));
                closer = None;
            }
            Some(_) => {}
            None if bare => closer = Some((i, c)),
            None => {}
        }
    }
    if let (Some((o, oc)), Some((cl, cc))) = (open_above, closer) {
        if oc == cc {
            pairs.push((o, cl));
        }
    }
    pairs.sort();
    pairs
}

/// A list open at the line being laid out: where its marker and its
/// items' text are in the source (columns) and on screen (points from the
/// left edge), and the number of its last item.
struct ListLevel {
    marker: usize,
    content: usize,
    ordered: bool,
    n: u64,
    x: f32,
    tx: f32,
}

/// Where an item goes: its depth (0 at the outermost list), its number,
/// and the columns of its marker and its text on screen.
#[derive(Clone, Copy)]
struct ItemPlace {
    depth: usize,
    n: u64,
    x: f32,
    tx: f32,
}

/// Columns of an indent (a tab to the next multiple of four).
fn indent_cols(indent: &str) -> usize {
    indent.chars().fold(0, |c, ch| if ch == '\t' { c + 4 - c % 4 } else { c + 1 })
}

/// A list item at source column `ind`, its text at `content`, with
/// `start` its number when ordered: which list it belongs to, as in
/// CommonMark (and GitHub). Indented to the text of the item before or
/// further, it starts a list inside that item; less, it is the next item
/// of the list whose marker it reaches, or of the outermost one. Its marker
/// is drawn under the text of the item it is inside: every level as wide
/// as its marker needs, whatever spaces the source has.
fn list_item(lists: &mut Vec<ListLevel>, ind: usize, content: usize, start: Option<u64>, size: f32) -> ItemPlace {
    let ordered = start.is_some();
    let col = if ordered { size * 1.8 } else { size * 1.4 };
    let mut sibling = None;
    while let Some(top) = lists.last() {
        if ind >= top.content {
            break;
        }
        if ind >= top.marker || lists.len() == 1 {
            sibling = lists.pop();
            break;
        }
        lists.pop();
    }
    // The next item of a list counts on from the one before ("1." written
    // for each still makes 1, 2, 3); one of another kind starts a new list
    // in its place.
    let (x, n) = match sibling {
        Some(prev) => (prev.x, if ordered && prev.ordered { prev.n + 1 } else { start.unwrap_or(0) }),
        None => (lists.last().map_or(0.0, |l| l.tx), start.unwrap_or(0)),
    };
    lists.push(ListLevel { marker: ind, content, ordered, n, x, tx: x + col });
    ItemPlace { depth: lists.len() - 1, n, x, tx: x + col }
}

/// An ordered item's number as its depth writes it: 1. then i. then a.,
/// and again.
fn list_label(depth: usize, n: u64) -> String {
    match depth % 3 {
        0 => format!("{n}."),
        1 => format!("{}.", roman(n)),
        _ => format!("{}.", alpha(n)),
    }
}

fn roman(mut n: u64) -> String {
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    let mut s = String::new();
    for (v, r) in [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")] {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// 1 a, 26 z, 27 aa.
fn alpha(n: u64) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut n = n;
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    s.iter().rev().collect()
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
    /// Line of the closing fence.
    close: usize,
    start: usize,
    mermaid: Option<(String, Option<f32>)>,
    /// The cursor is in the block (fences included): show both fence lines.
    editing: bool,
    /// Syntax highlighting for the code, from the fence's language (None: plain).
    hl: Option<crate::highlight::Lines>,
}

/// Lay out `text` as in-place-rendered markdown. `cursor_line` (0-based) shows
/// its markup. `diagram` gives the height to reserve for a rendered mermaid
/// source (None: show it as code).
///
/// `breaks`: every newline is a line break, as in GitHub PR descriptions and
/// comments; otherwise a single newline inside a paragraph is a space (CommonMark).
/// `measure` gives the unwrapped width of a text in a font (aligns table
/// columns), `fit` the size of a laid-out job (table cells wrapped to their
/// column), `wrap` the width the text is wrapped to.
#[allow(clippy::too_many_arguments)]
pub fn layout(
    text: &str,
    cursor_line: Option<usize>,
    size: f32,
    breaks: bool,
    wrap: f32,
    diagram: &mut dyn FnMut(&str) -> Option<f32>,
    measure: &mut dyn FnMut(&str, &FontId) -> f32,
    fit: &mut dyn FnMut(LayoutJob) -> egui::Vec2,
) -> MdLayout {
    let mut job = LayoutJob::default();
    let mut decos = Vec::new();
    let mut hangs = Vec::new();
    let mut char_pos = 0usize; // char index of the current line start
    let mut byte_pos = 0usize;
    let mut fence: Option<Fence> = None;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let blocks = fence_blocks(&lines, cursor_line);
    let soft = if breaks { vec![false; lines.len()] } else { soft_breaks(&lines, cursor_line, &blocks) };
    let mut table: Option<Table> = None;
    let mut in_comment = false;
    // The lists open at the line, outermost first (see `list_item`).
    let mut lists: Vec<ListLevel> = Vec::new();
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
        let comment_line = fence.is_none() && (in_comment || (trimmed.starts_with("<!--") && !trimmed[4..].contains("-->")));
        if comment_line {
            in_comment = if in_comment { !line.contains("-->") } else { true };
        }
        // Which list this line is an item of, or which item it goes on
        // (a line indented to an item's text); a blank line closes none.
        let digits = trimmed.bytes().take_while(|c| c.is_ascii_digit()).count();
        let bullet = (trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ")) && !is_rule(trimmed);
        let numbered = digits > 0 && digits <= 9 && trimmed[digits..].starts_with(". ");
        let mut item: Option<ItemPlace> = None;
        let mut under_item: Option<f32> = None;
        if fence.is_none() && table.is_none() && !comment_line {
            let ind = indent_cols(indent);
            if bullet || numbered {
                let start = numbered.then(|| trimmed[..digits].parse().unwrap_or(1));
                let content = ind + if numbered { digits + 2 } else { 2 };
                item = Some(list_item(&mut lists, ind, content, start, size));
            } else if !trimmed.is_empty() {
                let keep = lists.iter().position(|l| l.content > ind).unwrap_or(lists.len());
                lists.truncate(keep);
                under_item = lists.last().map(|l| l.tx);
            }
        }

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
                // Each cell is laid out on its own, wrapped to its column, and
                // drawn over the room the table's (hidden) source reserves: in
                // one text layout, a row wider than the editor wrapped back to
                // the left edge, over the other columns.
                let row_lines: Vec<&str> = lines[ln..end].iter().enumerate().filter(|(i, _)| *i != 1).map(|(_, l)| *l).collect();
                let cell_job = |cell: &str, header: bool| {
                    let st = if header { Style { family: Fam::Bold, ..body } } else { body };
                    let mut j = LayoutJob::default();
                    Ctx { job: &mut j, cursor_line: false }.inline(cell.trim(), st);
                    j
                };
                let mut natural = vec![0.0_f32; aligns.len()];
                for (i, l) in row_lines.iter().enumerate() {
                    for (c, cell) in table_cells(l).iter().enumerate().take(natural.len()) {
                        natural[c] = natural[c].max(sections_width(&cell_job(cell, i == 0), 0, measure));
                    }
                }
                let pad = size * 0.6;
                let widths = fit_columns(&natural, (wrap - 2.0 * pad * natural.len() as f32 - 2.0).max(0.0), size * 3.0);
                let mut cols = vec![0.0_f32];
                for w in &widths {
                    cols.push(cols.last().unwrap() + w + 2.0 * pad);
                }
                let vpad = size * 0.35;
                let mut cells = Vec::new();
                let mut rows_y = vec![0.0_f32];
                for (i, l) in row_lines.iter().enumerate() {
                    let texts = table_cells(l);
                    let mut row = Vec::new();
                    let mut h = size * 1.3;
                    for (c, w) in widths.iter().enumerate() {
                        let mut j = cell_job(texts.get(c).copied().unwrap_or(""), i == 0);
                        j.wrap.max_width = *w;
                        let s = fit(j.clone());
                        h = h.max(s.y);
                        let slack = (w - s.x).max(0.0);
                        let x = cols[c] + pad + match aligns[c] {
                            Align::Left => 0.0,
                            Align::Center => slack / 2.0,
                            Align::Right => slack,
                        };
                        row.push((j, x));
                    }
                    cells.push(row);
                    rows_y.push(rows_y.last().unwrap() + h + 2.0 * vpad);
                }
                table = Some(Table { header: ln, end, height: *rows_y.last().unwrap(), grid: Some((cells, cols, rows_y, vpad)), at: 0, row_chars: Vec::new() });
            }
        }

        if let Some(f) = fence.as_ref() {
            let collapsed = f.mermaid.as_ref().and_then(|m| m.1);
            if ln == f.close {
                if f.editing {
                    cx.cursor_line = true;
                }
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
                let code = Style { family: Fam::Mono, size: size * 0.92, ..body };
                match fence.as_mut().and_then(|f| f.hl.as_mut()) {
                    Some(hl) => {
                        for (a, b, color, italics) in hl.line(line) {
                            cx.text(&line[a..b], Style { color, italics, ..code });
                        }
                    }
                    None => cx.text(line, code),
                }
            }
        } else if comment_line || html_only(trimmed) {
            // HTML comments and wrapper-tag lines take no room.
            cx.marker(line, body);
            tiny_newline = !cx.cursor_line;
        } else if is_html_rule(trimmed) {
            if cx.cursor_line {
                cx.marker(line, body);
            } else {
                cx.blank(line, body);
                decos.push(Deco::Rule { at: char_pos });
            }
        } else if let Some(&(_, close)) = blocks.iter().find(|b| b.0 == ln) {
            let lang = trimmed[3..].trim();
            let editing = cursor_line.map(|cl| (ln..=close).contains(&cl)).unwrap_or(false);
            let mut mermaid = None;
            if lang.eq_ignore_ascii_case("mermaid") {
                let source: String = lines[ln + 1..close].concat();
                let h = if editing { None } else { diagram(&source) };
                mermaid = Some((source, h));
            }
            let ext = crate::highlight::ext_of_lang(lang);
            let hl = (!ext.is_empty()).then(|| crate::highlight::Lines::new(&ext, ""));
            fence = Some(Fence { close, start: char_pos, mermaid, editing, hl });
            if editing {
                cx.cursor_line = true;
            }
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
                if ln > 0 && soft[ln - 1] {
                    // Joined onto the previous line: no gap where the marker was.
                    cx.marker(&trimmed[..n], body);
                } else {
                    cx.blank(&trimmed[..n], body);
                }
            }
            cx.inline(&trimmed[n..], Style { color: theme::text_dim(), italics: true, ..body });
            decos.push(Deco::Quote { start: char_pos, end: char_pos + line_chars });
        } else if let (true, Some(place)) = (bullet, item) {
            let marker_byte = indent.len();
            cx.lead_in(indent, &trimmed[..2], place.tx, body, measure);
            hangs.push(Hang { start: byte_pos, content: byte_pos + indent.len() + 2, hang: place.tx });
            let item = &trimmed[2..];
            let task = item.len() >= 3 && item.starts_with('[') && item.as_bytes()[2] == b']' && matches!(item.as_bytes()[1], b' ' | b'x' | b'X');
            // A task's box stands for its bullet (as on GitHub).
            if !cx.cursor_line && !task {
                decos.push(Deco::Bullet { at: char_at(marker_byte), depth: place.depth, col: place.tx - place.x });
            }
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
            // The source is hidden: its first character, invisible and as tall
            // as the table, reserves the room the cells are drawn over (as a
            // rendered diagram does); every other line takes (almost) none.
            // The room is the line height of a 1 px character: a glyph that
            // tall (as before) drew a huge block when a selection crossed it.
            if ln == t.header {
                let first = line.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                let tall = TextFormat { font_id: FontId::new(1.0, FontFamily::Proportional), color: Color32::TRANSPARENT, line_height: Some(t.height.max(1.0)), ..Default::default() };
                cx.job.append(&line[..first], 0.0, tall);
                cx.marker(&line[first..], body);
                t.at = char_pos;
            } else {
                cx.marker(line, body);
            }
            // Each row's source (the separator row goes with the header), so a
            // selection over the table can be shown row by row.
            if ln != t.header + 1 {
                t.row_chars.push((char_pos, char_pos + line_chars));
            } else if let Some(last) = t.row_chars.last_mut() {
                last.1 = char_pos + line_chars;
            }
            tiny_newline = true;
            if ln + 1 == t.end {
                let t = table.take().unwrap();
                decos.push(Deco::Table { at: t.at, grid: t.grid.unwrap_or_default(), rows: t.row_chars });
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
        } else if let (true, Some(place)) = (numbered, item) {
            cx.lead_in(indent, &trimmed[..digits + 2], place.tx, body, measure);
            hangs.push(Hang { start: byte_pos, content: byte_pos + indent.len() + digits + 2, hang: place.tx });
            if !cx.cursor_line {
                decos.push(Deco::Number { at: char_at(indent.len()), label: list_label(place.depth, place.n) });
            }
            cx.inline(&trimmed[digits + 2..], body);
        } else if let Some(tx) = under_item.filter(|_| !(cx.cursor_line || ln > 0 && soft[ln - 1])) {
            // A paragraph of a list item: under the item's text. (Not when
            // the line before runs on into it: then it is the same line.)
            if indent.is_empty() {
                cx.inline(line, body);
            } else {
                cx.indent_to(indent, tx, measure);
                hangs.push(Hang { start: byte_pos, content: byte_pos + indent.len(), hang: tx });
                cx.inline(trimmed, body);
            }
        } else if !cx.cursor_line && ln > 0 && soft[ln - 1] && !indent.is_empty() {
            // It runs on from the line before, the newline between being a
            // space: its own indent is not more room (as in CommonMark).
            cx.marker(indent, body);
            cx.inline(trimmed, body);
        } else {
            cx.inline(line, body);
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
    MdLayout { job, decos, hangs }
}

/// Lay `job` out with hanging indents: a list item's paragraph (its text to
/// a real newline, from `start` in `hangs`) has its text, from `content`,
/// laid out `hang` narrower and `hang` from the left edge, every row of it,
/// as on GitHub; the marker and indent before it go on its first row. egui
/// has no hanging indent, so such a paragraph is laid out on its own; the
/// rest as egui does, together. The galley has the whole job's characters,
/// rows and sections, so cursors and selections work as on one laid out
/// at once. (Before, wrapped rows went back to the left edge, and a long
/// first word, a link, went whole to the next row: the item's marker was
/// left on a row of its own, a hair tall, its bullet over the row above.)
pub fn lay_out_hanging(job: LayoutJob, hangs: &[Hang], lay: &mut dyn FnMut(LayoutJob) -> Arc<Galley>) -> Arc<Galley> {
    let width = job.wrap.max_width;
    let hang_at = |start: usize| hangs.iter().find(|h| h.start == start && h.hang >= 1.0 && h.hang < width * 0.8);
    // Paragraphs; runs of those with no hang are laid out together.
    let mut pieces: Vec<(usize, usize, Option<&Hang>)> = Vec::new();
    let text = job.text.as_str();
    let mut start = 0;
    for end in text.match_indices('\n').map(|(i, _)| i + 1).chain(std::iter::once(text.len())) {
        let h = hang_at(start);
        match pieces.last_mut() {
            Some(p) if h.is_none() && p.2.is_none() => p.1 = end,
            _ => pieces.push((start, end, h)),
        }
        start = end;
    }
    if !width.is_finite() || pieces.iter().all(|p| p.2.is_none()) {
        return lay(job);
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut y = 0.0;
    let mut elided = false;
    let mut pixels_per_point = 1.0;
    let n = pieces.len();
    for (k, &(a, b, h)) in pieces.iter().enumerate() {
        let first = k == 0;
        let mut got: Vec<Row> = match h {
            None => {
                let (sub, map) = sub_job(&job, a, b, width, first);
                let g = lay(sub);
                elided |= g.elided;
                pixels_per_point = g.pixels_per_point;
                g.rows.iter().cloned().map(|mut r| {
                    shift_row(&mut r, egui::Vec2::ZERO, &map);
                    r
                }).collect()
            }
            Some(h) => {
                let c = h.content.clamp(a, b);
                let (lead, lead_map) = sub_job(&job, a, c, f32::INFINITY, first);
                let (body, body_map) = sub_job(&job, c, b, (width - h.hang).max(1.0), false);
                let lead = lay(lead);
                let body = lay(body);
                elided |= body.elided;
                pixels_per_point = body.pixels_per_point;
                let mut out: Vec<Row> = body.rows.iter().cloned().map(|mut r| {
                    shift_row(&mut r, egui::vec2(h.hang, 0.0), &body_map);
                    r
                }).collect();
                if let (Some(mut l), Some(r0)) = (lead.rows.first().cloned(), out.first_mut()) {
                    shift_row(&mut l, egui::Vec2::ZERO, &lead_map);
                    join_rows(l, r0);
                }
                out
            }
        };
        // A piece ending in a newline has an empty row after it: the next
        // piece's first row is that.
        if k + 1 < n && got.len() > 1 && got.last().is_some_and(|r| r.glyphs.is_empty() && !r.ends_with_newline) {
            got.pop();
        }
        let top = y;
        for mut row in got {
            shift_row(&mut row, egui::vec2(0.0, top), &[]);
            y = row.rect.max.y;
            rows.push(row);
        }
    }
    // The empty row after a last newline, laid out alone, has no height:
    // that of the row above (where the cursor goes at the end).
    if let [.., above, last] = rows.as_mut_slice() {
        if last.glyphs.is_empty() && last.rect.height() < 1.0 {
            last.rect.max.y = last.rect.min.y + above.rect.height();
        }
    }
    let mut rect = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::ZERO);
    let mut mesh_bounds = egui::Rect::NOTHING;
    let (mut num_vertices, mut num_indices) = (0, 0);
    for r in &rows {
        rect = rect.union(r.rect);
        mesh_bounds = mesh_bounds.union(r.visuals.mesh_bounds);
        num_vertices += r.visuals.mesh.vertices.len();
        num_indices += r.visuals.mesh.indices.len();
    }
    Arc::new(Galley { job: Arc::new(job), rows, elided, rect, mesh_bounds, num_vertices, num_indices, pixels_per_point })
}

/// A list item's paragraph: where it starts (byte), where its text starts,
/// and the x its text is at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hang {
    pub start: usize,
    pub content: usize,
    pub hang: f32,
}

/// `job`'s bytes a..b as a job of their own, `width` wide, and for each of
/// its sections the index of the section it came from.
fn sub_job(job: &LayoutJob, a: usize, b: usize, width: f32, first: bool) -> (LayoutJob, Vec<u32>) {
    let mut sub = LayoutJob {
        text: job.text[a..b].to_string(),
        wrap: egui::text::TextWrapping { max_width: width, ..job.wrap.clone() },
        first_row_min_height: if first { job.first_row_min_height } else { 0.0 },
        break_on_newline: job.break_on_newline,
        halign: job.halign,
        justify: job.justify,
        round_output_to_gui: job.round_output_to_gui,
        sections: Vec::new(),
    };
    let mut map = Vec::new();
    for (i, s) in job.sections.iter().enumerate() {
        let (s0, s1) = (s.byte_range.start, s.byte_range.end);
        if !(s0 < b && s1 > a) {
            continue;
        }
        let mut piece = s.clone();
        piece.byte_range = s0.max(a) - a..s1.min(b) - a;
        if s0 < a {
            piece.leading_space = 0.0;
        }
        sub.sections.push(piece);
        map.push(i as u32);
    }
    (sub, map)
}

/// Move a laid out row by `d`, its sections numbered as in the whole job
/// (`map`: from its own job's; empty: already so).
fn shift_row(row: &mut Row, d: egui::Vec2, map: &[u32]) {
    let at = |i: u32| map.get(i as usize).copied().unwrap_or(i);
    row.rect = row.rect.translate(d);
    row.section_index_at_start = at(row.section_index_at_start);
    for g in &mut row.glyphs {
        g.pos += d;
        g.section_index = at(g.section_index);
    }
    for v in &mut row.visuals.mesh.vertices {
        v.pos += d;
    }
    if row.visuals.mesh_bounds.is_positive() {
        row.visuals.mesh_bounds = row.visuals.mesh_bounds.translate(d);
    }
}

/// `lead` (a marker and indent) put in front of `row`, one row of both.
fn join_rows(lead: Row, row: &mut Row) {
    let mut glyphs = lead.glyphs;
    glyphs.append(&mut row.glyphs);
    row.glyphs = glyphs;
    row.section_index_at_start = lead.section_index_at_start;
    row.rect = egui::Rect::from_min_max(egui::pos2(lead.rect.min.x.min(row.rect.min.x), row.rect.min.y), egui::pos2(row.rect.max.x.max(lead.rect.max.x), row.rect.max.y.max(row.rect.min.y + lead.rect.height())));
    let mut mesh = lead.visuals.mesh;
    let offset = mesh.vertices.len();
    let index_offset = mesh.indices.len();
    let body = std::mem::take(&mut row.visuals.mesh);
    mesh.append(body);
    let lead_glyphs = lead.visuals.glyph_vertex_range.clone();
    let body_glyphs = row.visuals.glyph_vertex_range.clone();
    row.visuals.glyph_vertex_range = if lead_glyphs.is_empty() { body_glyphs.start + offset..body_glyphs.end + offset } else { lead_glyphs.start..body_glyphs.end + offset };
    row.visuals.glyph_index_start = if lead_glyphs.is_empty() { row.visuals.glyph_index_start + index_offset } else { lead.visuals.glyph_index_start };
    row.visuals.mesh_bounds = lead.visuals.mesh_bounds.union(row.visuals.mesh_bounds);
    row.visuals.mesh = mesh;
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
fn soft_breaks(lines: &[&str], cursor_line: Option<usize>, blocks: &[(usize, usize)]) -> Vec<bool> {
    let mut in_comment = false;
    let kinds: Vec<Kind> = lines
        .iter()
        .enumerate()
        .map(|(ln, raw)| {
            let line = raw.trim_end_matches(['\n', '\r']);
            let t = line.trim_start();
            if blocks.iter().any(|&(a, b)| (a..=b).contains(&ln)) {
                return Kind::Other;
            }
            if in_comment || (t.starts_with("<!--") && !t[4..].contains("-->")) {
                in_comment = if in_comment { !line.contains("-->") } else { true };
                return Kind::Other;
            }
            if html_only(t) || is_html_rule(t) {
                return Kind::Other;
            }
            let ordered = t.bytes().take_while(|c| c.is_ascii_digit()).count();
            if t.is_empty() || heading_level(line).is_some() || is_rule(line) || t.starts_with('|') || line.starts_with("    ") || line.starts_with('\t') {
                Kind::Other
            } else if t.starts_with('>') {
                // A bare ">" line separates quoted paragraphs.
                if t.trim_start_matches(['>', ' ', '\t']).is_empty() {
                    Kind::Other
                } else {
                    Kind::Quote
                }
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
            let lower = line.trim_end().to_ascii_lowercase();
            let hard = line.ends_with("  ") || line.ends_with('\\') || lower.ends_with("<br>") || lower.ends_with("<br/>") || lower.ends_with("<br />");
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
        let l = layout(src, None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert_eq!(l.job.text, src, "the editor needs the exact source text");
        assert_eq!(layout("a\nb", None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono).job.text, "a b");
        let covered: usize = l.job.sections.iter().map(|s| s.byte_range.len()).sum();
        assert_eq!(covered, src.len());
    }

    /// Size of a job laid out at its wrap width, monospace (6 per char at size
    /// 10), wrapping at spaces like egui: enough for the table tests.
    fn fit_mono(j: LayoutJob) -> egui::Vec2 {
        let text: String = j.sections.iter().map(|s| &j.text[s.byte_range.clone()]).collect();
        let size = j.sections.first().map(|s| s.format.font_id.size).unwrap_or(10.0);
        let (cw, w) = (size * 0.6, j.wrap.max_width);
        let (mut lines, mut cur, mut widest) = (1, 0.0_f32, 0.0_f32);
        for word in text.split(' ') {
            let ww = word.chars().count() as f32 * cw;
            let add = if cur > 0.0 { cw + ww } else { ww };
            if cur > 0.0 && cur + add > w {
                lines += 1;
                cur = ww;
            } else {
                cur += add;
            }
            widest = widest.max(cur);
        }
        egui::vec2(widest, lines as f32 * size * 1.3)
    }

    fn lay(src: &str, cursor: Option<usize>, size: f32, wrap: f32) -> MdLayout {
        layout(src, cursor, size, false, wrap, &mut |_| None, &mut mono, &mut fit_mono)
    }

    /// Laid out with real fonts, as the editor does (hanging indents).
    fn laid(src: &str, cursor: Option<usize>, wrap: f32) -> (MdLayout, Arc<Galley>) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        let _ = ctx.run(Default::default(), |_| {});
        let mut measure = |t: &str, f: &FontId| ctx.fonts(|fo| fo.layout_no_wrap(t.to_owned(), f.clone(), Color32::WHITE).size().x);
        let mut fit = |j: LayoutJob| ctx.fonts(|fo| fo.layout_job(j).size());
        let l = layout(src, cursor, 14.0, false, wrap, &mut |_| None, &mut measure, &mut fit);
        let mut job = l.job.clone();
        job.wrap.max_width = wrap;
        let g = ctx.fonts(|fo| lay_out_hanging(job, &l.hangs, &mut |j| fo.layout_job(j)));
        (l, g)
    }

    /// The rows each char of `src` is on, and its x.
    fn places(g: &Galley) -> Vec<(usize, f32)> {
        let mut out = Vec::new();
        for (r, row) in g.rows.iter().enumerate() {
            for gl in &row.glyphs {
                out.push((r, gl.pos.x));
            }
            if row.ends_with_newline {
                out.push((r, row.rect.max.x));
            }
        }
        out
    }

    /// A list item's wrapped rows start under its text, not at the left
    /// edge, and a first word too long for the row (a link, in a narrow
    /// pane) is cut there: it went whole to the next row at the left edge,
    /// leaving the marker on a row a hair tall, its bullet over the row
    /// above. The galley still has every char, row end and cursor place.
    #[test]
    fn list_items_wrap_under_their_text() {
        let src = "261116 weekly meeting\n\n- **[DONE]** ~~Resolve conflicts and open~~\n  - https://github.com/dn-inc/v110-bsp/pull/106\n- a long item that wraps onto more than one row in a narrow pane\n";
        for cursor in [None, Some(3), Some(4)] {
            let (l, g) = laid(src, cursor, 260.0);
            let p = places(&g);
            assert_eq!(p.len(), src.chars().count(), "{cursor:?}: every char has its place");
            assert_eq!(g.rows.iter().filter(|r| r.ends_with_newline).count(), src.matches('\n').count());
            let char_of = |needle: &str| src[..src.find(needle).unwrap()].chars().count();
            let link = char_of("https://");
            let hang = l.hangs.iter().find(|h| src[h.content..].starts_with("https://")).unwrap().hang;
            let (link_row, link_x) = p[link];
            assert!((link_x - hang).abs() < 2.0, "{cursor:?}: the link starts at its item's text: {link_x} vs {hang}");
            let marker = char_of("  - https") + 2;
            assert_eq!(p[marker].0, link_row, "{cursor:?}: the marker is on the link's row");
            assert!(g.rows[link_row].rect.height() > 10.0, "a whole row tall");
            // Every row of the link and of the long item is at its hang.
            let end = char_of("\n- a long");
            let rows: std::collections::BTreeSet<usize> = (link..end).map(|i| p[i].0).collect();
            assert!(rows.len() >= 2, "{cursor:?}: the link is cut over rows: {rows:?}");
            for r in rows.iter().skip(1) {
                assert!((g.rows[*r].glyphs[0].pos.x - hang).abs() < 2.0, "{cursor:?}: row {r} of the link at {}", g.rows[*r].glyphs[0].pos.x);
            }
            let long = char_of("a long item");
            let long_hang = p[long].1;
            let long_rows: std::collections::BTreeSet<usize> = (long..src.chars().count() - 1).map(|i| p[i].0).collect();
            assert!(long_rows.len() >= 2, "it wraps");
            for r in long_rows.iter().skip(1) {
                assert!((g.rows[*r].glyphs[0].pos.x - long_hang).abs() < 2.0, "{cursor:?}: a wrapped row under the item's text");
            }
            // Rows go down the page, none on another.
            for w in g.rows.windows(2) {
                assert!(w[1].rect.min.y >= w[0].rect.max.y - 0.5, "{cursor:?}: rows in order");
            }
            // Cursor places come back from where they are drawn (inside
            // text: right after a hidden marker, a hair wide, the place
            // before it is as near, as in egui's own layout).
            for i in [0, link + 5, link + 30, long + 3, src.chars().count() - 2] {
                let at = g.pos_from_ccursor(egui::text::CCursor::new(i));
                assert_eq!(g.cursor_from_pos(at.center().to_vec2()).ccursor.index, i, "{cursor:?}: cursor {i}");
            }
        }
        // Wide: the same rows as egui's own, the list items fitting.
        let (l, g) = laid(src, None, 2000.0);
        let mut job = l.job.clone();
        job.wrap.max_width = 2000.0;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        let _ = ctx.run(Default::default(), |_| {});
        let plain = ctx.fonts(|fo| fo.layout_job(job));
        assert_eq!(g.rows.len(), plain.rows.len());
        for (a, b) in g.rows.iter().zip(plain.rows.iter()) {
            assert_eq!(a.glyphs.len(), b.glyphs.len());
            assert!((a.rect.min.y - b.rect.min.y).abs() < 0.5 && (a.rect.height() - b.rect.height()).abs() < 0.5);
            if let (Some(x), Some(y)) = (a.glyphs.last(), b.glyphs.last()) {
                assert!((x.pos.x - y.pos.x).abs() < 1.0, "the same places: {} vs {}", x.pos.x, y.pos.x);
            }
        }
    }

    #[test]
    fn tables_hide_pipes_and_align_columns() {
        let src = "| name | n |\n|:--|--:|\n| a | 10 |\n| longer | 2 |\n";
        let l = lay(src, None, 10.0, 1000.0);
        assert_eq!(l.job.text, src);
        let Some(Deco::Table { grid: (cells, cols, rows_y, _), rows, .. }) = l.decos.iter().find(|d| matches!(d, Deco::Table { .. })) else { panic!("no table") };
        assert_eq!(cells.len(), 3, "header + 2 rows, separator excluded");
        assert_eq!(cols.len(), 3, "a border left of each column and one at the end");
        assert_eq!(rows_y.len(), 4);
        // Each drawn row knows its source (the separator row with the header):
        // a selection over the table is shown, and copied, row by row.
        let text = |(a, b): (usize, usize)| src.chars().skip(a).take(b - a).collect::<String>();
        assert_eq!(rows.len(), 3);
        assert_eq!(text(rows[0]), "| name | n |\n|:--|--:|");
        assert_eq!(text(rows[2]), "| longer | 2 |");
        // The room is a line height, not a giant glyph a selection would show.
        assert!(l.job.sections.iter().all(|s| s.format.font_id.size < 30.0));
        assert!(l.job.sections.iter().any(|s| s.format.line_height.is_some_and(|h| (h - rows_y[3]).abs() < 0.5)));
        // Left-aligned cells start at the same x; the right-aligned ones end
        // at the column's right side.
        assert_eq!(cells[0][0].1, cells[2][0].1);
        assert!(cells[1][1].1 < cells[2][1].1, "10 starts left of 2 (right-aligned)");
        // The source is all invisible: the cells are drawn over it.
        assert_eq!(visible(&l), "");
        // With the cursor inside, the table is raw.
        let raw = lay(src, Some(2), 10.0, 1000.0);
        assert!(!raw.decos.iter().any(|d| matches!(d, Deco::Table { .. })));
    }

    #[test]
    fn a_wide_table_wraps_inside_its_columns() {
        let long = "one two three four five six seven eight nine ten eleven twelve";
        let src = format!("| key | value |\n|---|---|\n| a | {long} |\n");
        let l = lay(&src, None, 10.0, 200.0);
        let Some(Deco::Table { grid: (cells, cols, rows_y, _), .. }) = l.decos.iter().find(|d| matches!(d, Deco::Table { .. })) else { panic!("no table") };
        assert!(*cols.last().unwrap() <= 200.0 + 0.5, "fits the editor: {cols:?}");
        let short_row = rows_y[1] - rows_y[0];
        let long_row = rows_y[2] - rows_y[1];
        assert!(long_row > short_row * 2.0, "the long cell wraps onto several lines: {rows_y:?}");
        // The narrow column keeps its width.
        assert!(cols[1] - cols[0] < 40.0, "{cols:?}");
        assert_eq!(cells[1][1].0.wrap.max_width, cols[2] - cols[1] - 12.0);
    }

    #[test]
    fn inline_html_renders() {
        let src = "a <b>bold</b> &amp; <kbd>K</kbd><!-- note --> x<br>y\n<div align=\"center\">\n<img src=\"p.png\" alt=\"pic\">\n</div>\n<hr>\nVec<T> &nope; a < b\n";
        let l = layout(src, None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert_eq!(l.job.text.chars().count(), src.chars().count());
        let v = visible(&l);
        assert!(v.contains("a bold & K x"), "{v}");
        assert!(v.contains("pic") && v.contains("Vec<T> &nope; a < b"), "{v}");
        for hidden in ["<b>", "</b>", "note", "<div", "src=", "<hr>", "amp;"] {
            assert!(!v.contains(hidden), "{hidden} in {v}");
        }
        assert!(l.job.text.contains(" x\nbr>y"), "<br> breaks the line");
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Rule { .. })));
        assert!(l.job.sections.iter().any(|s| s.format.font_id.family == FontFamily::Name(BOLD_FONT.into()) && &l.job.text[s.byte_range.clone()] == "bold"));
        // On the cursor line the source is shown as written.
        let raw = layout(src, Some(0), 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert!(raw.job.text.starts_with("a <b>bold</b> &amp;"));
    }

    #[test]
    fn code_blocks_are_highlighted_by_language() {
        let src = "```c\nint main() { return 0; } // hi\n```\n```\nint plain\n```\n";
        let l = layout(src, None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert_eq!(l.job.text.chars().count(), src.chars().count());
        let color_of = |needle: &str, nth: usize| {
            let at = src.match_indices(needle).nth(nth).unwrap().0;
            l.job.sections.iter().find(|s| s.byte_range.start == at).map(|s| s.format.color).unwrap()
        };
        assert_ne!(color_of("int", 0), color_of("main", 0), "keyword colored in ```c");
        assert_ne!(color_of("// hi", 0), color_of("main", 0), "comment colored");
        // No language: one plain run for the whole line.
        assert!(l.job.sections.iter().any(|s| &l.job.text[s.byte_range.clone()] == "int plain"));
        assert_eq!(crate::highlight::ext_of_lang("Rust"), "rs");
        assert_eq!(crate::highlight::ext_of_lang("python {.numberLines}"), "py");
    }

    #[test]
    fn fences_pair_outward_from_the_cursor() {
        let doc: Vec<&str> = "intro\n```\nmore\n```py\nx\n```\nend\n".split_inclusive('\n').collect();
        // Not editing: plain markdown, the lone ``` (line 1) swallows the py block's opener.
        assert_eq!(fence_blocks(&doc, None), [(1, 5)]);
        // Typing ``` on line 1 (cursor there): it stays text, the py block is intact.
        assert_eq!(fence_blocks(&doc, Some(1)), [(3, 5)]);
        // Then Enter and ``` again: the new pair is a block, the py block still intact.
        let doc2: Vec<&str> = "intro\n```\n```\nmore\n```py\nx\n```\n".split_inclusive('\n').collect();
        assert_eq!(fence_blocks(&doc2, Some(2)), [(1, 2), (4, 6)]);
        // Deleting the closing ``` of a pair (cursor on that now empty line).
        let doc3: Vec<&str> = "```\n\nmore\n```\ny\n```\n".split_inclusive('\n').collect();
        assert_eq!(fence_blocks(&doc3, Some(1)), [(3, 5)], "the lone opener stays text");
        // Cursor inside a block: its opener above and closer below still pair.
        let doc4: Vec<&str> = "```\ncode\n```\ntext\n```rs\nfn\n```\n".split_inclusive('\n').collect();
        assert_eq!(fence_blocks(&doc4, Some(1)), [(0, 2), (4, 6)]);
        assert_eq!(fence_blocks(&doc4, Some(5)), [(0, 2), (4, 6)]);
        assert_eq!(fence_blocks(&doc4, None), [(0, 2), (4, 6)]);
    }

    #[test]
    fn both_fences_show_while_editing_the_block() {
        let src = "```\n```\nafter";
        let shown = |cursor| visible(&layout(src, cursor, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono));
        assert_eq!(shown(Some(0)).matches("```").count(), 2, "{}", shown(Some(0)));
        assert_eq!(shown(Some(1)).matches("```").count(), 2);
        assert_eq!(shown(Some(2)).matches("```").count(), 0, "cursor outside: fences hidden");
    }

    #[test]
    fn unclosed_fence_is_plain_text() {
        let src = "```\n**bold** after\n";
        let l = layout(src, None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert!(!l.decos.iter().any(|d| matches!(d, Deco::Code { .. })));
        assert!(l.job.sections.iter().any(|s| s.format.font_id.family == FontFamily::Name(BOLD_FONT.into())));
        // ```rust inside a block does not close it.
        let src = "```\n```rust\nx\n```\n";
        let l = layout(src, None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        let codes: Vec<_> = l.decos.iter().filter(|d| matches!(d, Deco::Code { .. })).collect();
        assert_eq!(codes, [&Deco::Code { start: 0, end: src.chars().count() - 1 }]);
    }

    #[test]
    fn quotes_join_lines_and_split_on_bare_marker() {
        let src = "> a\n> b\n>\n> c\n";
        let l = layout(src, None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert_eq!(l.job.text, "> a > b\n>\n> c\n");
        // The joined line's second "> " takes no room.
        let b = l.job.sections.iter().find(|s| s.byte_range.start == 4).unwrap();
        assert!(b.format.font_id.size <= 1.0);
    }

    #[test]
    fn breaks_mode_keeps_every_newline() {
        let src = "> 678\n> 1234\none\ntwo\n";
        let l = layout(src, None, 14.0, true, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert_eq!(l.job.text, src);
    }

    #[test]
    fn single_newlines_join_a_paragraph() {
        let src = "one\ntwo  \nthree\n\nfour\n- item\nmore\n# h\nx\n```\na\nb\n```\n";
        let l = layout(src, None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert_eq!(l.job.text.chars().count(), src.chars().count(), "cursor positions must match the source");
        let lines: Vec<&str> = l.job.text.split('\n').collect();
        assert_eq!(&lines[..6], ["one two  ", "three", "", "four", "- item more", "# h"]);
        assert_eq!(&lines[6..10], ["x", "```", "a", "b"]);
        // The paragraph holding the cursor keeps its lines.
        let l = layout(src, Some(1), 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert!(l.job.text.starts_with("one\ntwo  \nthree\n"));
    }

    #[test]
    fn markup_hidden_except_on_cursor_line() {
        let src = "## Title **bold**\nnext";
        let away = visible(&layout(src, Some(1), 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono));
        assert!(!away.contains('#') && !away.contains("**"), "{away}");
        assert!(away.contains("Title") && away.contains("bold"));
        let here = visible(&layout(src, Some(0), 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono));
        assert!(here.contains("## ") && here.contains("**bold**"), "{here}");
    }

    /// Where the text that starts at `byte` is, from its line's left edge:
    /// the room before it (leading space and widths, as `mono` measures).
    fn x_of(l: &MdLayout, byte: usize) -> f32 {
        let mut x = 0.0;
        for s in &l.job.sections {
            let t = &l.job.text[s.byte_range.clone()];
            if s.byte_range.start == byte {
                return x + s.leading_space;
            }
            x += s.leading_space + mono(t, &s.format.font_id);
            if t.contains('\n') {
                x = 0.0;
            }
        }
        panic!("no section starts at {byte}");
    }

    fn bullet_depths(l: &MdLayout) -> Vec<usize> {
        l.decos.iter().filter_map(|d| if let Deco::Bullet { depth, .. } = d { Some(*depth) } else { None }).collect()
    }

    fn numbers(l: &MdLayout) -> Vec<String> {
        l.decos.iter().filter_map(|d| if let Deco::Number { label, .. } = d { Some(label.clone()) } else { None }).collect()
    }

    /// Lists nest as in CommonMark (and on GitHub): an item is inside the
    /// one before when indented to its text, else the next item of a list.
    /// One space is not enough under "- " (two are), nor two under "1. ".
    #[test]
    fn lists_nest_as_on_github() {
        let l = lay("- a\n  - b\n    - c\n  - d\n- e\n", None, 10.0, 1000.0);
        assert_eq!(bullet_depths(&l), [0, 1, 2, 1, 0]);
        // One space: the next item of the same list, not one inside it.
        let l = lay("- a\n - b\n- c\n", None, 10.0, 1000.0);
        assert_eq!(bullet_depths(&l), [0, 0, 0]);
        // Under "1. " three spaces; two make the next item.
        let l = lay("1. x\n   1. y\n", None, 10.0, 1000.0);
        assert_eq!(numbers(&l), ["1.", "i."]);
        let l = lay("1. x\n  1. y\n", None, 10.0, 1000.0);
        assert_eq!(numbers(&l), ["1.", "2."]);
        // Kinds mixed: depth counts every list it is inside.
        let l = lay("- a\n  1. b\n     - c\n", None, 10.0, 1000.0);
        assert_eq!((bullet_depths(&l), numbers(&l)), (vec![0, 2], vec!["i.".to_string()]));
        // A paragraph after the list is not in it; a blank line keeps it open.
        let l = lay("- a\n\n- b\n\npara\n- c\n", None, 10.0, 1000.0);
        assert_eq!(bullet_depths(&l), [0, 0, 0]);
    }

    /// Ordered items: 1. then i. then a. by depth, numbered as they come
    /// from the first one's number ("1." for each still counts up).
    #[test]
    fn ordered_items_are_numbered_by_depth() {
        let l = lay("1. a\n   1. b\n      1. c\n      1. d\n   1. e\n1. f\n", None, 10.0, 1000.0);
        assert_eq!(numbers(&l), ["1.", "i.", "a.", "b.", "ii.", "2."]);
        let l = lay("3. a\n7. b\n", None, 10.0, 1000.0);
        assert_eq!(numbers(&l), ["3.", "4."], "counted from the first");
        assert_eq!([roman(4), roman(9), roman(14), roman(1990)], ["iv", "ix", "xiv", "mcmxc"]);
        assert_eq!([alpha(1), alpha(26), alpha(27), alpha(52), alpha(53)], ["a", "z", "aa", "az", "ba"]);
        // The numbers as written are hidden (the labels are painted).
        let l = lay("1. a\n   1. b\n", None, 10.0, 1000.0);
        assert_eq!(visible(&l), "ab");
    }

    /// Bullets by depth (the painter cycles their shape); a task's box
    /// stands for its bullet.
    #[test]
    fn bullets_know_their_depth() {
        let l = lay("- a\n  - b\n    - c\n      - d\n", None, 10.0, 1000.0);
        assert_eq!(bullet_depths(&l), [0, 1, 2, 3]);
        let l = lay("- [ ] t\n- u\n", None, 10.0, 1000.0);
        assert_eq!(bullet_depths(&l), [0], "no bullet beside a box");
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Check { checked: false, .. })));
    }

    /// An item's text is further in than its parent's, whatever spaces the
    /// source has, and where it is does not move when the cursor comes to
    /// its line (the raw markers then show in the same room).
    #[test]
    fn list_text_keeps_its_place() {
        let src = "- a\n  - b\n    - c\n1. d\n   1. e\n\n   para of d\n";
        let byte = |s: &str| src.find(s).unwrap();
        let away = lay(src, None, 10.0, 1000.0);
        let (a, b, c) = (x_of(&away, byte("a\n")), x_of(&away, byte("b\n")), x_of(&away, byte("c\n")));
        assert!(a > 0.0 && b > a + 5.0 && c > b + 5.0, "{a} {b} {c}");
        let (d, e) = (x_of(&away, byte("d\n")), x_of(&away, byte("e\n")));
        assert!(e > d + 5.0);
        assert!((x_of(&away, byte("para")) - d).abs() < 0.01, "a paragraph of d is under d's text");
        for (ln, word) in [(0, "a\n"), (1, "b\n"), (2, "c\n"), (3, "d\n"), (4, "e\n")] {
            let here = lay(src, Some(ln), 10.0, 1000.0);
            assert!((x_of(&here, byte(word)) - x_of(&away, byte(word))).abs() < 0.01, "line {ln} moved when the cursor came to it");
        }
    }

    #[test]
    fn decorations_for_blocks() {
        let src = "- a\n- [x] done\n> q\n```\ncode\n```\n---\n# H\n";
        let l = layout(src, None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Bullet { at: 0, depth: 0, .. })));
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
        let l = layout(src, Some(0), 14.0, false, 1000.0, &mut |s: &str| {
            asked.push(s.to_string());
            Some(120.0)
        }, &mut mono, &mut fit_mono);
        assert_eq!(asked, vec!["graph TD; A-->B\n".to_string()]);
        assert_eq!(l.job.text, src);
        assert!(!visible(&l).contains("graph"), "code hidden when the diagram is shown");
        assert!(l.job.sections.iter().any(|s| s.format.font_id.size > 80.0), "space reserved");
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Diagram { collapsed: true, .. })));
        // Cursor inside the block: edit the source, no diagram requested.
        let mut asked = 0;
        let l = layout(src, Some(2), 14.0, false, 1000.0, &mut |_| {
            asked += 1;
            Some(120.0)
        }, &mut mono, &mut fit_mono);
        assert_eq!(asked, 0);
        assert!(visible(&l).contains("graph"));
        assert!(l.decos.iter().any(|d| matches!(d, Deco::Diagram { collapsed: false, .. })));
    }

    #[test]
    fn snake_case_is_not_italic() {
        let l = layout("call my_func_name now", None, 14.0, false, 1000.0, &mut |_| None, &mut mono, &mut fit_mono);
        assert!(l.job.sections.iter().all(|s| !s.format.italics));
    }
}
