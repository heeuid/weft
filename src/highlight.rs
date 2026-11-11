//! Small, fast, local syntax highlighter for the editor (no remote work).

use crate::theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

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

/// Highlighted layout job.  `find` (case-insensitive) matches get a background.
pub fn layout(text: &str, ext: &str, font: FontId, find: &str) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |c: Color32| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
    let spans = spans(text, ext);

    // Find highlights (char-boundary safe, ASCII case-insensitive).
    let mut marks: Vec<(usize, usize)> = Vec::new();
    if find.len() >= 2 {
        let hay = text.to_ascii_lowercase();
        let needle = find.to_ascii_lowercase();
        let mut from = 0;
        while let Some(p) = hay[from..].find(&needle) {
            marks.push((from + p, from + p + needle.len()));
            from += p + needle.len();
        }
    }
    let hit_bg = theme::search_hit();
    let mut mi = 0;
    for (s, e, c) in spans {
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
            let mut f = fmt(c);
            if in_mark {
                f.background = hit_bg;
            }
            if text.is_char_boundary(pos) && text.is_char_boundary(next) {
                job.append(&text[pos..next], 0.0, f);
            }
            pos = next;
        }
    }
    job
}
