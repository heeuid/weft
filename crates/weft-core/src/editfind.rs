//! Searching text: in the editor (Ctrl+F, Ctrl+H) and in the Search tab.
//! Four ways to read the query (MatchMode): the text as typed, fuzzy
//! (its characters in order), glob (* ? [..]) or a regular expression. All
//! but text become an extended regex (the syntax both Rust's regex and
//! `grep -E` take), so a remote grep finds what the editor would.

use regex::{NoExpand, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MatchMode {
    #[default]
    Text,
    Fuzzy,
    Glob,
    Regex,
}

impl MatchMode {
    pub const ALL: [MatchMode; 4] = [MatchMode::Text, MatchMode::Fuzzy, MatchMode::Glob, MatchMode::Regex];

    pub fn label(self) -> &'static str {
        match self {
            MatchMode::Text => "text",
            MatchMode::Fuzzy => "fuzzy",
            MatchMode::Glob => "glob",
            MatchMode::Regex => "regex",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            MatchMode::Text => "the text as typed",
            MatchMode::Fuzzy => "its characters in this order on one line: gst finds git status (any case unless Aa)",
            MatchMode::Glob => "* any run, ? one character, [ab] one of: Start*(int",
            MatchMode::Regex => "extended regex: ( | + ? are operators, \\( a bracket; $1 in a replacement",
        }
    }

    /// Fuzzy ignores case unless asked: its point is typing less.
    pub fn ignores_case(self, case: bool) -> bool {
        !case
    }
}

/// Escape for extended regex: what both Rust's regex and grep -E read as
/// the character itself (regex::escape also escapes - # & ~, which some
/// greps warn about).
pub fn ere_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ".[]()*+?{}|^$\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn glob_to_ere(g: &str) -> String {
    let cs: Vec<char> = g.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '[' => {
                // A class runs to the next ]; [! is [^. No ]: a plain [.
                match cs[i + 1..].iter().position(|&c| c == ']') {
                    Some(n) if n > 0 => {
                        let body: String = cs[i + 1..i + 1 + n].iter().collect();
                        let body = body.strip_prefix('!').map(|b| format!("^{b}")).unwrap_or(body);
                        out.push('[');
                        out.push_str(&body.replace('\\', "\\\\"));
                        out.push(']');
                        i += n + 1;
                    }
                    _ => out.push_str("\\["),
                }
            }
            c => out.push_str(&ere_escape(&c.to_string())),
        }
        i += 1;
    }
    out
}

/// The query as an extended regex for `mode`. `lazy` keeps fuzzy matches
/// short (Rust regex only; grep -E has no lazy repeat, nor needs one).
pub fn to_regex(mode: MatchMode, q: &str, lazy: bool) -> String {
    match mode {
        MatchMode::Text => ere_escape(q),
        MatchMode::Regex => q.to_string(),
        MatchMode::Glob => glob_to_ere(q),
        MatchMode::Fuzzy => {
            let gap = if lazy { ".*?" } else { ".*" };
            q.chars().filter(|c| !c.is_whitespace()).map(|c| ere_escape(&c.to_string())).collect::<Vec<_>>().join(gap)
        }
    }
}

/// A regex for marking matches in one line of text (the Search tab's
/// results), or None when the query can't be one.
pub fn line_matcher(mode: MatchMode, q: &str, icase: bool) -> Option<Regex> {
    if q.is_empty() {
        return None;
    }
    RegexBuilder::new(&to_regex(mode, q, true)).case_insensitive(icase).build().ok()
}

/// Byte ranges matched in `text` (empty matches skipped).
pub fn line_marks(re: &Regex, text: &str) -> Vec<(usize, usize)> {
    re.find_iter(text).map(|m| (m.start(), m.end())).filter(|m| m.0 < m.1).collect()
}

#[derive(Clone, Default)]
pub struct FindBar {
    pub query: String,
    pub replace: String,
    /// Match case (else any case).
    pub case: bool,
    pub mode: MatchMode,
    /// The replace row is shown.
    pub replacing: bool,
    /// Index of the current match.
    pub cur: usize,
    /// Put the keyboard in the query field next frame.
    pub focus: bool,
}

impl FindBar {
    fn regex(&self) -> Result<Regex, String> {
        RegexBuilder::new(&to_regex(self.mode, &self.query, true))
            .case_insensitive(self.mode.ignores_case(self.case))
            .multi_line(true)
            .build()
            .map_err(|e| e.to_string().lines().last().unwrap_or("bad regex").trim().to_string())
    }

    /// Byte ranges of every match (empty matches skipped).
    pub fn matches(&self, text: &str) -> Result<Vec<(usize, usize)>, String> {
        if self.query.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.regex()?.find_iter(text).map(|m| (m.start(), m.end())).filter(|m| m.0 < m.1).collect())
    }

    /// Replace the match at `m`; returns where the replacement ends.
    pub fn replace_at(&self, text: &mut String, m: (usize, usize)) -> Result<usize, String> {
        let re = self.regex()?;
        let caps = re.captures_at(text, m.0).filter(|c| c.get(0).map(|g| (g.start(), g.end())) == Some(m)).ok_or("the match moved")?;
        let mut new = String::new();
        if self.mode == MatchMode::Regex {
            caps.expand(&self.replace, &mut new);
        } else {
            new.push_str(&self.replace);
        }
        text.replace_range(m.0..m.1, &new);
        Ok(m.0 + new.len())
    }

    /// Replace every match; returns how many.
    pub fn replace_all(&self, text: &mut String) -> Result<usize, String> {
        let n = self.matches(text)?.len();
        if n == 0 {
            return Ok(0);
        }
        let re = self.regex()?;
        *text = if self.mode == MatchMode::Regex { re.replace_all(text, self.replace.as_str()).into_owned() } else { re.replace_all(text, NoExpand(&self.replace)).into_owned() };
        Ok(n)
    }
}

