//! Application state and logic (no drawing here; see ui_term.rs / ui_panels.rs).

use crate::host::{self, RemoteShell, Conn, ConnStatus, DataDir, HostKind, Profile};
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
fn d_one() -> f32 {
    1.0
}
fn d_true() -> bool {
    true
}
fn d_font_file() -> String {
    crate::theme::DEFAULT_FONT.into()
}

fn d_icon_font() -> String {
    crate::theme::DEFAULT_ICON_FONT.into()
}

fn d_palette_mode() -> crate::editfind::MatchMode {
    crate::editfind::MatchMode::Fuzzy
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

/// How the terminal cursor is drawn (settings "cursor_style").
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    /// A thin bar at the left edge of the cursor cell.
    Bar,
    /// The whole cursor cell (the cell after the text), the character in it
    /// (e.g. PSReadLine's grey prediction) drawn inverted so it stays readable.
    #[default]
    Block,
    /// A line under the cell.
    Underline,
}

/// Window edge a panel (side panel, editor) is docked to.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dock {
    Left,
    Right,
    Top,
    Bottom,
}

impl Dock {
    pub const ALL: [Dock; 4] = [Dock::Left, Dock::Right, Dock::Top, Dock::Bottom];

    pub fn label(self) -> &'static str {
        match self {
            Dock::Left => "left",
            Dock::Right => "right",
            Dock::Top => "top",
            Dock::Bottom => "bottom",
        }
    }
}

/// A kind of panel that can be dragged to another window edge.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Region {
    Side,
    Editor,
    Terms,
}

impl Region {
    pub fn name(self) -> &'static str {
        match self {
            Region::Side => "Side panel",
            Region::Editor => "Editor",
            Region::Terms => "Terminal",
        }
    }
}

/// One panel of the window. Side panels and editors can be split into
/// several (each its own panel in the layout, number `n`; 0 is the first);
/// there is one terminal panel, its panes split inside each tab.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PanelId {
    pub region: Region,
    pub n: u32,
}

impl PanelId {
    pub const TERMS: PanelId = PanelId { region: Region::Terms, n: 0 };

    pub fn first(region: Region) -> PanelId {
        PanelId { region, n: 0 }
    }

    pub fn name(self) -> String {
        if self.n == 0 || self.region == Region::Terms {
            self.region.name().to_string()
        } else {
            format!("{} {}", self.region.name(), self.n + 1)
        }
    }
}

/// The window's panels (side panels, editors, terminals) as a split tree;
/// every panel can be dragged to any side of another one or of the window.
/// Hidden panels stay in the tree (they come back where they were) and are
/// pruned when laid out. Split ids keep a divider's ratio addressable in the
/// pruned tree.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "t")]
pub enum Layout {
    /// `n`: which side panel or editor (settings from before there could be
    /// several have none: the first).
    Panel {
        region: Region,
        #[serde(default)]
        n: u32,
    },
    Split { id: u32, vertical: bool, ratio: f32, a: Box<Layout>, b: Box<Layout> },
}

impl Default for Layout {
    /// Files | terminals | editor.
    fn default() -> Layout {
        let panel = |region| Box::new(Layout::panel(PanelId::first(region)));
        Layout::Split {
            id: 1,
            vertical: true,
            ratio: 0.2,
            a: panel(Region::Side),
            b: Box::new(Layout::Split { id: 2, vertical: true, ratio: 0.58, a: panel(Region::Terms), b: panel(Region::Editor) }),
        }
    }
}

impl Layout {
    pub fn panel(p: PanelId) -> Layout {
        Layout::Panel { region: p.region, n: p.n }
    }

    pub fn contains(&self, p: PanelId) -> bool {
        match self {
            Layout::Panel { region, n } => *region == p.region && *n == p.n,
            Layout::Split { a, b, .. } => a.contains(p) || b.contains(p),
        }
    }

    /// Every panel in the tree, in order.
    pub fn panels(&self, out: &mut Vec<PanelId>) {
        match self {
            Layout::Panel { region, n } => out.push(PanelId { region: *region, n: *n }),
            Layout::Split { a, b, .. } => {
                a.panels(out);
                b.panels(out);
            }
        }
    }

    /// A number no panel of `region` has yet.
    pub fn free_n(&self, region: Region) -> u32 {
        let mut all = Vec::new();
        self.panels(&mut all);
        (0..).find(|n| !all.contains(&PanelId { region, n: *n })).unwrap_or(0)
    }

    fn max_id(&self) -> u32 {
        match self {
            Layout::Panel { .. } => 0,
            Layout::Split { id, a, b, .. } => (*id).max(a.max_id()).max(b.max_id()),
        }
    }

    /// The tree without the panels `shown` rejects.
    pub fn pruned(&self, shown: &dyn Fn(PanelId) -> bool) -> Option<Layout> {
        match self {
            Layout::Panel { region, n } => shown(PanelId { region: *region, n: *n }).then(|| self.clone()),
            Layout::Split { id, vertical, ratio, a, b } => match (a.pruned(shown), b.pruned(shown)) {
                (Some(a), Some(b)) => Some(Layout::Split { id: *id, vertical: *vertical, ratio: *ratio, a: Box::new(a), b: Box::new(b) }),
                (x, None) | (None, x) => x,
            },
        }
    }

    pub fn remove(self, p: PanelId) -> Option<Layout> {
        match self {
            Layout::Panel { region, n } if region == p.region && n == p.n => None,
            Layout::Panel { .. } => Some(self),
            Layout::Split { id, vertical, ratio, a, b } => match (a.remove(p), b.remove(p)) {
                (Some(a), Some(b)) => Some(Layout::Split { id, vertical, ratio, a: Box::new(a), b: Box::new(b) }),
                (x, None) | (None, x) => x,
            },
        }
    }

    /// `new` beside `target` on its `side`, taking `share` of the space.
    fn insert_beside(&mut self, target: PanelId, new: PanelId, side: Dock, share: f32, id: u32) -> bool {
        match self {
            Layout::Panel { region, n } if *region == target.region && *n == target.n => {
                let old = std::mem::replace(self, Layout::panel(new));
                *self = Layout::wrap(old, new, side, share, id);
                true
            }
            Layout::Panel { .. } => false,
            Layout::Split { a, b, .. } => a.insert_beside(target, new, side, share, id) || b.insert_beside(target, new, side, share, id),
        }
    }

    /// `inner` with `new` on its `side`, taking `share` of the space.
    fn wrap(inner: Layout, new: PanelId, side: Dock, share: f32, id: u32) -> Layout {
        let panel = Box::new(Layout::panel(new));
        let inner = Box::new(inner);
        let vertical = matches!(side, Dock::Left | Dock::Right);
        match side {
            Dock::Left | Dock::Top => Layout::Split { id, vertical, ratio: share, a: panel, b: inner },
            Dock::Right | Dock::Bottom => Layout::Split { id, vertical, ratio: 1.0 - share, a: inner, b: panel },
        }
    }

    /// Move panel `p` to `side` of panel `target`, or of the whole window (None).
    pub fn moved(self, p: PanelId, target: Option<PanelId>, side: Dock) -> Layout {
        if target == Some(p) || !self.contains(p) {
            return self;
        }
        let id = self.max_id() + 1;
        let Some(mut rest) = self.clone().remove(p) else { return self };
        match target {
            Some(t) if rest.insert_beside(t, p, side, 0.5, id) => rest,
            _ => {
                let share = match p.region {
                    Region::Side => 0.2,
                    Region::Editor => 0.42,
                    Region::Terms => 0.6,
                };
                Layout::wrap(rest, p, side, share, id)
            }
        }
    }

    /// A new panel `new` beside `target`, half of its space.
    pub fn split(mut self, target: PanelId, new: PanelId, side: Dock) -> Layout {
        let id = self.max_id() + 1;
        if self.contains(new) || !self.insert_beside(target, new, side, 0.5, id) {
            return self;
        }
        self
    }

    pub fn ratio_mut(&mut self, split: u32) -> Option<&mut f32> {
        match self {
            Layout::Panel { .. } => None,
            Layout::Split { id, ratio, a, b, .. } => {
                if *id == split {
                    Some(ratio)
                } else {
                    a.ratio_mut(split).or_else(|| b.ratio_mut(split))
                }
            }
        }
    }

    /// Panel rects and dividers (split id, divider rect, vertical, parent rect).
    pub fn rects(&self, rect: Rect, panels: &mut Vec<(PanelId, Rect)>, dividers: &mut Vec<(u32, Rect, bool, Rect)>) {
        match self {
            Layout::Panel { region, n } => panels.push((PanelId { region: *region, n: *n }, rect)),
            Layout::Split { id, vertical, ratio, a, b } => {
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
                dividers.push((*id, handle, *vertical, rect));
                a.rects(ra, panels, dividers);
                b.rects(rb, panels, dividers);
            }
        }
    }
}

#[cfg(test)]
mod node_tests {
    use super::*;

    fn order(n: &Node) -> Vec<(usize, [i32; 4])> {
        let mut panes = Vec::new();
        n.layout(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 1000.0)), &mut Vec::new(), &mut panes, &mut Vec::new());
        panes.into_iter().map(|(id, r)| (id, [r.left() as i32, r.top() as i32, r.right() as i32, r.bottom() as i32])).collect()
    }

    fn three() -> Node {
        // 1 | (2 over 3)
        Node::Split { vertical: true, ratio: 0.5, a: Box::new(Node::Leaf(1)), b: Box::new(Node::Split { vertical: false, ratio: 0.5, a: Box::new(Node::Leaf(2)), b: Box::new(Node::Leaf(3)) }) }
    }

    #[test]
    fn a_pane_dropped_on_a_side_of_another() {
        // 3 onto the left of 1: the right column is only 2 now.
        let o = order(&three().moved(3, 1, Some(Dock::Left)));
        let at = |id| o.iter().find(|p| p.0 == id).unwrap().1;
        assert!(at(3)[0] < at(1)[0] && at(3)[1] == at(1)[1], "3 left of 1: {o:?}");
        assert_eq!((at(2)[1], at(2)[3]), (0, 1000), "2 has the whole right column: {o:?}");
    }

    #[test]
    fn dropped_in_the_middle_they_swap() {
        let o = order(&three().moved(1, 3, None));
        assert_eq!(o.iter().map(|p| p.0).collect::<Vec<_>>(), [3, 2, 1]);
        // On itself: nothing.
        assert_eq!(order(&three().moved(2, 2, Some(Dock::Top))), order(&three()));
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use Dock::*;

    const SIDE: PanelId = PanelId { region: Region::Side, n: 0 };
    const EDITOR: PanelId = PanelId { region: Region::Editor, n: 0 };
    const TERMS: PanelId = PanelId::TERMS;

    fn order(l: &Layout) -> Vec<(PanelId, [i32; 4])> {
        let mut panels = Vec::new();
        l.rects(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 1000.0)), &mut panels, &mut Vec::new());
        panels.into_iter().map(|(r, x)| (r, [x.left() as i32, x.top() as i32, x.right() as i32, x.bottom() as i32])).collect()
    }

    #[test]
    fn terminal_moves_like_any_panel() {
        // Default: side | terms | editor. Terminal to the right window edge.
        let l = Layout::default().moved(TERMS, None, Right);
        let o = order(&l);
        assert_eq!(o.last().unwrap().0, TERMS, "{o:?}");
        assert!(o.last().unwrap().1[0] > 300, "terminal on the right: {o:?}");
    }

    #[test]
    fn junction_where_three_panels_meet() {
        // EDITOR under the side panel: the column's horizontal border meets
        // the vertical one beside the terminal.
        let l = Layout::default().moved(EDITOR, Some(SIDE), Bottom);
        let mut dividers = Vec::new();
        l.rects(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 1000.0)), &mut Vec::new(), &mut dividers);
        let j = super::panel_junctions(&dividers);
        assert_eq!(j.len(), 1, "{dividers:?}");
        let (vs, hs, at) = &j[0];
        assert_eq!((vs.len(), hs.len()), (1, 1));
        assert_eq!((at.x, at.y), (dividers[vs[0]].1.center().x, dividers[hs[0]].1.center().y));
        // Four panels in a +: both columns split at the same height. One
        // junction holding the vertical border and both horizontal ones.
        let plus = Layout::Split {
            id: 1,
            vertical: true,
            ratio: 0.5,
            a: Box::new(Layout::Split { id: 2, vertical: false, ratio: 0.5, a: Box::new(Layout::panel(SIDE)), b: Box::new(Layout::panel(EDITOR)) }),
            b: Box::new(Layout::Split { id: 3, vertical: false, ratio: 0.5, a: Box::new(Layout::panel(TERMS)), b: Box::new(Layout::panel(SIDE)) }),
        };
        let mut dividers = Vec::new();
        plus.rects(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 1000.0)), &mut Vec::new(), &mut dividers);
        let j = super::panel_junctions(&dividers);
        assert_eq!(j.len(), 1, "{j:?}");
        assert_eq!((j[0].0.len(), j[0].1.len()), (1, 2), "one vertical, two horizontal");
        // SIDE by side only (the default): borders never meet.
        let mut dividers = Vec::new();
        Layout::default().rects(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 1000.0)), &mut Vec::new(), &mut dividers);
        assert!(super::panel_junctions(&dividers).is_empty());
    }

    #[test]
    fn stacking_inside_a_column() {
        // EDITOR dropped on the bottom half of the side panel: they share the left column.
        let l = Layout::default().moved(EDITOR, Some(SIDE), Bottom);
        let o = order(&l);
        let side = o.iter().find(|p| p.0 == SIDE).unwrap().1;
        let ed = o.iter().find(|p| p.0 == EDITOR).unwrap().1;
        assert_eq!(side[0], ed[0], "same column");
        assert!(ed[1] > side[1], "editor below");
        // And the terminal can go into that column too, top.
        let l = l.moved(TERMS, Some(SIDE), Top);
        assert!(order(&l).iter().all(|p| p.1[0] == 0), "all three stacked: {:?}", order(&l));
    }

    #[test]
    fn hidden_panels_keep_their_place() {
        let l = Layout::default();
        let shown = l.pruned(&|r| r != EDITOR).unwrap();
        assert!(!shown.contains(EDITOR) && shown.contains(TERMS));
        // The stored tree still has the editor where it was.
        assert_eq!(order(&l).last().unwrap().0, EDITOR);
        // A divider of the pruned tree is the stored one.
        let Layout::Split { id, .. } = shown else { panic!() };
        let mut l = l;
        *l.ratio_mut(id).unwrap() = 0.3;
        assert_eq!(l.pruned(&|_| true).map(|x| matches!(x, Layout::Split { ratio, .. } if ratio == 0.3)), Some(true));
    }

    #[test]
    fn settings_from_before_split_panels_still_load() {
        // A layout saved before panels had numbers: each is the first of its kind.
        let old = r#"{"t":"Split","id":1,"vertical":true,"ratio":0.2,"a":{"t":"Panel","region":"Side"},"b":{"t":"Panel","region":"Terms"}}"#;
        let l: Layout = serde_json::from_str(old).unwrap();
        assert!(l.contains(SIDE) && l.contains(TERMS));
    }

    #[test]
    fn a_second_editor_beside_the_first() {
        let l = Layout::default();
        let n = l.free_n(Region::Editor);
        assert_eq!(n, 1);
        let second = PanelId { region: Region::Editor, n };
        let l = l.split(EDITOR, second, Bottom);
        let o = order(&l);
        let (a, b) = (o.iter().find(|p| p.0 == EDITOR).unwrap().1, o.iter().find(|p| p.0 == second).unwrap().1);
        assert_eq!((a[0], a[2]), (b[0], b[2]), "same column");
        assert!(b[1] > a[1], "below");
        assert_eq!(l.free_n(Region::Editor), 2);
        // It moves like any panel, and goes away on its own.
        let l = l.moved(second, Some(SIDE), Top);
        assert!(l.contains(second));
        let l = l.remove(second).unwrap();
        assert!(!l.contains(second) && l.contains(EDITOR));
        // Splitting off a panel already there does nothing.
        assert_eq!(Layout::default().split(EDITOR, SIDE, Left), Layout::default());
    }

    #[test]
    fn dropping_on_itself_changes_nothing() {
        assert_eq!(Layout::default().moved(SIDE, Some(SIDE), Left), Layout::default());
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    #[serde(default = "d_font")]
    pub font_size: f32,
    /// Scale of the whole window (icons, tabs, panels and text), 1.0 = 100%.
    #[serde(default = "d_one")]
    pub ui_scale: f32,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default = "d_true")]
    pub sidebar: bool,
    /// Where the panels are (drag a panel's grip to move it).
    #[serde(default)]
    pub layout: Layout,
    #[serde(default)]
    pub show_hidden: bool,
    /// Files shows what git ignores (.gitignore) too.
    #[serde(default = "d_true")]
    pub show_ignored: bool,
    /// Who draws mermaid diagrams: built-in, mermaid-cli on PATH, or at a path.
    #[serde(default)]
    pub mermaid: crate::mermaid::Renderer,
    /// Shell for local panes: auto (pwsh, else Windows PowerShell), pwsh,
    /// powershell, cmd or nu.
    #[serde(default)]
    pub local_shell: host::LocalShell,
    /// Terminal cursor: block (default), bar or underline.
    #[serde(default)]
    pub cursor_style: CursorStyle,
    /// File panel filter: fuzzy unless changed to text, glob or regex.
    #[serde(default = "d_palette_mode")]
    pub find_mode: crate::find::MatchMode,
    /// How the command palette reads what is typed: fuzzy (letters in
    /// order) unless changed to text, glob or regex.
    #[serde(default = "d_palette_mode")]
    pub palette_mode: crate::editfind::MatchMode,
    /// The file panel follows the active terminal's directory (OSC 7).
    #[serde(default = "d_true")]
    pub follow_cwd: bool,
    /// A theme id (see theme.rs), or "system" to follow the OS light/dark setting.
    #[serde(default = "d_theme")]
    pub theme: String,
    /// Themes used for "system": the last dark and light theme picked.
    #[serde(default = "d_theme_dark")]
    pub theme_dark: String,
    #[serde(default = "d_theme_light")]
    pub theme_light: String,
    /// Markdown: every newline is a line break (GitHub PR descriptions and
    /// comments). Off (default): a single newline inside a paragraph is a
    /// space, as in CommonMark and rendered .md files.
    #[serde(default)]
    pub markdown_breaks: bool,
    /// Terminal and editor font: "builtin:…" (see theme::BUILTIN_FONTS) or a
    /// .ttf/.otf/.ttc path.
    #[serde(default = "d_font_file")]
    pub term_font: String,
    /// Where icons the terminal font lacks come from (a Nerd Font), same form.
    #[serde(default = "d_icon_font")]
    pub icon_font: String,
    /// What is written to disk (commands, history, logs) and for how long.
    #[serde(default)]
    pub records: crate::records::Records,
    /// How ssh passwords are held in memory (never on disk).
    #[serde(default)]
    pub passwords: crate::secret::VaultSettings,
    /// Keyboard shortcuts that differ from the defaults: command id -> keys
    /// ("Ctrl+Shift+T"); an empty list leaves the command without a key.
    #[serde(default)]
    pub keys: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub session: Vec<SavedNode>,
    /// The tab each side panel shows, by panel number (the first one's is
    /// what the side panel showed before there could be several).
    #[serde(default)]
    pub side_tabs: std::collections::BTreeMap<u32, SideTab>,
    /// What was open besides the terminals when weft closed, to open again.
    #[serde(default)]
    pub open: SavedOpen,
    /// Ask before closing an editor file with unsaved changes (also when
    /// closing or restarting weft).
    #[serde(default = "d_true")]
    pub confirm_unsaved: bool,
    /// Ask before closing a terminal where a command is still running.
    #[serde(default = "d_true")]
    pub confirm_running: bool,
    /// Windows: the console host terminals run on. Empty: a conpty.dll
    /// beside weft.exe if there is one, else Windows' own. Or the path of a
    /// conpty.dll (or of its folder; OpenConsole.exe beside it). Read when
    /// weft starts.
    #[serde(default)]
    pub console_host: String,
    /// The graphics backend, read as weft starts (main::renderer_choice):
    /// "auto" (or none: the steadiest for the system), "wgpu" or "opengl".
    #[serde(default)]
    pub renderer: String,
    /// How each search was last set, so it is set so when it opens again
    /// (and after a restart): the Search panel, the editor's find bar,
    /// find in a terminal's scrollback.
    #[serde(default)]
    pub search: SearchModes,
    /// Pythons tried on remote hosts, in order (host::DEFAULT_PYTHONS).
    #[serde(default = "d_pythons")]
    pub remote_python: Vec<String>,
    /// The shell to start on a remote host instead of its user's login
    /// shell: host name -> a name on its PATH, or a path ("Shells…").
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub remote_shells: std::collections::BTreeMap<String, String>,
    /// The local shell on macOS and Linux instead of the login shell: a
    /// name or path (empty: the login shell). Windows: "local_shell".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unix_shell: String,
    /// Show command blocks guessed from the output where a shell has no
    /// integration. Off: a guess can be wrong, so none is shown or kept.
    #[serde(default)]
    pub guess_blocks: bool,
    /// The help's language: "ko", "en", or "" for the system's.
    #[serde(default)]
    pub help_language: String,
    /// As weft starts, the windows left open when weft last closed (all at
    /// once: shut down, crashed): "all" of them, the "last" one, or "none".
    #[serde(default = "d_restore")]
    pub restore_windows: String,
}

