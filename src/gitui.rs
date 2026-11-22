//! Git, where the files are: the Files tree marks what changed (M A D R ?
//! U, and a dot on a folder with changes in it) and says the branch; the
//! Git tab lists the changes, shows each one's diff, stages and unstages,
//! discards and commits. git runs on the host the files are on (the
//! agent's or this PC's "git" op: no shell, never asking on a terminal,
//! paths taken as they are); what it prints is read here
//! (weft_core::git). Pull, push and fetch, which may ask for a password or
//! a key's passphrase, run in a terminal of that host.

use crate::app::{App, Pending};
use crate::theme;
use egui::{vec2, Color32, RichText, ScrollArea, TextEdit, Ui};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use weft_core::git::{self, DiffLine, FileStatus, Status, Underway};

/// How often a work tree's state is asked again while it is shown, at the
/// least (a slow one, five times as long as it took).
const FRESH: Duration = Duration::from_secs(3);

/// An answer not come in this long (the agent restarted, its question
/// lost): asked again, and its buttons back.
const LOST: Duration = Duration::from_secs(150);

/// A Discard pressed once waits this long for the second press.
const ARMED: Duration = Duration::from_secs(5);

/// The diff shown is read again at most this often while its file's
/// letter stays the same (a big one is not read every few seconds).
const DIFF_FRESH: Duration = Duration::from_secs(10);

/// A command of ours running this long: it may be waiting for something
/// (a signing key's passphrase window, a hook); said, and the buttons can
/// be had back.
const SLOW: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct GitState {
    /// A folder's work tree, by (host, folder): its top and the folder's
    /// place in it; None, in none.
    roots: HashMap<(String, String), Option<(String, String)>>,
    /// When a folder's work tree is to be asked for (again): asked and
    /// not answered yet, then once its answer could be lost; not to be told
    /// then, a few seconds on. None: not to be asked.
    asked_roots: HashMap<(String, String), Instant>,
    /// Why a folder's work tree could not be told (git missing there, an
    /// ownership it does not trust): said, and asked again later.
    root_errors: HashMap<(String, String), String>,
    /// Each work tree's state, by (host, top).
    repos: HashMap<(String, String), Repo>,
    /// The Git tab's commit message.
    pub message: String,
    /// The diff shown in the Git tab.
    pub diff: Option<Diff>,
    /// A file (its work tree's top, its path) whose changes Discard was
    /// pressed for once, and when: pressed again soon, they go.
    discard: Option<(String, String, Instant)>,
    /// Stage all pressed with conflicts: pressed again, they are taken as
    /// resolved.
    stage_conflicts: Option<Instant>,
    /// Abort (a merge, a rebase…) pressed once, and when.
    abort: Option<Instant>,
    /// The message a commit on its way was given: cleared after it only if
    /// not changed meanwhile.
    committing: Option<String>,
}

#[derive(Default)]
struct Repo {
    status: Option<Arc<Status>>,
    at: Option<Instant>,
    /// Asked for, and when (None: not now).
    asking: Option<Instant>,
    /// How long the last asking took.
    took: Duration,
    /// Changed by a command of ours since the asking now on its way began:
    /// its answer is old, and asked again.
    stale: bool,
    error: Option<String>,
    /// A command of ours running there (stage, commit…): its name, since.
    busy: Option<(String, Instant)>,
    /// The state was cut (a huge list): said.
    cut: bool,
    /// Its git folder (where a merge or a rebase in progress shows).
    git_dir: Option<String>,
    /// A merge, a rebase… in progress there.
    underway: Option<Underway>,
}

pub struct Diff {
    pub host: String,
    pub top: String,
    pub file: FileStatus,
    pub staged: bool,
    pub text: Option<String>,
    pub cut: bool,
    /// Its lines as shown, their kinds and the longest's length: made once
    /// when it comes (made each frame, a big diff held the window).
    lines: Vec<String>,
    kinds: Vec<DiffLine>,
    longest: usize,
    /// When it was asked for.
    asked: Instant,
}

impl Diff {
    fn fill(&mut self, text: String, cut: bool) {
        self.lines = text.lines().map(diff_shown).collect();
        self.kinds = git::diff_lines(&text);
        self.longest = self.lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        self.text = Some(text);
        self.cut = cut;
    }
}

/// A folder's work tree as Files shows it.
pub struct GitView {
    /// The folder Files shows, and its place in the work tree.
    dir: String,
    prefix: String,
    pub top: String,
    pub status: Arc<Status>,
}

impl GitView {
    /// The work tree's path for a path under the folder shown.
    pub fn rel(&self, path: &str) -> Option<String> {
        let r = rel_in(&self.dir, path)?;
        Some(format!("{}{}", self.prefix, r))
    }

    pub fn mark(&self, path: &str, dir: bool) -> Option<char> {
        self.status.mark_of(&self.rel(path)?, dir)
    }
}

