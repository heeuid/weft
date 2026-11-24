//! How weft matches what is typed, everywhere: Files' filter, the palette,
//! History, the editor's Find bar, the Search tab and the serial log. One
//! mode means one thing wherever it is used, each by a library made for it:
//!
//! * text: the text as typed, anywhere (any case unless asked);
//! * fuzzy: fzf's matching and scoring (the nucleo matcher, as the Helix
//!   editor uses it): each word of the query, its characters in order
//!   anywhere, any case; runs, word and path-part starts score higher;
//! * glob: ripgrep's (globset): `*.rs`, `src/**/test_*`, `{a,b}.md`,
//!   `[ab]?` — the whole name (a path's, without a `/` in the pattern) or
//!   the whole line; text with no wildcard is found anywhere in it;
//! * regex: the regex crate, smart case (any case unless the query has a
//!   capital letter, or as asked).
//!
//! Text in lines (the editor, the Search tab, a log) is matched line by
//! line with the same matcher. A search on a remote host asks its `grep -E`
//! for the lines that could match (`Matcher::grep`), and the matcher here
//! decides (`exact_on_grep`): the host's grep only narrows them down.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::Utf32Str;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

thread_local! {
    /// nucleo's matcher keeps its working memory between calls: one per
    /// thread, made once.
    static NUCLEO: RefCell<nucleo_matcher::Matcher> = RefCell::new(nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT));
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
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

    /// Short: it is the hint in a narrow field too.
    pub fn hint(self) -> &'static str {
        match self {
            MatchMode::Text => "text: as typed, anywhere",
            MatchMode::Fuzzy => "fuzzy: letters in order, e.g. srcmain",
            MatchMode::Glob => "glob: *.rs, *TODO*, {a,b}",
            MatchMode::Regex => "regex: ^src/.*\\.rs$",
        }
    }

    /// Said on hover: what the mode does, in full.
    pub fn about(self) -> &'static str {
        match self {
            MatchMode::Text => "the text as typed, found anywhere",
            MatchMode::Fuzzy => "fzf's matching: the letters in order (gst finds git status), words typed apart each anywhere, the best first",
            MatchMode::Glob => "ripgrep's glob: * ? [ab] {a,b} match the whole name or line (*.rs, *TODO*); text with no wildcard is found anywhere",
            MatchMode::Regex => "a regular expression, any case unless it has a capital letter (or Aa); $1 in a replacement",
        }
    }

    /// Aa off: any case.
    pub fn ignores_case(self, case: bool) -> bool {
        !case
    }
}

/// What a query is matched against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// A relative path (Files): a glob's * stays in one folder, and a
    /// glob or text without a `/` is matched against the name.
    Path,
    /// A whole line: a command, a palette label, a line of text.
    Line,
}

enum Kind {
    /// Text, and glob text with no wildcard: found anywhere.
    Contains(Regex),
    Regex(Regex),
    Glob(globset::GlobMatcher),
    Fuzzy(Pattern),
}

pub struct Matcher {
    kind: Kind,
    target: Target,
    /// A path's whole, not only its name.
    full_path: bool,
    mode: MatchMode,
    query: String,
    /// Some(true): case as typed; Some(false): any case; None: the mode's
    /// own (regex smart, the rest any case).
    case: Option<bool>,
}

impl Matcher {
    /// For a path (Files), each mode's own case.
    pub fn new(mode: MatchMode, query: &str) -> Result<Matcher, String> {
        Self::build(mode, query, Target::Path, None)
    }

    /// For a whole line (History's commands, the palette's labels), each
    /// mode's own case.
    pub fn for_lines(mode: MatchMode, query: &str) -> Result<Matcher, String> {
        Self::build(mode, query, Target::Line, None)
    }

