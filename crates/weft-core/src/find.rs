//! File panel filter over everything under the root, not just the expanded
//! folders. The remote side only sends the list of names once (agent `tree`);
//! matching and ranking happen here, on every keystroke.
//!
//! * fuzzy (default): fzf's matching and scoring (the nucleo matcher, as
//!   the Helix editor uses it): each word of the query, its characters in
//!   order anywhere in the path, any case; runs, word and path-part starts
//!   score higher;
//! * glob: ripgrep's (globset): `*.rs`, `src/**/test_*`, `{a,b}.md`, `[ab]?`
//!   — without a `/` it matches the name only, with one the whole relative
//!   path; text with no wildcard is found anywhere in it;
//! * regex: against the relative path, smart case (case-insensitive unless the
//!   query has an uppercase letter).

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::Utf32Str;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

thread_local! {
    /// nucleo's matcher keeps its working memory between calls: one per
    /// thread, made once.
    static NUCLEO: RefCell<nucleo_matcher::Matcher> = RefCell::new(nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT));
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FindMode {
    #[default]
    Fuzzy,
    Glob,
    Regex,
}

impl FindMode {
    pub const ALL: [FindMode; 3] = [FindMode::Fuzzy, FindMode::Glob, FindMode::Regex];

    pub fn label(self) -> &'static str {
        match self {
            FindMode::Fuzzy => "fuzzy",
            FindMode::Glob => "glob",
            FindMode::Regex => "regex",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            FindMode::Fuzzy => "fuzzy: letters in order, e.g. srcmain",
            FindMode::Glob => "glob: *.rs, src/**/test_*, {a,b}.md; no wildcard: contains",
            FindMode::Regex => "regex on the path, e.g. ^src/.*\\.rs$",
        }
    }
}

pub enum Matcher {
    Fuzzy(Pattern),
    /// Regex, and glob text with no wildcard (found anywhere).
    Pattern { re: regex::Regex, full_path: bool },
    /// A glob: the whole name, or with a / in it the whole path.
    Glob { glob: globset::GlobMatcher, full_path: bool },
}

impl Matcher {
    pub fn new(mode: FindMode, query: &str) -> Result<Matcher, String> {
        match mode {
            FindMode::Fuzzy => Ok(Matcher::Fuzzy(Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy))),
            FindMode::Glob => Self::glob(query, true),
            FindMode::Regex => {
                let smart = !query.chars().any(char::is_uppercase);
                let re = regex::RegexBuilder::new(query).case_insensitive(smart).build().map_err(|e| e.to_string())?;
                Ok(Matcher::Pattern { re, full_path: true })
            }
        }
    }

    /// For whole lines (History's commands): a glob's * crosses /, and the
    /// line is matched whole.
    pub fn for_lines(mode: FindMode, query: &str) -> Result<Matcher, String> {
        match mode {
            FindMode::Glob => Self::glob(query, false).map(|m| match m {
                Matcher::Pattern { re, .. } => Matcher::Pattern { re, full_path: true },
                Matcher::Glob { glob, .. } => Matcher::Glob { glob, full_path: true },
                m => m,
            }),
            _ => Self::new(mode, query),
        }
    }

    /// A pattern is the whole name (or path): *.rs ends in .rs. Text with
    /// no wildcard is found anywhere in it: "ssh" finds "ssh user@host".
    fn glob(query: &str, paths: bool) -> Result<Matcher, String> {
        let q = query.trim();
        let full_path = q.contains('/');
        if !q.contains(['*', '?', '[', '{']) {
            let re = regex::RegexBuilder::new(&regex::escape(q)).case_insensitive(true).build().map_err(|e| e.to_string())?;
            return Ok(Matcher::Pattern { re, full_path });
        }
        let glob = globset::GlobBuilder::new(q).case_insensitive(true).literal_separator(paths).backslash_escape(true).empty_alternates(true).build().map_err(|e| e.to_string())?.compile_matcher();
        Ok(Matcher::Glob { glob, full_path })
    }

    /// Score and the matched char positions in `path`, or None.
    pub fn matches(&self, path: &str) -> Option<(i64, Vec<usize>)> {
        match self {
            Matcher::Fuzzy(q) => fuzzy(q, path),
            Matcher::Pattern { re, full_path } => {
                let name_at = path.rfind('/').map(|i| i + 1).unwrap_or(0);
                let (target, base) = if *full_path { (path, 0) } else { (&path[name_at..], name_at) };
                let m = re.find(target)?;
                let skip = path[..base + m.start()].chars().count();
                let n = target[m.start()..m.end()].chars().count();
                Some((0, (skip..skip + n).collect()))
            }
            Matcher::Glob { glob, full_path } => {
                // The whole name (or path) matched: all of it marked.
                let name_at = path.rfind('/').map(|i| i + 1).unwrap_or(0);
                let (target, base) = if *full_path { (path, 0) } else { (&path[name_at..], name_at) };
                if !glob.is_match(target) {
                    return None;
                }
                let skip = path[..base].chars().count();
                Some((0, (skip..skip + target.chars().count()).collect()))
            }
        }
    }

    pub fn ranked(&self) -> bool {
        matches!(self, Matcher::Fuzzy(_))
    }
}