/// Char index of a byte offset (egui cursors count chars).
pub fn char_index(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

/// 1-based line of a byte offset.
pub fn line_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(q: &str, case: bool, regex: bool) -> FindBar {
        FindBar { query: q.into(), case, mode: if regex { MatchMode::Regex } else { MatchMode::Text }, ..Default::default() }
    }

    fn found(mode: MatchMode, q: &str, t: &str) -> Vec<String> {
        let b = FindBar { query: q.into(), mode, ..Default::default() };
        b.matches(t).unwrap().into_iter().map(|(a, e)| t[a..e].to_string()).collect()
    }

    #[test]
    fn search_result_marks() {
        let line = "void Start(int x); // start here";
        let marks = |mode, q: &str, icase| line_matcher(mode, q, icase).map(|re| line_marks(&re, line).into_iter().map(|(a, b)| &line[a..b]).collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!(marks(MatchMode::Text, "start", true), ["Start", "start"]);
        assert_eq!(marks(MatchMode::Text, "start", false), ["start"]);
        assert_eq!(marks(MatchMode::Text, "Start(", false), ["Start("]);
        assert_eq!(marks(MatchMode::Fuzzy, "sti", true), ["Start(i"], "fuzzy: shortest run");
        assert_eq!(marks(MatchMode::Glob, "S*(", false), ["Start("]);
        assert_eq!(marks(MatchMode::Regex, r"\w+\(", false), ["Start("]);
        assert!(marks(MatchMode::Regex, "(", false).is_empty(), "a bad regex marks nothing");
        assert!(marks(MatchMode::Text, "", false).is_empty());
    }

    #[test]
    fn fuzzy_and_glob() {
        let t = "git status\ngit stash\nvoid Start(int x);\n[x] done";
        // Fuzzy: the characters in order, on one line, any case.
        assert_eq!(found(MatchMode::Fuzzy, "gst", t), ["git st", "git st"]);
        assert_eq!(found(MatchMode::Fuzzy, "STtus", t), ["status"]);
        assert!(found(MatchMode::Fuzzy, "gitvoid", t).is_empty(), "never across lines");
        // Glob: * ? [..], the rest as typed (brackets and parens too).
        assert_eq!(found(MatchMode::Glob, "Start*(int", t), ["Start(int"]);
        assert_eq!(found(MatchMode::Glob, "st?sh", t), ["stash"]);
        assert_eq!(found(MatchMode::Glob, "git sta[!t]", t), ["git stas"], "[!..]: none of these");
        assert_eq!(found(MatchMode::Glob, "[cd]one", t), ["done"], "[..]: one of these");
        assert!(found(MatchMode::Glob, "[x] d*", t).is_empty(), "[x] is a class, not the text [x]");
        assert_eq!(found(MatchMode::Glob, "\\[x", t).len(), 0);
        // The same text for grep -E: no Rust-only syntax.
        assert_eq!(to_regex(MatchMode::Fuzzy, "a.b", false), "a.*\\..*b");
        assert_eq!(to_regex(MatchMode::Glob, "*.rs", false), ".*\\.rs");
        assert_eq!(to_regex(MatchMode::Glob, "[!ab]x", false), "[^ab]x");
        assert_eq!(to_regex(MatchMode::Text, "f(x)+1", false), "f\\(x\\)\\+1");
    }

    #[test]
    fn finds_text_or_regex() {
        let t = "Start(a)\nstart(b)\nrestart";
        assert_eq!(bar("start(", false, false).matches(t).unwrap(), [(0, 6), (9, 15)]);
        assert_eq!(bar("start(", true, false).matches(t).unwrap(), [(9, 15)]);
        assert_eq!(bar(r"^start\(", false, true).matches(t).unwrap(), [(0, 6), (9, 15)], "^ at each line");
        assert_eq!(bar("start", false, true).matches(t).unwrap().len(), 3);
        assert!(bar("start(", false, true).matches(t).is_err(), "unclosed group");
        assert!(bar("", false, false).matches(t).unwrap().is_empty());
        assert!(bar("x*", false, true).matches(t).unwrap().is_empty(), "empty matches skipped");
    }

    #[test]
    fn replaces_one_or_all() {
        let mut t = "foo(1) foo(2) 한글 foo(3)".to_string();
        let mut b = bar(r"foo\((\d)\)", true, true);
        b.replace = "bar[$1]".into();
        let ms = b.matches(&t).unwrap();
        let end = b.replace_at(&mut t, ms[1]).unwrap();
        assert_eq!(t, "foo(1) bar[2] 한글 foo(3)");
        assert_eq!(&t[..end], "foo(1) bar[2]");
        assert_eq!(b.replace_all(&mut t).unwrap(), 2);
        assert_eq!(t, "bar[1] bar[2] 한글 bar[3]");
        // Text mode: $ is a character.
        let mut b = bar("한글", true, false);
        b.replace = "$1".into();
        assert_eq!(b.replace_all(&mut t).unwrap(), 1);
        assert_eq!(t, "bar[1] bar[2] $1 bar[3]");
        assert_eq!(char_index(&t, t.find("$1").unwrap()), 14);
        assert_eq!(line_of("a\nb\nc", 4), 3);
    }
}