fn d_restore() -> String {
    "all".into()
}

fn d_pythons() -> Vec<String> {
    crate::host::DEFAULT_PYTHONS.iter().map(|s| s.to_string()).collect()
}

/// See `Settings::search`.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct SearchModes {
    pub files_mode: crate::editfind::MatchMode,
    pub files_icase: bool,
    pub files_hidden: bool,
    pub editor_mode: crate::editfind::MatchMode,
    pub editor_case: bool,
    pub terminal_icase: bool,
    /// The serial log viewer: raw (else plain text), which way, times, and
    /// its search.
    pub serial_raw: bool,
    pub serial_show: weft_core::serial_log::Show,
    pub serial_times: bool,
    pub serial_mode: crate::editfind::MatchMode,
    pub serial_case: bool,
}

impl Default for SearchModes {
    fn default() -> Self {
        SearchModes {
            files_mode: Default::default(),
            files_icase: true,
            files_hidden: false,
            editor_mode: Default::default(),
            editor_case: false,
            terminal_icase: true,
            serial_raw: false,
            serial_show: Default::default(),
            serial_times: true,
            serial_mode: Default::default(),
            serial_case: false,
        }
    }
}

/// What was open besides the terminals, kept with the session: the
/// editor's files, the folders opened in Files, each panel's own view.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SavedOpen {
    /// The editor's files (memos too; not scratch buffers), in tab order.
    pub files: Vec<SavedFile>,
    /// Which of them was in front.
    pub active: usize,
    /// The editor was showing.
    pub editor: bool,
    /// What the further editor panels show (panel number: host, path).
    pub editor_shows: std::collections::BTreeMap<u32, (String, String)>,
    /// Folders opened in Files (host, path).
    pub expanded: Vec<(String, String)>,
    /// Each further side panel's folder, and whether it follows the terminal.
    pub side_views: std::collections::BTreeMap<u32, SavedView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SavedFile {
    pub host: String,
    pub path: String,
    /// Markdown shown rendered (else its source).
    pub rich: bool,
    pub read: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SavedView {
    pub root: Option<(String, String)>,
    pub follow: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let mut profiles = vec![Profile::local()];
        if let Some(git_bash) = host::git_bash().filter(|_| cfg!(windows)).map(|p| p.display().to_string().replace('\\', "/")) {
            // A POSIX "remote" that is really local: exercises exactly the code
            // path an ssh host takes (in-memory rc + stdin-shipped agent).
            profiles.push(Profile {
                name: "posix-loopback".into(),
                kind: HostKind::Custom {
                    pty: vec![git_bash.clone(), "-c".into()],
                    agent: vec![git_bash, "-c".into()],
                    shell: Default::default(),
                    plain: false,
                },
            });
        }
        Settings { font_size: d_font(), ui_scale: 1.0, profiles, sidebar: true, layout: Layout::default(), show_hidden: false, show_ignored: true, find_mode: d_palette_mode(), palette_mode: d_palette_mode(), cursor_style: Default::default(), local_shell: Default::default(), mermaid: Default::default(), follow_cwd: true, theme: d_theme(), theme_dark: d_theme_dark(), theme_light: d_theme_light(), markdown_breaks: false, term_font: d_font_file(), icon_font: d_icon_font(), records: Default::default(), passwords: Default::default(), keys: Default::default(), session: Vec::new(), side_tabs: Default::default(), open: Default::default(), confirm_unsaved: true, confirm_running: true, console_host: String::new(), renderer: String::new(), search: Default::default(), remote_python: d_pythons(), remote_shells: Default::default(), unix_shell: String::new(), guess_blocks: false, help_language: String::new(), restore_windows: d_restore() }
    }
}

/// "Shells…": the choices being edited.
#[derive(Clone, Default)]
pub struct ShellsWindow {
    /// The remote host whose shell is chosen (the terminal's that had the
    /// keyboard; none: a local one).
    pub host: Option<String>,
    pub remote: String,
    pub unix: String,
    pub local: host::LocalShell,
    /// This computer's choices as they were when the window opened: only
    /// what is changed here is saved (a local shell set elsewhere while it
    /// was open was put back).
    pub was: Option<(host::LocalShell, String)>,
}

/// What to say of a shell weft has less than everything in (none: all of
/// it is there).
pub fn shell_notice_text(name: &str) -> Option<String> {
    const SEE: &str = "Shells… (command palette) lists what weft has in each shell, and lets you choose another";
    match host::shell_info(name) {
        Some(i) if i.support == host::ShellSupport::Full => None,
        Some(i) => Some(format!("{} has part of weft's shell integration. Not there: {}. {SEE}", i.title, i.missing.join("; "))),
        None => {
            let name = if name.is_empty() { "this shell" } else { name };
            Some(format!("{name} has no shell integration in weft: a terminal as any other. Not there: {}. {SEE}", host::NOT_INTEGRATED.join("; ")))
        }
    }
}

/// What ssh itself says when it could not connect or log in (OpenSSH's
/// words are English on every system); none of it: ssh got in, and what
/// failed ran on the host.
pub fn ssh_failed(said: &str) -> bool {
    const SSH_WORDS: &[&str] = &[
        "ssh: ",
        "Permission denied",
        "Host key verification failed",
        "Connection closed by",
        "Connection reset by",
        "Connection refused",
        "Connection timed out",
        "kex_exchange_identification",
        "Too many authentication failures",
        "Received disconnect from",
        "Unable to negotiate",
        "Bad configuration option",
        "Bad owner or permissions",
        "No route to host",
        "Could not resolve hostname",
    ];
    SSH_WORDS.iter().any(|w| said.contains(w))
}

/// ssh's words for a connection that was lost after it got in.
pub fn connection_lost(said: &str) -> bool {
    ["closed by remote host", "Broken pipe", "Connection reset", "Connection timed out", "Connection aborted", "Timeout, server"].iter().any(|w| said.contains(w))
}

/// A saved tab without the terminals whose host is not kept: a split
/// left with one side is that side; nothing left is no tab.
fn keep_panes(n: SavedNode, kept: &dyn Fn(&str) -> bool) -> Option<SavedNode> {
    match n {
        SavedNode::Pane { ref host, .. } => kept(host).then_some(n),
        SavedNode::Split { vertical, ratio, a, b } => match (keep_panes(*a, kept), keep_panes(*b, kept)) {
            (Some(a), Some(b)) => Some(SavedNode::Split { vertical, ratio, a: Box::new(a), b: Box::new(b) }),
            (one, None) | (None, one) => one,
        },
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

    /// Put pane `src` on `side` of pane `target` (it leaves its own place),
    /// or with no side swap the two.
    pub fn moved(self, src: usize, target: usize, side: Option<Dock>) -> Node {
        if src == target {
            return self;
        }
        let Some(side) = side else {
            let mut n = self;
            n.swap(src, target);
            return n;
        };
        let Some(mut rest) = self.remove(src) else { return Node::Leaf(src) };
        rest.insert_beside(target, src, side);
        rest
    }

    fn swap(&mut self, x: usize, y: usize) {
        match self {
            Node::Leaf(id) if *id == x => *id = y,
            Node::Leaf(id) if *id == y => *id = x,
            Node::Leaf(_) => {}
            Node::Split { a, b, .. } => {
                a.swap(x, y);
                b.swap(x, y);
            }
        }
    }

    fn insert_beside(&mut self, target: usize, new: usize, side: Dock) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                let (old, new) = (Box::new(Node::Leaf(target)), Box::new(Node::Leaf(new)));
                let vertical = matches!(side, Dock::Left | Dock::Right);
                let (a, b) = if matches!(side, Dock::Left | Dock::Top) { (new, old) } else { (old, new) };
                *self = Node::Split { vertical, ratio: 0.5, a, b };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.insert_beside(target, new, side) || b.insert_beside(target, new, side),
        }
    }

    /// Move the border of the innermost split of this kind (`side_by_side`,
    /// or one above the other) that holds pane `target` by `delta` of that
    /// split, within the limits a drag has (0.1 to 0.9). (Whether the pane
    /// is in this tree, whether a border was moved.)
    pub fn nudge(&mut self, target: usize, side_by_side: bool, delta: f32) -> (bool, bool) {
        match self {
            Node::Leaf(id) => (*id == target, false),
            Node::Split { vertical, ratio, a, b } => {
                let inner = match a.nudge(target, side_by_side, delta) {
                    (true, done) => (true, done),
                    _ => b.nudge(target, side_by_side, delta),
                };
                if inner == (true, false) && *vertical == side_by_side {
                    *ratio = (*ratio + delta).clamp(0.1, 0.9);
                    return (true, true);
                }
                inner
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

/// What a terminal selection is made of between its two ends.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SelMode {
    Char,
    /// Double-click: whole words at both ends.
    Word,
    /// Triple-click: whole lines.
    Line,
    /// Alt+drag: the same columns on every row.
    Block,
}

/// `a` is where it started, `b` where the pointer is (any order).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Selection {
    pub a: (u64, usize),
    pub b: (u64, usize),
    pub mode: SelMode,
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
    /// The line the cursor was on when it came (see `widget_line`).
    pub came: u64,
    pub rows: usize,
    pub kind: WidgetKind,
    pub sort: Option<(usize, bool)>,
    pub chosen: Option<usize>,
}

/// The line a new widget goes on, and the line to remember it came at.
/// It is the cursor's line, unless the cursor is still where the widget
/// before it is to be: then the text of that one has not arrived yet, and
/// this one goes below it. (Windows' pseudo console hands a program's
/// events on at once and its text when it next draws: several widgets
/// printed in one go all came with the cursor on the first one's line, and
/// were drawn on top of each other.) Lines are never numbered twice, so a
/// cursor inside the last widget's lines means nothing else.
pub fn widget_line(last: Option<&InlineWidget>, cursor: u64) -> (u64, u64) {
    match last {
        Some(w) if cursor >= w.came && cursor < w.anchor + w.rows as u64 => (w.anchor + w.rows as u64, w.came),
        _ => (cursor, cursor),
    }
}

/// A record of one terminal's traffic, to see how a program draws (the
/// "Record this terminal" command): what it sent, what was typed into it,
/// and where weft drew the cursor, each with the milliseconds since the
/// recording began. One line each:
///
///     +1234.5 out 37 \e[?25l\e[3A> hi\e[?25h
///     +1240.1 in 1 h
///     +1241.0 cursor 12,3          (row on screen, column; or "none")
pub struct OutputTrace {
    pub path: std::path::PathBuf,
    file: std::fs::File,
    start: Instant,
    written: u64,
    last_cursor: String,
}

impl OutputTrace {
    /// No more than this is written; the recording stops itself there.
    pub const LIMIT: u64 = 32 << 20;

    pub fn start(path: std::path::PathBuf) -> std::io::Result<OutputTrace> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::fs::File::create(&path)?;
        Ok(OutputTrace { path, file, start: Instant::now(), written: 0, last_cursor: String::new() })
    }

    fn line(&mut self, what: &str, rest: &str) {
        use std::io::Write;
        let ms = self.start.elapsed().as_secs_f64() * 1000.0;
        let line = format!("+{ms:.1} {what} {rest}\n");
        if self.written < Self::LIMIT && self.file.write_all(line.as_bytes()).is_ok() {
            self.written += line.len() as u64;
        }
    }

    /// Bytes as text that can be read: ESC as \e, other control bytes as
    /// \r, \n, \t or \xNN, a backslash doubled; the rest as UTF-8.
    pub fn visible(bytes: &[u8]) -> String {
        let mut s = String::new();
        for c in String::from_utf8_lossy(bytes).chars() {
            match c {
                '\x1b' => s.push_str("\\e"),
                '\r' => s.push_str("\\r"),
                '\n' => s.push_str("\\n"),
                '\t' => s.push_str("\\t"),
                '\\' => s.push_str("\\\\"),
                c if (c as u32) < 0x20 || c as u32 == 0x7f => s.push_str(&format!("\\x{:02x}", c as u32)),
                c => s.push(c),
            }
        }
        s
    }

    pub fn output(&mut self, bytes: &[u8]) {
        self.line("out", &format!("{} {}", bytes.len(), Self::visible(bytes)));
    }

    pub fn input(&mut self, bytes: &[u8]) {
        self.line("in", &format!("{} {}", bytes.len(), Self::visible(bytes)));
    }

    /// Where the cursor was drawn this frame, written when it changed.
    pub fn cursor(&mut self, at: Option<(usize, usize)>, held: bool) {
        let now = match at {
            Some((r, c)) => format!("{r},{c}{}", if held { " held" } else { "" }),
            None => "none".to_string(),
        };
        if now != self.last_cursor {
            self.line("cursor", &now);
            self.last_cursor = now;
        }
    }

    pub fn full(&self) -> bool {
        self.written >= Self::LIMIT
    }

    pub fn size(&self) -> u64 {
        self.written
    }
}

/// The key weft's shell integrations bind to empty the line in any editing
/// mode: Ctrl+Alt+Shift+F12, as xterm sends it (one no one types).
pub const CLEAR_KEY: &[u8] = b"\x1b[24;8~";

pub struct Pane {
    /// A recording of this terminal's traffic, while one is being made.
    pub trace: Option<OutputTrace>,
    /// A serial console's log (settings "records": serial_log).
    pub serial_log: Option<weft_core::serial_log::Writer>,
    /// A serial console: the last byte shown, for showing a lone LF as
    /// CRLF (as picocom's --imap lfcrlf; the log keeps what came).
    pub serial_last: Option<u8>,
    /// Its port has been open (opened again: what was on the screen stays).
    pub serial_opened: bool,
    /// Synchronized output (DEC 2026) held until its update is whole.
    pub sync: crate::term::SyncOutput,
    /// The remote shell's process (from the bootstrap), for its directory
    /// when the shell has no integration.
    pub shell_pid: Option<u64>,
    /// The remote shell its terminal was started for (what its profile
    /// said then).
    pub started_as: RemoteShell,
    /// The shell chosen for its host that it was started with (none: the
    /// login shell).
    pub started_shell: Option<String>,
    /// The zsh integration was typed in for this shell start.
    pub injected: bool,
    /// weft's POSIX command ran there: it says so first thing (OSC 7777
    /// "sh"), before it unpacks anything.
    pub boot_ran: bool,
    /// The shell this start runs is Windows' PowerShell or cmd, whose line
    /// Esc empties (not nu, nor a POSIX shell: Ctrl-U). Of the shell that
    /// started, not of the settings now.
    pub clear_with_esc: bool,
    /// weft's integration bound its own key to empty the line (CLEAR_KEY):
    /// a run empties it so, in any editing mode (vi's, PSReadLine's Emacs).
    pub clear_key: bool,
    /// What its shell takes a cd and a path in (of the shell that
    /// started: a local cmd or nu on Windows, sh on macOS and Linux).
    pub lang: host::ShellLang,
    /// What to send it, and when: a run's command after the Esc that
    /// empties a Windows shell's line (a console that reads late, or ssh
    /// on the way, could take both in one read, and Esc the command's
    /// first key).
    pub send_at: Option<(Instant, Vec<u8>)>,
    /// When it did, and the lines sent by then: a chosen shell that did not
    /// start ends soon after, with nothing typed into it (however long a
    /// password and a one-time code took before).
    pub booted: Option<(Instant, u32)>,
    /// Ctrl-C was typed in it: one stopped at ssh's password prompt is not
    /// a host that refused weft's command.
    pub interrupted: bool,
    /// When this start of it began, and the lines the user sent it since
    /// (Enter): a host that refused weft's command, or a shell that did not
    /// start, ends at once, after a password at most; a session that was
    /// used (a forced command's tmux, a plain shell) is not one of those.
    pub started_at: Instant,
    pub lines_sent: u32,
    /// What ran weft's command said it does not know `exec`: cmd and
    /// PowerShell name it in quotes, the language's own ("'exec' is not
    /// recognized", "'exec'은(는) …", "Der Befehl "exec" …"; see
    /// host::says_exec_unknown).
    /// A refusal, whenever it comes (a password typed late, a one-time code
    /// after it). (Not the console's win32-input-mode request: this PC's
    /// own console makes it too, for every terminal on Windows.)
    pub exec_refused: bool,
    /// Startup timing logged: first output, first prompt.
    pub seen: (bool, bool),
    /// A prompt that is not a sign-in's question was seen (a shell's, or a
    /// menu's "Select host: "): what ends after it is no refusal.
    pub menu_seen: bool,
    /// Where its text area's top is on screen, as last drawn.
    pub text_top: f32,
    pub id: usize,
    pub host: String,
    pub term: Term,
    pub parser: vte::Parser,
    pub pty: Option<Pty>,
    pub exited: bool,
    /// Ended before its shell started: when to see why (what it said
    /// comes in a moment after the process ends, on Windows' console).
    pub judge_at: Option<Instant>,
    pub spawn_error: Option<String>,
    /// Not opened yet (a serial console coming back with the session: an
    /// opening sets DTR, which resets many boards); Enter opens it.
    pub held: bool,
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
    /// The running command's time when its badge is not on screen (its
    /// row full, or scrolled away): the status bar says it instead.
    pub running_hidden: Option<String>,
    /// Caret position last reported to the IME (kept while composing).
    pub ime_rect: Option<Rect>,
    /// Where the terminal's cursor was drawn in the last frame (None: it
    /// was not), and where and when the program last had it shown: a
    /// cursor the program hides stays for `CURSOR_HOLD`.
    pub cursor_drawn: Option<Rect>,
    /// A new size waiting to settle (splitter drag) before it is applied.
    pub pending_size: Option<((usize, usize), Instant)>,
    /// The first size was applied (the first layout applies immediately).
    pub sized: bool,
    pub hist_seen: u64,
    /// Copy mode (Ctrl+Shift+X): keys move a cursor and select, nothing goes to the shell.
    pub copy: Option<crate::copymode::CopyMode>,
    /// Last block written to the records, and where the running one started.
    pub rec_seen: u64,
    pub rec_cwd: Option<(u64, String)>,
    pub flash_block: Option<(u64, Instant)>,
}

