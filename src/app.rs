//! Application state and logic (no drawing here; see ui_term.rs / ui_panels.rs).

use crate::host::{self, Conn, ConnStatus, DataDir, HostKind, Profile};
use crate::pty::Pty;
use crate::term::{BlockState, Term, TermEvent};
use egui::Rect;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Settings & session (stored in data/settings.json next to the executable)
// ---------------------------------------------------------------------------

fn d_font() -> f32 {
    14.0
}
fn d_true() -> bool {
    true
}
fn d_theme() -> String {
    crate::theme::DEFAULT_DARK.into()
}
fn d_theme_dark() -> String {
    crate::theme::DEFAULT_DARK.into()
}
fn d_theme_light() -> String {
    crate::theme::DEFAULT_LIGHT.into()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    #[serde(default = "d_font")]
    pub font_size: f32,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default = "d_true")]
    pub sidebar: bool,
    #[serde(default)]
    pub show_hidden: bool,
    /// A theme id (see theme.rs), or "system" to follow the OS light/dark setting.
    #[serde(default = "d_theme")]
    pub theme: String,
    /// Themes used for "system": the last dark and light theme picked.
    #[serde(default = "d_theme_dark")]
    pub theme_dark: String,
    #[serde(default = "d_theme_light")]
    pub theme_light: String,
    #[serde(default)]
    pub session: Vec<SavedNode>,
}

