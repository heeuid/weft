//! A git work tree as weft shows it: its branch and how far from its
//! upstream, and each changed file's state, read from git's own
//! machine-readable output (`status --porcelain=v2 -z`: the same in any
//! language and version since git 2.11). git itself runs on the host the
//! files are on (the agent's or this PC's "git" op); the reading is here,
//! one for every host.

/// The arguments for a work tree's state. (Untracked files as git is set
/// to show them: by default an untracked folder is one entry, "dir/"; a
/// home folder kept as a repository is often set to show none, and is
/// not scanned every few seconds then.)
pub const STATUS_ARGS: &[&str] = &["status", "--porcelain=v2", "--branch", "-z", "--ignored=no"];

/// The arguments that print the work tree's top folder, where in it the
/// folder asked about is (a prefix: its paths are the work tree's paths
/// however the folder was reached, a link or a junction), and its git
/// folder (where a merge or a rebase in progress shows).
pub const ROOT_ARGS: &[&str] = &["rev-parse", "--show-toplevel", "--show-prefix", "--absolute-git-dir"];

/// `ROOT_ARGS`' output: the top, and the folder's place in it ("" or
/// "src/ui/").
pub fn parse_root(out: &str) -> Option<(String, String)> {
    let mut l = out.lines();
    let top = l.next()?.trim().replace('\\', "/");
    let prefix = l.next().unwrap_or("").trim().to_string();
    (!top.is_empty()).then_some((top, prefix))
}

/// `ROOT_ARGS`' third line: the git folder.
pub fn git_dir_of(out: &str) -> Option<String> {
    out.lines().nth(2).map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty())
}

/// What is in progress in a work tree, from the names in its git folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Underway {
    Merge,
    Rebase,
    /// git am (its folder is rebase-apply too, with "applying" in it).
    Am,
    CherryPick,
    Revert,
    /// A cherry-pick or a revert of several commits, between two of them
    /// (its next step: in a terminal, which of the two git knows).
    Sequence,
}

impl Underway {
    /// From the names in the git folder (and in rebase-apply, when there
    /// is one: `apply`).
    pub fn of(names: &[&str], apply: &[&str]) -> Option<Underway> {
        let has = |n: &str| names.contains(&n);
        if has("rebase-apply") && apply.contains(&"applying") {
            Some(Underway::Am)
        } else if has("rebase-merge") || has("rebase-apply") {
            Some(Underway::Rebase)
        } else if has("MERGE_HEAD") {
            Some(Underway::Merge)
        } else if has("CHERRY_PICK_HEAD") {
            Some(Underway::CherryPick)
        } else if has("REVERT_HEAD") {
            Some(Underway::Revert)
        } else if has("sequencer") {
            Some(Underway::Sequence)
        } else {
            None
        }
    }

    /// git's word for it, as in `git <word> --abort`.
    pub fn word(self) -> &'static str {
        match self {
            Underway::Merge => "merge",
            Underway::Rebase => "rebase",
            Underway::Am => "am",
            Underway::Sequence => "cherry-pick or revert",
            Underway::CherryPick => "cherry-pick",
            Underway::Revert => "revert",
        }
    }
}