impl Pane {
    fn new(id: usize, host: &str, cwd: Option<String>) -> Pane {
        Pane {
            sync: Default::default(),
            shell_pid: None,
            started_as: RemoteShell::Posix,
            started_shell: None,
            injected: false,
            boot_ran: false,
            started_at: Instant::now(),
            lines_sent: 0,
            booted: None,
            clear_with_esc: false,
            clear_key: false,
            lang: host::ShellLang::Posix,
            send_at: None,
            exec_refused: false,
            interrupted: false,
            id,
            host: host.to_string(),
            term: Term::new(100, 30),
            parser: vte::Parser::new(),
            pty: None,
            exited: false,
            judge_at: None,
            spawn_error: None,
            held: false,
            initial_cwd: cwd,
            scroll: 0,
            sel: None,
            selecting: false,
            widgets: Vec::new(),
            composer: None,
            search: None,
            bell_at: None,
            preedit: String::new(),
            running_hidden: None,
            ime_rect: None,
            cursor_drawn: None,
            trace: None,
            serial_log: None,
            serial_last: None,
            serial_opened: false,
            pending_size: None,
            sized: false,
            hist_seen: 0,
            copy: None,
            rec_seen: 0,
            rec_cwd: None,
            seen: (false, false),
            menu_seen: false,
            text_top: 0.0,
            flash_block: None,
        }
    }

    /// For showing (a folder's name from a Mac as syllables: `shown`).
    pub fn title(&self) -> String {
        let cwd = self.term.cwd.as_deref().or(self.initial_cwd.as_deref()).unwrap_or("");
        let cwd = &*weft_core::shown(cwd);
        let base = cwd.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let base = if base.is_empty() { cwd } else { base };
        if base.is_empty() {
            self.host.clone()
        } else {
            format!("{} · {}", self.host, base)
        }
    }

    /// What it says now may be a Windows shell's refusal of weft's command:
    /// nothing has run there yet, no prompt has come (a refusal comes
    /// before one), and no more lines were sent than a first connection
    /// takes: the host key's yes, a mistyped password, the password, a
    /// one-time code (five, for one more of those). (Later, a quoted exec is a session's
    /// own: `"exec": {` in a bastion's menu or a forced command's tmux.)
    pub fn may_be_refused(&self) -> bool {
        !self.boot_ran && self.shell_pid.is_none() && !self.seen.1 && self.lines_sent <= 5
    }

    /// weft's POSIX command said it ran there.
    fn boot_ran(&mut self) {
        self.boot_ran = true;
        self.booted.get_or_insert((Instant::now(), self.lines_sent));
    }