    pub fn build(mode: MatchMode, query: &str, target: Target, case: Option<bool>) -> Result<Matcher, String> {
        let any_case = case.map(|c| !c);
        let regex = |pat: &str, icase: bool| RegexBuilder::new(pat).case_insensitive(icase).multi_line(true).build().map_err(|e| e.to_string().lines().last().unwrap_or("bad regex").trim().to_string());
        let path_whole = |q: &str| target == Target::Line || q.contains('/');
        let (kind, full_path) = match mode {
            MatchMode::Text => (Kind::Contains(regex(&regex::escape(query), any_case.unwrap_or(true))?), path_whole(query)),
            MatchMode::Regex => (Kind::Regex(regex(query, any_case.unwrap_or(!query.chars().any(char::is_uppercase)))?), true),
            MatchMode::Fuzzy => {
                let c = if any_case == Some(false) { CaseMatching::Respect } else { CaseMatching::Ignore };
                (Kind::Fuzzy(Pattern::new(query, c, Normalization::Smart, AtomKind::Fuzzy)), true)
            }
            MatchMode::Glob => {
                let q = query.trim();
                if q.is_empty() {
                    return Err("type a pattern".into());
                }
                let icase = any_case.unwrap_or(true);
                if q.contains(['*', '?', '[', '{']) {
                    let g = globset::GlobBuilder::new(q).case_insensitive(icase).literal_separator(target == Target::Path).backslash_escape(true).empty_alternates(true).build().map_err(|e| e.to_string())?;
                    (Kind::Glob(g.compile_matcher()), path_whole(q))
                } else {
                    (Kind::Contains(regex(&regex::escape(q), icase)?), path_whole(q))
                }
            }
        };
        Ok(Matcher { kind, target, full_path, mode, query: query.to_string(), case })
    }

    /// Score and the matched char positions in `s` (a path or a line), or
    /// None.
    pub fn matches(&self, s: &str) -> Option<(i64, Vec<usize>)> {
        let name_at = if self.full_path { 0 } else { s.rfind('/').map(|i| i + 1).unwrap_or(0) };
        let target = &s[name_at..];
        let skip = s[..name_at].chars().count();
        match &self.kind {
            Kind::Fuzzy(p) => fuzzy(p, s),
            Kind::Contains(re) | Kind::Regex(re) => {
                let m = re.find(target)?;
                let from = skip + target[..m.start()].chars().count();
                Some((0, (from..from + target[m.start()..m.end()].chars().count()).collect()))
            }
            // The whole name (or path, or line) matched: all of it marked.
            Kind::Glob(g) => g.is_match(target).then(|| (0, (skip..skip + target.chars().count()).collect())),
        }
    }

    /// Ranked best first (fuzzy), else in the list's order.
    pub fn ranked(&self) -> bool {
        matches!(self.kind, Kind::Fuzzy(_))
    }

    /// Byte ranges matched in one line (empty matches left out): text and
    /// regex each place, a glob the whole line, fuzzy from its first
    /// character matched to its last.
    pub fn find_in(&self, line: &str) -> Vec<(usize, usize)> {
        match &self.kind {
            Kind::Contains(re) | Kind::Regex(re) => re.find_iter(line).map(|m| (m.start(), m.end())).filter(|m| m.0 < m.1).collect(),
            Kind::Glob(g) => if !line.is_empty() && g.is_match(line) { vec![(0, line.len())] } else { Vec::new() },
            Kind::Fuzzy(p) => {
                let Some((_, at)) = fuzzy(p, line) else { return Vec::new() };
                let (Some(&first), Some(&last)) = (at.first(), at.last()) else { return Vec::new() };
                let byte = |c: usize| line.char_indices().nth(c).map(|(b, _)| b).unwrap_or(line.len());
                let end = byte(last) + line[byte(last)..].chars().next().map(char::len_utf8).unwrap_or(0);
                vec![(byte(first), end)]
            }
        }
    }

    /// Byte ranges matched in a text of many lines: text and regex over it
    /// all (^ and $ at each line), glob and fuzzy line by line.
    pub fn find_all(&self, text: &str) -> Vec<(usize, usize)> {
        match &self.kind {
            Kind::Contains(re) | Kind::Regex(re) => re.find_iter(text).map(|m| (m.start(), m.end())).filter(|m| m.0 < m.1).collect(),
            _ => {
                let mut out = Vec::new();
                let mut at = 0;
                for line in text.split('\n') {
                    let body = line.strip_suffix('\r').unwrap_or(line);
                    out.extend(self.find_in(body).into_iter().map(|(a, b)| (at + a, at + b)));
                    at += line.len() + 1;
                }
                out
            }
        }
    }

    /// The regex of text and regex modes (what a replacement works on).
    pub fn regex(&self) -> Option<&Regex> {
        match (&self.kind, self.mode) {
            (Kind::Contains(re), MatchMode::Text) | (Kind::Regex(re), MatchMode::Regex) => Some(re),
            _ => None,
        }
    }