/// git's answer means the folder is in no work tree (and not that git could
/// not be asked: missing, an ownership it does not trust).
pub fn not_a_repo(said: &str) -> bool {
    // (Inside .git, or a bare repository: no work tree either.)
    said.contains("not a git repository") || said.contains("must be run in a work tree")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The branch checked out; None: none (a detached HEAD, or a repository
    /// with no commit yet: `head` says which).
    pub branch: Option<String>,
    /// What HEAD is when there is no branch: "(detached)" or the commit.
    pub head: String,
    /// The commit checked out ("(initial)": none yet).
    pub oid: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileStatus>,
    /// Each changed path's letter, the folders with a change under them,
    /// and the untracked folders: a row's mark found at once (a list of
    /// twenty thousand walked for each row held the window).
    pub marks: std::collections::HashMap<String, char>,
    pub changed_dirs: std::collections::HashSet<String>,
    pub untracked_dirs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileStatus {
    /// Relative to the work tree's top, with "/".
    pub path: String,
    /// What a rename or copy came from.
    pub from: Option<String>,
    /// The staged change ('.' none; M A D R C T).
    pub index: char,
    /// The change in the work tree, not staged ('.' none; M D T).
    pub work: char,
    pub untracked: bool,
    /// Both sides changed it in a merge, a rebase or a cherry-pick.
    pub conflict: bool,
    /// A submodule (another repository, at a commit of its own).
    pub submodule: bool,
}

impl FileStatus {
    pub fn staged(&self) -> bool {
        !self.untracked && !self.conflict && self.index != '.'
    }

    pub fn unstaged(&self) -> bool {
        !self.untracked && !self.conflict && self.work != '.'
    }

    /// Added with `git add -N` (intent to add): in the index with no
    /// content yet.
    pub fn intent_to_add(&self) -> bool {
        self.index == '.' && self.work == 'A'
    }

    /// One letter for it: U conflict, ? untracked, else the work tree's
    /// change, else the staged one.
    pub fn mark(&self) -> char {
        if self.conflict {
            'U'
        } else if self.untracked {
            '?'
        } else if self.work != '.' {
            self.work
        } else {
            self.index
        }
    }
}

impl Status {
    /// How many files have a change of any kind.
    pub fn changed(&self) -> usize {
        self.files.len()
    }

    /// The letter for a path (relative to the top): its own (in an
    /// untracked folder: '?'), or for a folder '•' when something under it
    /// changed.
    pub fn mark_of(&self, rel: &str, dir: bool) -> Option<char> {
        let bare = rel.trim_end_matches('/');
        if let Some(m) = self.marks.get(bare) {
            return Some(*m);
        }
        if self.untracked_dirs.iter().any(|d| rel.starts_with(d.as_str())) {
            return Some('?');
        }
        if dir && (rel.is_empty() && !self.files.is_empty() || self.changed_dirs.contains(bare)) {
            return Some('•');
        }
        None
    }

    /// The index of marks, made once the files are read.
    fn index(&mut self) {
        for f in &self.files {
            let p = f.path.trim_end_matches('/');
            self.marks.insert(p.to_string(), f.mark());
            if f.untracked && f.path.ends_with('/') {
                self.untracked_dirs.push(f.path.clone());
            }
            let mut d = p;
            while let Some((up, _)) = d.rsplit_once('/') {
                if !self.changed_dirs.insert(up.to_string()) {
                    break;
                }
                d = up;
            }
        }
    }
}

/// `git status --porcelain=v2 --branch -z`'s output, read.
pub fn parse_status(out: &str) -> Status {
    let mut st = Status::default();
    let mut parts = out.split('\0').peekable();
    while let Some(entry) = parts.next() {
        if entry.is_empty() {
            continue;
        }
        if let Some(h) = entry.strip_prefix("# ") {
            let (key, value) = h.split_once(' ').unwrap_or((h, ""));
            match key {
                "branch.oid" => st.oid = value.to_string(),
                "branch.head" => {
                    st.head = value.to_string();
                    st.branch = (value != "(detached)").then(|| value.to_string());
                }
                "branch.upstream" => st.upstream = Some(value.to_string()),
                "branch.ab" => {
                    for n in value.split_whitespace() {
                        if let Some(a) = n.strip_prefix('+') {
                            st.ahead = a.parse().unwrap_or(0);
                        } else if let Some(b) = n.strip_prefix('-') {
                            st.behind = b.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = entry.chars().next().unwrap_or(' ');
        let fields = |n: usize| -> Vec<&str> { entry.splitn(n, ' ').collect() };
        let xy = |s: &str| -> (char, char) {
            let mut c = s.chars();
            (c.next().unwrap_or('.'), c.next().unwrap_or('.'))
        };
        match kind {
            // 1 XY sub mH mI mW hH hI path
            '1' => {
                let f = fields(9);
                if let (Some(x), Some(path)) = (f.get(1), f.get(8)) {
                    let (index, work) = xy(x);
                    let submodule = f.get(2).is_some_and(|s| s.starts_with('S'));
                    st.files.push(FileStatus { path: path.to_string(), from: None, index, work, untracked: false, conflict: false, submodule });
                }
            }
            // 2 XY sub mH mI mW hH hI Xscore path, then the old path
            '2' => {
                let f = fields(10);
                if let (Some(x), Some(path)) = (f.get(1), f.get(9)) {
                    let (index, work) = xy(x);
                    let from = parts.next().map(String::from);
                    let submodule = f.get(2).is_some_and(|s| s.starts_with('S'));
                    st.files.push(FileStatus { path: path.to_string(), from, index, work, untracked: false, conflict: false, submodule });
                }
            }
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            'u' => {
                let f = fields(11);
                if let (Some(x), Some(path)) = (f.get(1), f.get(10)) {
                    let (index, work) = xy(x);
                    st.files.push(FileStatus { path: path.to_string(), from: None, index, work, untracked: false, conflict: true, submodule: false });
                }
            }
            '?' => {
                let path = entry[2.min(entry.len())..].to_string();
                st.files.push(FileStatus { path, from: None, index: '?', work: '?', untracked: true, conflict: false, submodule: false });
            }
            _ => {}
        }
    }
    st.files.sort_by(|a, b| a.path.cmp(&b.path));
    st.index();
    st
}

/// What `git diff` printed, as lines to colour: a header ("diff --git",
/// index, ---, +++), a hunk's "@@", a line added, removed or as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLine {
    Header,
    Hunk,
    Added,
    Removed,
    Same,
}

/// Each line of a diff's kind: in a hunk a line is what its first
/// character says (a removed "-- x" is no header), outside one a header.
/// (A conflict's combined diff, "@@@": a line's kind is in its first two
/// columns, one for each side.)
pub fn diff_lines(text: &str) -> Vec<DiffLine> {
    let mut in_hunk = false;
    let mut columns = 1;
    text.lines()
        .map(|l| {
            if l.starts_with("@@") {
                in_hunk = true;
                columns = if l.starts_with("@@@") { 2 } else { 1 };
                return DiffLine::Hunk;
            }
            if l.starts_with("diff ") {
                in_hunk = false;
            }
            if !in_hunk {
                return DiffLine::Header;
            }
            let head: String = l.chars().take(columns).collect();
            if head.contains('+') {
                DiffLine::Added
            } else if head.contains('-') {
                DiffLine::Removed
            } else {
                DiffLine::Same
            }
        })
        .collect()
}

pub fn diff_line(l: &str) -> DiffLine {
    if l.starts_with("diff --git") || l.starts_with("index ") || l.starts_with("--- ") || l.starts_with("+++ ") || l.starts_with("new file") || l.starts_with("deleted file") || l.starts_with("similarity") || l.starts_with("rename ") || l.starts_with("old mode") || l.starts_with("new mode") || l.starts_with("Binary files") {
        DiffLine::Header
    } else if l.starts_with("@@") {
        DiffLine::Hunk
    } else if l.starts_with('+') {
        DiffLine::Added
    } else if l.starts_with('-') {
        DiffLine::Removed
    } else {
        DiffLine::Same
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// git's porcelain v2: the branch, how far from its upstream, a file
    /// changed and staged, one only changed, a rename, a conflict, an
    /// untracked one with a space in its name (NUL-separated, never
    /// quoted).
    #[test]
    fn a_work_trees_state_is_read() {
        let out = [
            "# branch.oid 1234567890abcdef1234567890abcdef12345678",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 M. N... 100644 100644 100644 aaaa bbbb src/a.rs",
            "1 .M N... 100644 100644 100644 aaaa bbbb README.md",
            "2 R. N... 100644 100644 100644 aaaa bbbb R100 src/new name.rs",
            "src/old.rs",
            "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc src/both.rs",
            "? notes/todo list.txt",
            "",
        ]
        .join("\0");
        let st = parse_status(&out);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!((st.upstream.as_deref(), st.ahead, st.behind), (Some("origin/main"), 2, 1));
        assert_eq!(st.changed(), 5);
        let f = |p: &str| st.files.iter().find(|f| f.path == p).unwrap().clone();
        assert!(f("src/a.rs").staged() && !f("src/a.rs").unstaged() && f("src/a.rs").mark() == 'M');
        assert!(!f("README.md").staged() && f("README.md").unstaged());
        assert_eq!((f("src/new name.rs").from.as_deref(), f("src/new name.rs").mark()), (Some("src/old.rs"), 'R'));
        assert!(f("src/both.rs").conflict && f("src/both.rs").mark() == 'U' && !f("src/both.rs").staged());
        assert!(f("notes/todo list.txt").untracked && f("notes/todo list.txt").mark() == '?');
        assert_eq!(st.mark_of("src", true), Some('•'), "a folder with changes");
        assert_eq!(st.mark_of("docs", true), None);
        assert_eq!(st.mark_of("src/a.rs", false), Some('M'));
        // A detached HEAD; a repository with no commit yet.
        let d = parse_status("# branch.oid abc\0# branch.head (detached)\0");
        assert!(d.branch.is_none() && d.head == "(detached)");
        let fresh = parse_status("# branch.oid (initial)\0# branch.head main\0");
        assert_eq!(fresh.branch.as_deref(), Some("main"));
        assert_eq!(fresh.oid, "(initial)");
        // A submodule; one added with -N.
        let s = parse_status("1 .M SC.. 160000 160000 160000 aaaa bbbb lib\x001 .A N... 000000 000000 100644 0000 0000 new.rs\0");
        assert!(s.files[0].submodule && !s.files[1].submodule);
        assert!(s.files[1].intent_to_add());
        // Marks of folders deep down, at once.
        let deep = parse_status("1 .M N... 100644 100644 100644 a b a/b/c/d.rs\0");
        assert_eq!((deep.mark_of("a", true), deep.mark_of("a/b/c", true), deep.mark_of("a/x", true)), (Some('•'), Some('•'), None));
    }

    /// A conflict's combined diff: a line added or removed on either side.
    #[test]
    fn a_combined_diff() {
        let kinds = diff_lines("diff --cc x\n@@@ -1,1 -1,1 +1,3 @@@\n  same\n +ours\n+ theirs\n- gone\n");
        assert_eq!(kinds, [DiffLine::Header, DiffLine::Hunk, DiffLine::Same, DiffLine::Added, DiffLine::Added, DiffLine::Removed]);
        assert_eq!(Underway::of(&["HEAD", "MERGE_HEAD"], &[]), Some(Underway::Merge));
        assert_eq!(Underway::of(&["rebase-merge", "MERGE_HEAD"], &[]), Some(Underway::Rebase));
        assert_eq!(Underway::of(&["rebase-apply"], &["applying", "next"]), Some(Underway::Am));
        assert_eq!(Underway::of(&["rebase-apply"], &["next"]), Some(Underway::Rebase));
        assert_eq!(Underway::of(&["sequencer", "CHERRY_PICK_HEAD"], &[]), Some(Underway::CherryPick));
        assert_eq!(Underway::of(&["sequencer"], &[]), Some(Underway::Sequence), "between two of several");
        assert_eq!(Underway::of(&["HEAD"], &[]), None);
        assert_eq!(git_dir_of("/p\n\n/p/.git\n").as_deref(), Some("/p/.git"));
    }

    /// A folder's top and its place in it; an untracked folder, one entry,
    /// marks what is in it; in a hunk "--- x" removed is no header.
    #[test]
    fn a_folders_place_and_an_untracked_folder() {
        assert_eq!(parse_root("C:/w/proj\nsrc/ui/\n"), Some(("C:/w/proj".into(), "src/ui/".into())));
        assert_eq!(parse_root("/p\n\n"), Some(("/p".into(), String::new())));
        assert!(not_a_repo("fatal: not a git repository (or any of the parent directories): .git"));
        assert!(!not_a_repo("fatal: detected dubious ownership in repository"));
        assert!(not_a_repo("fatal: this operation must be run in a work tree"), "inside .git");
        let st = parse_status("# branch.head main\0? vendor/\0");
        assert_eq!(st.mark_of("vendor/lib/x.js", false), Some('?'));
        assert_eq!(st.mark_of("vendor", true), Some('?'));
        let kinds = diff_lines("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n--- x\n+++ y\n same\n");
        assert_eq!(kinds, [DiffLine::Header, DiffLine::Header, DiffLine::Header, DiffLine::Hunk, DiffLine::Removed, DiffLine::Added, DiffLine::Same]);
    }

    #[test]
    fn a_diffs_lines_are_told_apart() {
        assert_eq!(diff_line("diff --git a/x b/x"), DiffLine::Header);
        assert_eq!(diff_line("+++ b/x"), DiffLine::Header);
        assert_eq!(diff_line("@@ -1,2 +1,3 @@ fn x"), DiffLine::Hunk);
        assert_eq!(diff_line("+new"), DiffLine::Added);
        assert_eq!(diff_line("-old"), DiffLine::Removed);
        assert_eq!(diff_line(" same"), DiffLine::Same);
    }
}