    pub fn send(&mut self, bytes: &[u8]) {
        // A run's command waiting after its Esc: keys typed meanwhile go
        // after it, at its time (sent at once, they were put before it,
        // "x" and "ls": xls; or took it with them, and Esc with it).
        // Ctrl-C drops it: what was asked for is not wanted.
        if let Some((_, waiting)) = &mut self.send_at {
            if bytes.contains(&3) {
                self.send_at = None;
                self.term.user_runs(None);
            } else {
                waiting.extend_from_slice(bytes);
                return;
            }
        }
        // Enter starts a command for shells that don't mark it themselves;
        // an Enter alone on an empty line, none.
        // In their order: Ctrl-C or Ctrl-U empties the line, what follows it
        // is typed (weft's own run sends Ctrl-U, the command and Enter).
        let after = bytes.iter().rposition(|b| matches!(*b, 3 | 21));
        if after.is_some() {
            self.term.user_cleared();
        }
        if bytes[after.map_or(0, |i| i + 1)..].iter().any(|b| *b != b'\r') {
            self.term.user_typed();
        }
        if bytes.contains(&b'\r') {
            // A guessed question answered with the answer on the screen: a
            // menu's ("Select host: "), not a password's or a code's (asked
            // with echo off), in any language. (ssh's own host key question,
            // in English always, echoes its yes.)
            if !self.menu_seen && self.term.at_prompt_question() && self.term.question_answer_shown() {
                let line = self.term.line(self.term.cursor_abs()).map(|r| r.text()).unwrap_or_default();
                if !line.contains("(yes/no/") && !line.contains("Please type 'yes'") {
                    self.menu_seen = true;
                }
            }
            // The folder the command run at this prompt runs in, now: its
            // end and the next prompt's folder can come before the next
            // frame looks (a local `cd /tmp` was recorded as run in /tmp,
            // in every shell).
            let at = self.term.blocks.last().filter(|b| b.state() == BlockState::Prompt).map(|b| b.id);
            self.term.user_enter();
            if let Some(id) = at {
                if self.rec_cwd.as_ref().is_none_or(|c| c.0 != id) {
                    self.rec_cwd = Some((id, self.term.cwd.clone().or(self.initial_cwd.clone()).unwrap_or_default()));
                }
            }
        }
        if bytes.contains(&3) {
            self.interrupted = true;
        }
        self.lines_sent += bytes.iter().filter(|b| **b == b'\r').count() as u32;
        if let Some(t) = self.trace.as_mut() {
            t.input(bytes);
        }
        // Sent, and so logged, only while it runs (keys typed into an ended
        // serial tab went nowhere, and were logged as sent).
        if let (Some(l), false) = (self.serial_log.as_mut(), self.exited) {
            l.write(weft_core::serial_log::Dir::Tx, bytes);
        }
        if let Some(p) = self.pty.as_mut().filter(|_| !self.exited) {
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

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SideTab {
    Files,
    Blocks,
    History,
    Search,
    Procs,
}

/// The part of the side state each side panel has of its own.
#[derive(Clone, Default)]
pub struct SideView {
    pub root: Option<(String, String)>,
    pub follow: bool,
    pub selected: Option<String>,
    pub picked: Vec<String>,
    pub picked_in: (String, String),
    pub band: Option<(egui::Pos2, bool, Vec<String>)>,
}

pub struct SideState {
    pub tab: SideTab,
    pub root: Option<(String, String)>,
    pub follow: bool,
    pub expanded: HashSet<(String, String)>,
    pub filter: String,
    pub grep_q: String,
    pub grep_icase: bool,
    pub grep_mode: crate::editfind::MatchMode,
    /// Search hidden files and folders too.
    pub grep_hidden: bool,
    /// What the shown results were searched with (mode, query, ignore case), to mark them.
    pub grep_ran: Option<(crate::editfind::MatchMode, String, bool)>,
    /// The search's matcher, made once (its results decided and marked by it).
    pub grep_matcher: Option<std::sync::Arc<crate::find::Matcher>>,
    pub grep_busy: bool,
    pub grep: Option<Value>,
    pub procs: Option<Result<Vec<Value>, String>>,
    pub procs_at: Option<Instant>,
    pub procs_filter: String,
    pub procs_sort: (usize, bool),
    pub procs_host: String,
    /// Path selected in the file tree (single click); opening needs a double-click.
    /// With Shift, the range's other end.
    pub selected: Option<String>,
    /// The paths picked together (Ctrl+click, Shift+click, a box dragged
    /// in the space below the rows): what a drag, Download, Send to,
    /// Copy paths and Delete take. Empty: the selected one alone.
    pub picked: Vec<String>,
    /// The panel they were picked in (host, root): another shows none.
    pub picked_in: (String, String),
    /// A box being dragged to pick rows: where it started (and whether
    /// Ctrl added to what was picked).
    pub band: Option<(egui::Pos2, bool, Vec<String>)>,
}

pub struct EditorFile {
    pub host: String,
    pub path: String,
    pub text: String,
    pub dirty: bool,
    pub mtime: Option<f64>,
    /// Changed or gone on disk since it was opened or saved (asked every
    /// couple of seconds while shown: check_editor_on_disk).
    pub outdated: Option<Outdated>,
    /// When its file on disk was last asked about; which read or save it
    /// was asked after (an answer from before one is not about it).
    pub disk_asked: Option<Instant>,
    pub disk_gen: u64,
    pub binary: bool,
    pub scratch: bool,
    pub loading: bool,
    pub saving: bool,
    /// What is being saved (a hash of the text sent): the answer makes the
    /// file clean only if nothing was typed since.
    pub saving_of: Option<u64>,
    pub conflict: bool,
    pub goto: Option<usize>,
    /// Find bar matches (byte ranges) to paint, and the current one.
    pub marks: std::sync::Arc<Vec<(usize, usize)>>,
    pub mark_cur: Option<(usize, usize)>,
    /// Markdown shown rendered in place (false: plain source).
    pub rich: bool,
    /// Read mode: selectable and copyable, not editable (markdown fully
    /// rendered, its markup never shown at the cursor).
    pub read: bool,
    /// When it was last saved / copied with the Copy button (a moment of "✓").
    pub saved_at: Option<Instant>,
    pub copied_at: Option<Instant>,
    pub error: Option<String>,
    /// Raw bytes of a non-text file (images are shown, not edited).
    pub bytes: Option<std::sync::Arc<Vec<u8>>>,
    pub image: crate::media::ImageState,
    pub pdf: crate::media::PdfState,
}

/// How a file open in the editor differs from its file on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outdated {
    Changed,
    Gone,
}

pub struct EditorState {
    pub files: Vec<EditorFile>,
    pub active: usize,
    pub visible: bool,
    /// The find / replace bar, while open (applies to the active file).
    pub find: Option<crate::editfind::FindBar>,
    /// A char range to select in the active file next (a find result).
    pub select: Option<(usize, usize)>,
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

/// The font picker: which setting it edits and the fonts it offers.
pub struct FontMenu {
    /// Editing the icon font (else the terminal font).
    pub icons: bool,
    pub query: String,
    /// A font file path typed in.
    pub path: String,
    /// (name, path, in data/fonts) for each font file found.
    pub files: Vec<(String, String, bool)>,
}

/// The keyboard shortcuts window: the search, and the command (by id)
/// waiting for a key to be pressed.
#[derive(Default)]
pub struct KeysWindow {
    pub query: String,
    pub capture: Option<&'static str>,
    /// Show only the commands whose keys were changed / that share a key.
    pub changed_only: bool,
    pub clashes_only: bool,
}

/// A point where borders meet: the vertical ones and the horizontal ones
/// there (indices), so one grip moves them all. Three areas around it: one
/// of each; four (a +): two of one kind.
pub type Junction = (Vec<usize>, Vec<usize>, egui::Pos2);

/// Gather touching (vertical, horizontal, point) pairs into junctions: pairs
/// at the same point belong to one.
pub fn group_junctions(pairs: Vec<(usize, usize, egui::Pos2)>) -> Vec<Junction> {
    let mut out: Vec<Junction> = Vec::new();
    for (v, h, at) in pairs {
        match out.iter_mut().find(|j| (j.2.x - at.x).abs() <= 6.0 && (j.2.y - at.y).abs() <= 6.0) {
            Some(j) => {
                if !j.0.contains(&v) {
                    j.0.push(v);
                }
                if !j.1.contains(&h) {
                    j.1.push(h);
                }
            }
            None => out.push((vec![v], vec![h], at)),
        }
    }
    out
}

/// Where panel borders meet (indices into `dividers`).
pub fn panel_junctions(dividers: &[(u32, Rect, bool, Rect)]) -> Vec<Junction> {
    let mut pairs = Vec::new();
    for (vi, v) in dividers.iter().enumerate().filter(|d| d.1 .2) {
        for (hi, h) in dividers.iter().enumerate().filter(|d| !d.1 .2) {
            if v.1.expand(3.0).intersects(h.1.expand(3.0)) {
                pairs.push((vi, hi, egui::pos2(v.1.center().x, h.1.center().y)));
            }
        }
    }
    group_junctions(pairs)
}

/// A memo on disk, for the memo list.
#[derive(Clone)]
pub struct MemoInfo {
    pub path: String,
    pub title: String,
    /// The line after the title.
    pub preview: String,
    /// Last change, unix seconds.
    pub modified: f64,
}

/// What was clicked in the memo list.
pub enum MemoPick {
    Open(String),
    Delete(String),
}

/// A memo's tab name: its first line without markdown marks.
pub fn memo_title(text: &str) -> String {
    const MAX: usize = 24;
    let line = text.lines().map(|l| l.trim_start_matches(|c: char| "#>-*+".contains(c) || c.is_whitespace()).trim_end()).find(|l| !l.is_empty()).unwrap_or("");
    if line.is_empty() {
        return "new memo".into();
    }
    let short: String = line.chars().take(MAX).collect();
    if line.chars().count() > MAX { format!("{short}…") } else { short }
}

/// The memos in `dir` (*.md), newest first.
pub fn read_memos(dir: &str) -> Vec<MemoInfo> {
    let mut list: Vec<MemoInfo> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
                .filter_map(|e| {
                    let path = e.path().display().to_string().replace('\\', "/");
                    let modified = e.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
                    let title = memo_title(lines.next().unwrap_or(""));
                    let preview: String = lines.next().unwrap_or("").chars().take(80).collect();
                    Some(MemoInfo { path, title, preview, modified })
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by(|a, b| b.modified.total_cmp(&a.modified));
    list
}

/// The help window: which tab, and the search.
pub struct HelpState {
    pub features: bool,
    pub query: String,
}

#[derive(Default)]
pub struct PaletteState {
    pub query: String,
    pub sel: usize,
    /// Just opened: the list starts at the top.
    pub fresh: bool,
}

impl PaletteState {
    pub fn new() -> PaletteState {
        PaletteState { fresh: true, ..Default::default() }
    }
}

pub struct ConnectState {
    pub target: String,
    pub split: bool,
    /// Optional ssh password; empty means keys / ssh-agent.
    pub password: String,
}

/// Before weft is gone its terminals are closed for real: a pseudo console
/// still being closed when the process ends leaves its console host
/// running for good (pty::finish_closing).
impl Drop for App {
    fn drop(&mut self) {
        self.panes.clear();
        let left = crate::pty::finish_closing(std::time::Duration::from_secs(3));
        if left > 0 {
            self.data.log(&format!("exit: {left} pseudo console(s) were still closing"));
        }
    }
}

/// Search tab: the lines shown at most, and the lines a remote grep may
/// return for a glob or fuzzy search (weft decides which match).
pub const GREP_SHOWN: usize = 500;
pub const GREP_SCAN: usize = 5000;

/// A text's hash, to tell whether it is still the one that was saved.
pub fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

pub enum Pending {
    /// An editor file's time on disk (is it still the one opened?).
    EditorStat { path: String, generation: u64 },
    List { path: String },
    /// A Files widget's folder (wls), listed for it.
    WidgetFiles { pane: usize, id: String },
    Open { path: String, line: usize },
    Save { path: String },
    Complete { pane: usize },
    Procs,
    Sys,
    Grep,
    Kill,
    Tree { root: String },
    Cwd { pane: usize },
    /// A file transfer's request (see transfer.rs).
    Xfer { t: u64, step: crate::transfer::Step },
    /// The serial ports of a host, for the Serial console dialog.
    SerialPorts { host: String },
    /// A file operation asked from Files (fileops.rs): what to say when it
    /// is done, the folders to list again, and what was renamed to what.
    FileOp { done: String, refresh: Vec<String>, renamed: Option<(String, String)> },
}

/// Which tabs a tab's context menu closes, relative to that tab.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CloseWhich {
    This,
    Others,
    Right,
    Left,
    All,
}

impl CloseWhich {
    pub const MENU: [(CloseWhich, &'static str); 5] =
        [(CloseWhich::This, "Close"), (CloseWhich::Others, "Close others"), (CloseWhich::Right, "Close to the right"), (CloseWhich::Left, "Close to the left"), (CloseWhich::All, "Close all")];
}

/// Indices of the tabs to close among `n` around tab `at`, highest first
/// (so removing them one by one keeps the rest valid).
pub fn tabs_to_close(n: usize, at: usize, which: CloseWhich) -> Vec<usize> {
    (0..n)
        .rev()
        .filter(|&i| match which {
            CloseWhich::This => i == at,
            CloseWhich::Others => i != at,
            CloseWhich::Right => i > at,
            CloseWhich::Left => i < at,
            CloseWhich::All => true,
        })
        .collect()
}

#[cfg(test)]
mod close_tests {
    use super::*;

    #[test]
    fn tab_close_sets() {
        use CloseWhich::*;
        assert_eq!(tabs_to_close(5, 2, This), [2]);
        assert_eq!(tabs_to_close(5, 2, Others), [4, 3, 1, 0]);
        assert_eq!(tabs_to_close(5, 2, Right), [4, 3]);
        assert_eq!(tabs_to_close(5, 2, Left), [1, 0]);
        assert_eq!(tabs_to_close(3, 0, All), [2, 1, 0]);
        assert!(tabs_to_close(3, 0, Left).is_empty());
        assert!(tabs_to_close(3, 2, Right).is_empty());
    }
}

/// The last filter run: its inputs and the hits (entry index, matched chars).
pub struct FindResult {
    pub key: (String, String, String, crate::find::MatchMode, u64),
    pub hits: Vec<(usize, Vec<usize>)>,
    pub total: usize,
    pub error: Option<String>,
}

/// Every name under a folder (agent `tree`), for the file panel filter.
pub struct FindIndex {
    /// (relative path, is dir), sorted.
    pub entries: Vec<(String, bool)>,
    pub truncated: bool,
    pub at: Instant,
    pub generation: u64,
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
    NewMemo,
    Reconnect(String),
    ToggleZoom,
    /// "Serial console…" on this pane's host.
    SerialHere(String),
    /// The serial log viewer, on this serial tab's log.
    SerialLog(String),
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
    /// Ctrl+wheel zoom not yet turned into a size step (touchpads send small steps).
    pub zoom_acc: f32,
    /// Ctrl+Shift+wheel notches not yet turned into a UI scale step.
    pub ui_wheel_acc: f32,
    /// File filter: name lists per (host, root), and the last result.
    pub find_index: HashMap<(String, String), FindIndex>,
    pub find_inflight: HashSet<(String, String)>,
    pub find_result: Option<FindResult>,
    /// The help window, while open.
    pub help: Option<HelpState>,
    /// Focus mode (terminals only): whether the side panel and the editor
    /// were shown before, to bring them back.
    pub focus_mode: Option<(bool, bool)>,
    /// Memos on disk, read at most once a second.
    pub memo_cache: Option<(Instant, Vec<MemoInfo>)>,
    /// Search in the memo list.
    pub memo_query: String,
    /// The "Open memo…" window is open.
    pub memo_picker: bool,
    /// Keys in force per command (see rebuild_keys).
    pub keymap: Vec<(crate::ui_panels::Cmd, Vec<crate::keys::Shortcut>)>,
    /// The keyboard shortcuts window, while open.
    pub keys_window: Option<KeysWindow>,
    /// The font picker, while open.
    pub font_menu: Option<FontMenu>,
    /// When the oldest unsaved memo change was made (see autosave_memos).
    pub memo_edited: Option<Instant>,
    /// A panel being dragged by its grip to another window edge.
    pub dock_drag: Option<PanelId>,
    /// A terminal pane being dragged by its header to another place in its tab.
    pub pane_drag: Option<usize>,
    /// File transfers, running and finished (until cleared).
    pub transfers: Vec<crate::transfer::Transfer>,
    pub next_transfer: u64,
    /// When this weft started, and whether a newer one has appeared since
    /// (see restart.rs).
    pub started_at: std::time::SystemTime,
    pub update_ready: bool,
    pub update_checked: Instant,
    /// The "Serial console…" dialog, while open.
    pub serial_dialog: Option<crate::serial_ui::SerialDialog>,
    /// The Settings window.
    pub settings_window: Option<crate::settings_ui::SettingsWindow>,
    /// settings.json as found when it was not valid: not written over
    /// until it is fixed (it was overwritten with the defaults).
    pub settings_bad: Option<String>,
    /// settings.json as weft last read or wrote it (its text, its settings
    /// as weft reads them, when it was changed and its size), and when
    /// weft last looked at it (store.rs).
    pub prefs_written: String,
    pub prefs_seen: Option<serde_json::Map<String, serde_json::Value>>,
    pub prefs_stamp: Option<(std::time::SystemTime, u64)>,
    pub prefs_checked: Instant,
    /// This window's place in state.json as weft last wrote it.
    pub state_written: String,
    /// The window this weft is (instances.rs): its name, its lock held
    /// while weft runs, the windows to open besides (main starts them),
    /// and the process a restart started to take it over (kept for it as
    /// weft closes).
    pub window_id: String,
    pub window_lock: Option<std::fs::File>,
    pub windows_to_open: Vec<String>,
    pub handing_over: Option<u32>,
    /// The window asked for (--window) was taken by another weft.
    pub window_missed: bool,
    /// settings.json read empty or not valid once: believed when it reads
    /// the same a second later (an editor may be saving it).
    pub prefs_unsure: Option<(String, Instant)>,
    /// The last failure to save settings.json, said once.
    pub prefs_failed: Option<String>,
    /// When each setting weft changed and has not saved yet was changed
    /// (store.rs: the later of it and the file's change wins).
    pub changed_here: HashMap<String, std::time::SystemTime>,
    /// Profiles made in Connect that have not connected yet: used, not
    /// kept in settings (nor their tabs in the session) until they do.
    pub trial_profiles: Vec<Profile>,
    /// The shells tried on each host in this run (its terminal ended before
    /// the shell started), so a failure goes on to one not tried yet.
    pub shells_tried: HashMap<String, Vec<RemoteShell>>,
    pub serial_log: Option<crate::serial_ui::SerialLogView>,
    /// Which of the windows Esc closes were open last frame (a bit each),
    /// and what had the keyboard then.
    pub esc_windows: u32,
    pub focus_before: Option<egui::Id>,
    /// The console host this weft's terminals run on (pty::choose_console_host).
    pub console_host: crate::pty::ConsoleHost,
    /// The Display test window's page, while it is open (display_test.rs).
    pub display_test: Option<usize>,
    /// Where a newer weft would appear, when not where `restart` looks by
    /// itself (tests: the environment is one for the whole process).
    pub newer_weft_at: Option<std::path::PathBuf>,
    /// New folder / new file / rename / delete, while its dialog is open.
    pub file_op: Option<crate::fileops::FileOp>,
    /// A question before something is lost (confirm.rs), while it is asked;
    /// what goes on once the files it saves are saved; and weft may close
    /// (it was answered).
    pub confirm: Option<crate::confirm::Confirm>,
    /// ssh's questions about a host key it has not seen, from the askpass
    /// of a connection weft gave a password to: asked here, one at a time.
    pub host_keys: Vec<weft_core::secret::HostKeyQuestion>,
    /// This window takes them (a process has one window; tests make many,
    /// and only the one asked of takes them there).
    pub answers_host_keys: bool,
    pub after_save: Option<crate::confirm::AfterSave>,
    pub quit_confirmed: bool,
    /// Files being dragged from a Files panel: their host and paths.
    pub file_drag: Option<(String, Vec<String>)>,
    /// Files on this PC to hand to the system's drag (they left the
    /// window): main starts it, having the window.
    pub drag_out: Option<Vec<std::path::PathBuf>>,
    /// Remote files being fetched to this PC to be dragged out.
    pub drag_fetch: Option<crate::dragout::Fetch>,
    /// Fetched ones, ready to be dragged out again at once: (host, the
    /// paths there, the copies here, when).
    pub drag_ready: Option<crate::dragout::Copies>,
    /// The system's drag can be started from this window (not under
    /// Wayland): else a row leaving the window stays weft's own drag.
    pub drag_out_ok: bool,
    /// What is being dragged out: its host and paths there.
    pub drag_src: Option<(String, Vec<String>)>,
    /// The files last handed to the system's drag, and where they are
    /// from: dropped back on weft itself, the originals are copied (as
    /// weft's own drag would have), not the copies fetched here.
    pub dragged_out: Option<crate::dragout::Copies>,
    /// The system's drag took the button's release: egui is told it.
    pub release_pointer: bool,
    /// Folder rows drawn this frame (rect, host, path): drop targets.
    pub drop_dirs: Vec<(egui::Rect, String, String)>,
    /// Where each panel was drawn this frame (drop targets).
    pub panel_rects: HashMap<PanelId, egui::Rect>,
    /// The panel being drawn (its grip, split and close buttons act on it).
    pub drawing: PanelId,
    /// The side panel and the editor last clicked: where a command (History,
    /// opening a file) lands.
    pub side_focus: u32,
    pub editor_focus: u32,
    /// The file each editor panel shows (host, path), by panel number.
    pub editor_shows: HashMap<u32, (String, String)>,
    /// What each side panel's Files tab browses, by panel number: two side
    /// panels can show two hosts, and files dragged between them are copied.
    pub side_views: HashMap<u32, SideView>,
    /// The area the panels share (window minus the status bar).
    pub layout_rect: egui::Rect,
    /// ssh passwords for this session only; never written to disk or logs.
    pub secrets: crate::secret::Vault,
    auth_prompted: HashSet<String>,
    pub focus_term: bool,
    pub pane_rects: Vec<(usize, Rect)>,
    pub ssh_hosts: Vec<String>,
    pub actions: Vec<Action>,
    agent_retried: HashSet<String>,
    last_save: Instant,
    /// Records for the History panel, loaded when it is first shown.
    pub records: Option<Vec<crate::records::Record>>,
    pub records_query: String,
    /// Search the output too (else commands only).
    pub records_in_output: bool,
    pub records_host: Option<String>,
    /// The records settings window is open (with sizes measured on opening).
    pub records_window: Option<Vec<(crate::records::Item, u64)>>,
    /// "Graphics backend…": the choice being made ("auto", "wgpu", "opengl").
    pub graphics_window: Option<String>,
    /// What draws, as weft started: the backend, GPU and driver.
    pub graphics: String,
    /// "Python on remote hosts…": the list being edited, a line each.
    pub python_window: Option<String>,
    /// "Shells…": what weft has in each shell, and the ones to use.
    pub shells_window: Option<ShellsWindow>,
    /// What was said about a host's shell in this run ("host\nshell"): once.
    pub shell_notes: HashSet<String>,
    /// Hosts whose chosen shell did not start in this run: their login
    /// shell is used until another is chosen.
    pub shell_choice_off: HashSet<String>,
    last_prune: Option<Instant>,
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
    /// weft started as a window it is told (a restart's, or one main opens
    /// to bring it back).
    pub fn new_window(ctx: egui::Context, wanted: Option<String>, reopen: bool) -> App {
        App::new_in_window(ctx, DataDir::locate(), wanted.as_deref(), reopen)
    }

    /// weft on this data folder (settings, memos, logs).
    #[cfg(test)]
    pub fn new_in(ctx: egui::Context, data: DataDir) -> App {
        App::new_in_window(ctx, data, None, false)
    }

    pub fn new_in_window(ctx: egui::Context, data: DataDir, wanted: Option<&str>, reopen: bool) -> App {
        // settings.json (what a person sets) and state.json (what weft
        // keeps on its own, a place per window); see store.rs, instances.rs.
        let mut loaded = crate::store::load(&data, wanted, reopen);
        let data_prefs = data.path("settings.json");
        let settings_bad = loaded.bad.clone();
        let mut settings: Settings = loaded.settings.clone();
        // D2Coding became the default after JetBrains Mono. Settings still on
        // the old default for both fonts were never changed by hand: move them on.
        if settings.term_font == "builtin:jetbrains-mono" && settings.icon_font == "builtin:jetbrains-mono" {
            settings.term_font = crate::theme::DEFAULT_FONT.into();
        }
        crate::records::set_switches(&settings.records);
        crate::host::set_pythons(&settings.remote_python);
        // Before any terminal is opened.
        let console_host = crate::pty::choose_console_host(&settings.console_host);
        let settings_passwords = settings.passwords;
        data.log(&format!("start: {}, data dir {}", crate::version::line(), data.root.display()));
        data.log(&format!("console host: {}", crate::pty::choose_console_host("").what));
        let searched = settings.search;
        {
            // A host key question wakes the window to show it.
            let c = ctx.clone();
            weft_core::secret::on_host_key_question(move || c.request_repaint());
        }
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
                grep_icase: searched.files_icase,
                grep_mode: searched.files_mode,
                grep_hidden: searched.files_hidden,
                grep_ran: None,
                grep_matcher: None,
                grep_busy: false,
                grep: None,
                procs: None,
                procs_at: None,
                procs_filter: String::new(),
                procs_sort: (3, false),
                procs_host: String::new(),
                selected: None,
                picked: Vec::new(),
                picked_in: Default::default(),
                band: None,
            },
            editor: EditorState { files: Vec::new(), active: 0, visible: false, find: None, select: None },
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
            zoom_acc: 1.0,
            ui_wheel_acc: 0.0,
            find_index: HashMap::new(),
            find_inflight: HashSet::new(),
            find_result: None,
            dock_drag: None,
            panel_rects: HashMap::new(),
            drawing: PanelId::TERMS,
            pane_drag: None,
            transfers: Vec::new(),
            next_transfer: 0,
            started_at: std::time::SystemTime::now(),
            update_ready: false,
            update_checked: Instant::now(),
            serial_dialog: None,
            settings_window: None,
            settings_bad: None,
            prefs_written: loaded.prefs_text.clone(),
            prefs_seen: loaded.prefs_seen.clone(),
            prefs_stamp: std::fs::metadata(data_prefs.clone()).ok().and_then(|m| Some((m.modified().ok()?, m.len()))),
            prefs_checked: Instant::now(),
            state_written: String::new(),
            window_id: loaded.window.id.clone(),
            window_lock: loaded.window.lock.take(),
            windows_to_open: loaded.window.others.clone(),
            handing_over: None,
            window_missed: loaded.window.missed,
            prefs_unsure: None,
            prefs_failed: None,
            changed_here: HashMap::new(),
            trial_profiles: Vec::new(),
            shells_tried: HashMap::new(),
            serial_log: None,
            console_host,
            esc_windows: 0,
            focus_before: None,
            display_test: None,
            newer_weft_at: None,
            file_op: None,
            confirm: None,
            host_keys: Vec::new(),
            answers_host_keys: !cfg!(test),
            after_save: None,
            quit_confirmed: false,
            file_drag: None,
            drag_out: None,
            drag_fetch: None,
            drag_ready: None,
            drag_out_ok: true,
            dragged_out: None,
            drag_src: None,
            release_pointer: false,
            drop_dirs: Vec::new(),
            side_focus: 0,
            editor_focus: 0,
            editor_shows: HashMap::new(),
            side_views: HashMap::new(),
            layout_rect: egui::Rect::NOTHING,
            secrets: crate::secret::Vault::new(settings_passwords),
            auth_prompted: HashSet::new(),
            last_save: Instant::now(),
            records: None,
            records_query: String::new(),
            records_in_output: false,
            records_host: None,
            records_window: None,
            graphics_window: None,
            graphics: String::new(),
            python_window: None,
            shells_window: None,
            shell_notes: HashSet::new(),
            shell_choice_off: HashSet::new(),
            last_prune: None,
            memo_edited: None,
            font_menu: None,
            help: None,
            focus_mode: None,
            memo_cache: None,
            memo_query: String::new(),
            memo_picker: false,
            keymap: Vec::new(),
            keys_window: None,
        };
        app.side.follow = app.settings.follow_cwd;
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
        app.restore_open();
        // settings.json not valid: the defaults for now (the tabs come back
        // all the same, from state.json), the file kept as it is.
        if let Some((text, e)) = settings_bad {
            app.toast(format!("settings.json has an error ({e}): weft runs on the defaults and saves nothing to it until it is fixed (Settings… > Advanced)"), ToastKind::Error);
            app.settings_bad = Some(text);
        }
        // From before state.json: weft's part moved out of settings.json.
        if loaded.to_move {
            app.move_state_out();
        }
        // A console host that was asked for and isn't there: said once.
        if let Some(problem) = app.console_host.problem.clone() {
            app.toast(problem, ToastKind::Error);
        }
        app
    }

    /// What is open besides the terminals, to keep with the session.
    fn saved_open(&self) -> SavedOpen {
        let kept: Vec<usize> = (0..self.editor.files.len()).filter(|&i| !self.editor.files[i].scratch).collect();
        let mut expanded: Vec<(String, String)> = self.side.expanded.iter().cloned().collect();
        expanded.sort();
        SavedOpen {
            files: kept.iter().map(|&i| &self.editor.files[i]).map(|f| SavedFile { host: f.host.clone(), path: f.path.clone(), rich: f.rich, read: f.read }).collect(),
            active: kept.iter().position(|&i| i == self.editor.active).unwrap_or(0),
            editor: self.editor.visible,
            editor_shows: self.editor_shows.iter().map(|(n, v)| (*n, v.clone())).collect(),
            expanded,
            side_views: self.side_views.iter().map(|(n, v)| (*n, SavedView { root: v.root.clone(), follow: v.follow })).collect(),
        }
    }

    /// Open again what was open when weft closed: the editor's files (each
    /// as it was shown: rendered or source, read mode), the one in front,
    /// the folders opened in Files and each panel's view. A file that is
    /// gone says so in its tab.
    fn restore_open(&mut self) {
        let saved = self.settings.open.clone();
        for f in &saved.files {
            self.open_file(&f.host, &f.path, 1);
            if let Some(e) = self.editor.files.iter_mut().find(|e| e.host == f.host && e.path == f.path) {
                e.rich = f.rich;
                e.read = f.read;
                // Where it was is not kept: no jump to the first line either.
                e.goto = None;
            }
        }
        self.editor.active = saved.active.min(self.editor.files.len().saturating_sub(1));
        self.editor.visible = saved.editor && !self.editor.files.is_empty();
        self.editor_shows = saved.editor_shows.into_iter().collect();
        self.side.expanded = saved.expanded.into_iter().collect();
        for (n, v) in saved.side_views {
            self.side_views.insert(n, SideView { root: v.root, follow: v.follow, ..Default::default() });
        }
    }

    fn restore_node(&mut self, n: &SavedNode) -> Node {
        match n {
            // A serial console is not opened again by itself: opening one
            // resets many boards (Arduino, ESP32), as the port list never
            // opens a port to look.
            SavedNode::Pane { host, cwd } if matches!(self.profile(host).kind, HostKind::Serial { .. }) => Node::Leaf(self.spawn_pane_held(host, cwd.clone())),
            SavedNode::Pane { host, cwd } => Node::Leaf(self.spawn_pane(host, cwd.clone())),
            SavedNode::Split { vertical, ratio, a, b } => Node::Split {
                vertical: *vertical,
                ratio: *ratio,
                a: Box::new(self.restore_node(a)),
                b: Box::new(self.restore_node(b)),
            },
        }
    }

    /// A terminal of an ssh (or custom) host ended before weft's shell
    /// integration started there, not as it should (code 0) and not with
    /// ssh's own failure (no connection, refused keys or password: said in
    /// ssh's own words, which are English everywhere; its code, 255, says
    /// nothing on its own, as cmd ends with whatever the last piece of
    /// weft's command it ran did): the host ran weft's command and its
    /// shell could not. Which
    /// shell it is, is found by trying in turn and kept with the profile:
    /// a POSIX sh (weft's `exec sh -c …`), else Windows' cmd or PowerShell
    /// (cmd's "'exec' is not recognized…", in its own language: weft starts
    /// PowerShell there), else one weft does not know (a plain ssh, no
    /// agent). The terminal starts again, and the agent, as the shell is.
    /// How long after a terminal ends what it said is read (ssh's own
    /// error: Windows' console passes the last output on after the
    /// process has ended; without it, ssh's failure was taken for the
    /// shell's).
    const JUDGE_AFTER: Duration = Duration::from_millis(500);
    /// How soon a refusal (or a shell that did not start) ends the terminal.
    const AT_ONCE: Duration = Duration::from_secs(20);

    fn shell_failed(&mut self, id: usize, host: &str, code: Option<u32>, said: &str) {
        // No code: ended by a signal (stopped), not refused.
        let Some(code) = code else { return };
        if said.contains(crate::host::NO_DECODER_SAID) {
            return;
        }
        let refused = self.panes.get(&id).is_some_and(|p| p.exec_refused || (p.may_be_refused() && host::says_exec_unknown(said)));
        let (boot_ran, chose) = self.panes.get(&id).map(|p| (p.boot_ran || p.shell_pid.is_some(), p.started_shell.clone())).unwrap_or_default();
        // Started with the shell chosen for the host, which did not start:
        // its login shell instead, said (once: other terminals of the host
        // may have failed with it too).
        // (Where weft's command ran there. Not by its "shell" event: the
        // bash branch starts bash before it, and a chosen bash that did not
        // start had no fallback so.) ssh's words on the screen do not
        // count here: weft's command ran, so ssh got in ("Permission denied,
        // please try again" from a mistyped password before stopped this).
        // A connection lost just then is not the shell's: ssh ends with 255
        // and says so. (255 alone is not that: a shell killed by a signal
        // as it starts, pwsh without libicu say, ends ssh with 255 too.)
        let lost = code == 255 && connection_lost(said);
        if let Some(choice) = chose.filter(|_| boot_ran && code != 0 && !lost) {
            if self.shell_choice_off.insert(host.to_string()) {
                self.data.log(&format!("pane {id} [{host}]: the chosen shell {choice} ended with {code} before its prompt; its login shell instead"));
                self.toast(format!("{host}: the shell chosen for it, {choice}, did not start (it ended with code {code}): its login shell is used instead. Shells… (command palette) to choose another"), ToastKind::Info);
            }
            self.actions.push(Action::Restart(id));
            return;
        }
        if boot_ran {
            return;
        }
        // Ended with 0 and no word of `exec`: what the host runs instead (a
        // bastion's menu left with one line, a tmux detached, a note and
        // its end) ended as it does. Windows' refusal names exec in quotes,
        // in cmd and PowerShell alike, whatever its code.
        if code == 0 && !refused {
            return;
        }
        // ssh's own failure (it never got in), unless a shell there refused
        // weft's command: a mistyped password first ("Permission denied,
        // please try again") left ssh's words on the screen of one that did.
        if ssh_failed(said) && !refused {
            return;
        }
        let profile = self.profile(host);
        if !matches!(profile.kind, HostKind::Ssh { .. } | HostKind::Custom { .. }) {
            return;
        }
        let current = profile.remote_shell();
        // A custom profile's way in is its own (docker, kubectl, anything):
        // its failures say nothing weft can know in any tool's words. A
        // POSIX one moves on only where a shell refused weft's command in
        // so many words (cmd's and PowerShell's exec); a container not
        // running is left as it is (it was saved as Windows', then
        // "Other"). Once moved on, the next try's end counts as ever.
        if matches!(profile.kind, HostKind::Custom { .. }) && current == RemoteShell::Posix && !refused {
            return;
        }
        // Started for a shell already given up on (another terminal of the
        // host failed first): it starts again for the one tried now.
        if self.panes.get(&id).is_some_and(|p| p.started_as != current) {
            self.actions.push(Action::Restart(id));
            return;
        }
        if current == RemoteShell::Other {
            return;
        }
        let tried = self.shells_tried.entry(host.to_string()).or_default();
        if !tried.contains(&current) {
            tried.push(current);
        }
        // The ones weft supports not tried yet in this run, in turn (a
        // saved one may be wrong: the host was put in again with another
        // system); none left: a shell weft does not support.
        let next = [RemoteShell::Posix, RemoteShell::Windows].into_iter().find(|s| !tried.contains(s)).unwrap_or(RemoteShell::Other);
        let was = match current {
            RemoteShell::Posix => "a POSIX sh",
            _ => "Windows' cmd or PowerShell",
        };
        let now = match next {
            RemoteShell::Posix => "trying a POSIX sh",
            RemoteShell::Windows => "trying Windows' PowerShell",
            RemoteShell::Other => "a shell weft does not support: a plain ssh, without its shell integration; Files, editor and search do not run there",
        };
        self.data.log(&format!("pane {id} [{host}]: weft's command for {was} ended with {code} before the shell started; {now}"));
        self.mark_shell(host, next);
        if next == RemoteShell::Other {
            self.toast(format!("{host}: {}", crate::host::UNSUPPORTED_SHELL), ToastKind::Info);
        }
        self.actions.push(Action::Restart(id));
    }

    /// The shell chosen to run on this host instead of its login shell
    /// ("Shells…"): a remote host's, or on macOS and Linux this computer's.
    /// None where it did not start in this run.
    pub fn shell_choice(&self, host: &str) -> Option<String> {
        if self.shell_choice_off.contains(host) {
            return None;
        }
        let chosen = if self.profile(host).is_local() {
            if cfg!(windows) {
                return None;
            }
            self.settings.unix_shell.trim()
        } else {
            self.settings.remote_shells.get(host).map(|s| s.trim()).unwrap_or("")
        };
        (!chosen.is_empty()).then(|| chosen.to_string())
    }

    /// A host's shell is this one: where weft has less than everything in
    /// it, what is not there is said, once a run for a host and shell.
    pub fn shell_notice(&mut self, host: &str, name: &str) {
        let Some(text) = shell_notice_text(name) else { return };
        if !self.shell_notes.insert(format!("{host}\n{name}")) {
            return;
        }
        self.data.log(&format!("[{host}] {text}"));
        self.toast(format!("{host}: {text}"), ToastKind::Info);
    }

    /// A host kept as one whose shell weft does not support is tried again
    /// once a run, as its first terminal opens (it may have one now; with a
    /// plain ssh nothing fails, so nothing else would find out).
    fn recheck_shell(&mut self, host: &str) {
        if self.shells_tried.contains_key(host) {
            return;
        }
        let profile = self.profile(host);
        if !matches!(profile.kind, HostKind::Ssh { .. } | HostKind::Custom { .. }) {
            return;
        }
        self.shells_tried.insert(host.to_string(), Vec::new());
        if profile.remote_shell() == RemoteShell::Other {
            self.data.log(&format!("[{host}]: kept as a shell weft does not support; trying a POSIX sh and Windows' PowerShell again"));
            self.mark_shell(host, RemoteShell::Posix);
        }
    }

    /// The host's shell is (to be tried as) this one: kept with its
    /// profile, and its connection's copy, whose agent starts again.
    fn mark_shell(&mut self, host: &str, to: RemoteShell) {
        let profile = self.profile(host);
        let mut kept = false;
        for p in self.settings.profiles.iter_mut().filter(|p| p.name == host) {
            p.set_remote_shell(to);
            kept = true;
        }
        // One made in Connect has connected: the host ran weft's command.
        // Kept now (with a plain shell its prompt is never seen).
        if let Some(i) = self.trial_profiles.iter().position(|p| p.name == host) {
            let mut p = self.trial_profiles.remove(i);
            p.set_remote_shell(to);
            self.data.log(&format!("profile \"{host}\" kept: it connected"));
            self.settings.profiles.push(p);
            kept = true;
        }
        if !kept {
            // A host known by its name alone (~/.ssh/config): it becomes a
            // profile, so what was found stays with it.
            let mut p = profile;
            p.set_remote_shell(to);
            self.settings.profiles.push(p);
        }
        self.save_settings();
        // The connection keeps its own copy of the profile.
        let marked = self.profile(host);
        if let Some(c) = self.conns.get_mut(host) {
            c.profile = marked;
            self.reconnect(host);
        }
    }

    /// A profile made in Connect is kept in settings once it has connected:
    /// a terminal of it showed its prompt, or its agent is up.
    pub fn keep_connected_profiles(&mut self) {
        let connected = |app: &App, name: &str| {
            app.panes.values().any(|p| p.host == name && p.seen.1) || app.conns.get(name).is_some_and(|c| c.status == ConnStatus::Ready)
        };
        let (done, rest): (Vec<Profile>, Vec<Profile>) = std::mem::take(&mut self.trial_profiles).into_iter().partition(|p| connected(self, &p.name));
        self.trial_profiles = rest;
        if !done.is_empty() {
            for p in done {
                self.data.log(&format!("profile \"{}\" kept: it connected", p.name));
                self.settings.profiles.push(p);
            }
            self.save_settings();
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
        self.write_settings(true);
    }

    /// The save every few seconds (a terminal's folder, a tab, a file
    /// opened: the session as it is now). Written only when it differs from
    /// what is on disk: it runs inside a frame, and a file written every
    /// ten seconds for nothing is a frame held up every ten seconds where
    /// writes are slow (an antivirus reading each one).
    pub fn autosave_settings(&mut self) {
        self.write_settings(false);
    }

    fn write_settings(&mut self, always: bool) {
        // Not what is on a profile that has not connected yet (it is not
        // kept: another start would take its name for a host).
        let trial: Vec<String> = self.trial_profiles.iter().map(|p| p.name.clone()).collect();
        let kept = |h: &str| !trial.iter().any(|t| t == h);
        self.settings.session = self.tabs.iter().filter_map(|t| keep_panes(self.save_node(&t.root), &kept)).collect();
        let mut open = self.saved_open();
        open.files.retain(|f| kept(&f.host));
        open.expanded.retain(|(h, _)| kept(h));
        open.editor_shows.retain(|_, (h, _)| kept(h));
        for v in open.side_views.values_mut() {
            if v.root.as_ref().is_some_and(|(h, _)| !kept(h)) {
                v.root = None;
            }
        }
        self.settings.open = open;
        // What weft keeps on its own (state.json), and the settings a
        // person sets that changed (settings.json): store.rs.
        self.save_state(always);
        self.save_prefs();
        self.last_save = Instant::now();
    }

    pub fn profile(&self, name: &str) -> Profile {
        if let Some(p) = self.settings.profiles.iter().chain(self.trial_profiles.iter()).find(|p| p.name == name) {
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

    /// For terminal output: draw a few ms later, so what arrives in several
    /// reads (a line cleared, then written again; a character, then the
    /// cursor moved back) shows once, whole, instead of flickering through
    /// its steps (as kitty's input delay does).
    fn wake_term(&self) -> impl Fn() + Send + Clone + 'static {
        let ctx = self.ctx.clone();
        move || ctx.request_repaint_after(std::time::Duration::from_millis(4))
    }

    pub fn ensure_conn(&mut self, host: &str) {
        if !self.conns.contains_key(host) {
            let secret = self.secrets.get(host).map(|s| s.to_string());
            let conn = Conn::open_with_pythons(self.profile(host), &self.data, self.wake(), secret, Some(self.settings.remote_python.clone()));
            self.conns.insert(host.to_string(), conn);
        }
    }

    /// The Pythons tried on remote hosts (settings "remote_python"): kept,
    /// and given to every connection (each has its own: they no longer
    /// read one list of the whole program).
    pub fn set_remote_python(&mut self, list: Vec<String>) {
        self.settings.remote_python = if list.is_empty() { crate::host::DEFAULT_PYTHONS.iter().map(|s| s.to_string()).collect() } else { list };
        crate::host::set_pythons(&self.settings.remote_python);
        for c in self.conns.values_mut() {
            c.pythons = Some(self.settings.remote_python.clone());
        }
    }

    pub fn reconnect(&mut self, host: &str) {
        let wake = self.wake();
        let secret = self.secrets.get(host).map(|s| s.to_string());
        if let Some(c) = self.conns.get_mut(host) {
            c.connect(wake, secret);
        }
    }

    /// Remember a password for this session only (never saved) and retry the agent.
    pub fn set_password(&mut self, host: &str, password: String) {
        let password = zeroize::Zeroizing::new(password);
        let given = !password.is_empty();
        if password.is_empty() {
            self.secrets.remove(host);
        } else {
            self.secrets.insert(host, password);
        }
        self.agent_retried.insert(host.to_string());
        self.auth_prompted.remove(host);
        if self.conns.contains_key(host) {
            self.reconnect(host);
        }
        // Its terminals that have not got in, waiting at a question before
        // any shell (ssh's password, as all of a session's do after a start;
        // told by its shape, in any words) or ended: started again with it,
        // instead of each asking for it once more. One that got in is left
        // as it is.
        if given {
            let waiting: Vec<usize> = self
                .panes
                .iter()
                .filter(|(_, p)| p.host == host && !p.seen.1 && (p.exited || p.term.at_prompt_question()))
                .map(|(id, _)| *id)
                .collect();
            for id in waiting {
                self.data.log(&format!("pane {id} [{host}] started again with the password given"));
                self.actions.push(Action::Restart(id));
            }
        }
    }

    // ---- panes & tabs ----------------------------------------------------------

    pub fn spawn_pane(&mut self, host: &str, cwd: Option<String>) -> usize {
        let id = self.next_pane;
        self.next_pane += 1;
        self.recheck_shell(host);
        self.ensure_conn(host);
        let mut pane = Pane::new(id, host, cwd);
        self.start_pty(&mut pane);
        self.panes.insert(id, pane);
        if !self.history.contains_key(host) {
            let hist = std::fs::read_to_string(self.data.path(&format!("history/{}.txt", host::safe_name(host))))
                .map(|s| s.lines().filter_map(crate::records::history_command).map(String::from).collect())
                .unwrap_or_default();
            self.history.insert(host.to_string(), hist);
        }
        id
    }

    /// A pane for `host` not started: it waits for Enter (see Pane::held).
    pub fn spawn_pane_held(&mut self, host: &str, cwd: Option<String>) -> usize {
        let id = self.next_pane;
        self.next_pane += 1;
        let mut pane = Pane::new(id, host, cwd);
        pane.exited = true;
        pane.held = true;
        self.panes.insert(id, pane);
        id
    }

    fn start_pty(&mut self, pane: &mut Pane) {
        pane.held = false;
        let prof = self.profile(&pane.host);
        if let host::HostKind::Serial { via, port, baud } = prof.kind.clone() {
            self.start_serial(pane, &via, &port, baud);
            return;
        }
        let secret = if prof.is_local() { None } else { self.secrets.get(&pane.host) };
        pane.started_as = prof.remote_shell();
        let choice = self.shell_choice(&pane.host);
        pane.started_shell = choice.clone();
        let mut argv = prof.pty_argv_with(pane.initial_cwd.as_deref(), secret.is_some(), choice.as_deref());
        let mut env = secret.as_deref().map(|s| host::askpass_env(s)).unwrap_or_default();
        if prof.is_local() {
            let (a, e) = host::local_shell_with(self.settings.local_shell, choice.as_deref().unwrap_or(""));
            argv = a;
            env.extend(e);
            // Windows' shells say nothing of themselves as they start.
            if cfg!(windows) && self.settings.local_shell == host::LocalShell::Cmd {
                self.shell_notice(&pane.host.clone(), "cmd");
            }
        }
        // (A Windows host's terminal is PowerShell too: Esc there as well.)
        let local_esc = prof.is_local() && cfg!(windows) && !argv.first().and_then(|a| std::path::Path::new(a).file_stem()).is_some_and(|s| s.eq_ignore_ascii_case("nu"));
        pane.clear_with_esc = local_esc || (!prof.is_local() && pane.started_as == RemoteShell::Windows);
        pane.lang = if prof.is_local() && cfg!(windows) { argv.first().map(|a| host::ShellLang::of_local(a)).unwrap_or_default() } else { prof.shell_lang() };
        // X11 forwarding: ssh sends the GUI to the X server here (started if
        // installed and not running); DISPLAY last, so it wins over askpass's.
        if matches!(prof.kind, host::HostKind::Ssh { x11: true, .. }) {
            if cfg!(windows) {
                env.push(("DISPLAY".into(), crate::x11::DISPLAY.into()));
            }
            match crate::x11::ensure_server() {
                Ok(Some(name)) => self.toast(format!("started {name} for X11: GUI programs on {} show here", pane.host), ToastKind::Info),
                Ok(None) => {}
                Err(e) => self.toast(e, ToastKind::Error),
            }
        }
        let local_cwd = if prof.is_local() { pane.initial_cwd.clone() } else { None };
        let shown: Vec<String> =
            argv.iter().map(|a| if a.len() > 120 { format!("{}…({} bytes)", &a[..60], a.len()) } else { a.clone() }).collect();
        self.data.log(&format!("pane {} [{}] spawn {:?}", pane.id, pane.host, shown));
        pane.term = Term::new(pane.term.cols, pane.term.rows);
        pane.term.show_guesses = self.settings.guess_blocks;
        pane.injected = false;
        pane.shell_pid = None;
        pane.boot_ran = false;
        pane.interrupted = false;
        pane.started_at = Instant::now();
        pane.exec_refused = false;
        // This start's first output and prompt are yet to come (kept from
        // the last start, a chosen shell that ended at once after a restart
        // was never judged: no fallback to the login shell).
        pane.seen = (false, false);
        pane.menu_seen = false;
        pane.clear_key = false;
        pane.lines_sent = 0;
        pane.booted = None;
        pane.send_at = None;
        pane.parser = vte::Parser::new();
        pane.widgets.clear();
        pane.hist_seen = 0;
        // Its blocks are counted from 1 again: Records took the first ones
        // of a terminal started again for seen; and an end still to be
        // judged was judged against the new process.
        pane.rec_seen = 0;
        pane.rec_cwd = None;
        pane.judge_at = None;
        match Pty::spawn(&argv, local_cwd.as_deref(), &env, pane.term.cols as u16, pane.term.rows as u16, self.wake_term()) {
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

    /// A serial console pane: a local port opened here, or a remote one
    /// through that host's ssh with the bridge (remote/serial.py).
    fn start_serial(&mut self, pane: &mut Pane, via: &str, port: &str, baud: u32) {
        // A remote console's typing is kept once its port is open (not an
        // answer to the bridge, nor a sudo password).
        let logs = self.data.path("logs");
        pane.serial_log = Some(if via == "local" {
            weft_core::serial_log::Writer::new(logs, &pane.host)
        } else {
            // The bridge's own line for this port, whole (ssh's banner may
            // say "connected to" too, before a password is typed).
            weft_core::serial_log::Writer::after(logs, &pane.host, &format!("\x1b[2m{}{port} at {baud}", host::SERIAL_READY))
        });
        // Opened again (Enter after it ended): what the device said before
        // stays on the screen (a boot log), a line between.
        if pane.serial_opened {
            pane.parser = vte::Parser::new();
            pane.term.reset_modes();
            pane.term.feed(&mut pane.parser, b"\r\n\x1b[2m\xe2\x80\x94 opened again \xe2\x80\x94\x1b[0m\r\n");
        } else {
            pane.term = Term::new(pane.term.cols, pane.term.rows);
            pane.term.show_guesses = self.settings.guess_blocks;
        }
        pane.serial_last = Some(0);
        pane.injected = true; // no shell integration to inject
        pane.shell_pid = None;
        pane.parser = vte::Parser::new();
        pane.widgets.clear();
        pane.hist_seen = 0;
        pane.rec_seen = 0;
        pane.rec_cwd = None;
        pane.judge_at = None;
        let result = if via == "local" {
            self.data.log(&format!("pane {} serial {port} at {baud}", pane.id));
            Pty::serial(port, baud, self.wake_term()).map_err(|e| {
                // Is it one of ours?
                let ours = self.panes.values().find(|p| p.id != pane.id && p.host == pane.host && !p.exited).map(|p| p.title());
                match ours {
                    Some(t) => format!("{e} — open in this weft already, in the tab \"{t}\""),
                    None => e,
                }
            })
        } else {
            let through = self.profile(via);
            let secret = self.secrets.get(via);
            let argv = through.serial_argv_with(port, baud, secret.is_some(), Some(&self.settings.remote_python));
            let env = secret.as_deref().map(|s| host::askpass_env(s)).unwrap_or_default();
            self.data.log(&format!("pane {} serial {via}:{port} at {baud} over {}", pane.id, argv.first().cloned().unwrap_or_default()));
            if argv.is_empty() {
                Err(match through.remote_shell() {
                    crate::host::RemoteShell::Posix => format!("{via}: serial consoles go through ssh or a custom transport"),
                    _ => format!("{via}: a serial console on a remote host needs a POSIX shell there (Linux, macOS); {via}'s is Windows' or one weft does not support"),
                })
            } else {
                Pty::spawn(&argv, None, &env, pane.term.cols as u16, pane.term.rows as u16, self.wake_term())
            }
        };
        match result {
            Ok(p) => {
                pane.serial_opened = true;
                pane.pty = Some(p);
                pane.exited = false;
                pane.spawn_error = None;
            }
            Err(e) => {
                self.data.log(&format!("pane {} serial failed: {e}", pane.id));
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
        self.on_active_changed();
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
        // Another terminal has the keyboard now: Files goes with it.
        self.on_active_changed();
    }

    /// Close terminal tabs around tab `at` (tab context menu). The tab that
    /// was right-clicked stays selected when it survives.
    pub fn close_tabs(&mut self, at: usize, which: CloseWhich) {
        let keep = self.tabs.get(at).map(|t| t.active).filter(|_| !matches!(which, CloseWhich::This | CloseWhich::All));
        for ti in tabs_to_close(self.tabs.len(), at, which) {
            self.close_tab(ti);
        }
        if let Some(id) = keep {
            self.focus_pane(id);
        }
    }

    /// Close editor files around tab `at` (tab context menu).
    pub fn close_files(&mut self, at: usize, which: CloseWhich) {
        let kept = self.editor.files.get(self.editor.active).map(|f| (f.host.clone(), f.path.clone()));
        let target = self.editor.files.get(at).map(|f| (f.host.clone(), f.path.clone()));
        self.flush_memos();
        let mut discarded = 0;
        let mut emptied = false;
        for i in tabs_to_close(self.editor.files.len(), at, which) {
            let f = self.editor.files.remove(i);
            if f.dirty && !f.scratch {
                discarded += 1;
            }
            // A memo closed without anything in it is not kept.
            if self.is_memo(&f) && f.text.trim().is_empty() && std::fs::remove_file(&f.path).is_ok() {
                emptied = true;
            }
        }
        if emptied {
            self.forget_memo_listing();
        }
        if discarded > 0 {
            self.toast(format!("closed {discarded} file(s) with unsaved changes discarded"), ToastKind::Info);
        }
        // Stay on the right-clicked file when it is still open, else on the one before.
        let find = |k: &Option<(String, String)>, files: &[EditorFile]| k.as_ref().and_then(|k| files.iter().position(|f| f.host == k.0 && f.path == k.1));
        self.editor.active = find(&target, &self.editor.files).or(find(&kept, &self.editor.files)).unwrap_or(self.editor.active).min(self.editor.files.len().saturating_sub(1));
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

    /// Keyboard resizing: the border of the active pane's nearest split
    /// that way (left/right: panes side by side; up/down: one above the
    /// other) moves `delta` of that split, the way the arrow points. Said
    /// when there is no such split.
    pub fn resize_pane(&mut self, side_by_side: bool, delta: f32) {
        let Some(id) = self.active_pane_id() else { return };
        let Some(tab) = self.tabs.get_mut(self.active_tab) else { return };
        // Zoomed, one pane fills the tab: nothing to resize.
        if tab.zoomed {
            return;
        }
        if !tab.root.nudge(id, side_by_side, delta).1 {
            let way = if side_by_side { "beside it" } else { "above or below it" };
            self.toast(format!("no pane {way} to resize against"), ToastKind::Info);
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
        if let Some(p) = self.active_pane() {
            let host = p.host.clone();
            // (A shell that has not said where it is yet is where it was started.)
            if let Some(cwd) = p.term.cwd.clone().or(p.initial_cwd.clone()) {
                self.follow_terminal(&host, &cwd);
            }
        }
    }

    /// The terminal moved to `host:cwd`: every side panel following it goes
    /// there, whichever was clicked last (only that one used to, so a click
    /// in a second side panel that stays put stopped the first following).
    ///
    /// The focused panel's view is kept twice: in `side`, and in
    /// `side_views` as it was when the frame began, which `side` is set
    /// back to when the panels have been drawn. Both are changed here: a
    /// click on a tab comes while the panels are being drawn, and what was
    /// only put in `side` was undone at the end of that frame (Files stayed
    /// in the folder of the tab before).
    pub fn follow_terminal(&mut self, host: &str, cwd: &str) {
        let new = Some((host.to_string(), cwd.to_string()));
        let mut list = false;
        for v in self.side_views.values_mut() {
            if v.follow && v.root != new {
                v.root = new.clone();
                list = true;
            }
        }
        if self.side.follow && self.side.root != new {
            self.side.root = new;
            list = true;
        }
        if list {
            self.list_dir(host, cwd);
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
        let (hidden, ignored) = (self.settings.show_hidden, self.settings.show_ignored);
        self.request(host, json!({"op": "list", "path": path, "hidden": hidden, "ignored": ignored}), Pending::List { path: path.into() });
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
            bytes: None,
            image: Default::default(),
            pdf: Default::default(),
            host: host.into(),
            path: path.into(),
            text: String::new(),
            dirty: false,
            mtime: None,
            outdated: None,
            disk_asked: None,
            disk_gen: 0,
            binary: false,
            scratch: false,
            loading: true,
            saving: false,
            saving_of: None,
            conflict: false,
            goto: Some(line.max(1)),
            marks: Default::default(),
            mark_cur: None,
            rich: crate::md::is_markdown(path),
            read: false,
            saved_at: None,
            copied_at: None,
            error: None,
        });
        self.editor.active = self.editor.files.len() - 1;
        self.request(host, json!({"op": "read", "path": path, "max": crate::media::read_limit(path)}), Pending::Open { path: path.into(), line });
    }

    pub fn open_scratch(&mut self, title: &str, text: String) {
        self.editor.files.push(EditorFile {
            bytes: None,
            image: Default::default(),
            pdf: Default::default(),
            host: "scratch".into(),
            path: title.into(),
            text,
            dirty: false,
            mtime: None,
            outdated: None,
            disk_asked: None,
            disk_gen: 0,
            binary: false,
            scratch: true,
            loading: false,
            saving: false,
            saving_of: None,
            conflict: false,
            goto: None,
            marks: Default::default(),
            mark_cur: None,
            rich: false,
            read: false,
            saved_at: None,
            copied_at: None,
            error: None,
        });
        self.editor.active = self.editor.files.len() - 1;
        self.editor.visible = true;
    }

    // ---- records ----------------------------------------------------------------

    /// Apply the keep times and size limits, on another thread.
    pub fn prune_records(&mut self) {
        let (root, rules, data) = (self.data.root.clone(), self.settings.records.clone(), self.data.clone());
        std::thread::spawn(move || {
            for line in crate::records::prune(&root, &rules) {
                data.log(&line);
            }
        });
    }

    /// The records settings changed: switches, limits and the settings file.
    pub fn records_changed(&mut self) {
        crate::records::set_switches(&self.settings.records);
        self.prune_records();
        self.save_settings();
    }

    // ---- fonts ------------------------------------------------------------------

    /// Fonts you download go here (or anywhere: a path can be entered).
    pub fn fonts_dir(&self) -> std::path::PathBuf {
        self.data.path("fonts")
    }

    /// Load the fonts in the settings; a font that can't be read is reported
    /// and the built-in default is used in its place.
    pub fn apply_fonts(&mut self) {
        for p in crate::theme::install_fonts(&self.ctx, &self.settings.term_font, &self.settings.icon_font) {
            self.toast(format!("font: {p} — using the built-in font"), ToastKind::Error);
        }
    }

    pub fn open_font_menu(&mut self) {
        let _ = std::fs::create_dir_all(self.fonts_dir());
        let files = crate::theme::font_files(&self.fonts_dir());
        self.font_menu = Some(FontMenu { icons: false, query: String::new(), path: String::new(), files });
    }

    /// Choose a font for the terminal (or for icons): applied and saved now.
    pub fn set_font(&mut self, icons: bool, spec: String) {
        if icons {
            self.settings.icon_font = spec;
        } else {
            self.settings.term_font = spec;
        }
        self.apply_fonts();
        self.save_settings();
    }

    // ---- memos: markdown notes in data/memos, opened in the editor like files ----

    /// The memo folder, '/'-separated like every editor path.
    pub fn memo_dir(&self) -> String {
        self.data.path("memos").display().to_string().replace('\\', "/")
    }

    pub fn is_memo(&self, f: &EditorFile) -> bool {
        f.host == "local" && f.path.starts_with(&format!("{}/", self.memo_dir()))
    }

    /// A new empty memo (memo-N.md), opened with the keyboard in it.
    pub fn new_memo(&mut self) {
        let dir = self.memo_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast(format!("memo folder: {e}"), ToastKind::Error);
            return;
        }
        let path = (1..).map(|n| format!("{dir}/memo-{n}.md")).find(|p| !std::path::Path::new(p).exists()).unwrap();
        if let Err(e) = std::fs::write(&path, "") {
            self.toast(format!("new memo: {e}"), ToastKind::Error);
            return;
        }
        self.forget_memo_listing();
        self.editor.files.push(EditorFile {
            bytes: None,
            image: Default::default(),
            pdf: Default::default(),
            host: "local".into(),
            path: path.clone(),
            text: String::new(),
            dirty: false,
            mtime: None,
            outdated: None,
            disk_asked: None,
            disk_gen: 0,
            binary: false,
            scratch: false,
            loading: false,
            saving: false,
            saving_of: None,
            conflict: false,
            goto: None,
            marks: Default::default(),
            mark_cur: None,
            rich: true,
            read: false,
            saved_at: None,
            copied_at: None,
            error: None,
        });
        self.editor.active = self.editor.files.len() - 1;
        self.editor.visible = true;
        self.focus_term = false;
        self.ctx.memory_mut(|m| m.request_focus(egui::Id::new(("editor", self.editor.active, path))));
    }

    /// Write changed memos now. Memos have no Save step: they are written
    /// shortly after a change, when closed, and when weft exits.
    pub fn flush_memos(&mut self) {
        self.memo_edited = None;
        let dir = format!("{}/", self.memo_dir());
        let mut failed = Vec::new();
        for f in self.editor.files.iter_mut().filter(|f| f.dirty && f.host == "local" && f.path.starts_with(&dir)) {
            match std::fs::write(&f.path, &f.text) {
                Ok(()) => {
                    f.dirty = false;
                    f.saved_at = Some(Instant::now());
                    // Remember the time of this write: with the old one, a later
                    // save took the memo for changed elsewhere and asked to overwrite.
                    if let Ok(md) = std::fs::metadata(&f.path) {
                        f.mtime = Some(crate::host::mtime_of(&md));
                    }
                    f.conflict = false;
                }
                Err(e) => failed.push(format!("{}: {e}", f.path)),
            }
        }
        for e in failed {
            self.toast(format!("memo not saved — {e}"), ToastKind::Error);
        }
    }

    /// Called every frame: a changed memo is written within DELAY.
    fn autosave_memos(&mut self) {
        const DELAY: Duration = Duration::from_millis(700);
        if !self.editor.files.iter().any(|f| f.dirty && self.is_memo(f)) {
            self.memo_edited = None;
            return;
        }
        let since = *self.memo_edited.get_or_insert_with(Instant::now);
        if since.elapsed() >= DELAY {
            self.flush_memos();
        } else {
            self.ctx.request_repaint_after(DELAY - since.elapsed());
        }
    }

    /// The Files panel lists the memo folder afresh after a memo comes or goes.
    fn forget_memo_listing(&mut self) {
        let key = ("local".to_string(), self.memo_dir());
        if self.dir_cache.remove(&key).is_some() {
            self.list_dir(&key.0, &key.1);
        }
    }

    /// The memos on disk, newest first (re-read at most once a second).
    pub fn memos(&mut self) -> Vec<MemoInfo> {
        if let Some((at, list)) = &self.memo_cache {
            if at.elapsed() < Duration::from_secs(1) {
                return list.clone();
            }
        }
        let list = read_memos(&self.memo_dir());
        self.memo_cache = Some((Instant::now(), list.clone()));
        list
    }

    /// Delete a memo file, closing its tab if open.
    pub fn delete_memo_file(&mut self, path: &str) {
        if let Some(i) = self.editor.files.iter().position(|f| f.host == "local" && f.path == path) {
            self.delete_memo(i);
        } else {
            let _ = std::fs::remove_file(path);
            self.forget_memo_listing();
            self.toast(format!("deleted {}", path.rsplit('/').next().unwrap_or(path)), ToastKind::Info);
        }
        self.memo_cache = None;
    }

    /// Delete a memo's file and close its tab.
    pub fn delete_memo(&mut self, idx: usize) {
        let Some(f) = self.editor.files.get(idx) else { return };
        if !self.is_memo(f) {
            return;
        }
        let path = f.path.clone();
        let _ = std::fs::remove_file(&path);
        self.editor.files.remove(idx);
        self.editor.active = self.editor.active.min(self.editor.files.len().saturating_sub(1));
        self.forget_memo_listing();
        self.toast(format!("deleted {}", path.rsplit('/').next().unwrap_or(&path)), ToastKind::Info);
    }

    pub fn save_file(&mut self, idx: usize, force: bool) {
        // Memos save themselves (Ctrl+S on one just does it now).
        if self.editor.files.get(idx).is_some_and(|f| self.is_memo(f)) {
            self.flush_memos();
            return;
        }
        let Some(f) = self.editor.files.get_mut(idx) else { return };
        if f.scratch || f.binary || f.loading {
            return;
        }
        f.saving = true;
        f.saving_of = Some(text_hash(&f.text));
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
        f.disk_gen += 1;
        let (host, path) = (f.host.clone(), f.path.clone());
        self.request(&host, json!({"op": "read", "path": path, "max": crate::media::read_limit(&path)}), Pending::Open { path, line: 0 });
    }

    /// The file shown in the editor, asked about on disk every two seconds
    /// while the window is looked at (changed or deleted by another program:
    /// said above it). None of it while weft is not looked at.
    fn check_editor_on_disk(&mut self) {
        const EVERY: Duration = Duration::from_secs(2);
        if !self.editor.visible || !self.ctx.input(|i| i.focused) {
            return;
        }
        let Some(f) = self.editor.files.get_mut(self.editor.active) else { return };
        if f.scratch || f.loading || f.saving || f.mtime.is_none() {
            return;
        }
        self.ctx.request_repaint_after(EVERY);
        if f.disk_asked.is_some_and(|t| t.elapsed() < EVERY) {
            return;
        }
        f.disk_asked = Some(Instant::now());
        let (host, path, generation) = (f.host.clone(), f.path.clone(), f.disk_gen);
        self.request(&host, json!({"op": "stat", "path": path}), Pending::EditorStat { path, generation });
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        let text = text.into();
        self.data.log(&format!("toast: {text}"));
        self.toasts.push(Toast { text, kind, at: Instant::now() });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    /// The terminal Files' actions for `host` go to: the active one, if it
    /// is that host's (another host's got this one's paths: a Linux path
    /// cd'd in this PC's PowerShell).
    pub fn pane_of(&self, host: &str) -> Option<usize> {
        self.active_pane_id().filter(|id| self.panes.get(id).is_some_and(|p| p.host == host))
    }

    /// A cd to `path`, in the language of that terminal's shell.
    pub fn cd_in(&self, id: usize, path: &str) -> String {
        self.panes.get(&id).map(|p| p.lang).unwrap_or_default().cd(path)
    }

    /// `path` quoted for that terminal's shell.
    pub fn quote_in(&self, id: usize, path: &str) -> String {
        self.panes.get(&id).map(|p| p.lang).unwrap_or_default().quote(path)
    }

    pub fn run_in_pane(&mut self, id: usize, cmd: &str) {
        // Nothing to run (a block whose text the shell did not tell): not
        // an empty line sent.
        if cmd.trim().is_empty() {
            return;
        }
        let Some(p) = self.panes.get_mut(&id) else { return };
        // (A run's command still to go after its Esc: busy too, or the
        // second run took its place and the first was lost.)
        if !p.term.at_prompt() || p.send_at.is_some() {
            self.toast("pane is busy — command not sent", ToastKind::Error);
            return;
        }
        // Clear whatever is on the line, then run: Esc in Windows' own
        // PowerShell and cmd; Ctrl-U elsewhere (bash and zsh read Esc and
        // the command's first letter as one key, Meta-l for `ls`: the
        // command lost its first letter on macOS and Linux). Esc on a write
        // of its own: with the command in the same write the console takes
        // it with the next key, and what was on the line stayed; and on a
        // write of its own but at once, a console that reads late (a busy
        // PC, ssh on the way) took both together: the command 150 ms after
        // it. (By the shell that started: nu chosen but not installed is
        // PowerShell.)
        // Its text for the block (a shell that marks no start reads it off
        // the screen at the Enter, before the shell has echoed it).
        p.term.user_runs(Some(cmd));
        // weft's own key where its integration bound one: Esc and Ctrl-U are
        // the default editing modes' (vi's insert mode takes Esc as leaving
        // it, PSReadLine's Emacs mode as Meta).
        let mut bytes = if p.clear_key {
            CLEAR_KEY.to_vec()
        } else if p.clear_with_esc {
            p.send(b"\x1b");
            Vec::new()
        } else {
            b"\x15".to_vec()
        };
        bytes.extend_from_slice(cmd.as_bytes());
        bytes.push(b'\r');
        if p.clear_with_esc && !p.clear_key {
            p.send_at = Some((Instant::now() + Duration::from_millis(150), bytes));
            self.ctx.request_repaint_after(Duration::from_millis(150));
        } else {
            p.send(&bytes);
        }
    }

    pub fn paste_into(&mut self, id: usize, text: &str) {
        let Some(p) = self.panes.get_mut(&id) else { return };
        let text = text.replace("\r\n", "\n").replace('\n', "\r");
        // Not bracketed on a serial console: the device may have rebooted
        // under a shell that had asked for it, and its boot loader took
        // the markers as typing.
        if p.term.bracketed_paste && p.serial_log.is_none() {
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
        self.autosave_memos();
        let (forgotten, next) = self.secrets.tick();
        for h in forgotten {
            self.data.log(&format!("{h}: unused password forgotten"));
        }
        if let Some(d) = next {
            self.ctx.request_repaint_after(d.max(Duration::from_millis(200)));
        }
        let mut events: Vec<(usize, TermEvent)> = Vec::new();
        let mut never_started: Vec<(usize, String, Option<u32>, String)> = Vec::new();
        let mut new_history: Vec<(String, String)> = Vec::new();
        let mut new_records: Vec<crate::records::Record> = Vec::new();
        let mut timings: Vec<String> = Vec::new();
        let mut guessing = false;
        for (id, pane) in self.panes.iter_mut() {
            // Shells without OSC 133: prompts and command ends are guessed when
            // output settles, so look again shortly even without new output.
            guessing |= pane.term.tick();
            if let Some((at, _)) = &pane.send_at {
                if Instant::now() >= *at {
                    let (_, bytes) = pane.send_at.take().unwrap();
                    pane.send(&bytes);
                } else {
                    self.ctx.request_repaint_after(*at - Instant::now());
                }
            }
            // An ended serial console lets its port go (its last output
            // read): unplugged and plugged in again, it is the same name
            // (Linux gave it the next one while it was held) and Enter
            // opens it.
            // (A remote console's process is gone and its port with it; its
            // last words may still be on their way: read as any terminal's.)
            if pane.exited && pane.pty.as_mut().is_some_and(|p| p.is_port() && p.exited()) {
                pane.pty = None;
            }
            if let Some(pty) = pane.pty.as_mut() {
                // An app's synchronized update (DEC 2026) comes in whole.
                let now = Instant::now();
                // Seen ended before the read: its last output (ssh's own
                // error, say) is in this read, not left for after the end.
                let ended = pty.exited();
                let parts = pty.read_parts();
                let mut read: Vec<u8> = parts.iter().flat_map(|p| p.2.iter().copied()).collect();
                if let (Some(t), false) = (pane.trace.as_mut(), read.is_empty()) {
                    t.output(&read);
                }
                // A serial console's log: what the device sent, when it came
                // (not weft's own words).
                if let Some(l) = pane.serial_log.as_mut() {
                    for (t, info, b) in &parts {
                        if !info {
                            l.write_at(weft_core::serial_log::Dir::Rx, *t, b);
                        }
                    }
                    // Its day's limit reached: said, not only in the log.
                    if l.take_filled() {
                        let text = format!("{}: today's serial log is full; nothing more is logged today (Privacy… sets how much is kept)", pane.host);
                        self.data.log(&format!("toast: {text}"));
                        self.toasts.push(Toast { text, kind: ToastKind::Info, at: Instant::now() });
                    }
                }
                // Shown with a lone LF as CRLF.
                if let Some(last) = pane.serial_last.as_mut() {
                    let mut shown = Vec::with_capacity(read.len() + 16);
                    for &b in &read {
                        if b == b'\n' && *last != b'\r' {
                            shown.push(b'\r');
                        }
                        shown.push(b);
                        *last = b;
                    }
                    read = shown;
                }
                let mut bytes = pane.sync.push(&read, now);
                if ended {
                    bytes.extend(pane.sync.take());
                }
                if let Some(d) = pane.sync.due(now) {
                    self.ctx.request_repaint_after(d);
                }
                if !bytes.is_empty() {
                    let before = pane.term.end_line();
                    pane.term.feed(&mut pane.parser, &bytes);
                    if !pane.boot_ran && bytes.windows(host::NO_DECODER_SAID.len()).any(|w| w == host::NO_DECODER_SAID.as_bytes()) {
                        // (Pane::boot_ran, with pty borrowed.)
                        pane.boot_ran = true;
                        pane.booted.get_or_insert((now, pane.lines_sent));
                    }
                    // Its "sh" now, not with the other events after this
                    // loop: a chosen shell that ends at once ends in this
                    // read too, and was judged as if weft's command had not
                    // run yet (no fallback after a password and a code).
                    if !pane.boot_ran && pane.term.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["t"] == "sh")) {
                        pane.boot_ran = true;
                        pane.booted.get_or_insert((now, pane.lines_sent));
                    }
                    // (Pane::may_be_refused, with pty borrowed.)
                    if !pane.exec_refused && (!pane.boot_ran && pane.shell_pid.is_none() && !pane.seen.1 && pane.lines_sent <= 5) && host::says_exec_unknown(&String::from_utf8_lossy(&bytes)) {
                        pane.exec_refused = true;
                    }
                    if !pane.seen.0 {
                        pane.seen.0 = true;
                        timings.push(format!("startup: pane {id} [{}] first output at {}", pane.host, crate::since_start()));
                    }
                    let replies = std::mem::take(&mut pane.term.replies);
                    // A terminal's answers go to a serial device too: logged
                    // as sent.
                    if let (Some(l), false) = (pane.serial_log.as_mut(), replies.is_empty()) {
                        l.write(weft_core::serial_log::Dir::Tx, &replies);
                    }
                    pty.write(&replies);
                    if pane.scroll > 0 {
                        pane.scroll += (pane.term.end_line() - before) as usize;
                    }
                }
                // (Looked at without new output too: a guessed prompt is
                // found a moment after its text came.)
                // (Or a menu's question, told at its answer: see Pane::send.)
                if !pane.menu_seen && pane.term.shell_prompted() {
                    pane.menu_seen = true;
                }
                if !pane.seen.1 && pane.term.shell_prompted() {
                    pane.seen.1 = true;
                    timings.push(format!("startup: pane {id} [{}] first prompt at {}", pane.host, crate::since_start()));
                }
                if !pane.exited && ended {
                    pane.exited = true;

                    // Ended before weft's command said it ran there, or (a
                    // chosen shell) before its prompt: what ran it there?
                    // (see shell_failed). Not by the code alone: Windows'
                    // OpenSSH ends a session with a terminal with 0 whatever
                    // ran, and a refusal can look like a prompt.
                    let never_ran = pane.shell_pid.is_none() && !pane.boot_ran;
                    // At once, a password at most: not a session that was
                    // used and then ended (it reconnected as another shell,
                    // and a forced command's host was saved as "Other").
                    // (Pane::may_be_refused, with pty borrowed.)
                    let refused = pane.exec_refused || (never_ran && (!pane.boot_ran && pane.shell_pid.is_none() && !pane.seen.1 && pane.lines_sent <= 5) && host::says_exec_unknown(&pane.term.lines_text(pane.term.first_line(), pane.term.end_line() + 1)));
                    let at_once = refused || (pane.lines_sent <= 1 && pane.started_at.elapsed() < Self::AT_ONCE);
                    // A chosen shell: soon after weft's command ran, nothing
                    // typed into it since (not counting the time and lines
                    // a password and a one-time code took before).
                    let chose_quit = pane.started_shell.is_some() && !pane.seen.1 && pane.booted.is_some_and(|(at, n)| pane.lines_sent <= n && at.elapsed() < Self::AT_ONCE);
                    // (Not once the shell's prompt or a menu's was seen: a
                    // bastion left with `quit` and code 1 was taken for a
                    // refusal and the host saved as "Other". A refusal comes
                    // before any prompt, after questions only.)
                    if ((never_ran && (refused || (at_once && !pane.menu_seen))) || chose_quit) && !pane.interrupted {
                        pane.judge_at = Some(now + Self::JUDGE_AFTER);
                        self.ctx.request_repaint_after(Self::JUDGE_AFTER);
                    }
                }
                if pane.judge_at.is_some_and(|at| now >= at) {
                    pane.judge_at = None;
                    let t = &pane.term;
                    let said = t.lines_text(t.first_line(), t.end_line() + 1);
                    never_started.push((*id, pane.host.clone(), pty.exit_code(Duration::from_millis(300)), said));
                } else if let Some(at) = pane.judge_at {
                    self.ctx.request_repaint_after(at - now);
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
                    if pane.rec_cwd.as_ref().is_none_or(|c| c.0 != b.id) {
                        pane.rec_cwd = Some((b.id, pane.term.cwd.clone().or(pane.initial_cwd.clone()).unwrap_or_default()));
                    }
                    if !b.command.is_empty() && !b.private {
                        new_history.push((pane.host.clone(), b.command.clone()));
                    }
                }
                if b.id > pane.rec_seen && b.state() == BlockState::Done {
                    pane.rec_seen = b.id;
                    if b.private || !self.settings.records.commands.on {
                        continue;
                    }
                    let mut out = pane.term.block_output(b);
                    if b.command.is_empty() && out.trim().is_empty() {
                        continue;
                    }
                    let cut = crate::records::limit_output(&mut out, self.settings.records.output_kb);
                    let dur = b.duration.unwrap_or_default().as_secs_f64();
                    let cwd = pane.rec_cwd.take().filter(|c| c.0 == b.id).map(|c| c.1).unwrap_or_default();
                    new_records.push(crate::records::Record { t: crate::records::now() - dur, host: pane.host.clone(), cwd, cmd: b.command.clone(), exit: b.exit, dur, out, cut });
                }
            }
        }
        for t in timings {
            self.data.log(&t);
        }
        for (id, host, code, said) in never_started {
            self.shell_failed(id, &host, code, &said);
        }
        self.keep_connected_profiles();
        if guessing {
            self.ctx.request_repaint_after(Duration::from_millis(150));
        }
        for (host, cmd) in new_history {
            self.record_history(&host, cmd);
        }
        for r in new_records {
            if let Err(e) = crate::records::append(&self.data.root, &r) {
                self.data.log(&format!("record not written: {e}"));
            }
            if let Some(all) = self.records.as_mut() {
                all.push(r);
            }
        }
        // Old records and logs go at start and then hourly, off this thread.
        if self.last_prune.map(|t| t.elapsed() > Duration::from_secs(3600)).unwrap_or(true) {
            self.last_prune = Some(Instant::now());
            self.prune_records();
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
            // An agent that is gone answers nothing more: what was still
            // waiting for it fails now, instead of waiting for good (a file
            // "saving" for ever, and a "Save and close" behind it; a folder
            // that never lists).
            if let Some(ConnStatus::Failed(why)) = self.conns.get(&h).map(|c| c.status.clone()) {
                let waiting: Vec<u64> = self.pending.keys().filter(|(host, _)| host == &h).map(|k| k.1).collect();
                for id in waiting {
                    self.handle_response(&h, json!({"id": id, "ok": false, "error": format!("agent offline: {why}")}));
                }
            }
            // The agent uses BatchMode ssh; if it failed before the interactive
            // pane finished authenticating (host key prompt, etc.), retry once
            // after that pane shows its first prompt.
            let failed = matches!(self.conns.get(&h).map(|c| &c.status), Some(ConnStatus::Failed(_)));
            if failed && !self.agent_retried.contains(&h) {
                let prompted = self.panes.values().any(|p| p.host == h && p.term.shell_prompted());
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
                if self.secrets.remove(&h) {
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

        // settings.json changed on disk: read again (store.rs).
        self.poll_prefs();
        self.check_editor_on_disk();
        if self.last_save.elapsed() > Duration::from_secs(10) {
            self.autosave_settings();
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
        if !self.settings.records.history.on {
            return;
        }
        // Appending keeps it cheap and greppable; the time line is bash's
        // HISTTIMEFORMAT form, used for the keep time.
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.data.path(&format!("history/{}.txt", host::safe_name(host))))
        {
            let _ = writeln!(f, "#{}\n{}", crate::records::now() as i64, cmd.replace('\n', " "));
        }
    }

    fn handle_term_event(&mut self, pane_id: usize, ev: TermEvent) {
        let Some(host) = self.panes.get(&pane_id).map(|p| p.host.clone()) else { return };
        // A serial device is not a shell weft talks to: what it prints never
        // opens files, sets the clipboard or asks for anything (a bell still rings).
        if matches!(self.profile(&host).kind, host::HostKind::Serial { .. }) && !matches!(ev, TermEvent::Bell) {
            return;
        }
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
                if Some(pane_id) == self.active_pane_id() {
                    if let Some(cwd) = self.panes[&pane_id].term.cwd.clone() {
                        self.follow_terminal(&host, &cwd);
                    }
                }
            }
            TermEvent::Weft { anchor, value } => self.handle_weft(pane_id, &host, anchor, value),
            // No integration: read the shell's directory from its process.
            TermEvent::GuessedDone => {
                if let Some(pid) = self.panes.get(&pane_id).and_then(|p| p.shell_pid) {
                    self.request(&host, json!({"op": "cwd", "pid": pid}), Pending::Cwd { pane: pane_id });
                }
            }
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
            // weft's POSIX command runs there (not a Windows host's cmd).
            "sh" => {
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    p.boot_ran();
                }
            }
            // Its integration bound weft's key to empty the line.
            "clear-key" => {
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    p.clear_key = true;
                }
            }
            "shell" => {
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    // A shell starting anew: its own keys are still to be told.
                    p.clear_key = false;
                    p.shell_pid = v.get("pid").and_then(|x| x.as_u64());
                    // The shell it started, not the profile's guess: a nu or
                    // pwsh chosen, or the login shell (POSIX quotes broke
                    // there).
                    if p.lang == host::ShellLang::Posix {
                        p.lang = host::ShellLang::of_started(&s("name"));
                    }
                }
                self.data.log(&format!("pane {pane_id} [{host}] shell {} pid {}", s("name"), v["pid"]));
                self.shell_notice(host, &s("name"));
            }
            // The shell chosen for the host is not there (the bootstrap
            // looked): its login shell runs instead. Said.
            "shell-missing" => {
                let (wanted, using) = (s("wanted"), s("using"));
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    p.started_shell = None;
                }
                self.data.log(&format!("pane {pane_id} [{host}]: the chosen shell {wanted} is not there; its login shell ({using}) instead"));
                if self.shell_notes.insert(format!("{host}\nmissing {wanted}")) {
                    // (Most often there, but not on the PATH a command gets.)
                    let hint = if wanted.starts_with('/') { "" } else { " (not on the PATH a command gets there: give its full path)" };
                    self.toast(format!("{host}: the shell chosen for it, {wanted}, is not there{hint}: its login shell ({using}) is used instead. Shells… (command palette) to choose another"), ToastKind::Info);
                }
            }
            // A remote zsh takes its integration only by typing it (see
            // shell/zsh.zsh): once per shell start, before it has integrated.
            "inject" if s("shell") == "zsh" => {
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    if !p.injected && !p.term.integrated {
                        p.injected = true;
                        p.send(b" eval \"$WEFT_ZSH\"\r");
                        p.lines_sent = p.lines_sent.saturating_sub(1);
                    }
                }
            }
            "widget" => {
                let Some(kind) = parse_widget(&v) else { return };
                let id = s("id");
                // A Files widget that names its folder only (bash's wls): its
                // entries are listed by the host's agent, as Files' are.
                if matches!(kind, WidgetKind::Files { .. }) && v.get("entries").is_none() {
                    let hidden = v.get("hidden").and_then(|x| x.as_bool()).unwrap_or(false);
                    self.request(host, json!({"op": "list", "path": s("dir"), "hidden": hidden, "ignored": true, "mode": true}), Pending::WidgetFiles { pane: pane_id, id: id.clone() });
                }
                let rows = v.get("rows").and_then(|r| r.as_u64()).unwrap_or(1).clamp(1, 2000) as usize;
                let Some(p) = self.panes.get_mut(&pane_id) else { return };
                if let Some(w) = p.widgets.iter_mut().find(|w| w.id == id) {
                    w.kind = kind;
                } else {
                    let (anchor, came) = widget_line(p.widgets.last(), anchor);
                    p.widgets.push(InlineWidget { id, anchor, came, rows, kind, sort: None, chosen: None });
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
            Pending::Xfer { t, step } => self.on_transfer(t, step, ok, &r, err),
            Pending::WidgetFiles { pane, id } => {
                if !ok {
                    self.data.log(&format!("pane {pane} [{host}]: wls: {err}"));
                    return;
                }
                let entries = widget_entries(&r);
                if let Some(w) = self.panes.get_mut(&pane).and_then(|p| p.widgets.iter_mut().find(|w| w.id == id)) {
                    w.kind = WidgetKind::Files { dir: r["path"].as_str().unwrap_or("").into(), entries };
                }
            }
            Pending::SerialPorts { host } => {
                if let Some(d) = self.serial_dialog.as_mut().filter(|d| d.host == host) {
                    d.ports = Some(if ok { Ok(crate::serial_ui::parse_ports(&r)) } else { Err(err) });
                }
            }
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
            Pending::EditorStat { path, generation } => {
                let Some(f) = self.editor.files.iter_mut().find(|f| f.host == host && f.path == path) else { return };
                // Asked before a read or a save of its own: not about it.
                if !ok || generation != f.disk_gen || f.loading || f.saving {
                    return;
                }
                f.outdated = if !r["exists"].as_bool().unwrap_or(true) {
                    Some(Outdated::Gone)
                } else {
                    match (r["mtime"].as_f64(), f.mtime) {
                        (Some(disk), Some(mine)) if (disk - mine).abs() > 0.001 => Some(Outdated::Changed),
                        _ => None,
                    }
                };
            }
            Pending::Open { path, line } => {
                let Some(i) = self.editor.files.iter().position(|f| f.host == host && f.path == path) else { return };
                let f = &mut self.editor.files[i];
                f.loading = false;
                f.outdated = None;
                f.disk_gen += 1;
                // A folder, told so by the agent: shown in Files.
                let is_dir = r.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
                if !ok || is_dir {
                    if is_dir {
                        self.editor.files.remove(i);
                        self.editor.active = self.editor.active.min(self.editor.files.len().saturating_sub(1));
                        self.settings.sidebar = true;
                        self.side.tab = SideTab::Files;
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
                    f.bytes = None;
                } else {
                    use base64::Engine;
                    f.bytes = r["b64"].as_str().and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok()).map(std::sync::Arc::new);
                    f.binary = true;
                    f.text = format!("binary file, {} bytes — not editable", r["size"].as_u64().unwrap_or(0));
                }
                if line > 0 {
                    f.goto = Some(line);
                }
            }
            Pending::Save { path } => {
                // settings.json saved in the editor: applied at once (the
                // timed save reads it first).
                if ok && host == "local" && path.replace('\\', "/") == self.data.path("settings.json").display().to_string().replace('\\', "/") {
                    self.prefs_checked = Instant::now().checked_sub(Duration::from_secs(5)).unwrap_or(self.prefs_checked);
                }
                let Some(f) = self.editor.files.iter_mut().find(|f| f.host == host && f.path == path) else { return };
                f.saving = false;
                if !ok {
                    self.toast(format!("save failed: {err}"), ToastKind::Error);
                } else if r["conflict"].as_bool().unwrap_or(false) {
                    f.conflict = true;
                } else {
                    // On disk is what was sent. Typed since (the answer
                    // takes a moment: the write waits its turn): still
                    // unsaved, and not marked "saved".
                    let same = f.saving_of.take().is_none_or(|sent| sent == text_hash(&f.text));
                    f.dirty = !same;
                    f.mtime = r["mtime"].as_f64();
                    f.outdated = None;
                    f.disk_gen += 1;
                    // Said quietly in the editor's toolbar ("✓ saved"), not a toast.
                    if same {
                        f.saved_at = Some(Instant::now());
                    }
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
                // A late answer from the host shown before: its pids are
                // not this host's (Kill would end the wrong process).
                if host != self.side.procs_host {
                    return;
                }
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
                    // Glob and fuzzy: grep found the lines that may match;
                    // each is matched here.
                    let mut r = r;
                    if let (Some(m), Some(hits)) = (self.side.grep_matcher.clone().filter(|m| m.exact_on_grep()), r["hits"].as_array_mut()) {
                        // A line as long as a host cuts it may go on past what
                        // came: kept (its end can't be told).
                        hits.retain(|h| {
                            let t = h[2].as_str().unwrap_or("");
                            let t = t.strip_suffix('\r').unwrap_or(t);
                            t.chars().count() >= 4000 || !m.find_in(t).is_empty()
                        });
                        if hits.len() > GREP_SHOWN {
                            hits.truncate(GREP_SHOWN);
                            r["truncated"] = json!(true);
                        }
                    }
                    // Each line's marks, found once (not at every frame).
                    if let (Some(m), Some(hits)) = (self.side.grep_matcher.clone(), r["hits"].as_array_mut()) {
                        for h in hits.iter_mut() {
                            let t = h[2].as_str().unwrap_or("");
                            let t = t.strip_suffix('\r').unwrap_or(t);
                            let marks: Vec<[usize; 2]> = m.find_in(t).into_iter().map(|(a, b)| [a, b]).collect();
                            if let Some(a) = h.as_array_mut() {
                                a.push(json!(marks));
                            }
                        }
                    }
                    self.side.grep = Some(r);
                } else {
                    self.toast(format!("search failed: {err}"), ToastKind::Error);
                }
            }
            Pending::Tree { root } => {
                self.find_inflight.remove(&(host.to_string(), root.clone()));
                if ok {
                    let mut entries: Vec<(String, bool)> = r["entries"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|e| Some((e[0].as_str()?.to_string(), e[1].as_bool().unwrap_or(false)))).collect())
                        .unwrap_or_default();
                    entries.sort();
                    let generation = self.find_index.values().map(|i| i.generation).max().unwrap_or(0) + 1;
                    let truncated = r["truncated"].as_bool().unwrap_or(false);
                    self.find_index.insert((host.to_string(), root), FindIndex { entries, truncated, at: Instant::now(), generation });
                } else {
                    self.toast(format!("file filter: {err}"), ToastKind::Error);
                }
            }
            Pending::Cwd { pane } => {
                if let (true, Some(cwd), Some(p)) = (ok, r["cwd"].as_str(), self.panes.get_mut(&pane)) {
                    p.term.set_cwd_from_process(cwd.to_string());
                    let events = std::mem::take(&mut p.term.events);
                    for e in events {
                        self.handle_term_event(pane, e);
                    }
                }
            }
            Pending::FileOp { done, refresh, renamed } => {
                if ok {
                    self.toast(done, ToastKind::Ok);
                    if let Some((from, to)) = renamed {
                        self.file_renamed(host, &from, &to);
                    }
                    for dir in refresh {
                        self.inflight.remove(&(host.to_string(), dir.clone()));
                        self.list_dir(host, &dir);
                    }
                } else {
                    self.toast(err, ToastKind::Error);
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
                    self.settings.sidebar = true;
                    self.side.tab = SideTab::Files;
                    self.set_root(&h, &p);
                }
                Action::Focus(id) => {
                    self.focus_pane(id);
                    self.focus_term = true;
                }
                Action::ToggleFold(id, bid) => {
                    if let Some(p) = self.panes.get_mut(&id) {
                        let before = crate::ui_term::visual_rows(&p.term).len();
                        if let Some(b) = p.term.blocks.iter_mut().find(|b| b.id == bid) {
                            b.folded = !b.folded;
                        }
                        // Scrolled back, what is above the fold stays where
                        // it is: the view is counted from the end, which the
                        // fold has just moved (the command clicked on slid
                        // away from under the pointer, or off the screen).
                        if p.scroll > 0 {
                            let after = crate::ui_term::visual_rows(&p.term).len();
                            p.scroll = (p.scroll + after).saturating_sub(before);
                        }
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
                Action::SerialHere(host) => self.open_serial_dialog_on(Some(&host)),
                Action::SerialLog(host) => self.open_serial_log(Some(&host)),
                Action::ClosePane(id) => self.ask_close_panes(vec![id]),
                Action::Restart(id) => {
                    if let Some(mut p) = self.panes.remove(&id) {
                        p.initial_cwd = p.term.cwd.clone().or(p.initial_cwd.clone());
                        // The old one let go first: a serial port opens once
                        // at a time (Windows refused it, its own tab still
                        // holding it).
                        // A port of this PC still held: its reader and
                        // writer let it go within a tenth of a second.
                        let held_port = p.pty.as_ref().is_some_and(|t| t.is_port());
                        drop(p.pty.take());
                        if held_port {
                            std::thread::sleep(std::time::Duration::from_millis(250));
                        }
                        self.start_pty(&mut p);
                        self.panes.insert(id, p);
                    }
                }
                Action::Grep => {
                    let Some((host, root)) = self.side.root.clone() else { continue };
                    if self.side.grep_q.is_empty() {
                        continue;
                    }
                    // The host's grep is asked for the lines that may match;
                    // the matcher decides (crate::find), as everywhere.
                    let mode = self.side.grep_mode;
                    let m = match crate::find::Matcher::build(mode, &self.side.grep_q, crate::find::Target::Line, Some(!self.side.grep_icase)) {
                        Ok(m) => m,
                        Err(e) => {
                            self.toast(format!("search: {e}"), ToastKind::Error);
                            continue;
                        }
                    };
                    self.side.grep_busy = true;
                    self.side.grep = None;
                    self.side.grep_ran = Some((mode, self.side.grep_q.clone(), self.side.grep_icase));
                    let (pattern, regex, icase) = m.grep();
                    // Glob and fuzzy: a remote grep returns more lines, and
                    // whole ones, for weft to decide (this PC's decides itself:
                    // "match").
                    let wide = m.exact_on_grep();
                    let req = json!({"op": "grep", "pattern": pattern, "path": root,
                                     "icase": icase, "regex": regex, "hidden": self.side.grep_hidden,
                                     "limit": GREP_SHOWN, "scan": if wide { GREP_SCAN } else { GREP_SHOWN }, "line_max": if wide { 4000 } else { 400 },
                                     "match": {"mode": mode, "query": self.side.grep_q, "case": !self.side.grep_icase}});
                    self.side.grep_matcher = Some(std::sync::Arc::new(m));
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
                Action::NewMemo => self.new_memo(),
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

/// A Files widget's entries from an agent's list: folders first, by name
/// as ls sorts in no locale of its own (its case aside).
fn widget_entries(r: &Value) -> Vec<WEntry> {
    let dir = r["path"].as_str().unwrap_or("");
    let mut out: Vec<WEntry> = r["entries"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|e| {
            let name = e["n"].as_str().unwrap_or("");
            let isdir = e["d"].as_bool().unwrap_or(false);
            WEntry {
                name: if isdir { format!("{name}/") } else { name.to_string() },
                path: format!("{}/{name}", dir.trim_end_matches(['/', '\\'])),
                dir: isdir,
                size: e["s"].as_u64().unwrap_or(0),
                mtime: e["m"].as_i64().unwrap_or(0),
                mode: e["p"].as_str().unwrap_or("").into(),
            }
        })
        .collect();
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Columns of a command's text (ps aux, df -h): split at tabs where its
/// first line has one, else at runs of white space, the last column taking
/// the rest of its line (a command line with spaces). The first line names
/// them; blank lines are none.
fn table_of_text(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let Some(first) = lines.first() else { return (Vec::new(), Vec::new()) };
    let tab = first.contains('\t');
    let head: Vec<String> = if tab { first.split('\t').map(String::from).collect() } else { first.split_whitespace().map(String::from).collect() };
    let n = head.len().max(1);
    let data = lines[1..]
        .iter()
        .map(|l| {
            let mut cells: Vec<String> = if tab {
                l.split('\t').map(String::from).collect()
            } else {
                let mut cells = Vec::new();
                let mut rest = l.trim_start();
                while !rest.is_empty() && cells.len() + 1 < n {
                    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                    cells.push(rest[..end].to_string());
                    rest = rest[end..].trim_start();
                }
                if !rest.is_empty() {
                    cells.push(rest.trim_end().to_string());
                }
                cells
            };
            cells.resize(head.len().max(cells.len()), String::new());
            cells
        })
        .collect();
    (head, data)
}

fn parse_widget(v: &Value) -> Option<WidgetKind> {
    let strs = |a: &Value| -> Vec<String> {
        a.as_array().map(|a| a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect()).unwrap_or_default()
    };
    match v.get("kind")?.as_str()? {
        "files" => Some(WidgetKind::Files {
            dir: v["dir"].as_str().unwrap_or("").into(),
            // None yet where the host's agent lists them (wls).
            entries: v["entries"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or_default()
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
        // As text (bash's wtable: split here), or split already.
        "table" => Some(match v["text"].as_str() {
            Some(text) => {
                let (columns, data) = table_of_text(text);
                WidgetKind::Table { columns, data }
            }
            None => WidgetKind::Table { columns: strs(&v["columns"]), data: v["data"].as_array()?.iter().map(strs).collect() },
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

    /// A command's text split into columns as it was printed: at runs of
    /// white space (the last column keeps its spaces), or at tabs; short
    /// rows filled out.
    #[test]
    fn a_tables_text_is_split_into_its_columns() {
        let (h, d) = table_of_text("USER  PID COMMAND\nroot    1 /sbin/init splash\n\nme     42 vim\n");
        assert_eq!(h, ["USER", "PID", "COMMAND"]);
        assert_eq!(d, [vec!["root", "1", "/sbin/init splash"], vec!["me", "42", "vim"]]);
        let (h, d) = table_of_text("a\tb c\tz\n1\t2 3\n");
        assert_eq!(h, ["a", "b c", "z"]);
        assert_eq!(d, [vec!["1", "2 3", ""]]);
        assert_eq!(table_of_text("\n \n"), (Vec::<String>::new(), Vec::<Vec<String>>::new()));
    }

    /// wls' entries come from the agent's list: folders first, by name,
    /// each with its path and mode.
    #[test]
    fn wls_entries_come_from_the_agents_list() {
        let r = serde_json::json!({"path": "/d/", "entries": [
            {"n": "b.txt", "d": false, "l": false, "s": 5, "m": 9, "p": "-rw-r--r--"},
            {"n": "Sub", "d": true, "l": false, "s": 0, "m": 1, "p": "drwxr-xr-x"},
            {"n": "a.txt", "d": false, "l": false, "s": 1, "m": 2},
        ]});
        let e = widget_entries(&r);
        assert_eq!(e.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Sub/", "a.txt", "b.txt"]);
        assert_eq!(e[0].path, "/d/Sub");
        assert_eq!((e[2].size, e[2].mode.as_str()), (5, "-rw-r--r--"));
    }

    /// A process list that arrives late from the host shown before is not
    /// taken for this host's: Kill would end a process here with a pid from
    /// there.
    #[test]
    fn a_widget_goes_below_one_whose_text_has_not_come() {
        let w = |anchor: u64, came: u64, rows: usize| InlineWidget { id: String::new(), anchor, came, rows, kind: WidgetKind::Progress { pct: 0.0, label: String::new() }, sort: None, chosen: None };
        // The first, or one that comes after the text of the one before:
        // where the cursor is.
        assert_eq!(widget_line(None, 7), (7, 7));
        assert_eq!(widget_line(Some(&w(0, 0, 3)), 3), (3, 3));
        assert_eq!(widget_line(Some(&w(0, 0, 3)), 40), (40, 40));
        // The cursor still on the first one's line, or part way through
        // its text: below it. And so on down a run of them: the third
        // comes with the cursor still where the first came.
        assert_eq!(widget_line(Some(&w(0, 0, 3)), 0), (3, 0));
        assert_eq!(widget_line(Some(&w(0, 0, 3)), 2), (3, 0));
        assert_eq!(widget_line(Some(&w(3, 0, 1)), 0), (4, 0));
        assert_eq!(widget_line(Some(&w(4, 0, 3)), 0), (7, 0));
        // The text of the first two has come by now: the cursor is where
        // the third is to be, and the fourth goes below that.
        assert_eq!(widget_line(Some(&w(4, 0, 3)), 4), (7, 0));
        // A cursor above where the run began is somewhere else (a program
        // drawing higher up on the screen): left where it is.
        assert_eq!(widget_line(Some(&w(13, 10, 3)), 8), (8, 8));
    }

    #[test]
    fn a_process_list_from_another_host_is_dropped() {
        let mut h = crate::testkit::Harness::new(json!({}));
        let row = json!([4242, 1, "root", 0.0, 0.1, "00:01", "sshd"]);
        h.app.side.procs_host = "local".into();
        h.app.side.procs = None;
        h.app.pending.insert(("elsewhere".into(), 7), Pending::Procs);
        h.app.handle_response("elsewhere", json!({"id": 7, "ok": true, "procs": [row.clone()]}));
        assert!(h.app.side.procs.is_none(), "another host's list is not shown");
        h.app.pending.insert(("local".into(), 8), Pending::Procs);
        h.app.handle_response("local", json!({"id": 8, "ok": true, "procs": [row]}));
        assert_eq!(h.app.side.procs.as_ref().and_then(|p| p.as_ref().ok()).map(|p| p.len()), Some(1));
    }

    /// Typed on while a save was on its way: when the answer comes the file
    /// is still unsaved, and closing it asks (it was marked clean, and what
    /// was typed after Ctrl+S was dropped without a word).
    #[test]
    fn a_save_answered_after_more_typing_leaves_the_file_unsaved() {
        let mut h = crate::testkit::Harness::new(json!({}));
        let open = |h: &mut crate::testkit::Harness, path: &str, text: &str| {
            h.app.open_scratch("x", text.into());
            let f = h.app.editor.files.last_mut().unwrap();
            (f.host, f.path, f.scratch, f.dirty) = ("local".into(), path.into(), false, true);
            h.app.editor.files.len() - 1
        };
        // Saved, then typed on before the answer.
        let i = open(&mut h, "/d/a.txt", "one");
        h.app.editor.files[i].saving = true;
        h.app.editor.files[i].saving_of = Some(text_hash("one"));
        h.app.editor.files[i].text.push_str(" two");
        h.app.pending.insert(("local".into(), 21), Pending::Save { path: "/d/a.txt".into() });
        h.app.handle_response("local", json!({"id": 21, "ok": true, "mtime": 7.0}));
        let f = &h.app.editor.files[i];
        assert!(f.dirty && !f.saving && f.saved_at.is_none(), "still unsaved, and not marked saved");
        assert_eq!(f.mtime, Some(7.0), "the file on disk is the one that was written");
        // Saved and left alone: clean.
        let j = open(&mut h, "/d/b.txt", "same");
        h.app.editor.files[j].saving = true;
        h.app.editor.files[j].saving_of = Some(text_hash("same"));
        h.app.pending.insert(("local".into(), 22), Pending::Save { path: "/d/b.txt".into() });
        h.app.handle_response("local", json!({"id": 22, "ok": true, "mtime": 8.0}));
        let f = &h.app.editor.files[j];
        assert!(!f.dirty && f.saved_at.is_some());
    }

    /// The agent died with a save on its way: the save fails (and says so)
    /// instead of staying "saving" for good, with a "Save and close"
    /// waiting behind it.
    #[test]
    fn what_waited_for_a_dead_agent_fails() {
        let mut h = crate::testkit::Harness::new(json!({}));
        // The harness's host "t" has an agent that exits at once.
        assert!(h.wait_for(20.0, |h| matches!(h.app.conns.get("t").map(|c| &c.status), Some(ConnStatus::Failed(_)))), "the agent is gone");
        h.app.open_scratch("x", "text".into());
        let f = h.app.editor.files.last_mut().unwrap();
        (f.host, f.path, f.scratch, f.saving, f.dirty) = ("t".into(), "/d/a.txt".into(), false, true, true);
        h.app.pending.insert(("t".into(), 9001), Pending::Save { path: "/d/a.txt".into() });
        h.frames(3);
        let f = h.app.editor.files.last().unwrap();
        assert!(!f.saving && f.dirty, "the save failed: not saving any more, still unsaved");
        assert!(!h.app.pending.contains_key(&("t".to_string(), 9001)));
        assert!(h.app.toasts.iter().any(|t| t.text.contains("save failed")), "and it is said");
        // A request made now is answered too.
        h.app.save_file(h.app.editor.files.len() - 1, false);
        h.frames(3);
        assert!(!h.app.editor.files.last().unwrap().saving);
    }

    /// A file renamed while its save is on its way: the answer still finds
    /// it (it stayed "saving" for good, and a later "Save and close" waited
    /// on it for ever).
    #[test]
    fn a_save_finds_its_file_after_a_rename() {
        let mut h = crate::testkit::Harness::new(json!({}));
        h.app.open_scratch("x", "text".into());
        let f = h.app.editor.files.last_mut().unwrap();
        (f.host, f.path, f.scratch, f.saving, f.dirty) = ("local".into(), "/d/old.txt".into(), false, true, true);
        h.app.pending.insert(("local".into(), 9), Pending::Save { path: "/d/old.txt".into() });
        h.app.file_renamed("local", "/d/old.txt", "/d/new.txt");
        h.app.handle_response("local", json!({"id": 9, "ok": true, "mtime": 5.0}));
        let f = h.app.editor.files.last().unwrap();
        assert_eq!((f.path.as_str(), f.saving, f.dirty), ("/d/new.txt", false, false));
        // A folder renamed: what is under it follows too.
        h.app.pending.insert(("local".into(), 10), Pending::Save { path: "/d/new.txt".into() });
        h.app.file_renamed("local", "/d", "/e");
        assert_eq!(h.app.editor.files.last().unwrap().path, "/e/new.txt");
        assert!(matches!(h.app.pending.get(&("local".to_string(), 10)), Some(Pending::Save { path }) if path == "/e/new.txt"));
    }

    #[test]
    fn memo_list_newest_first_with_titles() {
        let d = std::env::temp_dir().join(format!("weft-memos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("memo-1.md"), "# 회의 메모\n\n결정: 금요일 배포\n").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(d.join("memo-2.md"), "- [ ] buy milk\n- [ ] eggs\n").unwrap();
        std::fs::write(d.join("notes.txt"), "not a memo").unwrap();
        let list = read_memos(&d.display().to_string());
        let got: Vec<(&str, &str)> = list.iter().map(|m| (m.title.as_str(), m.preview.as_str())).collect();
        assert_eq!(got, [("[ ] buy milk", "- [ ] eggs"), ("회의 메모", "결정: 금요일 배포")]);
        assert!(list[0].path.ends_with("memo-2.md") && list[0].modified >= list[1].modified);
        let _ = std::fs::remove_dir_all(&d);
    }

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