/// `path` relative to `base` ("" for itself), or None outside it. On
/// Windows' file systems the case of the letters is no matter.
pub fn rel_in(base: &str, path: &str) -> Option<String> {
    let t = base.trim_end_matches('/');
    let p = path.trim_end_matches('/');
    let drive = t.as_bytes().get(1) == Some(&b':');
    let same = |a: &str, b: &str| if drive { a.eq_ignore_ascii_case(b) } else { a == b };
    if same(p, t) {
        return Some(String::new());
    }
    let head = p.get(..t.len() + 1)?;
    (same(&head[..t.len()], t) && head.ends_with('/')).then(|| p[t.len() + 1..].to_string())
}

/// The colour for a file's letter.
pub fn mark_color(m: char) -> Color32 {
    match m {
        'M' | 'T' => theme::warn(),
        'A' | '?' => theme::ansi(2),
        'D' | 'U' => theme::error(),
        'R' | 'C' => theme::ansi(4),
        _ => theme::text_faint(),
    }
}

/// What the letter means, said on hover.
pub fn mark_meaning(m: char) -> &'static str {
    match m {
        'M' => "modified",
        'T' => "type changed",
        'A' => "added",
        '?' => "untracked (new, not added)",
        'D' => "deleted",
        'U' => "in conflict",
        'R' => "renamed",
        'C' => "copied",
        _ => "has changes in it",
    }
}

/// A line of a diff shown: tabs as spaces, CR and escapes written out
/// (a file's own escapes would draw boxes or nothing).
fn diff_shown(l: &str) -> String {
    l.replace('\t', "    ").replace('\r', "\\r").replace('\x1b', "\\e")
}

#[cfg(test)]
impl GitState {
    /// How many work trees have a command of ours running.
    pub fn repos_busy(&self) -> usize {
        self.repos.values().filter(|r| r.busy.is_some()).count()
    }
}

impl App {
    /// The work tree `dir` on `host` is in, asked once (None meanwhile, and
    /// outside one); asked again when an answer was lost or could not be
    /// had.
    fn git_root(&mut self, host: &str, dir: &str) -> Option<(String, String)> {
        let key = (host.to_string(), dir.trim_end_matches('/').to_string());
        let next = self.git.asked_roots.get(&key).copied();
        let known = self.git.roots.get(&key).cloned();
        let again = match (next, &known) {
            (None, None) => true,
            (Some(t), _) => Instant::now() >= t,
            _ => false,
        };
        if again {
            self.git.asked_roots.insert(key.clone(), Instant::now() + LOST);
            let args: Vec<&str> = git::ROOT_ARGS.to_vec();
            self.request(host, json!({"op": "git", "dir": dir, "args": args}), Pending::GitRoot { dir: key.1.clone() });
        }
        known.flatten()
    }

    /// Why `dir`'s work tree could not be told, if it could not.
    pub fn git_root_error(&self, host: &str, dir: &str) -> Option<String> {
        self.git.root_errors.get(&(host.to_string(), dir.trim_end_matches('/').to_string())).cloned()
    }

    /// The state of the work tree `dir` is in, kept fresh while asked for.
    pub fn git_status_at(&mut self, host: &str, dir: &str) -> Option<GitView> {
        let (top, prefix) = self.git_root(host, dir)?;
        let key = (host.to_string(), top.clone());
        let focused = self.ctx.input(|i| i.focused);
        let repo = self.git.repos.entry(key.clone()).or_default();
        let every = FRESH.max(repo.took * 5);
        let stale = repo.at.is_none_or(|t| t.elapsed() >= every);
        // Lost answers: asked again, buttons back.
        if repo.asking.is_some_and(|t| t.elapsed() > LOST) {
            repo.asking = None;
        }
        if repo.busy.as_ref().is_some_and(|(_, t)| t.elapsed() > LOST) {
            repo.busy = None;
        }
        // Not while the window is not looked at (a remote work tree asked
        // every few seconds for nothing).
        if stale && focused && repo.asking.is_none() && repo.busy.is_none() {
            self.git_refresh(host, &top);
        }
        if focused {
            self.ctx.request_repaint_after(every);
        }
        let repo = self.git.repos.get(&key)?;
        let status = repo.status.clone()?;
        Some(GitView { dir: dir.trim_end_matches('/').to_string(), prefix, top, status })
    }

    pub fn git_refresh(&mut self, host: &str, top: &str) {
        let repo = self.git.repos.entry((host.to_string(), top.to_string())).or_default();
        // Not while a command of ours runs there: the state read meanwhile
        // could be from before it ends (shown, and a click landed on a row
        // that moved); its end asks again.
        if repo.asking.is_some() || repo.busy.is_some() {
            return;
        }
        repo.asking = Some(Instant::now());
        repo.stale = false;
        let git_dir = repo.git_dir.clone();
        let args: Vec<&str> = git::STATUS_ARGS.to_vec();
        self.request(host, json!({"op": "git", "dir": top, "args": args}), Pending::GitStatus { top: top.to_string() });
        // A merge, a rebase… in progress: the names in its git folder.
        if let Some(d) = git_dir {
            self.request(host, json!({"op": "list", "path": d, "hidden": true}), Pending::GitUnderway { top: top.to_string() });
        }
    }