    /// What a host's grep is asked for: (pattern, as an extended regex,
    /// any case). For text and regex, the lines that match; for glob and
    /// fuzzy, lines that may (each piece of text a glob must have, a
    /// word's letters in order), matched again here (`exact_on_grep`).
    pub fn grep(&self) -> (String, bool, bool) {
        let icase = self.case.map(|c| !c);
        match (&self.kind, self.mode) {
            (_, MatchMode::Text) => (self.query.clone(), false, icase.unwrap_or(true)),
            (_, MatchMode::Regex) => (self.query.clone(), true, icase.unwrap_or(!self.query.chars().any(char::is_uppercase))),
            (Kind::Contains(_), _) => (ere_escape(self.query.trim()), true, icase.unwrap_or(true)),
            (Kind::Fuzzy(_), _) => {
                let word = fuzzy_words(&self.query).into_iter().max_by_key(|w| w.chars().count()).unwrap_or_default();
                (word.chars().map(fuzzy_class).collect::<Vec<_>>().join(".*"), true, true)
            }
            _ => (glob_pieces(self.query.trim()).iter().map(|p| ere_escape(p)).collect::<Vec<_>>().join(".*"), true, true),
        }
    }

    /// A host's grep only narrowed the lines down: each is matched here.
    pub fn exact_on_grep(&self) -> bool {
        matches!(self.kind, Kind::Glob(_) | Kind::Fuzzy(_))
    }

    pub fn target(&self) -> Target {
        self.target
    }
}

/// Escape for extended regex: what both Rust's regex and grep -E read as
/// the character itself (regex::escape also escapes - # & ~, which some
/// greps warn about).
fn ere_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ".[]()*+?{}|^$\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The runs of plain text a glob must have, in order (outside its
/// wildcards, classes and alternatives): what grep is asked for. Never
/// more than the glob asks: `**` may be no folder at all (the / beside it
/// not required), a class's first ] is in it, an escape in braces too.
fn glob_pieces(g: &str) -> Vec<String> {
    let cs: Vec<char> = g.chars().collect();
    let mut out = vec![String::new()];
    let mut depth = 0;
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '\\' => {
                if let (0, Some(&n)) = (depth, cs.get(i + 1)) {
                    out.last_mut().unwrap().push(n);
                }
                i += 2;
                continue;
            }
            '[' => {
                let mut j = i + 1;
                if matches!(cs.get(j), Some('!' | '^')) {
                    j += 1;
                }
                if cs.get(j) == Some(&']') {
                    j += 1;
                }
                while j < cs.len() && cs[j] != ']' {
                    // [:digit:] and its kind: their ] is not the class's (as
                    // globset reads one: up to 8 small letters between).
                    if cs[j] == '[' && cs.get(j + 1) == Some(&':') {
                        let name = cs[j + 2..].iter().take_while(|c| c.is_ascii_lowercase()).count();
                        if name <= 8 && cs.get(j + 2 + name) == Some(&':') && cs.get(j + 3 + name) == Some(&']') {
                            j += name + 4;
                            continue;
                        }
                    }
                    j += 1;
                }
                out.push(String::new());
                i = j + 1;
                continue;
            }
            '{' => {
                depth += 1;
                out.push(String::new());
            }
            '}' if depth > 0 => {
                depth -= 1;
                out.push(String::new());
            }
            '*' if depth == 0 && cs.get(i + 1) == Some(&'*') => {
                let p = out.last_mut().unwrap();
                if p.ends_with('/') {
                    p.pop();
                }
                i += 2;
                if cs.get(i) == Some(&'/') {
                    i += 1;
                }
                out.push(String::new());
                continue;
            }
            '*' | '?' if depth == 0 => out.push(String::new()),
            c if depth == 0 => out.last_mut().unwrap().push(c),
            _ => {}
        }
        i += 1;
    }
    out.retain(|p| !p.is_empty());
    out
}

/// A fuzzy query's words as nucleo reads them: apart at a space, `\ ` a
/// space in a word.
fn fuzzy_words(q: &str) -> Vec<String> {
    let mut words = vec![String::new()];
    let mut cs = q.chars().peekable();
    while let Some(c) = cs.next() {
        match c {
            '\\' if cs.peek() == Some(&' ') => words.last_mut().unwrap().push(cs.next().unwrap()),
            ' ' => words.push(String::new()),
            c => words.last_mut().unwrap().push(c),
        }
    }
    words.retain(|w| !w.is_empty());
    words
}

