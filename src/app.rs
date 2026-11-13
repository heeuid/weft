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

/// Tests that build an App (it reads WEFT_DATA_DIR) take turns.
#[cfg(test)]
pub static TEST_APP: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    /// File panel filter: fuzzy, glob or regex.
    #[serde(default)]
    pub find_mode: crate::find::FindMode,
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
        Settings { font_size: d_font(), ui_scale: 1.0, profiles, sidebar: true, layout: Layout::default(), show_hidden: false, show_ignored: true, find_mode: Default::default(), palette_mode: d_palette_mode(), cursor_style: Default::default(), local_shell: Default::default(), mermaid: Default::default(), follow_cwd: true, theme: d_theme(), theme_dark: d_theme_dark(), theme_light: d_theme_light(), markdown_breaks: false, term_font: d_font_file(), icon_font: d_icon_font(), records: Default::default(), passwords: Default::default(), keys: Default::default(), session: Vec::new(), side_tabs: Default::default(), open: Default::default(), confirm_unsaved: true, confirm_running: true }
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
    pub rows: usize,
    pub kind: WidgetKind,
    pub sort: Option<(usize, bool)>,
    pub chosen: Option<usize>,
}

pub struct Pane {
    /// Synchronized output (DEC 2026) held until its update is whole.
    pub sync: crate::term::SyncOutput,
    /// The remote shell's process (from the bootstrap), for its directory
    /// when the shell has no integration.
    pub shell_pid: Option<u64>,
    /// The zsh integration was typed in for this shell start.
    pub injected: bool,
    /// Startup timing logged: first output, first prompt.
    pub seen: (bool, bool),
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
    /// Caret position last reported to the IME (kept while composing).
    pub ime_rect: Option<Rect>,
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
            injected: false,
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
            ime_rect: None,
            pending_size: None,
            sized: false,
            hist_seen: 0,
            copy: None,
            rec_seen: 0,
            rec_cwd: None,
            seen: (false, false),
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
        // Enter starts a command for shells that don't mark it themselves.
        if bytes.contains(&b'\r') {
            self.term.user_enter();
        }
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
    pub grep_busy: bool,
    pub grep: Option<Value>,
    pub procs: Option<Result<Vec<Value>, String>>,
    pub procs_at: Option<Instant>,
    pub procs_filter: String,
    pub procs_sort: (usize, bool),
    pub procs_host: String,
    /// Path selected in the file tree (single click); opening needs a double-click.
    pub selected: Option<String>,
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

pub enum Pending {
    List { path: String },
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
    pub key: (String, String, String, crate::find::FindMode, u64),
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
    /// New folder / new file / rename / delete, while its dialog is open.
    pub file_op: Option<crate::fileops::FileOp>,
    /// A question before something is lost (confirm.rs), while it is asked;
    /// what goes on once the files it saves are saved; and weft may close
    /// (it was answered).
    pub confirm: Option<crate::confirm::Confirm>,
    pub after_save: Option<crate::confirm::AfterSave>,
    pub quit_confirmed: bool,
    /// A file or folder (host, path) being dragged from a Files panel.
    pub file_drag: Option<(String, String)>,
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
    pub fn new(ctx: egui::Context) -> App {
        let data = DataDir::locate();
        let mut settings: Settings = std::fs::read_to_string(data.path("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // D2Coding became the default after JetBrains Mono. Settings still on
        // the old default for both fonts were never changed by hand: move them on.
        if settings.term_font == "builtin:jetbrains-mono" && settings.icon_font == "builtin:jetbrains-mono" {
            settings.term_font = crate::theme::DEFAULT_FONT.into();
        }
        crate::records::set_switches(&settings.records);
        let settings_passwords = settings.passwords;
        data.log(&format!("start: {}, data dir {}", crate::version::line(), data.root.display()));
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
                grep_mode: Default::default(),
                grep_hidden: false,
                grep_ran: None,
                grep_busy: false,
                grep: None,
                procs: None,
                procs_at: None,
                procs_filter: String::new(),
                procs_sort: (3, false),
                procs_host: String::new(),
                selected: None,
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
            file_op: None,
            confirm: None,
            after_save: None,
            quit_confirmed: false,
            file_drag: None,
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
            self.side_views.insert(n, SideView { root: v.root, follow: v.follow, selected: None });
        }
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
        self.settings.open = self.saved_open();
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
            let conn = Conn::open(self.profile(host), &self.data, self.wake(), secret);
            self.conns.insert(host.to_string(), conn);
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
                .map(|s| s.lines().filter_map(crate::records::history_command).map(String::from).collect())
                .unwrap_or_default();
            self.history.insert(host.to_string(), hist);
        }
        id
    }

    fn start_pty(&mut self, pane: &mut Pane) {
        let prof = self.profile(&pane.host);
        if let host::HostKind::Serial { via, port, baud } = prof.kind.clone() {
            self.start_serial(pane, &via, &port, baud);
            return;
        }
        let secret = if prof.is_local() { None } else { self.secrets.get(&pane.host) };
        let mut argv = prof.pty_argv(pane.initial_cwd.as_deref(), secret.is_some());
        let mut env = secret.as_deref().map(|s| host::askpass_env(s)).unwrap_or_default();
        if prof.is_local() {
            let (a, e) = host::local_shell(self.settings.local_shell);
            argv = a;
            env.extend(e);
        }
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
        pane.injected = false;
        pane.shell_pid = None;
        pane.parser = vte::Parser::new();
        pane.widgets.clear();
        pane.hist_seen = 0;
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
        pane.term = Term::new(pane.term.cols, pane.term.rows);
        pane.injected = true; // no shell integration to inject
        pane.shell_pid = None;
        pane.parser = vte::Parser::new();
        pane.widgets.clear();
        pane.hist_seen = 0;
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
            let argv = through.serial_argv(port, baud, secret.is_some());
            let env = secret.as_deref().map(|s| host::askpass_env(s)).unwrap_or_default();
            self.data.log(&format!("pane {} serial {via}:{port} at {baud} over {}", pane.id, argv.first().cloned().unwrap_or_default()));
            if argv.is_empty() {
                Err(format!("{via}: serial consoles go through ssh or a custom transport"))
            } else {
                Pty::spawn(&argv, None, &env, pane.term.cols as u16, pane.term.rows as u16, self.wake_term())
            }
        };
        match result {
            Ok(p) => {
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
            if let Some(cwd) = p.term.cwd.clone() {
                self.follow_terminal(&host, &cwd);
            }
        }
    }

