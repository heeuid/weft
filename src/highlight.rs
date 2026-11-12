//! Syntax highlighting for the editor and markdown code blocks, all local.
//!
//! The grammars are bat's (two-face on syntect, about 200 languages, pure
//! Rust regex engine), colored with the active weft theme. They load in a
//! background thread at startup; until then (and for very large files) a
//! small keyword highlighter stands in.

use crate::theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, FontStyle, HighlightIterator, HighlightState, Highlighter, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
static LOADING: AtomicBool = AtomicBool::new(false);

/// Start loading the grammars (once); `ready` tells when they are there.
pub fn preload(ctx: &egui::Context) {
    if !LOADING.swap(true, Ordering::SeqCst) {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            SYNTAXES.get_or_init(two_face::syntax::extra_newlines);
            ctx.request_repaint();
        });
    }
}

pub fn ready() -> bool {
    SYNTAXES.get().is_some()
}

/// Files bigger than this keep the quick highlighter while typing.
const MAX_FULL: usize = 400_000;
/// A first full pass over more lines than this runs in the background (the
/// quick highlighter shows meanwhile), so opening a big file doesn't stall.
const BACKGROUND_LINES: usize = 300;
/// Bumped when a background pass finishes (part of the editors' cache keys).
static WARMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn warmed() -> u64 {
    WARMED.load(Ordering::Relaxed)
}

fn c32(c: Color32) -> Color {
    Color { r: c.r(), g: c.g(), b: c.b(), a: 255 }
}

/// A syntect theme from the active weft palette, built once per weft theme.
fn syntect_theme() -> &'static Theme {
    static THEMES: Mutex<Vec<(usize, &'static Theme)>> = Mutex::new(Vec::new());
    let id = theme::active();
    let mut themes = THEMES.lock().unwrap();
    if let Some((_, t)) = themes.iter().find(|(i, _)| *i == id) {
        return t;
    }
    let item = |scopes: &str, color: Color32, style: Option<FontStyle>| ThemeItem {
        scope: scopes.parse::<ScopeSelectors>().unwrap_or_default(),
        style: StyleModifier { foreground: Some(c32(color)), background: None, font_style: style },
    };
    let t = Theme {
        name: Some(format!("weft-{id}")),
        author: None,
        settings: ThemeSettings { foreground: Some(c32(theme::text())), ..Default::default() },
        scopes: vec![
            item("comment, punctuation.definition.comment", theme::text_faint(), Some(FontStyle::ITALIC)),
            item("string, constant.character, punctuation.definition.string", theme::ansi(2), None),
            item("constant.numeric, constant.language, constant.other, support.constant", theme::ansi(3), None),
            item("keyword, storage, meta.preprocessor, keyword.control.import", theme::ansi(5), None),
            item("keyword.operator", theme::text_dim(), None),
            item("entity.name.function, support.function, meta.function-call variable.function, variable.function", theme::ansi(4), None),
            item("entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, support.type, support.class, storage.type.rust", theme::ansi(3), None),
            item("entity.name.tag", theme::ansi(4), None),
            item("entity.other.attribute-name", theme::ansi(3), None),
            item("variable.parameter, variable.language", theme::ansi(1), None),
            item("punctuation", theme::text_dim(), None),
            item("markup.heading", theme::ansi(4), Some(FontStyle::BOLD)),
            item("markup.inserted", theme::ansi(2), None),
            item("markup.deleted", theme::ansi(1), None),
            item("invalid", theme::error(), None),
        ],
    };
    let t: &'static Theme = Box::leak(Box::new(t));
    themes.push((id, t));
    t
}

