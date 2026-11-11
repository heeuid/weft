//! File panel filter over everything under the root, not just the expanded
//! folders. The remote side only sends the list of names once (agent `tree`);
//! matching and ranking happen here, on every keystroke.
//!
//! * fuzzy (default): the query's characters in order anywhere in the
//!   relative path, ranked like editor "go to file" (consecutive runs, word
//!   starts and the file name score higher);
//! * glob: `*.rs`, `src/**/test_*`, `{a,b}.md`, `[ab]?` — without a `/` it
//!   matches the name only, with one the whole relative path;
//! * regex: against the relative path, smart case (case-insensitive unless the
//!   query has an uppercase letter).

use serde::{Deserialize, Serialize};

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
            FindMode::Glob => "glob: *.rs, src/**/test_*, {a,b}.md",
            FindMode::Regex => "regex on the path, e.g. ^src/.*\\.rs$",
        }
    }
}

pub enum Matcher {
    Fuzzy(Vec<char>),
    Pattern { re: regex::Regex, full_path: bool },
}

impl Matcher {
    pub fn new(mode: FindMode, query: &str) -> Result<Matcher, String> {
        match mode {
            FindMode::Fuzzy => Ok(Matcher::Fuzzy(query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect())),
            FindMode::Glob => {
                let q = query.trim();
                let re = regex::RegexBuilder::new(&format!("^{}$", glob_fragment(&q.chars().collect::<Vec<_>>()))).case_insensitive(true).build().map_err(|e| e.to_string())?;
                Ok(Matcher::Pattern { re, full_path: q.contains('/') })
            }
            FindMode::Regex => {
                let smart = !query.chars().any(char::is_uppercase);
                let re = regex::RegexBuilder::new(query).case_insensitive(smart).build().map_err(|e| e.to_string())?;
                Ok(Matcher::Pattern { re, full_path: true })
            }
        }
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
        }
    }

    pub fn ranked(&self) -> bool {
        matches!(self, Matcher::Fuzzy(_))
    }
}

/// Glob syntax as an (unanchored) regex.
fn glob_fragment(cs: &[char]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '*' if cs.get(i + 1) == Some(&'*') => {
                if cs.get(i + 2) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    out.push_str(".*");
                    i += 2;
                }
                continue;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '[' => {
                if let Some(end) = cs[i + 1..].iter().position(|&c| c == ']') {
                    let inner: String = cs[i + 1..i + 1 + end].iter().collect();
                    let inner = match inner.strip_prefix('!') {
                        Some(rest) => format!("^{rest}"),
                        None => inner,
                    };
                    out.push('[');
                    out.push_str(&inner.replace('\\', "\\\\"));
                    out.push(']');
                    i += end + 2;
                    continue;
                }
                out.push_str("\\[");
            }
            '{' => {
                if let Some(end) = cs[i + 1..].iter().position(|&c| c == '}') {
                    let body = &cs[i + 1..i + 1 + end];
                    let alts: Vec<String> = body.split(|&c| c == ',').map(glob_fragment).collect();
                    out.push_str(&format!("(?:{})", alts.join("|")));
                    i += end + 2;
                    continue;
                }
                out.push_str("\\{");
            }
            c => out.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    out
}

/// Query characters in order; tries a few starting points and keeps the best.
fn fuzzy(q: &[char], path: &str) -> Option<(i64, Vec<usize>)> {
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let p: Vec<char> = path.chars().collect();
    let lower: Vec<char> = p.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let name_start = p.iter().rposition(|&c| c == '/').map(|i| i + 1).unwrap_or(0);
    let starts: Vec<usize> = (0..p.len()).filter(|&i| lower[i] == q[0]).collect();
    let mut best: Option<(i64, Vec<usize>)> = None;
    // Prefer starts in the file name, then the rest; a handful is plenty.
    let ordered = starts.iter().filter(|&&s| s >= name_start).chain(starts.iter().filter(|&&s| s < name_start)).take(12);
    for &s in ordered {
        let mut pos = vec![s];
        let mut at = s + 1;
        for &c in &q[1..] {
            match (at..p.len()).find(|&i| lower[i] == c) {
                Some(i) => {
                    pos.push(i);
                    at = i + 1;
                }
                None => break,
            }
        }
        if pos.len() < q.len() {
            continue;
        }
        let score = score(&p, &pos, name_start);
        if best.as_ref().map(|b| score > b.0).unwrap_or(true) {
            best = Some((score, pos));
        }
    }
    best
}

fn score(p: &[char], pos: &[usize], name_start: usize) -> i64 {
    let mut s = 0i64;
    for (k, &i) in pos.iter().enumerate() {
        s += 16;
        if k > 0 && pos[k - 1] + 1 == i {
            s += 24;
        }
        let word_start = i == 0 || matches!(p[i - 1], '/' | '_' | '-' | '.' | ' ') || (p[i - 1].is_lowercase() && p[i].is_uppercase());
        if word_start {
            s += 20;
        }
        if i >= name_start {
            s += 12;
        }
    }
    // Shorter paths and earlier matches win ties.
    s - p.len() as i64 / 4 - pos[0] as i64 / 8
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