/// One letter of a fuzzy word for grep: every character nucleo takes for
/// it (any case, and what its own table makes that letter: é for e).
fn fuzzy_class(c: char) -> String {
    let lc = c.to_lowercase().next().unwrap_or(c);
    if !lc.is_ascii_alphabetic() {
        return ere_escape(&c.to_string());
    }
    let mut set = vec![lc, lc.to_ascii_uppercase()];
    set.extend((0x80u32..0x3000).filter_map(char::from_u32).filter(|&x| nucleo_matcher::chars::to_lower_case(nucleo_matcher::chars::normalize(x)) == lc));
    format!("[{}]", set.into_iter().collect::<String>())
}

/// fzf's match of the query's words in `s` (nucleo): its score and the
/// characters matched (their places, for highlighting).
fn fuzzy(pattern: &Pattern, s: &str) -> Option<(i64, Vec<usize>)> {
    if pattern.atoms.is_empty() {
        return Some((0, Vec::new()));
    }
    let mut buf = Vec::new();
    let hay = Utf32Str::new(s, &mut buf);
    let mut at = Vec::new();
    let score = NUCLEO.with(|m| pattern.indices(hay, &mut m.borrow_mut(), &mut at))?;
    at.sort_unstable();
    at.dedup();
    Some((score as i64, at.into_iter().map(|i| i as usize).collect()))
}

/// A list's own filter field (help, keys, memos, processes, fonts, the
/// terminal's find): the text mode, made once for the list.
pub struct TextFilter(Option<Matcher>);

impl TextFilter {
    /// Any case unless `case`; nothing typed lets everything through.
    pub fn new(query: &str) -> TextFilter {
        Self::with_case(query, false)
    }

    pub fn with_case(query: &str, case: bool) -> TextFilter {
        TextFilter(if query.is_empty() { None } else { Matcher::build(MatchMode::Text, query, Target::Line, Some(case)).ok() })
    }

    pub fn hit(&self, s: &str) -> bool {
        self.0.as_ref().is_none_or(|m| m.matches(s).is_some())
    }

    /// Byte ranges of each place in `s` (none when nothing is typed).
    pub fn find_in(&self, s: &str) -> Vec<(usize, usize)> {
        self.0.as_ref().map(|m| m.find_in(s)).unwrap_or_default()
    }
}