/// The grammar for a file extension / fence language, else from the first line (#!).
fn syntax_for(set: &'static SyntaxSet, name: &str, first_line: &str) -> Option<&'static SyntaxReference> {
    (!name.is_empty())
        .then(|| set.find_syntax_by_token(name).or_else(|| set.find_syntax_by_extension(name)))
        .flatten()
        .or_else(|| set.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text")
}

/// Colors a block line by line, keeping state across lines (multi-line
/// strings and comments). Without the grammars, the quick highlighter.
pub struct Lines {
    hl: Option<HighlightLines<'static>>,
    ext: String,
}

impl Lines {
    pub fn new(name: &str, first_line: &str) -> Lines {
        let hl = SYNTAXES.get().and_then(|set| syntax_for(set, name, first_line)).map(|s| HighlightLines::new(s, syntect_theme()));
        Lines { hl, ext: name.to_string() }
    }

    /// Spans (byte ranges into `line`, which has no newline) with colors.
    pub fn line(&mut self, line: &str) -> Vec<(usize, usize, Color32, bool)> {
        let (Some(hl), Some(set)) = (self.hl.as_mut(), SYNTAXES.get()) else {
            return spans(line, &self.ext).into_iter().map(|(a, b, c)| (a, b, c, false)).collect();
        };
        let with_nl = format!("{line}\n");
        let Ok(regions) = hl.highlight_line(&with_nl, set) else {
            return vec![(0, line.len(), theme::text(), false)];
        };
        let mut out = Vec::new();
        let mut at = 0;
        for (style, piece) in regions {
            let end = (at + piece.len()).min(line.len());
            if end > at {
                let c = style.foreground;
                out.push((at, end, Color32::from_rgb(c.r, c.g, c.b), style.font_style.contains(FontStyle::ITALIC)));
            }
            at += piece.len();
        }
        out
    }
}

const KW_COMMON: &[&str] = &[
    "if", "else", "for", "while", "return", "break", "continue", "in", "true", "false", "null", "None", "True", "False",
];

fn keywords(ext: &str) -> &'static [&'static str] {
    match ext {
        "rs" => &[
            "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "match", "loop", "where", "as",
            "const", "static", "self", "Self", "crate", "super", "move", "ref", "async", "await", "dyn", "unsafe", "type",
            "Some", "None", "Ok", "Err",
        ],
        "py" => &[
            "def", "class", "import", "from", "as", "elif", "try", "except", "finally", "with", "lambda", "yield", "pass",
            "raise", "global", "nonlocal", "and", "or", "not", "is", "async", "await", "self", "assert", "del",
        ],
        "sh" | "bash" | "zsh" => &[
            "then", "fi", "do", "done", "case", "esac", "elif", "function", "local", "export", "unset", "set", "shift",
            "echo", "printf", "read", "exec", "eval", "source", "trap", "until",
        ],
        "c" | "h" | "cpp" | "hpp" | "cc" | "go" | "java" | "js" | "ts" | "tsx" | "jsx" => &[
            "int", "char", "void", "long", "short", "unsigned", "float", "double", "struct", "typedef", "static", "const",
            "switch", "case", "default", "do", "goto", "sizeof", "class", "public", "private", "protected", "new",
            "delete", "namespace", "template", "func", "package", "import", "var", "let", "function", "interface",
            "type", "export", "async", "await", "this", "try", "catch", "throw", "auto", "include", "define",
        ],
        _ => &[],
    }
}

fn comment_prefix(ext: &str) -> &'static [&'static str] {
    match ext {
        "py" | "sh" | "bash" | "zsh" | "toml" | "yaml" | "yml" | "conf" | "cfg" | "ini" | "rb" | "pl" | "" => &["#"],
        "lua" | "sql" | "hs" => &["--"],
        "vim" => &["\""],
        _ => &["//", "#"],
    }
}

pub fn ext_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => {
            if name == "Makefile" || name.starts_with(".bash") || name.starts_with(".profile") {
                "sh".into()
            } else {
                String::new()
            }
        }
    }
}

/// Extension for a code fence's language name (```rust -> "rs").
pub fn ext_of_lang(lang: &str) -> String {
    let l = lang.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    match l.as_str() {
        "rust" => "rs",
        "python" | "python3" => "py",
        "shell" | "console" | "shellsession" | "bash" | "zsh" | "sh" => "sh",
        "c++" | "cxx" => "cpp",
        "javascript" | "node" => "js",
        "typescript" => "ts",
        "golang" => "go",
        "yml" => "yaml",
        "powershell" | "ps1" | "pwsh" => "ps1",
        _ => return l,
    }
    .to_string()
}