    /// Folders listed anew (Files' refresh): their work trees are asked
    /// for again (what was known shown meanwhile).
    pub fn git_forget(&mut self, host: &str) {
        let keys: Vec<_> = self.git.roots.keys().filter(|(h, _)| h == host).cloned().collect();
        for k in keys {
            self.git.asked_roots.insert(k, Instant::now());
        }
    }

    /// A git command of ours there (stage, commit…), the state asked again
    /// after it.
    fn git_run(&mut self, host: &str, top: &str, what: &str, args: Vec<String>, input: Option<String>) {
        let repo = self.git.repos.entry((host.to_string(), top.to_string())).or_default();
        repo.busy = Some((what.to_string(), Instant::now()));
        if repo.asking.is_some() {
            repo.stale = true;
        }
        let mut req = json!({"op": "git", "dir": top, "args": args});
        if let Some(text) = input {
            req["input"] = Value::String(text);
        }
        self.request(host, req, Pending::GitDone { top: top.to_string(), what: what.to_string() });
    }

    /// The diff shown read again, its old text kept until the new comes
    /// (no "reading…" flashing every few seconds).
    fn git_refresh_diff(&mut self, host: &str, top: &str, f: &FileStatus, staged: bool) {
        let old = self.git.diff.take().filter(|d| d.file.path == f.path && d.staged == staged);
        self.git_show_diff(host, top, f, staged);
        if let (Some(d), Some(old)) = (self.git.diff.as_mut(), old) {
            (d.text, d.cut, d.lines, d.kinds, d.longest) = (old.text, old.cut, old.lines, old.kinds, old.longest);
        }
    }

    fn git_show_diff(&mut self, host: &str, top: &str, f: &FileStatus, staged: bool) {
        let s = |v: &[&str]| -> Vec<String> { v.iter().map(|x| x.to_string()).collect() };
        // A rename: both paths (one of them alone is a file added or gone).
        let paths: Vec<&str> = std::iter::once(f.path.as_str()).chain(f.from.as_deref()).collect();
        let mut args = if f.untracked {
            // A new file: all of it, as added.
            s(&["diff", "--no-color", "--no-ext-diff", "--no-index", "--", "/dev/null", &f.path])
        } else if staged {
            s(&["diff", "--no-color", "--no-ext-diff", "-M", "--cached", "--"])
        } else {
            s(&["diff", "--no-color", "--no-ext-diff", "--"])
        };
        if !f.untracked {
            args.extend(paths.iter().map(|p| p.to_string()));
        }
        self.git.diff = Some(Diff { host: host.into(), top: top.into(), file: f.clone(), staged, text: None, cut: false, lines: Vec::new(), kinds: Vec::new(), longest: 0, asked: Instant::now() });
        self.request(host, json!({"op": "git", "dir": top, "args": args}), Pending::GitDiff { top: top.to_string(), path: f.path.clone(), staged });
    }