impl Default for Settings {
    fn default() -> Self {
        let mut profiles = vec![Profile::local()];
        let git_bash = "C:/Program Files/Git/bin/bash.exe";
        if cfg!(windows) && std::path::Path::new(git_bash).is_file() {
            // A POSIX "remote" that is really local: exercises exactly the code
            // path an ssh host takes (in-memory rc + stdin-shipped agent).
            profiles.push(Profile {
                name: "posix-loopback".into(),
                kind: HostKind::Custom {
                    pty: vec![git_bash.into(), "-c".into()],
                    agent: vec![git_bash.into(), "-c".into()],
                },
            });
        }
        Settings { font_size: d_font(), profiles, sidebar: true, show_hidden: false, theme: d_theme(), theme_dark: d_theme_dark(), theme_light: d_theme_light(), session: Vec::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t")]
pub enum SavedNode {
    Pane { host: String, cwd: Option<String> },
    Split { vertical: bool, ratio: f32, a: Box<SavedNode>, b: Box<SavedNode> },
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

pub enum Node {
    Leaf(usize),
    /// `vertical`: children side by side (a | b).
    Split { vertical: bool, ratio: f32, a: Box<Node>, b: Box<Node> },
}

impl Node {
    pub fn leaves(&self, out: &mut Vec<usize>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.leaves(out);
                b.leaves(out);
            }
        }
    }

    fn split_leaf(&mut self, target: usize, new_id: usize, vertical: bool) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split {
                    vertical,
                    ratio: 0.5,
                    a: Box::new(Node::Leaf(target)),
                    b: Box::new(Node::Leaf(new_id)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split_leaf(target, new_id, vertical) || b.split_leaf(target, new_id, vertical),
        }
    }

    fn remove(self, target: usize) -> Option<Node> {
        match self {
            Node::Leaf(id) if id == target => None,
            Node::Leaf(id) => Some(Node::Leaf(id)),
            Node::Split { vertical, ratio, a, b } => match (a.remove(target), b.remove(target)) {
                (Some(a), Some(b)) => Some(Node::Split { vertical, ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(x), None) | (None, Some(x)) => Some(x),
                (None, None) => None,
            },
        }
    }

    /// Pane rects plus splitter handles (path to the split node, handle rect, vertical, parent rect).
    pub fn layout(&self, rect: Rect, path: &mut Vec<bool>, panes: &mut Vec<(usize, Rect)>, splits: &mut Vec<(Vec<bool>, Rect, bool, Rect)>) {
        match self {
            Node::Leaf(id) => panes.push((*id, rect)),
            Node::Split { vertical, ratio, a, b } => {
                let gap = 4.0;
                let (ra, rb, handle) = if *vertical {
                    let x = rect.left() + rect.width() * ratio;
                    (
                        Rect::from_min_max(rect.min, egui::pos2(x - gap / 2.0, rect.bottom())),
                        Rect::from_min_max(egui::pos2(x + gap / 2.0, rect.top()), rect.max),
                        Rect::from_min_max(egui::pos2(x - gap / 2.0, rect.top()), egui::pos2(x + gap / 2.0, rect.bottom())),
                    )
                } else {
                    let y = rect.top() + rect.height() * ratio;
                    (
                        Rect::from_min_max(rect.min, egui::pos2(rect.right(), y - gap / 2.0)),
                        Rect::from_min_max(egui::pos2(rect.left(), y + gap / 2.0), rect.max),
                        Rect::from_min_max(egui::pos2(rect.left(), y - gap / 2.0), egui::pos2(rect.right(), y + gap / 2.0)),
                    )
                };
                splits.push((path.clone(), handle, *vertical, rect));
                path.push(false);
                a.layout(ra, path, panes, splits);
                path.pop();
                path.push(true);
                b.layout(rb, path, panes, splits);
                path.pop();
            }
        }
    }

    pub fn ratio_at(&mut self, path: &[bool]) -> Option<&mut f32> {
        match self {
            Node::Leaf(_) => None,
            Node::Split { ratio, a, b, .. } => match path.split_first() {
                None => Some(ratio),
                Some((false, rest)) => a.ratio_at(rest),
                Some((true, rest)) => b.ratio_at(rest),
            },
        }
    }
}

pub struct Tab {
    pub root: Node,
    pub active: usize,
    /// Show only the active pane, full size (tmux-style zoom).
    pub zoomed: bool,
}

// ---------------------------------------------------------------------------
// Panes and their GUI layers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub struct Selection {
    pub a: (u64, usize),
    pub b: (u64, usize),
}

pub struct SearchState {
    pub query: String,
    pub icase: bool,
    pub hits: Vec<(u64, usize, usize)>,
    pub cur: usize,
    pub seen_generation: u64,
    pub focus: bool,
}

pub struct Composer {
    pub text: String,
    pub hist_prefix: Option<String>,
    pub hist_pos: usize,
    pub cands: Vec<String>,
    pub cand_idx: usize,
    /// Byte offset in `text` where the token being completed starts.
    pub token_start: usize,
    pub token_dir: String,
    pub pending: bool,
    pub focus: bool,
}

impl Composer {
    pub fn new(text: String) -> Self {
        Composer {
            text,
            hist_prefix: None,
            hist_pos: 0,
            cands: Vec::new(),
            cand_idx: usize::MAX,
            token_start: 0,
            token_dir: String::new(),
            pending: false,
            focus: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct WEntry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub mtime: i64,
    pub mode: String,
}

#[derive(Clone, Debug)]
pub enum WidgetKind {
    Files { dir: String, entries: Vec<WEntry> },
    Table { columns: Vec<String>, data: Vec<Vec<String>> },
    Choose { options: Vec<String> },
    Progress { pct: f32, label: String },
}

/// A GUI widget woven into the text grid: it covers `rows` lines starting at
/// absolute line `anchor`, exactly where the helper printed its text fallback.
pub struct InlineWidget {
    pub id: String,
    pub anchor: u64,
    pub rows: usize,
    pub kind: WidgetKind,
    pub sort: Option<(usize, bool)>,
    pub chosen: Option<usize>,
}

pub struct Pane {
    pub id: usize,
    pub host: String,
    pub term: Term,
    pub parser: vte::Parser,
    pub pty: Option<Pty>,
    pub exited: bool,
    pub spawn_error: Option<String>,
    pub initial_cwd: Option<String>,
    /// Visual rows scrolled back from the bottom.
    pub scroll: usize,
    pub sel: Option<Selection>,
    pub selecting: bool,
    pub widgets: Vec<InlineWidget>,
    pub composer: Option<Composer>,
    pub search: Option<SearchState>,
    pub bell_at: Option<Instant>,
    pub preedit: String,
    pub hist_seen: u64,
    pub flash_block: Option<(u64, Instant)>,
}

impl Pane {
    fn new(id: usize, host: &str, cwd: Option<String>) -> Pane {
        Pane {
            id,
            host: host.to_string(),
            term: Term::new(100, 30),
            parser: vte::Parser::new(),
            pty: None,
            exited: false,
            spawn_error: None,
            initial_cwd: cwd,
            scroll: 0,
            sel: None,
            selecting: false,
            widgets: Vec::new(),
            composer: None,
            search: None,
            bell_at: None,
            preedit: String::new(),
            hist_seen: 0,
            flash_block: None,
        }
    }

    pub fn title(&self) -> String {
        let cwd = self.term.cwd.as_deref().or(self.initial_cwd.as_deref()).unwrap_or("");
        let base = cwd.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let base = if base.is_empty() { cwd } else { base };
        if base.is_empty() {
            self.host.clone()
        } else {
            format!("{} · {}", self.host, base)
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        if let Some(p) = self.pty.as_mut() {
            p.write(bytes);
        }
        self.scroll = 0;
    }
}

// ---------------------------------------------------------------------------
// Side panel, editor, misc state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub dir: bool,
    pub link: bool,
    pub size: u64,
}

pub struct DirListing {
    pub entries: Vec<DirEntry>,
    pub at: Instant,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SideTab {
    Files,
    Blocks,
    Search,
    Procs,
}

pub struct SideState {
    pub tab: SideTab,
    pub root: Option<(String, String)>,
    pub follow: bool,
    pub expanded: HashSet<(String, String)>,
    pub filter: String,
    pub grep_q: String,
    pub grep_icase: bool,
    pub grep_regex: bool,
    pub grep_busy: bool,
    pub grep: Option<Value>,
    pub procs: Option<Result<Vec<Value>, String>>,
    pub procs_at: Option<Instant>,
    pub procs_filter: String,
    pub procs_sort: (usize, bool),
    pub procs_host: String,
}

pub struct EditorFile {
    pub host: String,
    pub path: String,
    pub text: String,
    pub dirty: bool,
    pub mtime: Option<f64>,
    pub binary: bool,
    pub scratch: bool,
    pub loading: bool,
    pub saving: bool,
    pub conflict: bool,
    pub goto: Option<usize>,
    pub find: String,
    pub error: Option<String>,
}

pub struct EditorState {
    pub files: Vec<EditorFile>,
    pub active: usize,
    pub visible: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ToastKind {
    Info,
    Ok,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub at: Instant,
}

pub struct PaletteState {
    pub query: String,
    pub sel: usize,
}

pub struct ConnectState {
    pub target: String,
    pub split: bool,
    /// Optional ssh password; empty means keys / ssh-agent.
    pub password: String,
}

pub enum Pending {
    List { path: String },
    Open { path: String, line: usize },
    Save { path: String },
    Complete { pane: usize },
    Procs,
    Sys,
    Grep,
    Kill,
}

/// Things UI code asks the app to do once drawing is finished.
pub enum Action {
    Send(usize, Vec<u8>),
    Run(usize, String),
    Paste(usize, String),
    Copy(String),
    OpenUrl(String),
    OpenFile(String, String, usize),
    OpenScratch(String, String),
    ListDir(String, String),
    SetRoot(String, String),
    Focus(usize),
    ToggleFold(usize, u64),
    ComposerOpen(usize, String),
    Complete(usize),
    NewTab(String, Option<String>),
    Split(bool),
    ClosePane(usize),
    Restart(usize),
    Grep,
    Procs,
    Kill(i64, &'static str),
    Save(usize),
    Reconnect(String),
    ToggleZoom,
}

pub struct App {
    pub ctx: egui::Context,
    pub data: DataDir,
    pub settings: Settings,
    pub conns: HashMap<String, Conn>,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub panes: HashMap<usize, Pane>,
    next_pane: usize,
    pub pending: HashMap<(String, u64), Pending>,
    pub dir_cache: HashMap<(String, String), DirListing>,
    pub inflight: HashSet<(String, String)>,
    pub side: SideState,
    pub editor: EditorState,
    pub sys: HashMap<String, Value>,
    sys_at: HashMap<String, Instant>,
    pub history: HashMap<String, Vec<String>>,
    pub toasts: Vec<Toast>,
    pub palette: Option<PaletteState>,
    pub connect: Option<ConnectState>,
    /// Agent authentication prompt: (host, password being typed).
    pub auth_prompt: Option<(String, String)>,
    /// Theme picker popup, anchored below the button (rect) that opened it.
    pub theme_menu: Option<egui::Rect>,
    /// ssh passwords for this session only; never written to disk or logs.
    pub secrets: HashMap<String, String>,
    auth_prompted: HashSet<String>,
    pub focus_term: bool,
    pub pane_rects: Vec<(usize, Rect)>,
    pub ssh_hosts: Vec<String>,
    pub actions: Vec<Action>,
    agent_retried: HashSet<String>,
    last_save: Instant,
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parent(path: &str) -> String {
    let t = path.trim_end_matches('/');
    match t.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => {
            let p = &t[..i];
            if p.ends_with(':') {
                format!("{p}/")
            } else {
                p.to_string()
            }
        }
        None => path.to_string(),
    }
}

impl App {
    pub fn new(ctx: egui::Context) -> App {
        let data = DataDir::locate();
        let settings: Settings = std::fs::read_to_string(data.path("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        data.log(&format!("start, data dir {}", data.root.display()));
        let mut app = App {
            ctx,
            data,
            settings,
            conns: HashMap::new(),
            tabs: Vec::new(),
            active_tab: 0,
            panes: HashMap::new(),
            next_pane: 1,
            pending: HashMap::new(),
            dir_cache: HashMap::new(),
            inflight: HashSet::new(),
            side: SideState {
                tab: SideTab::Files,
                root: None,
                follow: true,
                expanded: HashSet::new(),
                filter: String::new(),
                grep_q: String::new(),
                grep_icase: true,
                grep_regex: false,
                grep_busy: false,
                grep: None,
                procs: None,
                procs_at: None,
                procs_filter: String::new(),
                procs_sort: (3, false),
                procs_host: String::new(),
            },
            editor: EditorState { files: Vec::new(), active: 0, visible: false },
            sys: HashMap::new(),
            sys_at: HashMap::new(),
            history: HashMap::new(),
            toasts: Vec::new(),
            palette: None,
            connect: None,
            focus_term: true,
            pane_rects: Vec::new(),
            ssh_hosts: host::ssh_config_hosts(),
            actions: Vec::new(),
            agent_retried: HashSet::new(),
            auth_prompt: None,
            theme_menu: None,
            secrets: HashMap::new(),
            auth_prompted: HashSet::new(),
            last_save: Instant::now(),
        };
        let session = std::mem::take(&mut app.settings.session);
        for node in &session {
            let root = app.restore_node(node);
            let mut leaves = Vec::new();
            root.leaves(&mut leaves);
            app.tabs.push(Tab { root, active: leaves[0], zoomed: false });
        }
        app.settings.session = session;
        if app.tabs.is_empty() {
            app.new_tab("local", None);
        }
        app
    }

    fn restore_node(&mut self, n: &SavedNode) -> Node {
        match n {
            SavedNode::Pane { host, cwd } => Node::Leaf(self.spawn_pane(host, cwd.clone())),
            SavedNode::Split { vertical, ratio, a, b } => Node::Split {
                vertical: *vertical,
                ratio: *ratio,
                a: Box::new(self.restore_node(a)),
                b: Box::new(self.restore_node(b)),
            },
        }
    }

    fn save_node(&self, n: &Node) -> SavedNode {
        match n {
            Node::Leaf(id) => {
                let p = &self.panes[id];
                SavedNode::Pane { host: p.host.clone(), cwd: p.term.cwd.clone().or(p.initial_cwd.clone()) }
            }
            Node::Split { vertical, ratio, a, b } => SavedNode::Split {
                vertical: *vertical,
                ratio: *ratio,
                a: Box::new(self.save_node(a)),
                b: Box::new(self.save_node(b)),
            },
        }
    }

    pub fn save_settings(&mut self) {
        self.settings.session = self.tabs.iter().map(|t| self.save_node(&t.root)).collect();
        if let Ok(s) = serde_json::to_string_pretty(&self.settings) {
            let _ = std::fs::write(self.data.path("settings.json"), s);
        }
        self.last_save = Instant::now();
    }

    pub fn profile(&self, name: &str) -> Profile {
        if let Some(p) = self.settings.profiles.iter().find(|p| p.name == name) {
            return p.clone();
        }
        if name == "local" {
            return Profile::local();
        }
        Profile::ssh(name)
    }

    fn wake(&self) -> impl Fn() + Send + Clone + 'static {
        let ctx = self.ctx.clone();
        move || ctx.request_repaint()
    }

    pub fn ensure_conn(&mut self, host: &str) {
        if !self.conns.contains_key(host) {
            let conn = Conn::open(self.profile(host), &self.data, self.wake(), self.secrets.get(host).cloned());
            self.conns.insert(host.to_string(), conn);
        }
    }

    pub fn reconnect(&mut self, host: &str) {
        let wake = self.wake();
        let secret = self.secrets.get(host).cloned();
        if let Some(c) = self.conns.get_mut(host) {
            c.connect(wake, secret);
        }
    }

    /// Remember a password for this session only (never saved) and retry the agent.
    pub fn set_password(&mut self, host: &str, password: String) {
        if password.is_empty() {
            self.secrets.remove(host);
        } else {
            self.secrets.insert(host.to_string(), password);
        }
        self.agent_retried.insert(host.to_string());
        self.auth_prompted.remove(host);
        if self.conns.contains_key(host) {
            self.reconnect(host);
        }
    }

    // ---- panes & tabs ----------------------------------------------------------

    pub fn spawn_pane(&mut self, host: &str, cwd: Option<String>) -> usize {
        let id = self.next_pane;
        self.next_pane += 1;
        self.ensure_conn(host);
        let mut pane = Pane::new(id, host, cwd);
        self.start_pty(&mut pane);
        self.panes.insert(id, pane);
        if !self.history.contains_key(host) {
            let hist = std::fs::read_to_string(self.data.path(&format!("history/{}.txt", host::safe_name(host))))
                .map(|s| s.lines().map(String::from).collect())
                .unwrap_or_default();
            self.history.insert(host.to_string(), hist);
        }
        id
    }

    fn start_pty(&self, pane: &mut Pane) {
        let prof = self.profile(&pane.host);
        let secret = if prof.is_local() { None } else { self.secrets.get(&pane.host) };
        let argv = prof.pty_argv(pane.initial_cwd.as_deref(), secret.is_some());
        let env = secret.map(|s| host::askpass_env(s)).unwrap_or_default();
        let local_cwd = if prof.is_local() { pane.initial_cwd.clone() } else { None };
        let shown: Vec<String> =
            argv.iter().map(|a| if a.len() > 120 { format!("{}…({} bytes)", &a[..60], a.len()) } else { a.clone() }).collect();
        self.data.log(&format!("pane {} [{}] spawn {:?}", pane.id, pane.host, shown));
        pane.term = Term::new(pane.term.cols, pane.term.rows);
        pane.parser = vte::Parser::new();
        pane.widgets.clear();
        pane.hist_seen = 0;
        match Pty::spawn(&argv, local_cwd.as_deref(), &env, pane.term.cols as u16, pane.term.rows as u16, self.wake()) {
            Ok(p) => {
                pane.pty = Some(p);
                pane.exited = false;
                pane.spawn_error = None;
            }
            Err(e) => {
                self.data.log(&format!("pane {} spawn failed: {e}", pane.id));
                pane.pty = None;
                pane.exited = true;
                pane.spawn_error = Some(e);
            }
        }
    }

    pub fn active_pane_id(&self) -> Option<usize> {
        self.tabs.get(self.active_tab).map(|t| t.active)
    }

    pub fn active_pane(&self) -> Option<&Pane> {
        self.active_pane_id().and_then(|id| self.panes.get(&id))
    }

    pub fn active_host(&self) -> String {
        self.active_pane().map(|p| p.host.clone()).unwrap_or_else(|| "local".into())
    }

    pub fn new_tab(&mut self, host: &str, cwd: Option<String>) {
        let id = self.spawn_pane(host, cwd);
        self.tabs.push(Tab { root: Node::Leaf(id), active: id, zoomed: false });
        self.active_tab = self.tabs.len() - 1;
        self.focus_term = true;
    }

    pub fn split(&mut self, vertical: bool) {
        let Some(cur) = self.active_pane_id() else { return };
        let (host, cwd) = {
            let p = &self.panes[&cur];
            (p.host.clone(), p.term.cwd.clone().or(p.initial_cwd.clone()))
        };
        let id = self.spawn_pane(&host, cwd);
        let tab = &mut self.tabs[self.active_tab];
        tab.root.split_leaf(cur, id, vertical);
        tab.active = id;
        tab.zoomed = false;
        self.focus_term = true;
    }

    /// Zoom the active pane to the whole tab, or restore the split layout.
    pub fn toggle_zoom(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else { return };
        let mut leaves = Vec::new();
        tab.root.leaves(&mut leaves);
        if leaves.len() < 2 && !tab.zoomed {
            self.toast("zoom needs a split tab (Ctrl+Shift+D / E)", ToastKind::Info);
            return;
        }
        tab.zoomed = !tab.zoomed;
        self.focus_term = true;
    }

    /// Pick a theme; it also becomes the dark/light theme used by "system".
    pub fn pick_theme(&mut self, index: usize) {
        if let Some((_, id, _, light)) = crate::theme::themes().find(|t| t.0 == index) {
            self.settings.theme = id.to_string();
            if light {
                self.settings.theme_light = id.to_string();
            } else {
                self.settings.theme_dark = id.to_string();
            }
        }
    }

    pub fn close_pane(&mut self, id: usize) {
        self.panes.remove(&id);
        for ti in 0..self.tabs.len() {
            let mut leaves = Vec::new();
            self.tabs[ti].root.leaves(&mut leaves);
            if !leaves.contains(&id) {
                continue;
            }
            let root = std::mem::replace(&mut self.tabs[ti].root, Node::Leaf(0));
            match root.remove(id) {
                Some(r) => {
                    let mut rest = Vec::new();
                    r.leaves(&mut rest);
                    self.tabs[ti].root = r;
                    self.tabs[ti].zoomed = false;
                    if self.tabs[ti].active == id {
                        self.tabs[ti].active = rest[0];
                    }
                }
                None => {
                    self.tabs.remove(ti);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len().saturating_sub(1);
                    }
                }
            }
            break;
        }
        if self.tabs.is_empty() {
            self.new_tab("local", None);
        }
        self.focus_term = true;
    }

    pub fn close_tab(&mut self, ti: usize) {
        let mut leaves = Vec::new();
        self.tabs[ti].root.leaves(&mut leaves);
        for id in leaves {
            self.close_pane(id);
        }
    }

    pub fn focus_pane(&mut self, id: usize) {
        for (ti, t) in self.tabs.iter_mut().enumerate() {
            let mut leaves = Vec::new();
            t.root.leaves(&mut leaves);
            if leaves.contains(&id) {
                t.active = id;
                self.active_tab = ti;
                self.on_active_changed();
                return;
            }
        }
    }

    pub fn focus_dir(&mut self, dx: f32, dy: f32) {
        let Some(cur) = self.active_pane_id() else { return };
        let Some(&(_, r)) = self.pane_rects.iter().find(|(id, _)| *id == cur) else { return };
        let c = r.center();
        let best = self
            .pane_rects
            .iter()
            .filter(|(id, _)| *id != cur)
            .filter_map(|&(id, o)| {
                let d = o.center() - c;
                let along = d.x * dx + d.y * dy;
                if along <= 1.0 {
                    return None;
                }
                let across = (d.x * dy).abs() + (d.y * dx).abs();
                Some((id, along + across * 2.0))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = best {
            self.focus_pane(id);
            self.focus_term = true;
        }
    }

    pub fn on_active_changed(&mut self) {
        if self.side.follow {
            if let Some(p) = self.active_pane() {
                let host = p.host.clone();
                if let Some(cwd) = p.term.cwd.clone() {
                    self.set_root(&host, &cwd);
                }
            }
        }
    }

    // ---- requests ---------------------------------------------------------------

    pub fn request(&mut self, host: &str, req: Value, pending: Pending) {
        self.ensure_conn(host);
        let id = self.conns.get_mut(host).unwrap().request(req);
        self.pending.insert((host.to_string(), id), pending);
    }

    pub fn list_dir(&mut self, host: &str, path: &str) {
        let key = (host.to_string(), path.to_string());
        if self.inflight.contains(&key) {
            return;
        }
        self.inflight.insert(key);
        let hidden = self.settings.show_hidden;
        self.request(host, json!({"op": "list", "path": path, "hidden": hidden}), Pending::List { path: path.into() });
    }

    pub fn set_root(&mut self, host: &str, dir: &str) {
        let new = (host.to_string(), dir.to_string());
        if self.side.root.as_ref() != Some(&new) {
            self.side.root = Some(new);
            self.list_dir(host, dir);
        }
    }

    pub fn open_file(&mut self, host: &str, path: &str, line: usize) {
        self.editor.visible = true;
        if let Some(i) = self.editor.files.iter().position(|f| f.host == host && f.path == path) {
            self.editor.active = i;
            self.editor.files[i].goto = Some(line.max(1));
            return;
        }
        self.editor.files.push(EditorFile {
            host: host.into(),
            path: path.into(),
            text: String::new(),
            dirty: false,
            mtime: None,
            binary: false,
            scratch: false,
            loading: true,
            saving: false,
            conflict: false,
            goto: Some(line.max(1)),
            find: String::new(),
            error: None,
        });
        self.editor.active = self.editor.files.len() - 1;
        self.request(host, json!({"op": "read", "path": path}), Pending::Open { path: path.into(), line });
    }

    pub fn open_scratch(&mut self, title: &str, text: String) {
        self.editor.files.push(EditorFile {
            host: "scratch".into(),
            path: title.into(),
            text,
            dirty: false,
            mtime: None,
            binary: false,
            scratch: true,
            loading: false,
            saving: false,
            conflict: false,
            goto: None,
            find: String::new(),
            error: None,
        });
        self.editor.active = self.editor.files.len() - 1;
        self.editor.visible = true;
    }

    pub fn save_file(&mut self, idx: usize, force: bool) {
        let Some(f) = self.editor.files.get_mut(idx) else { return };
        if f.scratch || f.binary || f.loading {
            return;
        }
        f.saving = true;
        f.conflict = false;
        let mut req = json!({"op": "write", "path": f.path, "text": f.text});
        if let Some(m) = f.mtime {
            req["expect_mtime"] = json!(m);
        }
        if force {
            req["force"] = json!(true);
        }
        let (host, path) = (f.host.clone(), f.path.clone());
        self.request(&host, req, Pending::Save { path });
    }

    pub fn reload_file(&mut self, idx: usize) {
        let Some(f) = self.editor.files.get_mut(idx) else { return };
        if f.scratch {
            return;
        }
        f.loading = true;
        f.conflict = false;
        let (host, path) = (f.host.clone(), f.path.clone());
        self.request(&host, json!({"op": "read", "path": path}), Pending::Open { path, line: 0 });
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        let text = text.into();
        self.data.log(&format!("toast: {text}"));
        self.toasts.push(Toast { text, kind, at: Instant::now() });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    pub fn run_in_pane(&mut self, id: usize, cmd: &str) {
        let local = self.profile(&self.panes.get(&id).map(|p| p.host.clone()).unwrap_or_default()).is_local();
        let Some(p) = self.panes.get_mut(&id) else { return };
        if !p.term.at_prompt() {
            self.toast("pane is busy — command not sent", ToastKind::Error);
            return;
        }
        // Clear whatever is on the line, then run.
        let mut bytes = if local { b"\x1b".to_vec() } else { b"\x15".to_vec() };
        bytes.extend_from_slice(cmd.as_bytes());
        bytes.push(b'\r');
        p.send(&bytes);
    }

    pub fn paste_into(&mut self, id: usize, text: &str) {
        let Some(p) = self.panes.get_mut(&id) else { return };
        let text = text.replace("\r\n", "\n").replace('\n', "\r");
        if p.term.bracketed_paste {
            let mut b = b"\x1b[200~".to_vec();
            b.extend_from_slice(text.replace('\x1b', "").as_bytes());
            b.extend_from_slice(b"\x1b[201~");
            p.send(&b);
        } else {
            p.send(text.as_bytes());
        }
    }

    // ---- per-frame polling ------------------------------------------------------

    pub fn poll(&mut self) {
        let mut events: Vec<(usize, TermEvent)> = Vec::new();
        let mut new_history: Vec<(String, String)> = Vec::new();
        for (id, pane) in self.panes.iter_mut() {
            if let Some(pty) = pane.pty.as_mut() {
                let bytes = pty.read();
                if !bytes.is_empty() {
                    let before = pane.term.end_line();
                    pane.term.feed(&mut pane.parser, &bytes);
                    let replies = std::mem::take(&mut pane.term.replies);
                    pty.write(&replies);
                    if pane.scroll > 0 {
                        pane.scroll += (pane.term.end_line() - before) as usize;
                    }
                }
                if !pane.exited && pty.exited() {
                    pane.exited = true;
                }
            }
            for e in std::mem::take(&mut pane.term.events) {
                events.push((*id, e));
            }
            let first = pane.term.first_line();
            pane.widgets.retain(|w| w.anchor >= first);
            for b in &pane.term.blocks {
                if b.id > pane.hist_seen && b.state() != BlockState::Prompt {
                    pane.hist_seen = b.id;
                    if !b.command.is_empty() {
                        new_history.push((pane.host.clone(), b.command.clone()));
                    }
                }
            }
        }
        for (host, cmd) in new_history {
            self.record_history(&host, cmd);
        }
        for (id, e) in events {
            self.handle_term_event(id, e);
        }

        let hosts: Vec<String> = self.conns.keys().cloned().collect();
        for h in hosts {
            let resps = self.conns.get_mut(&h).map(|c| c.poll()).unwrap_or_default();
            for r in resps {
                self.handle_response(&h, r);
            }
            // The agent uses BatchMode ssh; if it failed before the interactive
            // pane finished authenticating (host key prompt, etc.), retry once
            // after that pane shows its first prompt.
            let failed = matches!(self.conns.get(&h).map(|c| &c.status), Some(ConnStatus::Failed(_)));
            if failed && !self.agent_retried.contains(&h) {
                let prompted = self.panes.values().any(|p| p.host == h && !p.term.blocks.is_empty());
                if prompted {
                    self.agent_retried.insert(h.clone());
                    self.reconnect(&h);
                }
            }
            // Keys / ssh-agent were refused (or the password was wrong): ask
            // for a password once instead of leaving the panels dead.
            let denied = matches!(self.conns.get(&h).map(|c| &c.status), Some(ConnStatus::Failed(e)) if e.contains("Permission denied"));
            if denied && self.auth_prompt.is_none() && !self.auth_prompted.contains(&h) {
                self.auth_prompted.insert(h.clone());
                if self.secrets.remove(&h).is_some() {
                    self.toast(format!("{h}: password was rejected"), ToastKind::Error);
                }
                self.auth_prompt = Some((h.clone(), String::new()));
            }
        }

        // Periodic remote system stats for the active host.
        let host = self.active_host();
        let ready = self.conns.get(&host).map(|c| c.status == ConnStatus::Ready && !c.profile.is_local()).unwrap_or(false);
        if ready && self.sys_at.get(&host).map(|t| t.elapsed() > Duration::from_secs(5)).unwrap_or(true) {
            self.sys_at.insert(host.clone(), Instant::now());
            let cwd = self.active_pane().and_then(|p| p.term.cwd.clone()).unwrap_or_else(|| "~".into());
            self.request(&host, json!({"op": "sys", "path": cwd}), Pending::Sys);
        }

        if self.last_save.elapsed() > Duration::from_secs(10) {
            self.save_settings();
        }
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(if t.kind == ToastKind::Error { 8 } else { 4 }));
    }

    fn record_history(&mut self, host: &str, cmd: String) {
        let hist = self.history.entry(host.to_string()).or_default();
        if hist.last() == Some(&cmd) {
            return;
        }
        hist.retain(|h| h != &cmd);
        hist.push(cmd.clone());
        if hist.len() > 5000 {
            hist.remove(0);
        }
        // Rewrite occasionally would be nicer; appending keeps it cheap and greppable.
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.data.path(&format!("history/{}.txt", host::safe_name(host))))
        {
            let _ = writeln!(f, "{}", cmd.replace('\n', " "));
        }
    }

    fn handle_term_event(&mut self, pane_id: usize, ev: TermEvent) {
        let Some(host) = self.panes.get(&pane_id).map(|p| p.host.clone()) else { return };
        match ev {
            TermEvent::Bell => {
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    p.bell_at = Some(Instant::now());
                }
            }
            TermEvent::Clipboard(text) => {
                let n = text.chars().count();
                self.ctx.copy_text(text);
                self.toast(format!("{host}: copied {n} chars to the local clipboard"), ToastKind::Ok);
            }
            TermEvent::Notify(msg) => self.toast(format!("{host}: {msg}"), ToastKind::Info),
            TermEvent::CwdChanged => {
                if Some(pane_id) == self.active_pane_id() && self.side.follow {
                    if let Some(cwd) = self.panes[&pane_id].term.cwd.clone() {
                        self.set_root(&host, &cwd);
                    }
                }
            }
            TermEvent::Weft { anchor, value } => self.handle_weft(pane_id, &host, anchor, value),
        }
    }

    fn handle_weft(&mut self, pane_id: usize, host: &str, anchor: u64, v: Value) {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        match s("t").as_str() {
            "open" => {
                let line = v.get("line").and_then(|l| l.as_u64()).unwrap_or(1) as usize;
                self.open_file(host, &s("path"), line);
            }
            "notify" => self.toast(format!("{host}: {}", s("msg")), ToastKind::Info),
            "widget" => {
                let Some(kind) = parse_widget(&v) else { return };
                let id = s("id");
                let rows = v.get("rows").and_then(|r| r.as_u64()).unwrap_or(1).clamp(1, 2000) as usize;
                let Some(p) = self.panes.get_mut(&pane_id) else { return };
                if let Some(w) = p.widgets.iter_mut().find(|w| w.id == id) {
                    w.kind = kind;
                } else {
                    p.widgets.push(InlineWidget { id, anchor, rows, kind, sort: None, chosen: None });
                    if p.widgets.len() > 300 {
                        p.widgets.remove(0);
                    }
                }
            }
            other => self.data.log(&format!("unknown weft event {other:?}")),
        }
    }

    fn handle_response(&mut self, host: &str, r: Value) {
        let Some(id) = r.get("id").and_then(|v| v.as_u64()) else { return };
        let Some(pending) = self.pending.remove(&(host.to_string(), id)) else { return };
        let ok = r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let err = r.get("error").and_then(|v| v.as_str()).unwrap_or("error").to_string();
        match pending {
            Pending::List { path } => {
                self.inflight.remove(&(host.to_string(), path.clone()));
                if !ok {
                    self.dir_cache.insert((host.into(), path), DirListing { entries: Vec::new(), at: Instant::now(), error: Some(err) });
                    return;
                }
                let mut entries: Vec<DirEntry> = r["entries"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|e| DirEntry {
                                name: e["n"].as_str().unwrap_or("").into(),
                                dir: e["d"].as_bool().unwrap_or(false),
                                link: e["l"].as_bool().unwrap_or(false),
                                size: e["s"].as_u64().unwrap_or(0),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                self.dir_cache.insert((host.into(), path), DirListing { entries, at: Instant::now(), error: None });
            }
            Pending::Open { path, line } => {
                let Some(i) = self.editor.files.iter().position(|f| f.host == host && f.path == path) else { return };
                let f = &mut self.editor.files[i];
                f.loading = false;
                if !ok {
                    if err.contains("IsADirectory") {
                        self.editor.files.remove(i);
                        self.editor.active = self.editor.active.min(self.editor.files.len().saturating_sub(1));
                        self.settings.sidebar = true;
                        self.side.tab = SideTab::Files;
                        self.side.follow = false;
                        self.set_root(host, &path);
                    } else {
                        f.error = Some(err);
                    }
                    return;
                }
                f.mtime = r["mtime"].as_f64();
                f.dirty = false;
                f.error = None;
                if let Some(t) = r["text"].as_str() {
                    f.text = t.to_string();
                    f.binary = false;
                } else {
                    f.binary = true;
                    f.text = format!("binary file, {} bytes — not editable", r["size"].as_u64().unwrap_or(0));
                }
                if line > 0 {
                    f.goto = Some(line);
                }
            }
            Pending::Save { path } => {
                let Some(f) = self.editor.files.iter_mut().find(|f| f.host == host && f.path == path) else { return };
                f.saving = false;
                if !ok {
                    self.toast(format!("save failed: {err}"), ToastKind::Error);
                } else if r["conflict"].as_bool().unwrap_or(false) {
                    f.conflict = true;
                } else {
                    f.dirty = false;
                    f.mtime = r["mtime"].as_f64();
                    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                    self.toast(format!("saved {name} on {host}"), ToastKind::Ok);
                }
            }
            Pending::Complete { pane } => {
                let Some(p) = self.panes.get_mut(&pane) else { return };
                let Some(c) = p.composer.as_mut() else { return };
                c.pending = false;
                if ok {
                    c.cands = r["names"].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(String::from)).collect()).unwrap_or_default();
                    c.cand_idx = usize::MAX;
                    apply_completion(c);
                }
            }
            Pending::Procs => {
                self.side.procs_at = Some(Instant::now());
                self.side.procs = Some(if ok { Ok(r["procs"].as_array().cloned().unwrap_or_default()) } else { Err(err) });
            }
            Pending::Sys => {
                if ok {
                    self.sys.insert(host.into(), r);
                }
            }
            Pending::Grep => {
                self.side.grep_busy = false;
                if ok {
                    self.side.grep = Some(r);
                } else {
                    self.toast(format!("search failed: {err}"), ToastKind::Error);
                }
            }
            Pending::Kill => {
                if ok {
                    self.toast(format!("signal sent to {}", r["pid"]), ToastKind::Ok);
                    self.actions.push(Action::Procs);
                } else {
                    self.toast(format!("kill failed: {err}"), ToastKind::Error);
                }
            }
        }
    }

    /// Start path completion for the composer of `pane_id` (Tab).
    pub fn start_completion(&mut self, pane_id: usize) {
        let Some(p) = self.panes.get(&pane_id) else { return };
        let host = p.host.clone();
        let cwd = p.term.cwd.clone().or(p.initial_cwd.clone());
        let home = self.conns.get(&host).and_then(|c| c.home());
        let Some(c) = self.panes.get_mut(&pane_id).and_then(|p| p.composer.as_mut()) else { return };
        if !c.cands.is_empty() {
            // Cycle through the current candidates.
            c.cand_idx = if c.cand_idx >= c.cands.len() { 0 } else { (c.cand_idx + 1) % c.cands.len() };
            apply_completion(c);
            return;
        }
        let start = c.text.rfind([' ', '\t', '\n', '=', '"', '\'']).map(|i| i + 1).unwrap_or(0);
        let token = c.text[start..].to_string();
        let (dir_part, prefix) = match token.rfind('/') {
            Some(i) => (token[..=i].to_string(), token[i + 1..].to_string()),
            None => (String::new(), token.clone()),
        };
        let resolved = if dir_part.starts_with('/') || (dir_part.len() > 1 && dir_part.as_bytes()[1] == b':') {
            dir_part.clone()
        } else if let Some(rest) = dir_part.strip_prefix("~/") {
            join(&home.clone().unwrap_or_default(), rest)
        } else {
            let base = cwd.or(home).unwrap_or_else(|| ".".into());
            if dir_part.is_empty() {
                base
            } else {
                join(&base, &dir_part)
            }
        };
        c.token_start = start + dir_part.len();
        c.token_dir = resolved.clone();
        c.pending = true;
        let _ = prefix;
        // Prefer the local cache of the remote directory: no round trip at all.
        let key = (host.clone(), resolved.trim_end_matches('/').to_string());
        let key = if key.1.is_empty() { (host.clone(), "/".to_string()) } else { key };
        if let Some(l) = self.dir_cache.get(&key).filter(|l| l.at.elapsed() < Duration::from_secs(15)) {
            let names: Vec<String> = l.entries.iter().map(|e| if e.dir { format!("{}/", e.name) } else { e.name.clone() }).collect();
            let c = self.panes.get_mut(&pane_id).and_then(|p| p.composer.as_mut()).unwrap();
            let pre = c.text[c.token_start..].to_string();
            c.cands = names.into_iter().filter(|n| n.starts_with(&pre)).collect();
            c.cand_idx = usize::MAX;
            c.pending = false;
            apply_completion(c);
            return;
        }
        let pre = {
            let c = self.panes.get_mut(&pane_id).and_then(|p| p.composer.as_mut()).unwrap();
            c.text[c.token_start..].to_string()
        };
        self.request(&host, json!({"op": "complete", "dir": key.1, "prefix": pre}), Pending::Complete { pane: pane_id });
    }

    pub fn apply_actions(&mut self) {
        let actions = std::mem::take(&mut self.actions);
        for a in actions {
            match a {
                Action::Send(id, b) => {
                    if let Some(p) = self.panes.get_mut(&id) {
                        p.send(&b);
                    }
                }
                Action::Run(id, cmd) => self.run_in_pane(id, &cmd),
                Action::Paste(id, t) => self.paste_into(id, &t),
                Action::Copy(t) => {
                    let n = t.chars().count();
                    self.ctx.copy_text(t);
                    self.toast(format!("copied {n} chars"), ToastKind::Ok);
                }
                Action::OpenUrl(u) => self.ctx.open_url(egui::OpenUrl::new_tab(u)),
                Action::OpenFile(h, p, l) => self.open_file(&h, &p, l),
                Action::OpenScratch(t, s) => self.open_scratch(&t, s),
                Action::ListDir(h, p) => self.list_dir(&h, &p),
                Action::SetRoot(h, p) => {
                    self.side.follow = false;
                    self.settings.sidebar = true;
                    self.side.tab = SideTab::Files;
                    self.set_root(&h, &p);
                }
                Action::Focus(id) => {
                    self.focus_pane(id);
                    self.focus_term = true;
                }
                Action::ToggleFold(id, bid) => {
                    if let Some(b) = self.panes.get_mut(&id).and_then(|p| p.term.blocks.iter_mut().find(|b| b.id == bid)) {
                        b.folded = !b.folded;
                    }
                }
                Action::ComposerOpen(id, text) => {
                    if let Some(p) = self.panes.get_mut(&id) {
                        p.composer = Some(Composer::new(text));
                    }
                }
                Action::Complete(id) => self.start_completion(id),
                Action::NewTab(h, cwd) => self.new_tab(&h, cwd),
                Action::Split(v) => self.split(v),
                Action::ToggleZoom => self.toggle_zoom(),
                Action::ClosePane(id) => self.close_pane(id),
                Action::Restart(id) => {
                    if let Some(mut p) = self.panes.remove(&id) {
                        p.initial_cwd = p.term.cwd.clone().or(p.initial_cwd.clone());
                        self.start_pty(&mut p);
                        self.panes.insert(id, p);
                    }
                }
                Action::Grep => {
                    let Some((host, root)) = self.side.root.clone() else { continue };
                    if self.side.grep_q.is_empty() {
                        continue;
                    }
                    self.side.grep_busy = true;
                    self.side.grep = None;
                    let req = json!({"op": "grep", "pattern": self.side.grep_q, "path": root,
                                     "icase": self.side.grep_icase, "regex": self.side.grep_regex});
                    self.request(&host, req, Pending::Grep);
                }
                Action::Procs => {
                    let host = self.active_host();
                    self.side.procs_host = host.clone();
                    self.side.procs_at = Some(Instant::now());
                    self.request(&host, json!({"op": "procs"}), Pending::Procs);
                }
                Action::Kill(pid, sig) => {
                    let host = self.side.procs_host.clone();
                    self.request(&host, json!({"op": "kill", "pid": pid, "sig": sig}), Pending::Kill);
                }
                Action::Save(i) => self.save_file(i, false),
                Action::Reconnect(h) => self.reconnect(&h),
            }
        }
    }
}

/// Replace the token being completed with the current candidate (or the
/// longest common prefix when the candidates were just fetched).
pub fn apply_completion(c: &mut Composer) {
    if c.cands.is_empty() {
        return;
    }
    let pick = if c.cands.len() == 1 {
        c.cands[0].clone()
    } else if c.cand_idx >= c.cands.len() {
        // First Tab: extend to the longest common prefix, show the candidates.
        let first = &c.cands[0];
        let mut n = first.len();
        for o in &c.cands[1..] {
            n = n.min(first.chars().zip(o.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum());
        }
        first[..n].to_string()
    } else {
        c.cands[c.cand_idx].clone()
    };
    let start = c.token_start.min(c.text.len());
    c.text.truncate(start);
    c.text.push_str(&pick);
    if c.cands.len() == 1 && !pick.ends_with('/') {
        c.text.push(' ');
        c.cands.clear();
    } else if c.cands.len() == 1 {
        // Descend into the directory on the next Tab.
        c.cands.clear();
    }
}

fn parse_widget(v: &Value) -> Option<WidgetKind> {
    let strs = |a: &Value| -> Vec<String> {
        a.as_array().map(|a| a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect()).unwrap_or_default()
    };
    match v.get("kind")?.as_str()? {
        "files" => Some(WidgetKind::Files {
            dir: v["dir"].as_str().unwrap_or("").into(),
            entries: v["entries"]
                .as_array()?
                .iter()
                .map(|e| WEntry {
                    name: e["name"].as_str().unwrap_or("").into(),
                    path: e["path"].as_str().unwrap_or("").into(),
                    dir: e["dir"].as_bool().unwrap_or(false),
                    size: e["size"].as_u64().unwrap_or(0),
                    mtime: e["mtime"].as_i64().unwrap_or(0),
                    mode: e["mode"].as_str().unwrap_or("").into(),
                })
                .collect(),
        }),
        "table" => Some(WidgetKind::Table {
            columns: strs(&v["columns"]),
            data: v["data"].as_array()?.iter().map(strs).collect(),
        }),
        "choose" => Some(WidgetKind::Choose { options: strs(&v["options"]) }),
        "progress" => Some(WidgetKind::Progress {
            pct: v["pct"].as_f64().unwrap_or(0.0).clamp(0.0, 100.0) as f32,
            label: v["label"].as_str().unwrap_or("").into(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("/", "b"), "/b");
        assert_eq!(parent("/a/b"), "/a");
        assert_eq!(parent("/a"), "/");
        assert_eq!(parent("C:/Users"), "C:/");
    }

    #[test]
    fn completion_lcp_then_cycle() {
        let mut c = Composer::new("cat sr".into());
        c.token_start = 4;
        c.cands = vec!["src/".into(), "srv.txt".into()];
        apply_completion(&mut c);
        assert_eq!(c.text, "cat sr");
        c.cand_idx = 1;
        apply_completion(&mut c);
        assert_eq!(c.text, "cat srv.txt");
        c.cand_idx = 0;
        apply_completion(&mut c);
        assert_eq!(c.text, "cat src/");
        let mut c = Composer::new("vim ma".into());
        c.token_start = 4;
        c.cands = vec!["main.rs".into()];
        apply_completion(&mut c);
        assert_eq!(c.text, "vim main.rs ");
    }

    #[test]
    fn layout_split_and_remove() {
        let mut n = Node::Leaf(1);
        assert!(n.split_leaf(1, 2, true));
        assert!(n.split_leaf(2, 3, false));
        let mut v = Vec::new();
        n.leaves(&mut v);
        assert_eq!(v, vec![1, 2, 3]);
        let n = n.remove(2).unwrap();
        let mut v = Vec::new();
        n.leaves(&mut v);
        assert_eq!(v, vec![1, 3]);
    }
}