/// Colored spans (byte ranges) covering all of `text`.
pub fn spans(text: &str, ext: &str) -> Vec<(usize, usize, Color32)> {
    let kws = keywords(ext);
    let comments = comment_prefix(ext);
    let c_text = theme::text();
    let c_kw = theme::ansi(5);
    let c_str = theme::ansi(2);
    let c_num = theme::ansi(3);
    let c_com = theme::text_faint();
    let c_fn = theme::ansi(4);

    let mut spans: Vec<(usize, usize, Color32)> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        let b = bytes[i];
        if let Some(cp) = comments.iter().find(|cp| rest.starts_with(**cp)) {
            // '#' in C-like files only counts at line start (preprocessor) — keep it simple.
            let _ = cp;
            let end = rest.find('\n').map(|e| i + e).unwrap_or(bytes.len());
            spans.push((i, end, c_com));
            i = end;
            continue;
        }
        if b == b'"' || b == b'\'' || b == b'`' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b && bytes[j] != b'\n' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(bytes.len());
            spans.push((i, end, c_str));
            i = end;
            continue;
        }
        if b.is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.' || bytes[j] == b'_') {
                j += 1;
            }
            spans.push((i, j, c_num));
            i = j;
            continue;
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let word = &text[i..j];
            let color = if kws.contains(&word) || KW_COMMON.contains(&word) {
                c_kw
            } else if bytes.get(j) == Some(&b'(') {
                c_fn
            } else {
                c_text
            };
            spans.push((i, j, color));
            i = j;
            continue;
        }
        // Advance one UTF-8 character.
        let len = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        spans.push((i, i + len, c_text));
        i += len;
    }
    spans
}

type Span = (usize, usize, Color32, bool);

/// One highlighted line: its text, the grammar state it started in, and its
/// spans (relative to the line).
#[derive(Clone)]
struct CachedLine {
    text: String,
    start: (ParseState, HighlightState),
    spans: Vec<Span>,
}

/// Highlighting of one open file, kept between edits: only lines whose text
/// or starting state changed are highlighted again, so typing re-colors a
/// line or two instead of the whole file.
#[derive(Default)]
pub struct FileState {
    key: String,
    lines: Vec<CachedLine>,
    /// The state after the last line.
    end: Option<(ParseState, HighlightState)>,
    /// A background first pass has been started.
    warming: bool,
}

fn highlight_one(line: &str, ps: &mut ParseState, hs: &mut HighlightState, hl: &Highlighter, set: &SyntaxSet) -> Vec<Span> {
    let with_nl = format!("{line}\n");
    let Ok(ops) = ps.parse_line(&with_nl, set) else {
        return vec![(0, line.len(), theme::text(), false)];
    };
    let mut out = Vec::new();
    let mut at = 0;
    for (style, piece) in HighlightIterator::new(hs, &ops, &with_nl, hl) {
        let end = (at + piece.len()).min(line.len());
        if end > at {
            let c = style.foreground;
            out.push((at, end, Color32::from_rgb(c.r, c.g, c.b), style.font_style.contains(FontStyle::ITALIC)));
        }
        at += piece.len();
    }
    out
}

/// Spans for a whole file: the grammars when loaded, reusing `state` from the
/// previous call; else the quick highlighter.
fn file_spans(text: &str, ext: &str, state: &mut FileState) -> Vec<Span> {
    let quick = || spans(text, ext).into_iter().map(|(a, b, c)| (a, b, c, false)).collect();
    let Some(set) = SYNTAXES.get().filter(|_| text.len() <= MAX_FULL) else { return quick() };
    let first = text.lines().next().unwrap_or("");
    let Some(syntax) = syntax_for(set, ext, first) else { return quick() };
    let key = format!("{}|{}", syntax.name, theme::active());
    if state.key != key {
        *state = FileState { key, ..Default::default() };
    }
    let highlighter = Highlighter::new(syntect_theme());
    let lines: Vec<&str> = text.split('\n').collect();
    let old = std::mem::take(&mut state.lines);
    // Unchanged lines at the start keep everything; at the end they keep
    // their spans once the grammar state at that point is what it was.
    let prefix = old.iter().zip(&lines).take_while(|(o, l)| o.text == **l).count();
    let suffix = old.iter().rev().zip(lines.iter().rev()).take_while(|(o, l)| o.text == **l).count().min(lines.len() - prefix).min(old.len() - prefix);
    let (mut ps, mut hs) = match old.get(prefix) {
        Some(c) => c.start.clone(),
        None if prefix > 0 && prefix == old.len() => state.end.clone().unwrap_or_else(|| (ParseState::new(syntax), HighlightState::new(&highlighter, ScopeStack::new()))),
        None => (ParseState::new(syntax), HighlightState::new(&highlighter, ScopeStack::new())),
    };
    let mut new: Vec<CachedLine> = old[..prefix].to_vec();
    let old_suffix_start = old.len() - suffix;
    let mut i = prefix;
    while i < lines.len() {
        let line = lines[i].strip_suffix('\r').unwrap_or(lines[i]);
        // In the unchanged tail and back in step: reuse the rest as it was.
        if i >= lines.len() - suffix {
            let o = &old[old_suffix_start + (i - (lines.len() - suffix))];
            if o.start == (ps.clone(), hs.clone()) {
                new.extend_from_slice(&old[old_suffix_start + (i - (lines.len() - suffix))..]);
                state.lines = new;
                return assemble(text, &state.lines);
            }
        }
        let start = (ps.clone(), hs.clone());
        let spans = highlight_one(line, &mut ps, &mut hs, &highlighter, set);
        new.push(CachedLine { text: lines[i].to_string(), start, spans });
        i += 1;
    }
    state.end = Some((ps, hs));
    state.lines = new;
    assemble(text, &state.lines)
}