    /// The agent's answers for these.
    pub fn on_git(&mut self, host: &str, pending: Pending, ok: bool, r: &Value, err: String) {
        let out = r["out"].as_str().unwrap_or("").to_string();
        let code = r["code"].as_i64().unwrap_or(-1);
        let said = r["err"].as_str().unwrap_or("").trim().to_string();
        match pending {
            Pending::GitRoot { dir } => {
                let key = (host.to_string(), dir);
                self.git.asked_roots.remove(&key);
                if ok && code == 0 {
                    let root = git::parse_root(&out);
                    if let (Some((top, _)), Some(d)) = (&root, git::git_dir_of(&out)) {
                        self.git.repos.entry((host.to_string(), top.clone())).or_default().git_dir = Some(d);
                    }
                    self.git.roots.insert(key.clone(), root);
                    self.git.root_errors.remove(&key);
                } else if ok && git::not_a_repo(&said) {
                    // Asked again now and then (a git init or a clone in a
                    // terminal there).
                    self.git.roots.insert(key.clone(), None);
                    self.git.root_errors.remove(&key);
                    self.git.asked_roots.insert(key, Instant::now() + Duration::from_secs(30));
                } else {
                    // git missing there, an ownership it does not trust, the
                    // agent not there yet: said, asked again later (not
                    // taken for "in no work tree").
                    let why = if ok { said.lines().last().unwrap_or("git failed").to_string() } else { err };
                    self.git.root_errors.insert(key.clone(), why);
                    self.git.asked_roots.insert(key, Instant::now() + Duration::from_secs(10));
                }
            }
            Pending::GitStatus { top } => {
                let key = (host.to_string(), top.clone());
                let repo = self.git.repos.entry(key.clone()).or_default();
                repo.took = repo.asking.map(|t| t.elapsed()).unwrap_or_default();
                repo.asking = None;
                repo.at = Some(Instant::now());
                if repo.stale {
                    // A change of ours came between: this answer is old.
                    self.git_refresh(host, &top);
                    return;
                }
                if ok && code == 0 {
                    let st = Arc::new(git::parse_status(&out));
                    repo.status = Some(st.clone());
                    repo.error = None;
                    repo.cut = r["cut"].as_bool().unwrap_or(false);
                    // The diff shown follows: the file where it is now (staged
                    // or not), read again when its state changed or now and
                    // then (its text may have changed with its letter the
                    // same), or none.
                    if let Some(d) = self.git.diff.as_ref().filter(|d| d.host == host && d.top == top) {
                        let now = st.files.iter().find(|f| f.path == d.file.path).cloned();
                        let (staged, was, asked) = (d.staged, d.file.clone(), d.asked);
                        match now {
                            Some(f) => {
                                let gone = (staged && !f.staged()) || (!staged && !f.unstaged() && !f.untracked && !f.conflict);
                                let s = if gone { !staged } else { staged };
                                if f != was || s != staged || asked.elapsed() >= DIFF_FRESH {
                                    self.git_refresh_diff(host, &top, &f, s);
                                }
                            }
                            None => self.git.diff = None,
                        }
                    }
                } else {
                    repo.error = Some(if ok { said } else { err });
                }
            }
            Pending::GitDone { top, what } => {
                if let Some(repo) = self.git.repos.get_mut(&(host.to_string(), top.clone())) {
                    repo.busy = None;
                }
                let sent = if what == "commit" { self.git.committing.take() } else { None };
                if ok && code == 0 {
                    if sent.is_some_and(|m| m == self.git.message) {
                        self.git.message.clear();
                    }
                    // Another repository in a folder staged (git adds it as
                    // a bare commit, with a warning only): taken back.
                    let inner: Vec<String> = said.lines().filter_map(|l| l.strip_prefix("warning: adding embedded git repository: ")).map(|p| p.trim().to_string()).collect();
                    if inner.is_empty() {
                        self.toast(format!("git {what}: done"), crate::app::ToastKind::Info);
                    } else {
                        self.toast(format!("git {what}: {} is another git repository, not added (to keep it as a submodule: git submodule add, in a terminal)", inner.join(", ")), crate::app::ToastKind::Error);
                        // (-f: its staged commit differs from the folder and HEAD both;
                        // only the index is touched.)
                        let mut args = vec!["rm".to_string(), "--cached".into(), "-f".into(), "-r".into(), "-q".into(), "--".into()];
                        args.extend(inner);
                        self.git_run(host, &top, "unstage the other repository", args, None);
                    }
                } else {
                    // Its last lines (a hook's, a signing key's say), not one.
                    let why = if ok {
                        let lines: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
                        if lines.is_empty() { "failed".to_string() } else { lines[lines.len().saturating_sub(6)..].join("\n") }
                    } else {
                        err
                    };
                    let hint = if what != "commit" {
                        ""
                    } else if said.contains("No secret key") || said.contains("secret key not available") || said.contains("no default secret key") {
                        "\n(commits here are signed and git found no key for it: user.signingkey)"
                    } else if said.contains("stopped after") || said.contains("passphrase") || said.contains("pinentry") || said.contains("Inappropriate ioctl") {
                        "\n(a signing key that asks for its passphrase: commit in a terminal)"
                    } else {
                        ""
                    };
                    self.toast(format!("git {what}: {why}{hint}"), crate::app::ToastKind::Error);
                }
                self.git_refresh(host, &top);
            }
            Pending::GitDiff { top, path, staged } => {
                if let Some(d) = self.git.diff.as_mut().filter(|d| d.host == host && d.top == top && d.file.path == path && d.staged == staged) {
                    // --no-index ends with 1 when there are differences.
                    let text = if ok && (code == 0 || code == 1) { out } else if ok { said } else { err };
                    d.fill(text, r["cut"].as_bool().unwrap_or(false));
                }
            }
            Pending::GitUnderway { top } => {
                let names: Vec<String> = r["entries"].as_array().map(|a| a.iter().filter_map(|e| e["n"].as_str().map(String::from)).collect()).unwrap_or_default();
                let key = (host.to_string(), top.clone());
                if !ok {
                    return;
                }
                // rebase-apply: git am's or a rebase's, told by what is in it.
                let git_dir = self.git.repos.get(&key).and_then(|r| r.git_dir.clone());
                if let (true, Some(d)) = (names.iter().any(|n| n == "rebase-apply"), git_dir) {
                    self.request(host, json!({"op": "list", "path": format!("{}/rebase-apply", d.trim_end_matches('/')), "hidden": true}), Pending::GitUnderwayApply { top, names });
                    return;
                }
                if let Some(repo) = self.git.repos.get_mut(&key) {
                    let names: Vec<&str> = names.iter().map(String::as_str).collect();
                    repo.underway = Underway::of(&names, &[]);
                }
            }
            Pending::GitUnderwayApply { top, names } => {
                let apply: Vec<&str> = r["entries"].as_array().map(|a| a.iter().filter_map(|e| e["n"].as_str()).collect()).unwrap_or_default();
                if let Some(repo) = self.git.repos.get_mut(&(host.to_string(), top)) {
                    let names: Vec<&str> = names.iter().map(String::as_str).collect();
                    repo.underway = Underway::of(&names, &apply);
                }
            }
            _ => {}
        }
    }