/// fzf's match of the query's words in `path` (nucleo): its score and the
/// characters matched (their places, for highlighting).
fn fuzzy(pattern: &Pattern, path: &str) -> Option<(i64, Vec<usize>)> {
    if pattern.atoms.is_empty() {
        return Some((0, Vec::new()));
    }
    let mut buf = Vec::new();
    let hay = Utf32Str::new(path, &mut buf);
    let mut at = Vec::new();
    let score = NUCLEO.with(|m| pattern.indices(hay, &mut m.borrow_mut(), &mut at))?;
    at.sort_unstable();
    at.dedup();
    Some((score as i64, at.into_iter().map(|i| i as usize).collect()))
}

/// Matches over `paths`, best first (fuzzy) or in path order (glob, regex).
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

    fn names(mode: FindMode, q: &str, paths: &[&str]) -> Vec<String> {
        let list: Vec<(String, bool)> = paths.iter().map(|p| (p.to_string(), false)).collect();
        let m = Matcher::new(mode, q).unwrap();
        search(&m, &list, 100).0.into_iter().map(|(i, _)| list[i].0.clone()).collect()
    }

    const TREE: &[&str] = &["src", "src/main.rs", "src/md.rs", "src/ui/main_view.rs", "remote/agent.py", "README.md", "docs/architecture.md", "target/x.rs"];

    #[test]
    fn fuzzy_ranks_names_and_runs() {
        let r = names(FindMode::Fuzzy, "main", TREE);
        assert_eq!(r[0], "src/main.rs");
        assert!(r.contains(&"src/ui/main_view.rs".to_string()));
        assert_eq!(names(FindMode::Fuzzy, "srcmain", TREE)[0], "src/main.rs");
        // Letters in order, not adjacent: m…rs. md.rs is shorter, so first.
        assert_eq!(names(FindMode::Fuzzy, "mrs", TREE)[..2], ["src/md.rs", "src/main.rs"]);
        assert!(names(FindMode::Fuzzy, "zzz", TREE).is_empty());
        assert_eq!(names(FindMode::Fuzzy, "README", TREE)[0], "README.md", "case-insensitive");
        // Words each anywhere, as fzf.
        assert_eq!(names(FindMode::Fuzzy, "view main", TREE)[0], "src/ui/main_view.rs");
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
        assert_eq!(names(FindMode::Fuzzy, "works/abc", &refs)[0], "works/abc");
    }

    #[test]
    fn glob_name_or_path() {
        assert_eq!(names(FindMode::Glob, "*.md", TREE), ["README.md", "docs/architecture.md"]);
        assert_eq!(names(FindMode::Glob, "src/*.rs", TREE), ["src/main.rs", "src/md.rs"]);
        assert_eq!(names(FindMode::Glob, "src/**/*.rs", TREE), ["src/main.rs", "src/md.rs", "src/ui/main_view.rs"]);
        assert_eq!(names(FindMode::Glob, "{agent,md}.*", TREE), ["src/md.rs", "remote/agent.py"]);
        assert_eq!(names(FindMode::Glob, "m?.rs", TREE), ["src/md.rs"]);
        assert_eq!(names(FindMode::Glob, "[!m]*.rs", TREE), ["target/x.rs"]);
    }

    /// Text with no wildcard is found anywhere in the name; a pattern is
    /// the whole name. (Plain text found only a name that was all of it.)
    #[test]
    fn glob_without_a_wildcard_is_contained() {
        assert_eq!(names(FindMode::Glob, "main", TREE), ["src/main.rs", "src/ui/main_view.rs"]);
        assert_eq!(names(FindMode::Glob, "MAIN", TREE), ["src/main.rs", "src/ui/main_view.rs"], "any case");
        assert_eq!(names(FindMode::Glob, "src/m", TREE), ["src/main.rs", "src/md.rs"], "with a slash: in the path");
        assert_eq!(names(FindMode::Glob, "main*", TREE), ["src/main.rs", "src/ui/main_view.rs"], "a pattern: from the start of the name");
        assert!(names(FindMode::Glob, "ain*", TREE).is_empty());
        // Commands, as the History panel matches them (the whole line).
        let cmds = ["ssh user@host", "git status", "vim a.txt"];
        let m = Matcher::for_lines(FindMode::Glob, "ssh").unwrap();
        assert!(m.matches(cmds[0]).is_some() && m.matches(cmds[1]).is_none());
        // A pattern over a whole command: * crosses / there.
        let m = Matcher::for_lines(FindMode::Glob, "vim *").unwrap();
        assert!(m.matches("vim src/a.txt").is_some() && m.matches("git vim").is_none());
        assert!(Matcher::new(FindMode::Glob, "a.txt").unwrap().matches(cmds[2]).is_some(), "a dot is a dot");
    }

    #[test]
    fn regex_smart_case_on_path() {
        assert_eq!(names(FindMode::Regex, r"^src/m.*\.rs$", TREE), ["src/main.rs", "src/md.rs"]);
        assert_eq!(names(FindMode::Regex, "readme", TREE), ["README.md"]);
        assert!(names(FindMode::Regex, "Readme", TREE).is_empty(), "uppercase: case-sensitive");
        assert!(Matcher::new(FindMode::Regex, "(").is_err());
    }

    #[test]
    fn positions_are_chars_for_highlighting() {
        let m = Matcher::new(FindMode::Glob, "*.md").unwrap();
        let (_, pos) = m.matches("docs/한글.md").unwrap();
        assert_eq!(pos, (5..10).collect::<Vec<_>>());
        let m = Matcher::new(FindMode::Fuzzy, "글m").unwrap();
        assert_eq!(m.matches("docs/한글.md").unwrap().1, vec![6, 8]);
    }
}