/// Absolute spans covering all of `text` from per-line spans.
fn assemble(text: &str, lines: &[CachedLine]) -> Vec<Span> {
    let mut out = Vec::new();
    let mut at = 0;
    for (raw, c) in text.split('\n').zip(lines) {
        for &(a, b, color, it) in &c.spans {
            out.push((at + a, at + b, color, it));
        }
        let line_end = at + raw.len();
        let covered = at + c.spans.last().map(|s| s.1).unwrap_or(0);
        if covered < line_end {
            out.push((covered, line_end, theme::text(), false));
        }
        if line_end < text.len() {
            out.push((line_end, line_end + 1, theme::text(), false));
        }
        at = line_end + 1;
    }
    out
}

/// Highlighted layout job. `marks` (find results, byte ranges) get a
/// background, `cur` (the current one) a stronger one.
/// `state` carries the highlighting of the previous version of this file; a
/// big file's first pass goes to a background thread (quick colors meanwhile).
pub fn layout(text: &str, ext: &str, font: FontId, marks: &[(usize, usize)], cur: Option<(usize, usize)>, state: &std::sync::Arc<Mutex<FileState>>, ctx: &egui::Context) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |c: Color32, italics: bool| TextFormat { font_id: font.clone(), color: c, italics, ..Default::default() };
    let quick = || spans(text, ext).into_iter().map(|(a, b, c)| (a, b, c, false)).collect::<Vec<_>>();
    let spans = match state.try_lock() {
        // A background pass is running (or about to take the lock).
        Err(_) => quick(),
        Ok(st) if st.warming => quick(),
        Ok(mut st) => {
            let fresh = st.lines.is_empty() && !st.warming;
            if fresh && ready() && text.len() <= MAX_FULL && text.matches('\n').count() > BACKGROUND_LINES {
                st.warming = true;
                drop(st);
                let (state, text, ext, ctx) = (state.clone(), text.to_string(), ext.to_string(), ctx.clone());
                std::thread::spawn(move || {
                    let mut st = state.lock().unwrap();
                    file_spans(&text, &ext, &mut st);
                    st.warming = false;
                    drop(st);
                    WARMED.fetch_add(1, Ordering::Relaxed);
                    ctx.request_repaint();
                });
                quick()
            } else {
                file_spans(text, ext, &mut st)
            }
        }
    };

    let hit_bg = theme::search_hit();
    let mut mi = 0;
    for (s, e, c, italics) in spans {
        // Split the span at find-mark boundaries.
        let mut pos = s;
        while pos < e {
            while mi < marks.len() && marks[mi].1 <= pos {
                mi += 1;
            }
            let (next, in_mark) = match marks.get(mi) {
                Some(&(ms, me)) if ms <= pos => (me.min(e), true),
                Some(&(ms, _)) if ms < e => (ms, false),
                _ => (e, false),
            };
            let mut f = fmt(c, italics);
            if in_mark {
                f.background = if cur.map(|c| c.0 <= pos && pos < c.1).unwrap_or(false) { current_hit() } else { hit_bg };
            }
            if text.is_char_boundary(pos) && text.is_char_boundary(next) {
                job.append(&text[pos..next], 0.0, f);
            }
            pos = next;
        }
    }
    job
}

/// Background of the current find result.
pub fn current_hit() -> Color32 {
    theme::tint(theme::warn(), 150)
}