    /// The Git side tab: the work tree of the folder Files shows.
    pub fn ui_git(&mut self, ui: &mut Ui) {
        let Some((host, dir)) = self.side.root.clone() else {
            ui.label(RichText::new("open a folder in Files first").color(theme::text_faint()));
            return;
        };
        let Some(view) = self.git_status_at(&host, &dir) else {
            let none = self.git.roots.get(&(host.clone(), dir.trim_end_matches('/').to_string())).is_some_and(|r| r.is_none());
            let msg = match self.git_root_error(&host, &dir) {
                Some(e) => format!("git could not tell: {e}"),
                None if none => format!("{dir} is not in a git work tree"),
                None => "looking…".into(),
            };
            ui.label(RichText::new(msg).color(theme::text_faint()));
            return;
        };
        let (top, st) = (view.top.clone(), view.status.clone());
        let repo = self.git.repos.get(&(host.clone(), top.clone()));
        let busy = repo.and_then(|r| r.busy.clone());
        let error = repo.and_then(|r| r.error.clone());
        let cut = repo.is_some_and(|r| r.cut);
        let underway = repo.and_then(|r| r.underway);
        if self.git.discard.as_ref().is_some_and(|(_, _, t)| t.elapsed() > ARMED) {
            self.git.discard = None;
        }
        if self.git.stage_conflicts.is_some_and(|t| t.elapsed() > ARMED) {
            self.git.stage_conflicts = None;
        }
        if self.git.abort.is_some_and(|t| t.elapsed() > ARMED) {
            self.git.abort = None;
        }
        if busy.is_some() || self.git.discard.is_some() || self.git.stage_conflicts.is_some() || self.git.abort.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        // The branch and how far from its upstream.
        ui.horizontal_wrapped(|ui| {
            let short: String = st.oid.chars().take(7).collect();
            let branch = st.branch.clone().unwrap_or_else(|| if st.head == "(detached)" && !short.is_empty() { format!("detached at {short}") } else { format!("no branch {}", st.head) });
            ui.label(RichText::new(format!("\u{e0a0} {branch}")).strong());
            if let Some(u) = &st.upstream {
                let ab = match (st.ahead, st.behind) {
                    (0, 0) => "up to date".to_string(),
                    (a, 0) => format!("↑{a} to push"),
                    (0, b) => format!("↓{b} to pull"),
                    (a, b) => format!("↑{a} ↓{b}"),
                };
                ui.label(RichText::new(format!("{ab} · {u}")).size(11.5).color(theme::text_dim()));
            } else if st.branch.is_some() {
                ui.label(RichText::new("no upstream").size(11.5).color(theme::text_faint()));
            }
        });
        ui.label(RichText::new(weft_core::shown(&top)).size(11.0).color(theme::text_faint()));
        ui.horizontal(|ui| {
            for (cmd, tip) in [("fetch", "fetch from the remote"), ("pull", "pull: fetch and merge (or rebase, as git is set)"), ("push", "push the branch")] {
                if ui.small_button(cmd).on_hover_text(format!("{tip}, in a terminal of {host} (it may ask for a password)")).clicked() {
                    self.git_in_terminal(&host, &top, cmd);
                }
            }
            if crate::icons::button(ui, crate::icons::Icon::Refresh, "ask git again").clicked() {
                self.git_refresh(&host, &top);
            }
            if let Some((b, _)) = &busy {
                ui.label(RichText::new(format!("{b}…")).size(11.5).color(theme::text_faint()));
            }
        });
        // Long at it: waiting for something it will not get here?
        if let Some((b, since)) = busy.as_ref().filter(|(_, t)| t.elapsed() > SLOW) {
            ui.horizontal_wrapped(|ui| {
                let why = if b == "commit" { "a signing key's passphrase window may be open, or a hook still at work" } else { "a hook may still be at work" };
                ui.label(RichText::new(format!("{b}: {}s so far ({why})", since.elapsed().as_secs())).size(11.5).color(theme::warn()));
                if ui.small_button("stop waiting").on_hover_text("the buttons back now; git still runs there until it ends (2 minutes at most), its answer said when it comes").clicked() {
                    if let Some(r) = self.git.repos.get_mut(&(host.clone(), top.clone())) {
                        r.busy = None;
                    }
                }
            });
        }
        let busy = busy.map(|b| b.0);
        // A merge, a rebase… in progress: go on with it, or back out of it.
        if let Some(u) = underway {
            let conflicted = st.files.iter().any(|f| f.conflict);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{} in progress", u.word())).strong().color(theme::warn()));
                let sequence = u == Underway::Sequence;
                let tip = if sequence {
                    "between commits of a cherry-pick or a revert: git cherry-pick --continue (or revert --continue, --abort) in a terminal"
                } else if conflicted {
                    "resolve the conflicts first (+ each one)"
                } else if u == Underway::Merge {
                    "commit what is staged (the message typed, else git's) and end the merge"
                } else {
                    "go on: what is staged committed with git's message, and on to the next step"
                };
                if ui.add_enabled(!sequence && !conflicted && busy.is_none(), egui::Button::new("Continue").small()).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                    // Its own message (the editor it would open: none, GIT_EDITOR
                    // is "true" for weft's git).
                    let mut a = vec![u.word().to_string(), "--continue".into()];
                    let mut input = None;
                    // A merge's own message, if one was typed (a cherry-pick's
                    // or a revert's --continue goes on to their next commits).
                    if u == Underway::Merge && !self.git.message.trim().is_empty() {
                        a = vec!["commit".into(), "-F".into(), "-".into()];
                        input = Some(self.git.message.clone());
                        self.git.committing = Some(self.git.message.clone());
                    }
                    let what = if input.is_some() { "commit" } else { "continue" };
                    self.git_run(&host, &top, what, a, input);
                }
                let armed = self.git.abort.is_some();
                if !sequence && ui.add_enabled(busy.is_none(), egui::Button::new(RichText::new(if armed { "Abort?" } else { "Abort" }).color(if armed { theme::error() } else { theme::text_dim() })).small()).on_hover_text(format!("git {} --abort: back to before it (press again within 5 s)", u.word())).clicked() {
                    if armed {
                        self.git.abort = None;
                        self.git_run(&host, &top, "abort", vec![u.word().into(), "--abort".into()], None);
                    } else {
                        self.git.abort = Some(Instant::now());
                    }
                }
            });
        }
        if let Some(e) = error {
            ui.label(RichText::new(e).size(11.5).color(theme::error()));
        }
        if cut {
            ui.label(RichText::new("git listed more than can be shown: only the first part is here").size(11.5).color(theme::warn()));
        }
        ui.separator();
        // The commit message, and Commit for what is staged.
        let staged: Vec<FileStatus> = st.files.iter().filter(|f| f.staged()).cloned().collect();
        let unstaged: Vec<FileStatus> = st.files.iter().filter(|f| f.unstaged()).cloned().collect();
        let untracked: Vec<FileStatus> = st.files.iter().filter(|f| f.untracked).cloned().collect();
        let conflicts: Vec<FileStatus> = st.files.iter().filter(|f| f.conflict).cloned().collect();
        let msg_id = ui.make_persistent_id("git-message");
        let ctrl_enter = ui.memory(|m| m.has_focus(msg_id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        ui.add(TextEdit::multiline(&mut self.git.message).id(msg_id).hint_text("commit message (the first line a short summary) · Ctrl+Enter: commit").desired_rows(2).desired_width(f32::INFINITY));
        let mut act: Option<(String, Vec<String>, Option<String>)> = None;
        let s = |v: &[&str]| -> Vec<String> { v.iter().map(|x| x.to_string()).collect() };
        ui.horizontal_wrapped(|ui| {
            let can = !staged.is_empty() && !self.git.message.trim().is_empty() && busy.is_none() && underway.is_none();
            let tip = if underway.is_some() { "a merge or a rebase is in progress: Continue above" } else if staged.is_empty() { "nothing staged: + a file below first" } else if self.git.message.trim().is_empty() { "write a message first" } else { "commit what is staged (Ctrl+Enter in the message)" };
            if ui.add_enabled(can, egui::Button::new(format!("Commit {} file{}", staged.len(), if staged.len() == 1 { "" } else { "s" }))).on_hover_text(tip).on_disabled_hover_text(tip).clicked() || ctrl_enter && can {
                self.git.committing = Some(self.git.message.clone());
                act = Some(("commit".into(), s(&["commit", "-F", "-"]), Some(self.git.message.clone())));
            }
            let free = busy.is_none();
            if (!unstaged.is_empty() || !untracked.is_empty() || !conflicts.is_empty()) && ui.add_enabled(free, egui::Button::new("Stage all").small()).on_hover_text("git add -A: every change and new file").clicked() {
                // Conflicts too: asked first (staged, they are taken as
                // resolved, markers and all).
                if conflicts.is_empty() || self.git.stage_conflicts.is_some() {
                    self.git.stage_conflicts = None;
                    act = Some(("stage all".into(), s(&["add", "-A"]), None));
                } else {
                    self.git.stage_conflicts = Some(Instant::now());
                }
            }
            if !staged.is_empty() && ui.add_enabled(free, egui::Button::new("Unstage all").small()).on_hover_text("keep the changes, staged none").clicked() {
                act = Some(("unstage all".into(), s(&["reset", "-q"]), None));
            }
        });
        if self.git.stage_conflicts.is_some() {
            ui.label(RichText::new(format!("{} file(s) in conflict: Stage all again takes them as resolved, markers and all", conflicts.len())).size(11.5).color(theme::warn()));
        }
        if st.files.is_empty() {
            ui.label(RichText::new("nothing changed").color(theme::text_faint()));
        }
        let mut show: Option<(FileStatus, bool)> = None;
        let mut open: Option<String> = None;
        let list_h = (ui.available_height() * if self.git.diff.is_some() { 0.4 } else { 1.0 }).max(80.0);
        // Each group's title, then its files: only the rows in sight drawn
        // (twenty thousand changes held the window).
        let groups = [("Conflicts", &conflicts, false), ("Staged", &staged, true), ("Changes", &unstaged, false), ("Untracked", &untracked, false)];
        let rows: Vec<(usize, Option<usize>)> = groups.iter().enumerate().filter(|(_, g)| !g.1.is_empty()).flat_map(|(gi, g)| std::iter::once((gi, None)).chain((0..g.1.len()).map(move |i| (gi, Some(i))))).collect();
        let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
        ScrollArea::vertical().id_salt("git-files").max_height(list_h).auto_shrink([false, true]).show_rows(ui, row_h - ui.spacing().item_spacing.y, rows.len(), |ui, range| {
            for &(gi, row) in &rows[range] {
                let (title, files, staged_list) = groups[gi];
                let Some(i) = row else {
                    ui.label(RichText::new(format!("{title} ({})", files.len())).size(11.5).strong().color(theme::text_dim()));
                    continue;
                };
                {
                    let f = &files[i];
                    let m = if staged_list { f.index } else { f.mark() };
                    let folder = f.path.ends_with('/');
                    let shown = self.git.diff.as_ref().is_some_and(|d| d.file.path == f.path && d.staged == staged_list);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(m.to_string()).monospace().color(mark_color(m))).on_hover_text(mark_meaning(m));
                        let name = match &f.from {
                            Some(from) => format!("{from} → {}", f.path),
                            None => f.path.clone(),
                        };
                        let tip = if folder { "a folder git does not track yet (+ adds all in it)" } else { "click: its changes · double-click: open it" };
                        let r = ui.selectable_label(shown, RichText::new(weft_core::shown(&name)).size(12.5)).on_hover_text(tip);
                        if r.clicked() && !folder {
                            show = Some((f.clone(), staged_list));
                        }
                        if r.double_clicked() && !folder {
                            open = Some(f.path.clone());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if busy.is_some() {
                                return;
                            }
                            // A rename's both paths: one alone leaves half of it.
                            let paths = || -> Vec<String> { std::iter::once(f.path.clone()).chain(f.from.clone()).collect() };
                            if staged_list {
                                if ui.small_button("−").on_hover_text("unstage: keep the change, not staged").clicked() {
                                    let mut a = s(&["reset", "-q", "--"]);
                                    a.extend(paths());
                                    act = Some((format!("unstage {}", f.path), a, None));
                                }
                            } else if !f.conflict {
                                // Discard: not for a folder (another repository
                                // in it, files git would not say it took), nor a
                                // submodule (its own commits: in it, in a
                                // terminal).
                                if !folder && !f.submodule {
                                    let armed = self.git.discard.as_ref().is_some_and(|(t, p, _)| *t == top && *p == f.path);
                                    let label = if armed { "Discard?" } else { "↶" };
                                    let tip = if f.untracked {
                                        "delete this new file (press again within 5 s)"
                                    } else if f.intent_to_add() {
                                        "not to be added after all: untracked again, the file kept (press again within 5 s)"
                                    } else if f.staged() {
                                        "throw away the change not staged, back to what is staged (press again within 5 s)"
                                    } else {
                                        "throw the change away, back to what is committed (press again within 5 s)"
                                    };
                                    let b = ui.small_button(RichText::new(label).color(if armed { theme::error() } else { theme::text_dim() })).on_hover_text(tip);
                                    if b.clicked() {
                                        if armed {
                                            self.git.discard = None;
                                            act = Some(if f.untracked {
                                                (format!("delete {}", f.path), s(&["clean", "-f", "-q", "--", &f.path]), None)
                                            } else if f.intent_to_add() {
                                                // (checkout would empty it.)
                                                (format!("un-add {}", f.path), s(&["rm", "--cached", "-q", "--", &f.path]), None)
                                            } else {
                                                (format!("discard {}", f.path), s(&["checkout", "-q", "--", &f.path]), None)
                                            });
                                        } else {
                                            self.git.discard = Some((top.clone(), f.path.clone(), Instant::now()));
                                        }
                                    }
                                }
                                if ui.small_button("+").on_hover_text("stage it for the next commit").clicked() {
                                    act = Some((format!("stage {}", f.path), s(&["add", "-A", "--", &f.path]), None));
                                }
                            } else if ui.small_button("+").on_hover_text("mark it resolved (stage it as it is now)").clicked() {
                                // (-A: one both sides deleted is staged gone.)
                                act = Some((format!("resolve {}", f.path), s(&["add", "-A", "--", &f.path]), None));
                            }
                        });
                    });
                }
            }
        });
        if let Some((what, args, input)) = act {
            self.git_run(&host, &top, &what, args, input);
        }
        if let Some((f, staged)) = show {
            self.git_show_diff(&host, &top, &f, staged);
        }
        if let Some(p) = open {
            self.open_file(&host, &format!("{}/{}", top.trim_end_matches('/'), p), 1);
        }
        // The diff of the file clicked.
        let mut close = false;
        if let Some(d) = self.git.diff.as_ref().filter(|d| d.host == host && d.top == top) {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} {}", weft_core::shown(&d.file.path), if d.staged { "(staged)" } else { "" })).size(12.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, crate::icons::Icon::Cross, "close the diff").clicked() {
                        close = true;
                    }
                });
            });
            match &d.text {
                None => {
                    ui.label(RichText::new("reading…").color(theme::text_faint()));
                }
                Some(text) if text.trim().is_empty() => {
                    ui.label(RichText::new("no changes in text (a mode, or a binary file)").color(theme::text_faint()));
                }
                Some(_) => {
                    let font = egui::FontId::monospace((self.settings.font_size - 1.5).max(9.0));
                    let (lines, kinds, longest) = (&d.lines, &d.kinds, d.longest);
                    let row_h = ui.fonts(|f| f.row_height(&font)) + 2.0;
                    // Only the lines in sight are drawn (a big diff held the
                    // window).
                    // As wide as its longest line, the same at any scroll
                    // (a monospace font: its characters times one's width).
                    let char_w = crate::widgets::text_width(ui.painter(), "M", font.clone());
                    ScrollArea::both().id_salt("git-diff").auto_shrink([false, false]).show_rows(ui, row_h, lines.len() + usize::from(d.cut), |ui, range| {
                        let w = ui.available_width().max(longest as f32 * char_w + 8.0);
                        for i in range {
                            let (rect, _) = ui.allocate_exact_size(vec2(w, row_h), egui::Sense::hover());
                            let Some(l) = lines.get(i) else {
                                ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, "(cut here: a big diff)", font.clone(), theme::warn());
                                continue;
                            };
                            let (col, bg) = match kinds.get(i).copied().unwrap_or(DiffLine::Same) {
                                DiffLine::Header => (theme::text_faint(), None),
                                DiffLine::Hunk => (theme::ansi(6), None),
                                DiffLine::Added => (theme::ansi(2), Some(theme::tint(theme::ansi(2), 28))),
                                DiffLine::Removed => (theme::ansi(1), Some(theme::tint(theme::ansi(1), 28))),
                                DiffLine::Same => (theme::text_dim(), None),
                            };
                            if let Some(bg) = bg {
                                ui.painter().rect_filled(rect, 0.0, bg);
                            }
                            ui.painter().text(rect.left_center() + vec2(2.0, 0.0), egui::Align2::LEFT_CENTER, l, font.clone(), col);
                        }
                    });
                }
            }
        }
        if close {
            self.git.diff = None;
        }
    }

    /// Fetch, pull or push in a terminal of `host` at its prompt (they may
    /// ask for a password or a key's passphrase, and print as they go).
    fn git_in_terminal(&mut self, host: &str, top: &str, cmd: &str) {
        let at_prompt = |a: &App, id: usize| a.panes.get(&id).is_some_and(|p| p.host == host && p.term.at_prompt() && !p.exited);
        let id = self.active_pane_id().filter(|id| at_prompt(self, *id)).or_else(|| self.panes.keys().copied().find(|id| at_prompt(self, *id)));
        match id {
            Some(id) => {
                let line = format!("git -C {} {cmd}", self.quote_in(id, top));
                self.focus_pane(id);
                self.run_in_pane(id, &line);
            }
            None => self.toast(format!("git {cmd}: open a terminal on {host} at its prompt first (it may ask for a password there)"), crate::app::ToastKind::Info),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path inside a work tree, relative to its top; Windows' drive
    /// paths in any case; outside it, none (a folder that only starts the
    /// same is outside); through the folder's place in the work tree.
    #[test]
    fn a_path_in_a_work_tree() {
        assert_eq!(rel_in("/home/u/proj", "/home/u/proj/src/a.rs").as_deref(), Some("src/a.rs"));
        assert_eq!(rel_in("/home/u/proj/", "/home/u/proj").as_deref(), Some(""));
        assert_eq!(rel_in("/home/u/proj", "/home/u/project/x"), None);
        assert_eq!(rel_in("C:/Work/Proj", "c:/work/proj/src").as_deref(), Some("src"));
        assert_eq!(rel_in("/home/u/proj", "/home/u"), None);
        // Files shows a link into the work tree's src/: its paths are the
        // work tree's.
        let v = GitView { dir: "/home/u/link".into(), prefix: "src/".into(), top: "/srv/proj".into(), status: Arc::new(git::parse_status("1 .M N... 100644 100644 100644 a b src/a.rs\0")) };
        assert_eq!(v.rel("/home/u/link/a.rs").as_deref(), Some("src/a.rs"));
        assert_eq!(v.mark("/home/u/link/a.rs", false), Some('M'));
    }
}