    /// The terminal moved to `host:cwd`: every side panel following it goes
    /// there, whichever was clicked last (only that one used to, so a click
    /// in a second side panel that stays put stopped the first following).
    pub fn follow_terminal(&mut self, host: &str, cwd: &str) {
        let new = Some((host.to_string(), cwd.to_string()));
        let mut list = false;
        for (n, v) in self.side_views.iter_mut() {
            if v.follow && v.root != new && *n != self.side_focus {
                v.root = new.clone();
                list = true;
            }
        }
        if self.side.follow {
            self.set_root(host, cwd);
        } else if list {
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
            binary: false,
            scratch: false,
            loading: true,
            saving: false,
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
        self.request(host, json!({"op": "read", "path": path, "max": crate::media::read_limit(&path)}), Pending::Open { path: path.into(), line });
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
            binary: false,
            scratch: true,
            loading: false,
            saving: false,
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
            binary: false,
            scratch: false,
            loading: false,
            saving: false,
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
        self.request(&host, json!({"op": "read", "path": path, "max": crate::media::read_limit(&path)}), Pending::Open { path, line: 0 });
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
        self.autosave_memos();
        let (forgotten, next) = self.secrets.tick();
        for h in forgotten {
            self.data.log(&format!("{h}: unused password forgotten"));
        }
        if let Some(d) = next {
            self.ctx.request_repaint_after(d.max(Duration::from_millis(200)));
        }
        let mut events: Vec<(usize, TermEvent)> = Vec::new();
        let mut new_history: Vec<(String, String)> = Vec::new();
        let mut new_records: Vec<crate::records::Record> = Vec::new();
        let mut timings: Vec<String> = Vec::new();
        let mut guessing = false;
        for (id, pane) in self.panes.iter_mut() {
            // Shells without OSC 133: prompts and command ends are guessed when
            // output settles, so look again shortly even without new output.
            guessing |= pane.term.tick();
            if let Some(pty) = pane.pty.as_mut() {
                // An app's synchronized update (DEC 2026) comes in whole.
                let now = Instant::now();
                let bytes = pane.sync.push(&pty.read(), now);
                if let Some(d) = pane.sync.due(now) {
                    self.ctx.request_repaint_after(d);
                }
                if !bytes.is_empty() {
                    let before = pane.term.end_line();
                    pane.term.feed(&mut pane.parser, &bytes);
                    if !pane.seen.0 {
                        pane.seen.0 = true;
                        timings.push(format!("startup: pane {id} [{}] first output at {}", pane.host, crate::since_start()));
                    }
                    if !pane.seen.1 && !pane.term.blocks.is_empty() {
                        pane.seen.1 = true;
                        timings.push(format!("startup: pane {id} [{}] first prompt at {}", pane.host, crate::since_start()));
                    }
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
                    pane.rec_cwd = Some((b.id, pane.term.cwd.clone().or(pane.initial_cwd.clone()).unwrap_or_default()));
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
            "shell" => {
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    p.shell_pid = v.get("pid").and_then(|x| x.as_u64());
                }
                self.data.log(&format!("pane {pane_id} [{host}] shell {} pid {}", s("name"), v["pid"]));
            }
            // A remote zsh takes its integration only by typing it (see
            // shell/zsh.zsh): once per shell start, before it has integrated.
            "inject" if s("shell") == "zsh" => {
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    if !p.injected && !p.term.integrated {
                        p.injected = true;
                        p.send(b" eval \"$WEFT_ZSH\"\r");
                    }
                }
            }
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
            Pending::Xfer { t, step } => self.on_transfer(t, step, ok, &r, err),
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
                let Some(f) = self.editor.files.iter_mut().find(|f| f.host == host && f.path == path) else { return };
                f.saving = false;
                if !ok {
                    self.toast(format!("save failed: {err}"), ToastKind::Error);
                } else if r["conflict"].as_bool().unwrap_or(false) {
                    f.conflict = true;
                } else {
                    f.dirty = false;
                    f.mtime = r["mtime"].as_f64();
                    // Said quietly in the editor's toolbar ("✓ saved"), not a toast.
                    f.saved_at = Some(Instant::now());
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
                Action::ClosePane(id) => self.ask_close_panes(vec![id]),
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
                    // Fuzzy and glob go as the extended regex they mean, so a
                    // remote grep -E and the local search agree.
                    let mode = self.side.grep_mode;
                    self.side.grep_ran = Some((mode, self.side.grep_q.clone(), self.side.grep_icase));
                    let regex = mode != crate::editfind::MatchMode::Text;
                    let pattern = if regex { crate::editfind::to_regex(mode, &self.side.grep_q, false) } else { self.side.grep_q.clone() };
                    let req = json!({"op": "grep", "pattern": pattern, "path": root,
                                     "icase": self.side.grep_icase, "regex": regex, "hidden": self.side.grep_hidden});
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
