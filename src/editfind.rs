//! Find and replace in the editor (Ctrl+F, Ctrl+H): the text as typed or a
//! regular expression, with or without case; replace one match or all, with
//! $1-style groups in regex mode.

use regex::{NoExpand, Regex, RegexBuilder};

#[derive(Clone, Default)]
pub struct FindBar {
    pub query: String,
    pub replace: String,
    /// Match case (else any case).
    pub case: bool,
    pub regex: bool,
    /// The replace row is shown.
    pub replacing: bool,
    /// Index of the current match.
    pub cur: usize,
    /// Put the keyboard in the query field next frame.
    pub focus: bool,
}

impl FindBar {
    fn regex(&self) -> Result<Regex, String> {
        let pat = if self.regex { self.query.clone() } else { regex::escape(&self.query) };
        RegexBuilder::new(&pat).case_insensitive(!self.case).multi_line(true).build().map_err(|e| e.to_string().lines().last().unwrap_or("bad regex").trim().to_string())
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
        if self.regex {
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
        *text = if self.regex { re.replace_all(text, self.replace.as_str()).into_owned() } else { re.replace_all(text, NoExpand(&self.replace)).into_owned() };
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
        FindBar { query: q.into(), case, regex, ..Default::default() }
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
