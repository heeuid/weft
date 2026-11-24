//! The editor's Find bar (Ctrl+F, Ctrl+H): what it finds is found by
//! crate::find's matcher, as everywhere else in weft (the same four modes,
//! line by line). Replacing works in text and regex modes.

pub use crate::find::MatchMode;
use crate::find::{Matcher, Target};
use regex::NoExpand;

/// What the bar found: byte ranges, or why the query can't be read.
type Found = Result<Vec<(usize, usize)>, String>;

/// The bar's last matches and a hash of what they were of.
#[derive(Clone, Default)]
pub struct FindCache(Option<(u64, Found)>);

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
    /// The last matches, and of what (the bar is drawn every frame: a long
    /// file is not searched again until something changed).
    pub cache: FindCache,
}

impl FindBar {
    fn matcher(&self) -> Result<Matcher, String> {
        Matcher::build(self.mode, &self.query, Target::Line, Some(self.case))
    }

    /// Replacing works on text and regex (a glob is whole lines, fuzzy
    /// letters apart: nothing that would be put back whole).
    pub fn can_replace(&self) -> bool {
        matches!(self.mode, MatchMode::Text | MatchMode::Regex)
    }

    /// `matches`, kept until the query, its mode or case, or the text changes.
    pub fn matches_cached(&mut self, text: &str) -> Result<Vec<(usize, usize)>, String> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&self.query, self.mode, self.case, text).hash(&mut h);
        let key = h.finish();
        if let Some((k, r)) = &self.cache.0 {
            if *k == key {
                return r.clone();
            }
        }
        let r = self.matches(text);
        self.cache = FindCache(Some((key, r.clone())));
        r
    }

    /// Byte ranges of every match (empty matches skipped).
    pub fn matches(&self, text: &str) -> Result<Vec<(usize, usize)>, String> {
        if self.query.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.matcher()?.find_all(text))
    }

    /// Replace the match at `m`; returns where the replacement ends.
    pub fn replace_at(&self, text: &mut String, m: (usize, usize)) -> Result<usize, String> {
        let matcher = self.matcher()?;
        let re = matcher.regex().ok_or("replacing works in text and regex modes")?;
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
        let matcher = self.matcher()?;
        let re = matcher.regex().ok_or("replacing works in text and regex modes")?;
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
    fn fuzzy_and_glob() {
        let t = "git status\ngit stash\nvoid Start(int x);\n[x] done";
        // Fuzzy: the characters in order, on one line, any case.
        assert_eq!(found(MatchMode::Fuzzy, "gst", t), ["git st", "git st"]);
        assert_eq!(found(MatchMode::Fuzzy, "STtus", t), ["status"]);
        assert!(found(MatchMode::Fuzzy, "gitvoid", t).is_empty(), "never across lines");
        // Glob: the whole line, as everywhere (* ? [..] {a,b}).
        assert_eq!(found(MatchMode::Glob, "*Start*(int*", t), ["void Start(int x);"]);
        assert_eq!(found(MatchMode::Glob, "git st?sh", t), ["git stash"]);
        assert_eq!(found(MatchMode::Glob, "git sta[!t]*", t), ["git stash"], "[!..]: none of these");
        assert_eq!(found(MatchMode::Glob, "*[cd]one", t), ["[x] done"], "[..]: one of these");
        assert!(found(MatchMode::Glob, "[x] d*", t).is_empty(), "[x] is a class, not the text [x]");
        assert_eq!(found(MatchMode::Glob, "\\[x] *", t), ["[x] done"], "\\[ is the text [");
        assert_eq!(found(MatchMode::Glob, "git {status,stash}", t), ["git status", "git stash"]);
        // Replacing: text and regex only.
        let mut b = FindBar { query: "git*".into(), mode: MatchMode::Glob, ..Default::default() };
        assert!(!b.can_replace() && b.replace_all(&mut t.to_string()).is_err());
        b.mode = MatchMode::Text;
        assert!(b.can_replace());
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