/// Paint find results onto an already laid-out job (rendered markdown):
/// sections are split at the marks' edges, which get a background.
pub fn mark_job(job: &mut LayoutJob, marks: &[(usize, usize)], cur: Option<(usize, usize)>) {
    if marks.is_empty() {
        return;
    }
    let mut cuts: Vec<usize> = marks.iter().flat_map(|m| [m.0, m.1]).collect();
    cuts.sort_unstable();
    let mut out = Vec::with_capacity(job.sections.len() + cuts.len());
    for s in std::mem::take(&mut job.sections) {
        let mut start = s.byte_range.start;
        let end = s.byte_range.end;
        let inner: Vec<usize> = cuts.iter().copied().filter(|&c| c > start && c < end && job.text.is_char_boundary(c)).collect();
        for c in inner.into_iter().chain(std::iter::once(end)) {
            let mut sec = s.clone();
            sec.leading_space = if start == s.byte_range.start { s.leading_space } else { 0.0 };
            sec.byte_range = start..c;
            if let Some(m) = marks.iter().find(|m| m.0 <= start && start < m.1) {
                sec.format.background = if Some(*m) == cur { current_hit() } else { theme::search_hit() };
            }
            out.push(sec);
            start = c;
        }
    }
    job.sections = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_split_sections() {
        let mut job = LayoutJob::default();
        job.append("hello ", 0.0, TextFormat::default());
        job.append("world", 0.0, TextFormat::default());
        mark_job(&mut job, &[(3, 8)], Some((3, 8)));
        let ranges: Vec<_> = job.sections.iter().map(|s| (s.byte_range.clone(), s.format.background != Color32::TRANSPARENT)).collect();
        assert_eq!(ranges, [(0..3, false), (3..6, true), (6..8, true), (8..11, false)]);
    }

    fn load() {
        SYNTAXES.get_or_init(two_face::syntax::extra_newlines);
    }

    #[test]
    fn bat_grammars_color_many_languages() {
        load();
        let set = SYNTAXES.get().unwrap();
        for (name, first) in [("rs", ""), ("c", ""), ("py", ""), ("toml", ""), ("ts", ""), ("Dockerfile", ""), ("", "#!/usr/bin/env bash")] {
            assert!(syntax_for(set, name, first).is_some(), "{name:?} {first:?}");
        }
        // A keyword and a plain name get different colors; a C block comment
        // spans lines.
        let mut l = Lines::new("c", "");
        let a = l.line("int main() { /* start");
        let b = l.line("still comment */ return 0; }");
        let color_at = |spans: &[(usize, usize, Color32, bool)], at: usize| spans.iter().find(|s| s.0 <= at && at < s.1).map(|s| s.2).unwrap();
        assert_ne!(color_at(&a, 0), color_at(&a, 4), "int vs main");
        assert_eq!(color_at(&b, 0), color_at(&a, 15), "comment continues on the next line");
        assert!(b.iter().all(|s| s.1 <= "still comment */ return 0; }".len()));
    }

    /// Editing re-highlights only what changed, and the result is the same
    /// as highlighting the edited text from scratch.
    #[test]
    fn incremental_matches_full() {
        load();
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/find.rs")).unwrap();
        let mut st = FileState::default();
        let full = file_spans(&text, "rs", &mut st);
        let covered: usize = full.iter().map(|s| s.1 - s.0).sum();
        assert_eq!(covered, text.len(), "spans cover the text");
        // Type in the middle, open a comment, add and delete lines.
        let mid = text.find("fn fuzzy").unwrap();
        for edited in [
            format!("{}x{}", &text[..mid], &text[mid..]),
            format!("{}/* {}", &text[..mid], &text[mid..]),
            format!("{}\n\n{}", &text[..mid], &text[mid..]),
            text[..mid].to_string(),
            text.clone(),
        ] {
            let inc = file_spans(&edited, "rs", &mut st);
            let fresh = file_spans(&edited, "rs", &mut FileState::default());
            assert!(inc == fresh, "incremental result differs after an edit");
        }
    }

    #[test]
    #[ignore]
    fn highlighting_a_large_file_is_fast() {
        load();
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui_panels.rs")).unwrap();
        let mut st = FileState::default();
        let t = std::time::Instant::now();
        let spans = file_spans(&text, "rs", &mut st);
        let open = t.elapsed().as_secs_f64() * 1000.0;
        let at = text.len() / 2;
        let edited = format!("{}x{}", &text[..at], &text[at..]);
        let t = std::time::Instant::now();
        file_spans(&edited, "rs", &mut st);
        let typed = t.elapsed().as_secs_f64() * 1000.0;
        println!("{} lines, {} spans: open {open:.1} ms, one keystroke {typed:.1} ms", text.lines().count(), spans.len());
    }
}