/// Matches over `paths`, best first (fuzzy) or in path order (the rest).
pub fn search(m: &Matcher, paths: &[(String, bool)], limit: usize) -> (Vec<(usize, Vec<usize>)>, usize) {
    let mut hits: Vec<(usize, i64, Vec<usize>)> = paths.iter().enumerate().filter_map(|(i, (p, _))| m.matches(p).map(|(s, pos)| (i, s, pos))).collect();
    let total = hits.len();
    if m.ranked() {
        hits.sort_by(|a, b| b.1.cmp(&a.1).then(paths[a.0].0.len().cmp(&paths[b.0].0.len())));
    }
    hits.truncate(limit);
    (hits.into_iter().map(|(i, _, pos)| (i, pos)).collect(), total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(mode: MatchMode, q: &str, paths: &[&str]) -> Vec<String> {
        let list: Vec<(String, bool)> = paths.iter().map(|p| (p.to_string(), false)).collect();
        let m = Matcher::new(mode, q).unwrap();
        search(&m, &list, 100).0.into_iter().map(|(i, _)| list[i].0.clone()).collect()
    }

    const TREE: &[&str] = &["src", "src/main.rs", "src/md.rs", "src/ui/main_view.rs", "remote/agent.py", "README.md", "docs/architecture.md", "target/x.rs"];

    #[test]
    fn fuzzy_ranks_names_and_runs() {
        let r = names(MatchMode::Fuzzy, "main", TREE);
        assert_eq!(r[0], "src/main.rs");
        assert!(r.contains(&"src/ui/main_view.rs".to_string()));
        assert_eq!(names(MatchMode::Fuzzy, "srcmain", TREE)[0], "src/main.rs");
        // Letters in order, not adjacent: m…rs. md.rs is shorter, so first.
        assert_eq!(names(MatchMode::Fuzzy, "mrs", TREE)[..2], ["src/md.rs", "src/main.rs"]);
        assert!(names(MatchMode::Fuzzy, "zzz", TREE).is_empty());
        assert_eq!(names(MatchMode::Fuzzy, "README", TREE)[0], "README.md", "case-insensitive");
        // Words each anywhere, as fzf.
        assert_eq!(names(MatchMode::Fuzzy, "view main", TREE)[0], "src/ui/main_view.rs");
    }

    /// A folder next to the root found by its path among thousands that
    /// have the same letters spread out (works/abc was not found).
    #[test]
    fn fuzzy_finds_a_path_among_many() {
        let mut list: Vec<String> = Vec::new();
        for i in 0..3000 {
            list.push(format!("projects/work{i}/src/abc_{i}.rs"));
            list.push(format!("w/o/r/k/s/a/b/c{i}"));
        }
        list.extend(["works".into(), "works/abc".into(), "works/abc/readme.md".into()]);
        let refs: Vec<&str> = list.iter().map(String::as_str).collect();
        assert_eq!(names(MatchMode::Fuzzy, "works/abc", &refs)[0], "works/abc");
    }

    #[test]
    fn glob_name_or_path() {
        assert_eq!(names(MatchMode::Glob, "*.md", TREE), ["README.md", "docs/architecture.md"]);
        assert_eq!(names(MatchMode::Glob, "src/*.rs", TREE), ["src/main.rs", "src/md.rs"]);
        assert_eq!(names(MatchMode::Glob, "src/**/*.rs", TREE), ["src/main.rs", "src/md.rs", "src/ui/main_view.rs"]);
        assert_eq!(names(MatchMode::Glob, "{agent,md}.*", TREE), ["src/md.rs", "remote/agent.py"]);
        assert_eq!(names(MatchMode::Glob, "m?.rs", TREE), ["src/md.rs"]);
        assert_eq!(names(MatchMode::Glob, "[!m]*.rs", TREE), ["target/x.rs"]);
    }

    /// Text with no wildcard is found anywhere in the name; a pattern is
    /// the whole name. (Plain text found only a name that was all of it.)
    #[test]
    fn glob_without_a_wildcard_is_contained() {
        assert_eq!(names(MatchMode::Glob, "main", TREE), ["src/main.rs", "src/ui/main_view.rs"]);
        assert_eq!(names(MatchMode::Glob, "MAIN", TREE), ["src/main.rs", "src/ui/main_view.rs"], "any case");
        assert_eq!(names(MatchMode::Glob, "src/m", TREE), ["src/main.rs", "src/md.rs"], "with a slash: in the path");
        assert_eq!(names(MatchMode::Glob, "main*", TREE), ["src/main.rs", "src/ui/main_view.rs"], "a pattern: from the start of the name");
        assert!(names(MatchMode::Glob, "ain*", TREE).is_empty());
        // Commands, as the History panel matches them (the whole line).
        let cmds = ["ssh user@host", "git status", "vim a.txt"];
        let m = Matcher::for_lines(MatchMode::Glob, "ssh").unwrap();
        assert!(m.matches(cmds[0]).is_some() && m.matches(cmds[1]).is_none());
        // A pattern over a whole command: * crosses / there.
        let m = Matcher::for_lines(MatchMode::Glob, "vim *").unwrap();
        assert!(m.matches("vim src/a.txt").is_some() && m.matches("git vim").is_none());
        assert!(Matcher::new(MatchMode::Glob, "a.txt").unwrap().matches(cmds[2]).is_some(), "a dot is a dot");
    }

    #[test]
    fn regex_smart_case_on_path() {
        assert_eq!(names(MatchMode::Regex, r"^src/m.*\.rs$", TREE), ["src/main.rs", "src/md.rs"]);
        assert_eq!(names(MatchMode::Regex, "readme", TREE), ["README.md"]);
        assert!(names(MatchMode::Regex, "Readme", TREE).is_empty(), "uppercase: case-sensitive");
        assert!(Matcher::new(MatchMode::Regex, "(").is_err());
    }

    /// The same matcher in lines of text: text and regex each place, a
    /// glob the whole line, fuzzy its letters' span; and what a host's grep
    /// is asked for (glob and fuzzy: lines that may match, matched again).
    #[test]
    fn in_lines_and_for_grep() {
        let line = "void Start(int x); // start here";
        let marks = |mode, q: &str, case| Matcher::build(mode, q, Target::Line, Some(case)).map(|m| m.find_in(line).into_iter().map(|(a, b)| &line[a..b]).collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!(marks(MatchMode::Text, "start", false), ["Start", "start"]);
        assert_eq!(marks(MatchMode::Text, "start", true), ["start"]);
        assert_eq!(marks(MatchMode::Text, "Start(", true), ["Start("]);
        assert_eq!(marks(MatchMode::Fuzzy, "sti", false), ["Start(i"]);
        assert_eq!(marks(MatchMode::Glob, "*Start(*", false), [line], "a glob: the whole line");
        assert!(marks(MatchMode::Glob, "S*(", false).is_empty(), "not a part of it");
        assert_eq!(marks(MatchMode::Glob, "start", false), ["Start", "start"], "no wildcard: anywhere");
        assert_eq!(marks(MatchMode::Regex, r"\w+\(", true), ["Start("]);
        assert!(Matcher::build(MatchMode::Regex, "(", Target::Line, None).is_err());
        let t = "git status\r\ngit stash\nvoid Start(int x);";
        let all = |mode, q: &str| { let m = Matcher::build(mode, q, Target::Line, Some(false)).unwrap(); m.find_all(t).into_iter().map(|(a, b)| t[a..b].to_string()).collect::<Vec<_>>() };
        assert_eq!(all(MatchMode::Fuzzy, "gst"), ["git st", "git st"]);
        assert_eq!(all(MatchMode::Glob, "git st?sh"), ["git stash"]);
        assert_eq!(all(MatchMode::Glob, "git sta*"), ["git status", "git stash"], "a CR is not the line's");
        assert!(all(MatchMode::Fuzzy, "gitvoid").is_empty(), "never across lines");
        // For grep -E: no Rust-only syntax.
        let grep = |mode, q: &str| Matcher::build(mode, q, Target::Line, None).unwrap().grep();
        // Fuzzy: each letter as nucleo takes it, a dot a dot.
        let p = grep(MatchMode::Fuzzy, "a.b").0;
        let re = regex::RegexBuilder::new(&p).case_insensitive(true).build().unwrap();
        assert!(re.is_match("a x.y b") && re.is_match("À.B") && !re.is_match("axb"), "{p}");
        assert_eq!(grep(MatchMode::Glob, "*.rs").0, "\\.rs");
        assert_eq!(grep(MatchMode::Glob, "{src,lib}/*_[ab]x.rs").0, "/.*_.*x\\.rs", "alternatives and classes left out");
        assert_eq!(grep(MatchMode::Glob, "f(x)+1").0, "f\\(x\\)\\+1");
        assert_eq!(grep(MatchMode::Text, "f(x)+1"), ("f(x)+1".into(), false, true));
        // Never more than the glob asks (grep would lose lines it takes).
        let pre = |q: &str| regex::RegexBuilder::new(&grep(MatchMode::Glob, q).0).case_insensitive(true).build().unwrap();
        for (q, line) in [("src/**/x.rs", "src/x.rs"), ("**/x.rs", "x.rs"), ("[]a]x", "]x"), ("x{a\\}b,c}y", "xcy"), ("*\\*z", "a*z"), ("*[[:digit:]]x*", "5x"), ("[[:]{:]]x}", "[:]]x")] {
            let m = Matcher::build(MatchMode::Glob, q, Target::Line, None).unwrap();
            assert!(m.matches(line).is_some() && pre(q).is_match(line), "{q} on {line}: grep {}", grep(MatchMode::Glob, q).0);
        }
        // Fuzzy as nucleo reads it: é for e, "\ " a space in a word.
        let fz = |q: &str| regex::RegexBuilder::new(&grep(MatchMode::Fuzzy, q).0).case_insensitive(true).build().unwrap();
        assert!(Matcher::build(MatchMode::Fuzzy, "cafe", Target::Line, None).unwrap().matches("Café au lait").is_some());
        assert!(fz("cafe").is_match("Café au lait"), "{}", grep(MatchMode::Fuzzy, "cafe").0);
        assert!(grep(MatchMode::Fuzzy, "k").0.contains('\u{212A}'), "the Kelvin sign is a k to nucleo");
        assert!(fz("foo\\ bar").is_match("foo bar") && !grep(MatchMode::Fuzzy, "foo\\ bar").0.contains('\\'));
        // A blank glob is no pattern (it was every line).
        assert!(Matcher::build(MatchMode::Glob, "   ", Target::Line, None).is_err());
        assert!(Matcher::build(MatchMode::Glob, "*.rs", Target::Line, None).unwrap().exact_on_grep());
        assert!(!Matcher::build(MatchMode::Glob, "main", Target::Line, None).unwrap().exact_on_grep());
    }

    #[test]
    fn positions_are_chars_for_highlighting() {
        let m = Matcher::new(MatchMode::Glob, "*.md").unwrap();
        let (_, pos) = m.matches("docs/한글.md").unwrap();
        assert_eq!(pos, (5..10).collect::<Vec<_>>());
        let m = Matcher::new(MatchMode::Fuzzy, "글m").unwrap();
        assert_eq!(m.matches("docs/한글.md").unwrap().1, vec![6, 8]);
    }
}
