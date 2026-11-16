//! Chrome around the terminals: tabs, status bar, side panel, editor,
//! command palette, connect dialog, toasts and global shortcuts.

use crate::app::*;
use crate::find::{self, FindMode};
use crate::editfind;
use crate::highlight;
use crate::md;
use crate::host::{ConnStatus, HostKind};
use crate::icons::{self, Icon};
use crate::term::BlockState;
use crate::theme;
use crate::ui_term::{fmt_dur, take_key, visual_rows, VRow};
use crate::widgets::{human_size, text_fit, text_width};
use egui::text::LayoutJob;
use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Id, Key, Layout, Modifiers, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub fn host_color(host: &str) -> Color32 {
    if host == "local" {
        return theme::text_dim();
    }
    let palette = [theme::accent(), theme::ansi(6), theme::ansi(5), theme::ansi(3), theme::ansi(2), theme::ansi(9)];
    let h = host.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    palette[(h as usize) % palette.len()]
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Cmd {
    LocalShell(crate::host::LocalShell),
    CursorStyle,
    MarkdownBreaks,
    NewTab,
    Connect,
    SplitRight,
    SplitDown,
    ClosePane,
    Zoom,
    Sidebar,
    Editor,
    Find,
    Composer,
    About,
    DisplayTest,
    Trace,
    ConfirmUnsaved,
    ConfirmRunning,
    PrevBlock,
    NextBlock,
    FoldAll,
    UnfoldAll,
    ClearScrollback,
    Reconnect,
    FontUp,
    FontReset,
    UiUp,
    UiDown,
    UiReset,
    FontDown,
    Theme(usize),
    ThemeSystem,
    ThemePicker,
    Hidden,
    Settings,
    AgentLog,
    NewMemo,
    Memos,
    OpenMemo,
    Fonts,
    History,
    Records,
    Help,
    Features,
    Palette,
    Keys,
    NextTab,
    PrevTab,
    /// Tab n (0-based); `usize::MAX` is the last tab.
    GoTab(usize),
    /// Another side panel or editor beside the focused one (true: to the right).
    PanelSplit(Region, bool),
    /// Start the newest weft (a new build or install) with the same tabs and settings.
    Restart,
    /// Open a serial port (UART) of this PC or a remote host in a tab.
    Serial,
    /// X11 forwarding on/off for the active tab's ssh host (new tabs).
    X11,
    /// Move the active pane's border: (panes side by side, toward the
    /// right or down).
    Resize(bool, bool),
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    CopyMode,
    FocusMode,
}

/// Every command: (command, id for settings "keys", label, default keys).
const COMMANDS: &[(Cmd, &str, &str, &[&str])] = &[
    (Cmd::NewTab, "new_tab", "New tab (same host)", &["Ctrl+Shift+T"]),
    (Cmd::Connect, "connect", "Connect to host…", &["Ctrl+Shift+N"]),
    (Cmd::Serial, "serial", "Serial console… (UART: a COM port here or a /dev/tty port on a remote host)", &[]),
    (Cmd::X11, "x11", "X11 forwarding on/off for this host (GUI programs there show here; new tabs)", &[]),
    (Cmd::SplitRight, "split_right", "Split right", &["Ctrl+Shift+D"]),
    (Cmd::SplitDown, "split_down", "Split down", &["Ctrl+Shift+E"]),
    (Cmd::ClosePane, "close_pane", "Close pane", &["Ctrl+Shift+W"]),
    (Cmd::Zoom, "zoom", "Zoom / restore pane", &["Ctrl+Shift+Z"]),
    (Cmd::FocusMode, "focus_mode", "Focus mode: terminals only (again to leave)", &["Ctrl+Shift+Enter"]),
    (Cmd::NextTab, "next_tab", "Next tab", &["Ctrl+Tab"]),
    (Cmd::PrevTab, "prev_tab", "Previous tab", &["Ctrl+Shift+Tab"]),
    (Cmd::GoTab(0), "tab_1", "Go to tab 1", &["Alt+1"]),
    (Cmd::GoTab(1), "tab_2", "Go to tab 2", &["Alt+2"]),
    (Cmd::GoTab(2), "tab_3", "Go to tab 3", &["Alt+3"]),
    (Cmd::GoTab(3), "tab_4", "Go to tab 4", &["Alt+4"]),
    (Cmd::GoTab(4), "tab_5", "Go to tab 5", &["Alt+5"]),
    (Cmd::GoTab(5), "tab_6", "Go to tab 6", &["Alt+6"]),
    (Cmd::GoTab(6), "tab_7", "Go to tab 7", &["Alt+7"]),
    (Cmd::GoTab(7), "tab_8", "Go to tab 8", &["Alt+8"]),
    (Cmd::GoTab(usize::MAX), "tab_last", "Go to the last tab", &["Alt+9"]),
    (Cmd::Resize(true, false), "resize_left", "Resize: move the pane border left (split)", &["Alt+Shift+Left"]),
    (Cmd::Resize(true, true), "resize_right", "Resize: move the pane border right (split)", &["Alt+Shift+Right"]),
    (Cmd::Resize(false, false), "resize_up", "Resize: move the pane border up (split)", &["Alt+Shift+Up"]),
    (Cmd::Resize(false, true), "resize_down", "Resize: move the pane border down (split)", &["Alt+Shift+Down"]),
    (Cmd::FocusLeft, "focus_left", "Focus the pane on the left (split)", &["Alt+Left"]),
    (Cmd::FocusRight, "focus_right", "Focus the pane on the right (split)", &["Alt+Right"]),
    (Cmd::FocusUp, "focus_up", "Focus the pane above (split)", &["Alt+Up"]),
    (Cmd::FocusDown, "focus_down", "Focus the pane below (split)", &["Alt+Down"]),
    (Cmd::Composer, "composer", "Open composer (GUI command line)", &["Ctrl+Space"]),
    (Cmd::Find, "find", "Find in scrollback", &["Ctrl+Shift+F"]),
    (Cmd::CopyMode, "copy_mode", "Copy mode: select with the keyboard (vim keys; again to leave)", &["Ctrl+Shift+X"]),
    (Cmd::PrevBlock, "prev_command", "Jump to previous command", &["Ctrl+Shift+Up"]),
    (Cmd::NextBlock, "next_command", "Jump to next command", &["Ctrl+Shift+Down"]),
    (Cmd::FoldAll, "fold_all", "Fold all command outputs", &[]),
    (Cmd::UnfoldAll, "unfold_all", "Unfold all command outputs", &[]),
    (Cmd::ClearScrollback, "clear_scrollback", "Clear scrollback", &[]),
    (Cmd::Sidebar, "side_panel", "Toggle side panel", &["Ctrl+Shift+B"]),
    (Cmd::PanelSplit(Region::Side, false), "split_side_down", "Side panel: another one below", &[]),
    (Cmd::PanelSplit(Region::Side, true), "split_side_right", "Side panel: another one to the right", &[]),
    (Cmd::PanelSplit(Region::Editor, false), "split_editor_down", "Editor: another one below (shows the same file)", &[]),
    (Cmd::PanelSplit(Region::Editor, true), "split_editor_right", "Editor: another one to the right (shows the same file)", &[]),
    (Cmd::Editor, "editor", "Toggle editor", &["Ctrl+Shift+J"]),
    (Cmd::NewMemo, "new_memo", "New memo (markdown, saved automatically)", &["Ctrl+Shift+M"]),
    (Cmd::OpenMemo, "open_memo", "Open memo… (list, search)", &[]),
    (Cmd::Memos, "memos", "Show memos in Files", &[]),
    (Cmd::Hidden, "hidden_files", "Toggle hidden files", &[]),
    (Cmd::Reconnect, "reconnect", "Reconnect agent for this host", &[]),
    (Cmd::FontUp, "font_bigger", "Bigger font", &["Ctrl+=", "Ctrl+Plus"]),
    (Cmd::FontDown, "font_smaller", "Smaller font", &["Ctrl+-"]),
    (Cmd::FontReset, "font_default", "Default font size", &["Ctrl+0"]),
    (Cmd::UiUp, "ui_bigger", "Bigger UI (icons, tabs, panels)", &[]),
    (Cmd::UiDown, "ui_smaller", "Smaller UI (icons, tabs, panels)", &[]),
    (Cmd::UiReset, "ui_default", "UI scale 100%", &[]),
    (Cmd::ThemePicker, "theme", "Theme…", &[]),
    (Cmd::Fonts, "fonts", "Fonts… (terminal font, icon font)", &[]),
    (Cmd::History, "history", "History: search every command and its output", &["Ctrl+Shift+H"]),
    (Cmd::Records, "privacy", "Privacy… (records, logs, passwords in memory)", &[]),
    (Cmd::Help, "help", "Help: all commands and keys", &["F1"]),
    (Cmd::Features, "features", "Help: what each feature does", &[]),
    (Cmd::Palette, "palette", "Command palette", &["Ctrl+Shift+P"]),
    (Cmd::Keys, "keys", "Keyboard shortcuts… (bind keys to commands)", &[]),
    (Cmd::Restart, "restart", "Restart weft: the newest build or install, same tabs, folders and settings", &[]),
    (Cmd::ThemeSystem, "theme_system", "Theme: follow system (dark/light)", &[]),
    (Cmd::CursorStyle, "cursor_style", "Terminal cursor: block / bar / underline", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Auto), "shell_auto", "Local shell: auto (pwsh, else Windows PowerShell)", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Pwsh), "shell_pwsh", "Local shell: PowerShell 7 (pwsh)", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Powershell), "shell_powershell", "Local shell: Windows PowerShell 5.1", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Cmd), "shell_cmd", "Local shell: cmd", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Nu), "shell_nu", "Local shell: nushell", &[]),
    (Cmd::MarkdownBreaks, "markdown_breaks", "Markdown: toggle line breaks (GitHub PR style / CommonMark)", &[]),
    (Cmd::About, "about", "About weft: which version this is, and which console host", &[]),
    (Cmd::DisplayTest, "display_test", "Display test: see how blocks, colours, columns and boxes are drawn", &[]),
    (Cmd::Trace, "record_terminal", "Record this terminal: what the program sends, what you type, where the cursor is drawn (to a file in data/logs; again to stop)", &[]),
    (Cmd::ConfirmUnsaved, "confirm_unsaved", "Ask before closing unsaved files: on / off", &[]),
    (Cmd::ConfirmRunning, "confirm_running", "Ask before ending running commands: on / off", &[]),
    (Cmd::Settings, "settings_file", "Open settings.json", &[]),
    (Cmd::AgentLog, "agent_log", "Open agent log for this host", &[]),
];

#[derive(Clone, Default)]
struct HlCache {
    key: u64,
    job: Option<Arc<LayoutJob>>,
    /// Per-line highlighting kept between edits of this file.
    state: Arc<std::sync::Mutex<highlight::FileState>>,
}

/// A laid-out text kept between frames. Asking egui for it each frame means
/// copying the whole layout job and hashing it to find the galley egui
/// already has: 12 ms a frame for an 8 000-line source file (its colours are
/// 216 000 sections), every frame it is on screen.
///
/// It holds for as long as the fonts are the ones it was laid out with:
/// egui makes new ones, with a new atlas, when the scale changes or the
/// atlas fills up, and the glyphs of an old galley point into the old
/// atlas. The atlas it was made with is kept, to tell (kept, not just its
/// address: a new one can't then be given the same address).
#[derive(Clone, Default)]
struct KeptGalley {
    key: u64,
    galley: Option<Arc<egui::Galley>>,
    atlas: Option<Arc<egui::mutex::Mutex<egui::epaint::TextureAtlas>>>,
}

#[cfg(test)]
thread_local! {
    /// How often an editor's text was laid out again on this thread (each
    /// test draws on its own: not once a frame).
    static EDITOR_LAYOUTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many laid-out texts are being kept.
#[cfg(test)]
pub fn kept_galleys(ctx: &egui::Context) -> usize {
    ctx.memory_mut(|m| m.data.get_temp_mut_or_default::<Vec<(Id, KeptGalley)>>(Id::new("kept-galleys")).len())
}

/// How often an editor's text has been laid out on this thread.
#[cfg(test)]
pub fn editor_layouts() -> usize {
    EDITOR_LAYOUTS.with(|n| n.get())
}

impl KeptGalley {
    /// How many are kept: the editors on screen (a few, side by side),
    /// not every file ever opened (a big file's galley is big).
    const KEPT: usize = 4;

    fn all(ui: &Ui) -> Vec<(Id, KeptGalley)> {
        ui.ctx().memory_mut(|m| m.data.get_temp_mut_or_default::<Vec<(Id, KeptGalley)>>(Id::new("kept-galleys")).clone())
    }

    /// The galley kept under `id`, if it is for `key` and these fonts.
    fn get(ui: &Ui, id: Id, key: u64) -> Option<Arc<egui::Galley>> {
        let kept = Self::all(ui).into_iter().find(|(i, _)| *i == id)?.1;
        let atlas = ui.fonts(|f| f.texture_atlas());
        match (kept.galley, kept.atlas) {
            (Some(g), Some(a)) if kept.key == key && Arc::ptr_eq(&a, &atlas) => Some(g),
            _ => None,
        }
    }

    /// Lay `job` out and keep the result under `id` for `key`.
    fn lay_out(ui: &Ui, id: Id, key: u64, job: LayoutJob) -> Arc<egui::Galley> {
        #[cfg(test)]
        EDITOR_LAYOUTS.with(|n| n.set(n.get() + 1));
        let (galley, atlas) = ui.fonts(|f| (f.layout_job(job), f.texture_atlas()));
        let mut all = Self::all(ui);
        all.retain(|(i, _)| *i != id);
        all.push((id, KeptGalley { key, galley: Some(galley.clone()), atlas: Some(atlas) }));
        let over = all.len().saturating_sub(Self::KEPT);
        all.drain(..over);
        ui.ctx().memory_mut(|m| m.data.insert_temp(Id::new("kept-galleys"), all));
        galley
    }
}

/// A short key for find results, for the layout caches.
fn marks_key(marks: &[(usize, usize)], cur: Option<(usize, usize)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    marks.hash(&mut h);
    cur.hash(&mut h);
    h.finish()
}

fn hash_str(s: &str, extra: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    extra.hash(&mut h);
    h.finish()
}

impl App {
    /// Run a command by its settings id ("new_tab", "palette"…), for tests.
    #[cfg(test)]
    pub fn run_command_id(&mut self, id: &str) {
        let c = COMMANDS.iter().find(|c| c.1 == id).unwrap_or_else(|| panic!("no command {id}")).0;
        self.run_cmd(c);
    }

    fn run_cmd(&mut self, c: Cmd) {
        let pane = self.active_pane_id();
        match c {
            Cmd::NewTab => {
                let host = self.active_host();
                let cwd = self.active_pane().and_then(|p| p.term.cwd.clone());
                self.new_tab(&host, cwd);
            }
            Cmd::Connect => self.connect = Some(ConnectState { target: String::new(), split: false, password: String::new() }),
            Cmd::SplitRight => self.split(true),
            Cmd::SplitDown => self.split(false),
            Cmd::ClosePane => {
                if let Some(id) = pane {
                    self.ask_close_panes(vec![id])
                }
            }
            Cmd::About => {
                let about = if cfg!(windows) { format!("{}\nconsole host: {}", crate::version::line(), self.console_host.what) } else { crate::version::line() };
                self.toast(about, crate::app::ToastKind::Info)
            }
            Cmd::DisplayTest => self.display_test = Some(0),
            Cmd::Trace => {
                let Some(id) = pane else { return };
                let stopped = self.panes.get_mut(&id).and_then(|p| p.trace.take());
                match stopped {
                    Some(t) => {
                        let msg = format!("recording stopped: {} ({} KB)", t.path.display(), t.size() / 1024);
                        self.data.log(&msg);
                        self.toast(msg, ToastKind::Info);
                    }
                    None => {
                        let host = self.panes[&id].host.clone();
                        let stamp = crate::records::stamp();
                        let name: String = host.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
                        let path = self.data.path(&format!("logs/terminal-{name}-{stamp}.log"));
                        match crate::app::OutputTrace::start(path) {
                            Ok(t) => {
                                let msg = format!("recording this terminal to {} (run the command again to stop)", t.path.display());
                                self.data.log(&msg);
                                self.toast(msg, ToastKind::Info);
                                if let Some(p) = self.panes.get_mut(&id) {
                                    p.trace = Some(t);
                                }
                            }
                            Err(e) => self.toast(format!("can't record: {e}"), ToastKind::Error),
                        }
                    }
                }
            }
            Cmd::ConfirmUnsaved => {
                self.settings.confirm_unsaved = !self.settings.confirm_unsaved;
                let how = if self.settings.confirm_unsaved { "weft asks before closing a file with unsaved changes" } else { "files close without asking, unsaved changes are dropped" };
                self.toast(how, crate::app::ToastKind::Info);
                self.save_settings();
            }
            Cmd::ConfirmRunning => {
                self.settings.confirm_running = !self.settings.confirm_running;
                let how = if self.settings.confirm_running { "weft asks before closing a terminal where a command is running" } else { "terminals close without asking, running commands are ended" };
                self.toast(how, crate::app::ToastKind::Info);
                self.save_settings();
            }
            Cmd::Zoom => self.toggle_zoom(),
            Cmd::Sidebar => self.settings.sidebar = !self.settings.sidebar,
            Cmd::Restart => self.restart(),
            Cmd::Serial => self.open_serial_dialog(),
            Cmd::X11 => {
                let host = self.active_host();
                let now = match self.settings.profiles.iter_mut().find(|p| p.name == host).map(|p| &mut p.kind) {
                    Some(HostKind::Ssh { x11, .. }) => {
                        *x11 = !*x11;
                        Some(*x11)
                    }
                    _ => None,
                };
                match now {
                    Some(on) => {
                        self.save_settings();
                        let what = if on { "on: GUI programs started there show here (new tabs; an X server runs here, VcXsrv or X410)" } else { "off (new tabs)" };
                        self.toast(format!("X11 forwarding for {host} {what}"), ToastKind::Info);
                        if on {
                            if let Err(e) = crate::x11::ensure_server() {
                                self.toast(e, ToastKind::Error);
                            }
                        }
                    }
                    None => self.toast(format!("{host}: X11 forwarding is for ssh hosts"), ToastKind::Info),
                }
            }
            Cmd::PanelSplit(region, right) => {
                let focus = if region == Region::Side { self.side_focus } else { self.editor_focus };
                let all = self.panels_of(region);
                if let Some(&at) = all.iter().find(|p| p.n == focus).or(all.first()) {
                    self.split_panel(at, if right { Dock::Right } else { Dock::Bottom });
                }
            }
            Cmd::Editor => self.editor.visible = !self.editor.visible,
            Cmd::Find => {
                if let Some(p) = pane.and_then(|id| self.panes.get_mut(&id)) {
                    p.search = match p.search.take() {
                        Some(_) => None,
                        None => Some(SearchState { query: String::new(), icase: self.settings.search.terminal_icase, hits: Vec::new(), cur: 0, seen_generation: u64::MAX, focus: true }),
                    };
                    if p.search.is_none() {
                        self.focus_term = true;
                    }
                }
            }
            Cmd::Composer => {
                if let Some(p) = pane.and_then(|id| self.panes.get_mut(&id)) {
                    if p.composer.is_some() {
                        p.composer = None;
                        self.focus_term = true;
                    } else {
                        p.composer = Some(Composer::new(String::new()));
                    }
                }
            }
            Cmd::PrevBlock => self.jump_block(-1),
            Cmd::NextBlock => self.jump_block(1),
            Cmd::FoldAll | Cmd::UnfoldAll => {
                if let Some(p) = pane.and_then(|id| self.panes.get_mut(&id)) {
                    for b in p.term.blocks.iter_mut() {
                        if b.state() == BlockState::Done {
                            b.folded = c == Cmd::FoldAll;
                        }
                    }
                }
            }
            Cmd::ClearScrollback => {
                if let Some(p) = pane.and_then(|id| self.panes.get_mut(&id)) {
                    p.term.clear_history();
                    p.scroll = 0;
                }
            }
            Cmd::Reconnect => {
                let h = self.active_host();
                self.reconnect(&h);
            }
            Cmd::FontUp => self.settings.font_size = (self.settings.font_size + 1.0).min(32.0),
            Cmd::FontDown => self.settings.font_size = (self.settings.font_size - 1.0).max(8.0),
            Cmd::FontReset => self.settings.font_size = 14.0,
            Cmd::UiUp => self.settings.ui_scale = ((self.settings.ui_scale * 10.0).round() + 1.0).min(25.0) / 10.0,
            Cmd::UiDown => self.settings.ui_scale = ((self.settings.ui_scale * 10.0).round() - 1.0).max(6.0) / 10.0,
            Cmd::UiReset => self.settings.ui_scale = 1.0,
            Cmd::Theme(i) => self.pick_theme(i),
            Cmd::ThemeSystem => self.settings.theme = "system".into(),
            Cmd::LocalShell(sh) => {
                self.settings.local_shell = sh;
                self.toast(format!("local shell: {} (new tabs)", sh.label()), crate::app::ToastKind::Info);
                self.save_settings();
            }
            Cmd::CursorStyle => {
                self.settings.cursor_style = match self.settings.cursor_style {
                    CursorStyle::Block => CursorStyle::Bar,
                    CursorStyle::Bar => CursorStyle::Underline,
                    CursorStyle::Underline => CursorStyle::Block,
                };
                self.toast(format!("cursor: {:?}", self.settings.cursor_style).to_lowercase(), crate::app::ToastKind::Info);
            }
            Cmd::MarkdownBreaks => {
                self.settings.markdown_breaks = !self.settings.markdown_breaks;
                let mode = if self.settings.markdown_breaks { "every newline breaks the line (GitHub PR style)" } else { "a single newline is a space (CommonMark)" };
                self.toast(format!("Markdown: {mode}"), crate::app::ToastKind::Info);
            }
            Cmd::ThemePicker => self.theme_menu = Some(egui::Rect::from_min_size(pos2(f32::MAX, 34.0), vec2(0.0, 0.0))),
            Cmd::Hidden => {
                self.settings.show_hidden = !self.settings.show_hidden;
                self.dir_cache.clear();
            }
            Cmd::Settings => {
                self.save_settings();
                let p = self.data.path("settings.json").display().to_string().replace('\\', "/");
                self.open_file("local", &p, 1);
            }
            Cmd::NewMemo => self.new_memo(),
            Cmd::OpenMemo => {
                self.memo_query.clear();
                self.memo_picker = true;
            }
            Cmd::Fonts => self.open_font_menu(),
            Cmd::History => {
                self.settings.sidebar = true;
                self.side.tab = SideTab::History;
            }
            Cmd::Records => self.open_records_window(),
            Cmd::Palette => self.palette = if self.palette.is_some() { None } else { Some(PaletteState::new()) },
            Cmd::Keys => self.keys_window = Some(KeysWindow::default()),
            Cmd::GoTab(n) => {
                // Alt+9 is the last tab, as in browsers; a number past the end does nothing.
                let to = if n == usize::MAX { self.tabs.len().checked_sub(1) } else { Some(n).filter(|&n| n < self.tabs.len()) };
                if let Some(to) = to {
                    self.active_tab = to;
                    self.focus_term = true;
                    self.on_active_changed();
                }
            }
            Cmd::NextTab | Cmd::PrevTab => {
                if !self.tabs.is_empty() {
                    let n = self.tabs.len() as i32;
                    let step = if c == Cmd::NextTab { 1 } else { -1 };
                    self.active_tab = (self.active_tab as i32 + step).rem_euclid(n) as usize;
                    self.focus_term = true;
                    self.on_active_changed();
                }
            }
            Cmd::CopyMode => {
                if let Some(p) = pane.and_then(|id| self.panes.get_mut(&id)) {
                    p.copy = match p.copy {
                        Some(_) => None,
                        None => Some(crate::copymode::CopyMode { cur: (p.term.cursor_abs(), p.term.cx), g: false }),
                    };
                    p.sel = None;
                }
                self.focus_term = true;
            }
            Cmd::FocusMode => {
                match self.focus_mode.take() {
                    Some((side, editor)) => {
                        self.settings.sidebar = side;
                        self.editor.visible = editor;
                    }
                    None => {
                        self.focus_mode = Some((self.settings.sidebar, self.editor.visible));
                        self.settings.sidebar = false;
                        self.editor.visible = false;
                    }
                }
                self.focus_term = true;
            }
            Cmd::FocusLeft => self.focus_dir(-1.0, 0.0),
            Cmd::FocusRight => self.focus_dir(1.0, 0.0),
            Cmd::FocusUp => self.focus_dir(0.0, -1.0),
            Cmd::FocusDown => self.focus_dir(0.0, 1.0),
            Cmd::Resize(side_by_side, forward) => self.resize_pane(side_by_side, if forward { 0.05 } else { -0.05 }),
            Cmd::Help | Cmd::Features => self.help = Some(HelpState { features: c == Cmd::Features, query: String::new() }),
            Cmd::Memos => {
                let dir = self.memo_dir();
                let _ = std::fs::create_dir_all(&dir);
                self.settings.sidebar = true;
                self.side.tab = SideTab::Files;
                self.set_root("local", &dir);
            }
            Cmd::AgentLog => {
                let h = self.active_host();
                let p = self.data.path(&format!("logs/{}", crate::host::agent_log_name(&h, &crate::records::today()))).display().to_string().replace('\\', "/");
                self.open_file("local", &p, 1);
            }
        }
    }

    /// Esc closes the window on top that has no way of its own to take it:
    /// Help, Keyboard shortcuts, Privacy, Serial console, the memo picker
    /// and Fonts only had their ×. Before the terminal sees the key: the
    /// shell gets no Esc for it. (A key being bound has taken its Esc by
    /// now: that cancels the binding, not the window.)
    pub fn escape_closes_windows(&mut self, ctx: &egui::Context) {
        let windows = [self.display_test.is_some(), self.serial_dialog.is_some(), self.font_menu.is_some(), self.memo_picker, self.keys_window.is_some(), self.records_window.is_some(), self.help.is_some()];
        let now = windows.iter().enumerate().fold(0u8, |m, (i, open)| m | ((*open as u8) << i));
        // One that has just opened takes the keyboard from the terminal;
        // a click in a terminal gives it back, and Esc is then the shell's
        // again (vim beside an open Help keeps its Esc).
        // Who has the keyboard: as it was before this frame when Esc is in
        // it (egui has already taken the focus from a text field for Esc).
        let focused = if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.focus_before
        } else {
            self.focus_before = ctx.memory(|m| m.focused());
            self.focus_before
        };
        let in_terminal = focused.is_some_and(|f| self.panes.keys().any(|id| f == Id::new(("pane", *id))));
        if now & !self.esc_windows != 0 {
            self.focus_term = false;
            if let (true, Some(f)) = (in_terminal, focused) {
                ctx.memory_mut(|m| m.surrender_focus(f));
            }
            self.esc_windows = now;
            return;
        }
        self.esc_windows = now;
        if now == 0 || in_terminal || self.focus_term {
            return;
        }
        // What is on top and takes Esc itself goes first: the palette, the
        // connect box, a question, a file dialog; a composer, a search or
        // the editor's find bar that has the keyboard.
        if self.palette.is_some() || self.connect.is_some() || self.confirm.is_some() || self.file_op.is_some() {
            return;
        }
        if let Some(f) = focused {
            if f == Id::new("editor-find-q") || self.panes.keys().any(|id| f == Id::new(("composer", *id)) || f == Id::new(("search", *id))) {
                return;
            }
        }
        if !ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            return;
        }
        // One at a time, the smaller ones (opened from the others) first.
        if self.display_test.take().is_some() || self.serial_dialog.take().is_some() || self.font_menu.take().is_some() || std::mem::take(&mut self.memo_picker) || self.keys_window.take().is_some() || self.records_window.take().is_some() {
        } else {
            self.help = None;
        }
        self.focus_term = true;
    }

    pub fn handle_global_keys(&mut self, ctx: &egui::Context) {
        if self.keys_window.as_ref().map(|w| w.capture.is_some()).unwrap_or(false) {
            self.capture_key(ctx);
            return;
        }
        if self.keymap.is_empty() {
            self.rebuild_keys();
        }
        // Moving between split panes only takes its keys while there is a
        // split; otherwise Alt+arrows stay the shell's.
        let multi = self.pane_rects.len() > 1;
        let usable = |c: Cmd| multi || !matches!(c, Cmd::FocusLeft | Cmd::FocusRight | Cmd::FocusUp | Cmd::FocusDown | Cmd::Resize(..));
        // A bound key runs its command and never reaches the terminal.
        let keymap = &self.keymap;
        // In a text field (the editor, a search box) its own keys win: Ctrl+
        // Shift+Z is redo there, not the Zoom command. A field keeps a text
        // state; the terminal, which also takes the keyboard, does not.
        let in_text = ctx.memory(|m| m.focused()).is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some());
        let text_key = |key: Key, m: egui::Modifiers| in_text && key == Key::Z && m.shift && (m.ctrl || m.command);
        let mut cmds: Vec<Cmd> = ctx.input_mut(|i| {
            let mut out = Vec::new();
            let mods = i.modifiers;
            i.events.retain(|e| {
                let pressed = match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } if !text_key(*key, *modifiers) => Some((*key, *modifiers)),
                    egui::Event::Key { .. } => None,
                    // Ctrl+Shift+X and the like arrive as clipboard events only.
                    _ => crate::keys::clipboard_key(e, mods),
                };
                match pressed.and_then(|(key, m)| crate::keys::lookup(keymap, key, m, usable)) {
                    Some(c) => {
                        out.push(c);
                        false
                    }
                    None => true,
                }
            });
            out
        });
        // Ctrl+wheel: text size; Ctrl+Shift+wheel: the whole UI. With Shift
        // held egui turns the wheel into horizontal scrolling, so its zoom
        // delta stays 1: read those wheel events directly (one notch = one step).
        let (zoom, ui_wheel) = ctx.input(|i| {
            let notches: f32 = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, modifiers } if modifiers.ctrl && modifiers.shift => Some(match unit {
                        egui::MouseWheelUnit::Point => (delta.x + delta.y) / 50.0,
                        _ => delta.x + delta.y,
                    }),
                    _ => None,
                })
                .sum();
            (i.zoom_delta(), notches)
        });
        if zoom != 1.0 {
            self.zoom_acc *= zoom;
            while self.zoom_acc > 1.1 {
                self.zoom_acc /= 1.1;
                cmds.push(Cmd::FontUp);
            }
            while self.zoom_acc < 1.0 / 1.1 {
                self.zoom_acc *= 1.1;
                cmds.push(Cmd::FontDown);
            }
        }
        if ui_wheel != 0.0 {
            self.ui_wheel_acc += ui_wheel;
            while self.ui_wheel_acc >= 1.0 {
                self.ui_wheel_acc -= 1.0;
                cmds.push(Cmd::UiUp);
            }
            while self.ui_wheel_acc <= -1.0 {
                self.ui_wheel_acc += 1.0;
                cmds.push(Cmd::UiDown);
            }
        }
        for c in cmds {
            self.run_cmd(c);
        }
    }

    // ---- keyboard shortcuts --------------------------------------------------------

    /// The keys in force: the defaults, with settings "keys" replacing them per command.
    pub fn rebuild_keys(&mut self) {
        let parse = |l: &[String]| l.iter().filter_map(|k| crate::keys::Shortcut::parse(k)).collect::<Vec<_>>();
        self.keymap = COMMANDS
            .iter()
            .map(|(c, id, _, defaults)| {
                let keys = match self.settings.keys.get(*id) {
                    Some(l) => parse(l),
                    None => parse(&defaults.iter().map(|d| d.to_string()).collect::<Vec<_>>()),
                };
                (*c, keys)
            })
            .collect();
    }

    /// A command's keys as shown in menus: "Ctrl+= / Ctrl+Plus".
    /// The keys bound to a command now, as labels ("Ctrl+Shift+T").
    fn keymap_labels(&self, c: Cmd) -> Vec<String> {
        self.keymap.iter().find(|k| k.0 == c).map(|k| k.1.iter().map(|s| s.label()).collect()).unwrap_or_default()
    }

    fn keys_label(&self, c: Cmd) -> String {
        let keys = self.keymap_labels(c);
        let mut s = keys.join(" / ");
        if matches!(c, Cmd::UiUp | Cmd::UiDown) {
            s = if s.is_empty() { "Ctrl+Shift+wheel".into() } else { s + " / Ctrl+Shift+wheel" };
        }
        s
    }

    /// The next key press becomes a binding of the command being set
    /// (Esc cancels). Nothing typed meanwhile reaches the terminal.
    fn capture_key(&mut self, ctx: &egui::Context) {
        let got = ctx.input_mut(|i| {
            let mods = i.modifiers;
            let got = i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => crate::keys::clipboard_key(e, mods),
            });
            i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)));
            got
        });
        let Some((key, m)) = got else { return };
        let Some(id) = self.keys_window.as_mut().and_then(|w| w.capture.take()) else { return };
        if key == Key::Escape && !m.any() {
            return;
        }
        let sc = crate::keys::Shortcut { ctrl: m.ctrl || m.command, shift: m.shift, alt: m.alt, key };
        let mut list = self.bindings_of(id);
        if !list.contains(&sc.label()) {
            list.push(sc.label());
        }
        self.set_bindings(id, Some(list));
    }

    fn bindings_of(&self, id: &str) -> Vec<String> {
        let c = COMMANDS.iter().find(|c| c.1 == id).map(|c| c.0);
        self.keymap.iter().find(|k| Some(k.0) == c).map(|k| k.1.iter().map(|s| s.label()).collect()).unwrap_or_default()
    }

    /// New keys for a command (None: back to its defaults), kept in settings.
    fn set_bindings(&mut self, id: &str, keys: Option<Vec<String>>) {
        match keys {
            Some(k) => self.settings.keys.insert(id.to_string(), k),
            None => self.settings.keys.remove(id),
        };
        self.rebuild_keys();
        self.save_settings();
    }

    // ---- tab bar -----------------------------------------------------------------

    pub fn ui_tabbar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            self.dock_grip(ui);
            // The buttons on the right get their room first; the tabs share
            // what is left (they used to run on under the buttons).
            // In a narrow window the buttons that do not fit beside one tab
            // and its + are left out (all of them are in the ☰ menu): they
            // used to be drawn under the tab's + and connect buttons.
            let for_buttons = ui.available_width() - TAB_STRIP_LEAST;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.tabbar_buttons(ui, for_buttons);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| self.tab_strip(ui));
            });
        });
    }

    /// Terminal tabs: shrink to fit, then scroll sideways; drag one to move it.
    fn tab_strip(&mut self, ui: &mut Ui) {
        let mut activate = None;
        let mut close = None;
        let mut moved: Option<(usize, usize)> = None;
        let n_tabs = self.tabs.len();
        let font = FontId::proportional(12.5);
        // The buttons' wider spacing would carry over and push the strip's end
        // onto the buttons on the right (the palette button lost its tooltip).
        ui.spacing_mut().item_spacing.x = 2.0;
        // Room for the tabs: all of it but the + and connect buttons and a margin.
        // (Never more than there is: past it the tabs ran under the buttons.)
        let room = (ui.available_width() - 4.0 - 2.0 * (22.0 + 2.0) - 2.0 - 12.0).max(0.0);
        let natural: Vec<f32> = self
            .tabs
            .iter()
            .map(|t| self.panes.get(&t.active).map(|p| ui.painter().layout_no_wrap(p.title(), font.clone(), theme::text()).size().x).unwrap_or(0.0))
            .map(|w| (w + 52.0).clamp(110.0, 260.0))
            .collect();
        let total: f32 = natural.iter().sum::<f32>() + 2.0 * n_tabs as f32;
        // Too wide: every tab gets the same share, but not less than 90.
        let cap = if total > room { (room / n_tabs.max(1) as f32 - 2.0).max(90.0) } else { f32::INFINITY };
        let drag_id = Id::new("tab-drag");
        let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
        let last_active_id = Id::new("tab-last-active");
        let reveal = ui.data(|d| d.get_temp::<usize>(last_active_id)) != Some(self.active_tab);
        ui.data_mut(|d| d.insert_temp(last_active_id, self.active_tab));
        ui.scope(|ui| {
            ui.style_mut().always_scroll_the_only_direction = true;
            let scroll = &mut ui.style_mut().spacing.scroll;
            scroll.floating = false;
            scroll.bar_width = 3.0;
            scroll.bar_inner_margin = 1.0;
            scroll.bar_outer_margin = 0.0;
            ScrollArea::horizontal().id_salt("term-tabs").max_width(room).auto_shrink([true, true]).drag_to_scroll(false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let mut rects: Vec<egui::Rect> = Vec::with_capacity(n_tabs);
                    // The tab being dragged: drawn under the pointer (title, dot color, width).
                    let mut ghost: Option<(String, Color32, f32)> = None;
                    for (ti, tab) in self.tabs.iter().enumerate() {
                        let Some(p) = self.panes.get(&tab.active) else {
                            rects.push(egui::Rect::NOTHING);
                            continue;
                        };
                        let title = p.title();
                        let running = p.term.blocks.last().map(|b| b.state() == BlockState::Running).unwrap_or(false);
                        let active = ti == self.active_tab;
                        let w = natural[ti].min(cap);
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click_and_drag());
                        rects.push(rect);
                        if active && reveal {
                            ui.scroll_to_rect(rect, None);
                        }
                        // contains_pointer, not hovered(): the close button drawn on hover
                        // sits on top of the tab and would "steal" hovered() from it, hiding
                        // itself again next frame -> endless flicker.
                        let hovered = resp.contains_pointer();
                        let lifted = dragging == Some(ti);
                        if lifted {
                            // Its place: an empty slot; the tab itself rides under the pointer.
                            ui.painter().rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, theme::tint(theme::accent(), 22));
                            ui.painter().rect_stroke(rect.shrink(1.0), CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, Stroke::new(1.0_f32, theme::tint(theme::accent(), 110)), egui::StrokeKind::Inside);
                            ghost = Some((title.clone(), host_color(&p.host), rect.width()));
                        }
                        let bg = if active || lifted {
                            theme::bg_term()
                        } else if hovered {
                            theme::bg_hover()
                        } else {
                            theme::bg_base()
                        };
                        if !lifted {
                            ui.painter().rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, bg);
                            if active {
                                ui.painter().hline(rect.x_range().shrink(6.0), rect.top() + 1.0, Stroke::new(2.0_f32, host_color(&p.host)));
                            }
                            let dot = Rect2::dot(rect);
                            if running {
                                // A still ring, not a pulse: an animation here redrew the
                                // whole window ~16 times a second for as long as anything
                                // ran (an open nvim, a server).
                                ui.painter().circle_filled(dot, 3.5, theme::accent());
                                ui.painter().circle_stroke(dot, 5.5, Stroke::new(1.2_f32, theme::tint(theme::accent(), 140)));
                            } else {
                                ui.painter().circle_filled(dot, 3.5, host_color(&p.host));
                            }
                            let color = if active { theme::text() } else { theme::text_dim() };
                            let text_w = rect.width() - 52.0;
                            let mut g = ui.painter().layout_no_wrap(title.clone(), font.clone(), color);
                            if g.size().x > text_w {
                                // Drop characters until it fits, then add the ellipsis.
                                let chars: Vec<char> = title.chars().collect();
                                let mut n = chars.len();
                                while n > 1 {
                                    n = (n * 9 / 10).min(n - 1);
                                    let t: String = chars[..n].iter().collect::<String>() + "…";
                                    g = ui.painter().layout_no_wrap(t, font.clone(), color);
                                    if g.size().x <= text_w {
                                        break;
                                    }
                                }
                            }
                            ui.painter().galley(pos2(rect.left() + 22.0, rect.center().y - g.size().y / 2.0), g, theme::text());
                        }
                        let cr = egui::Rect::from_center_size(pos2(rect.right() - 15.0, rect.center().y), vec2(18.0, 18.0));
                        if (hovered || active) && dragging.is_none() {
                            let cresp = ui.interact(cr, Id::new(("tabclose", ti)), Sense::click());
                            if cresp.hovered() {
                                ui.painter().rect_filled(cr, CornerRadius::same(4), theme::bg_hover());
                            }
                            icons::paint(ui.painter(), cr.shrink(4.5), Icon::Cross, if cresp.hovered() { theme::text() } else { theme::text_faint() });
                            if cresp.clicked() {
                                close = Some((ti, CloseWhich::This));
                            }
                        }
                        if resp.drag_started_by(egui::PointerButton::Primary) {
                            dragging = Some(ti);
                            activate = Some(ti);
                            // Where on the tab it was picked up, so the ghost stays under the pointer.
                            let grab = ui.input(|i| i.pointer.press_origin()).map(|p| p.x - rect.left()).unwrap_or(rect.width() / 2.0);
                            ui.data_mut(|d| d.insert_temp(Id::new("tab-grab"), grab));
                        }
                        if resp.clicked() {
                            activate = Some(ti);
                        }
                        if resp.middle_clicked() {
                            close = Some((ti, CloseWhich::This));
                        }
                        resp.on_hover_text(format!("{title}\nclick: switch · drag: move · middle-click: close · right-click: more")).context_menu(|ui| {
                            if let Some(w) = close_menu(ui, ti, n_tabs) {
                                close = Some((ti, w));
                            }
                        });
                    }
                    // Dragging: where it would land, between tabs; it moves on release.
                    if let Some(from) = dragging {
                        let pointer = ui.input(|i| i.pointer.interact_pos());
                        let to = pointer.map(|p| tab_drop_index(&rects, p.x)).unwrap_or(from);
                        if let Some(p) = pointer {
                            // Near the edges, keep the strip scrolling.
                            let clip = ui.clip_rect();
                            if p.x < clip.left() + 24.0 || p.x > clip.right() - 24.0 {
                                ui.scroll_with_delta(vec2(if p.x < clip.left() + 24.0 { 8.0 } else { -8.0 }, 0.0));
                                ui.ctx().request_repaint();
                            }
                        }
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        if to != from && to != from + 1 {
                            // Where it lands: a thick line with a dot at each end.
                            let x = if to < rects.len() { rects[to].left() - 1.0 } else { rects.last().map(|r| r.right() + 1.0).unwrap_or(0.0) };
                            let y = rects.first().map(|r| r.y_range()).unwrap_or(egui::Rangef::new(0.0, 28.0));
                            ui.painter().vline(x, y, Stroke::new(3.0_f32, theme::accent()));
                            ui.painter().circle_filled(pos2(x, y.min + 2.0), 3.5, theme::accent());
                            ui.painter().circle_filled(pos2(x, y.max - 2.0), 3.5, theme::accent());
                        }
                        // The tab itself, lifted, following the pointer along the strip.
                        if let (Some(p), Some((title, dot, w))) = (pointer, ghost.take()) {
                            let grab: f32 = ui.data(|d| d.get_temp(Id::new("tab-grab"))).unwrap_or(w / 2.0);
                            let top = rects.first().map(|r| r.top()).unwrap_or(p.y) - 3.0;
                            egui::Area::new(Id::new("tab-ghost")).order(egui::Order::Tooltip).fixed_pos(pos2(p.x - grab, top)).interactable(false).show(ui.ctx(), |ui| {
                                egui::Frame::new()
                                    .fill(theme::bg_term())
                                    .stroke(Stroke::new(1.5_f32, theme::accent()))
                                    .corner_radius(CornerRadius::same(6))
                                    .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) })
                                    .show(ui, |ui| {
                                        let (r, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                                        ui.painter().circle_filled(Rect2::dot(r), 3.5, dot);
                                        let g = ui.painter().layout_no_wrap(title, FontId::proportional(12.5), theme::text());
                                        let clip = r.with_min_x(r.left() + 22.0).with_max_x(r.right() - 10.0);
                                        ui.painter().with_clip_rect(clip).galley(pos2(r.left() + 22.0, r.center().y - g.size().y / 2.0), g, theme::text());
                                    });
                            });
                        }
                        if ui.input(|i| i.pointer.any_released()) {
                            // Index after removing the dragged tab.
                            let dest = if to > from { to - 1 } else { to };
                            if dest != from {
                                moved = Some((from, dest));
                            }
                            dragging = None;
                        }
                    }
                });
            });
        });
        ui.data_mut(|d| match dragging {
            Some(i) => d.insert_temp(drag_id, i),
            None => d.remove::<usize>(drag_id),
        });
        ui.add_space(4.0);
        if icons::button(ui, Icon::Plus, "new tab on the same host (Ctrl+Shift+T)").clicked() {
            self.run_cmd(Cmd::NewTab);
        }
        if icons::button(ui, Icon::Plug, "connect to a host (Ctrl+Shift+N)").clicked() {
            self.run_cmd(Cmd::Connect);
        }
        if let Some((from, to)) = moved {
            let active = self.tabs.get(self.active_tab).map(|t| t.active);
            let tab = self.tabs.remove(from);
            self.tabs.insert(to, tab);
            // The same tab stays active wherever it now is.
            if let Some(i) = active.and_then(|a| self.tabs.iter().position(|t| t.active == a)) {
                self.active_tab = i;
            }
            self.save_settings();
        } else if let Some(ti) = activate {
            if ti != self.active_tab {
                self.active_tab = ti;
                self.on_active_changed();
            }
            self.focus_term = true;
        }
        if let Some((ti, which)) = close {
            self.ask_close_tabs(ti, which);
        }
    }

    /// The buttons at the right of the tab bar, right to left.
    /// Within `width`: the ☰ menu always, the rest as far as they fit, the
    /// least needed (palette, splits) first to go.
    fn tabbar_buttons(&mut self, ui: &mut Ui, width: f32) {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(6.0);
        let start = ui.cursor().right();
        self.commands_menu(ui);
        // Room left after what is drawn so far, for one more of `w`, with
        // the separator that ends the row.
        let fits = |ui: &Ui, w: f32| start - ui.cursor().right() + w + 6.0 + 8.0 <= width;
        if !fits(ui, 14.0) {
            return;
        }
        if ui.add(egui::Button::new(RichText::new("?").size(13.0).color(theme::text_dim())).frame(false)).on_hover_text("help (F1)").clicked() {
            self.help = if self.help.is_some() { None } else { Some(HelpState { features: false, query: String::new() }) };
        }
        if !fits(ui, 24.0) {
            ui.separator();
            return;
        }
        let fr = ui.add(egui::Button::new(RichText::new("Aa").size(12.5).color(theme::text_dim())).frame(false)).on_hover_text("fonts");
        if fr.clicked() {
            if self.font_menu.is_some() {
                self.font_menu = None;
            } else {
                self.open_font_menu();
            }
        }
        // Each icon button is 22 wide.
        let icon_fits = |ui: &Ui| fits(ui, 22.0);
        if !icon_fits(ui) {
            ui.separator();
            return;
        }
        let ti = if theme::is_light() { Icon::Sun } else { Icon::Moon };
        let tr = icons::button(ui, ti, "theme");
        if tr.clicked() {
            self.theme_menu = if self.theme_menu.is_some() { None } else { Some(tr.rect) };
        }
        if !icon_fits(ui) {
            ui.separator();
            return;
        }
        let ec = if self.editor.visible { theme::accent() } else { theme::text_dim() };
        if icons::button_colored(ui, Icon::Editor, "editor (Ctrl+Shift+J)", ec).clicked() {
            self.editor.visible = !self.editor.visible;
        }
        if !icon_fits(ui) {
            ui.separator();
            return;
        }
        let sc = if self.settings.sidebar { theme::accent() } else { theme::text_dim() };
        if icons::button_colored(ui, Icon::Sidebar, "side panel (Ctrl+Shift+B)", sc).clicked() {
            self.settings.sidebar = !self.settings.sidebar;
        }
        for (icon, tip, cmd) in [(Icon::SplitDown, "split down (Ctrl+Shift+E)", Cmd::SplitDown), (Icon::SplitRight, "split right (Ctrl+Shift+D)", Cmd::SplitRight), (Icon::Search, "command palette (Ctrl+Shift+P)", Cmd::Palette)] {
            if !icon_fits(ui) {
                break;
            }
            if icons::button(ui, icon, tip).clicked() {
                self.run_cmd(cmd);
            }
        }
        ui.separator();
    }

    // ---- status bar ----------------------------------------------------------------

    pub fn ui_status(&mut self, ui: &mut Ui) {
        let host = self.active_host();
        let small = |s: String, c: Color32| RichText::new(s).size(11.5).color(c);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            let status = self.conns.get(&host).map(|c| c.status.clone()).unwrap_or(ConnStatus::Connecting);
            let serial = match self.profile(&host).kind {
                HostKind::Serial { via, port, baud } => Some(if via == "local" { format!("serial {port} \u{00b7} {baud}") } else { format!("serial {port} on {via} \u{00b7} {baud}") }),
                _ => None,
            };
            let (dot, label) = match &status {
                _ if serial.is_some() => (theme::success(), serial.clone().unwrap_or_default()),
                ConnStatus::Ready => (theme::success(), if host == "local" { "native fs".to_string() } else { "agent ready".to_string() }),
                ConnStatus::Connecting => (theme::warn(), "agent connecting…".to_string()),
                ConnStatus::Failed(e) => (theme::error(), format!("agent offline: {}", e.chars().take(80).collect::<String>())),
            };
            ui.painter().circle_filled(rect.center(), 4.0, dot);
            // The right side first; the left side (host, agent, directory) gets
            // what is left, each cut with … (whole on hover) rather than running
            // under the right side.
            let width = ui.available_width();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Sizes, always at hand: clicking doesn't close anything; the value resets.
                let mut step: Option<Cmd> = None;
                // A newer weft was built or installed: restart into it here.
                if self.update_ready {
                    let b = egui::Button::new(small("\u{21bb} new weft \u{00b7} restart".into(), theme::bg_base())).fill(theme::accent()).min_size(vec2(0.0, 18.0));
                    if ui.add(b).on_hover_text("start the new weft with the same tabs, folders and settings (programs running in terminals stop; shells start again in their folders)").clicked() {
                        step = Some(Cmd::Restart);
                    }
                    ui.add_space(12.0);
                }
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut btn = |ui: &mut Ui, label: String, tip: &str, cmd: Cmd, color: Color32| {
                    let b = egui::Button::new(small(label, color)).frame(false).min_size(vec2(18.0, 18.0));
                    if ui.add(b).on_hover_text(tip).clicked() {
                        step = Some(cmd);
                    }
                };
                let pct = format!("{:.0}%", self.settings.ui_scale * 100.0);
                btn(ui, "+".into(), "bigger UI: icons, tabs, panels (Ctrl+Shift+wheel)", Cmd::UiUp, theme::text_dim());
                btn(ui, pct, "UI scale - click: 100%", Cmd::UiReset, theme::text_faint());
                btn(ui, "\u{2212}".into(), "smaller UI (Ctrl+Shift+wheel)", Cmd::UiDown, theme::text_dim());
                ui.add_space(10.0);
                let size = format!("{:.0}", self.settings.font_size);
                btn(ui, "A+".into(), "bigger text: terminal, editor (Ctrl+wheel, Ctrl+=)", Cmd::FontUp, theme::text_dim());
                btn(ui, size, "text size - click: default (Ctrl+0)", Cmd::FontReset, theme::text_faint());
                btn(ui, "A\u{2212}".into(), "smaller text (Ctrl+wheel, Ctrl+-)", Cmd::FontDown, theme::text_dim());
                if let Some(c) = step {
                    self.run_cmd(c);
                }
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add_space(14.0);
                // Optional extras, only on a wide window.
                if width > 1000.0 {
                    ui.label(small("Ctrl+Space composer · Ctrl+Shift+P commands".into(), theme::text_faint()));
                }
                if let Some(p) = self.active_pane() {
                    ui.label(small(format!("{}×{}", p.term.cols, p.term.rows), theme::text_faint()));
                }
                if let Some(s) = self.sys.get(&host).filter(|_| width > 700.0) {
                    let mut parts = Vec::new();
                    if let Some(l) = s["load"].as_array().and_then(|a| a.first()).and_then(|v| v.as_f64()) {
                        parts.push(format!("load {l:.2}"));
                    }
                    if let (Some(t), Some(u)) = (s["mem_total"].as_f64(), s["mem_used"].as_f64()) {
                        parts.push(format!("mem {:.0}%", 100.0 * u / t.max(1.0)));
                    }
                    if let (Some(t), Some(f)) = (s["disk_total"].as_f64(), s["disk_free"].as_f64()) {
                        parts.push(format!("disk {} free", human_size(f as u64)));
                        let _ = t;
                    }
                    ui.label(small(parts.join(" · "), theme::text_dim()));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let cut = |ui: &mut Ui, text: String, color: Color32| ui.add(egui::Label::new(small(text.clone(), color)).truncate()).on_hover_text(text);
                    if let Some(t) = self.active_pane().and_then(|p| p.trace.as_ref()) {
                        let label = if t.full() { "\u{25cf} recording full".to_string() } else { format!("\u{25cf} recording {} KB", t.size() / 1024) };
                        ui.label(small(label, theme::error())).on_hover_text(format!("{}\nRecord this terminal again to stop", t.path.display()));
                    }
                    cut(ui, host.clone(), theme::text());
                    let r = cut(ui, label, theme::text_dim());
                    if matches!(status, ConnStatus::Failed(_)) {
                        r.on_hover_text("Without a password the agent only uses keys / ssh-agent. Terminals work regardless.");
                        if ui.small_button("reconnect").clicked() {
                            self.actions.push(Action::Reconnect(host.clone()));
                        }
                        if !self.profile(&host).is_local() && ui.small_button("password…").clicked() {
                            self.auth_prompt = Some((host.clone(), String::new()));
                        }
                    }
                    if let Some(cwd) = self.active_pane().and_then(|p| p.term.cwd.clone()) {
                        cut(ui, weft_core::shown(&cwd).into_owned(), theme::text_faint());
                    }
                });
            });
        });
    }

    // ---- side panel -------------------------------------------------------------------

    pub fn ui_sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            self.dock_grip(ui);
            self.panel_buttons(ui);
            const TABS: [(SideTab, &str); 5] = [(SideTab::Files, "Files"), (SideTab::Blocks, "Commands"), (SideTab::History, "History"), (SideTab::Search, "Search"), (SideTab::Procs, "Procs")];
            // A narrow panel shows the tabs that fit (the open one always) and
            // the rest under ».
            let pad = ui.spacing().button_padding.x * 2.0 + ui.spacing().item_spacing.x;
            let widths: Vec<f32> = TABS.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), FontId::proportional(12.5), theme::text()).size().x + pad).collect();
            let active = TABS.iter().position(|t| t.0 == self.side.tab).unwrap_or(0);
            let (shown, hidden) = fit_tabs(&widths, ui.available_width(), 28.0, active);
            for i in shown {
                let (tab, label) = TABS[i];
                let sel = self.side.tab == tab;
                let txt = RichText::new(label).size(12.5).color(if sel { theme::text() } else { theme::text_dim() });
                if ui.selectable_label(sel, txt).clicked() {
                    self.side.tab = tab;
                }
            }
            if !hidden.is_empty() {
                let names: Vec<&str> = hidden.iter().map(|&i| TABS[i].1).collect();
                ui.menu_button(RichText::new("»").size(13.0).color(theme::text_dim()), |ui| {
                    for &i in &hidden {
                        let (tab, label) = TABS[i];
                        if ui.selectable_label(false, RichText::new(label).size(12.5)).clicked() {
                            self.side.tab = tab;
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text(format!("more: {}", names.join(", ")));
            }
        });
        ui.add_space(4.0);
        ui.separator();
        match self.side.tab {
            SideTab::Files => self.ui_files(ui),
            SideTab::Blocks => self.ui_blocks(ui),
            SideTab::History => self.ui_history(ui),
            SideTab::Search => self.ui_grep(ui),
            SideTab::Procs => self.ui_procs(ui),
        }
    }

    fn ui_files(&mut self, ui: &mut Ui) {
        if self.side.root.is_none() {
            let host = self.active_host();
            let dir = self
                .active_pane()
                .and_then(|p| p.term.cwd.clone().or(p.initial_cwd.clone()))
                .or_else(|| self.conns.get(&host).and_then(|c| c.home()));
            match dir {
                Some(d) => self.set_root(&host, &d),
                None => {
                    ui.label(RichText::new("waiting for the shell to report its directory…").color(theme::text_faint()));
                    return;
                }
            }
        }
        let (host, root) = self.side.root.clone().unwrap();
        // The active terminal can be sent to the browsed folder when it is on
        // this host and waiting at a prompt.
        let cd_target = self.active_pane_id().filter(|id| self.panes.get(id).map(|p| p.host == host && p.term.at_prompt()).unwrap_or(false));
        let cd_prof = self.profile(&host);
        let mut cd_to: Option<String> = None;
        // Header: host + breadcrumbs.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.painter().circle_filled(ui.cursor().min + vec2(5.0, 11.0), 3.5, host_color(&host));
            ui.add_space(12.0);
            // Buttons first, from the right; the host name gets what is left
            // (cut with … and shown whole on hover) instead of running under them.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let tip = match cd_target {
                    Some(_) => format!("cd the terminal here: {root}"),
                    None => "cd the terminal here (needs a terminal on this host at a prompt)".to_string(),
                };
                let color = if cd_target.is_some() { theme::text_dim() } else { theme::text_faint() };
                if icons::button_colored(ui, Icon::EnterHere, &tip, color).clicked() && cd_target.is_some() {
                    cd_to = Some(root.clone());
                }
                if icons::button(ui, Icon::Refresh, "refresh (back to the terminal's directory when following)").clicked() {
                    // Following: go back to the terminal's directory even after
                    // browsing elsewhere (Up, breadcrumbs).
                    let target = self.active_pane().filter(|_| self.side.follow).and_then(|p| p.term.cwd.clone().map(|c| (p.host.clone(), c)));
                    let (h, dir) = target.unwrap_or_else(|| (host.clone(), root.clone()));
                    let keys: Vec<_> = self.dir_cache.keys().filter(|(k, _)| k == &h).cloned().collect();
                    for k in keys {
                        self.dir_cache.remove(&k);
                    }
                    self.side.root = Some((h.clone(), dir.clone()));
                    self.actions.push(Action::ListDir(h, dir));
                }
                if icons::button(ui, Icon::Up, "parent directory").clicked() {
                    self.set_root(&host, &parent(&root));
                }
                let fc = if self.side.follow { theme::accent() } else { theme::text_faint() };
                if icons::button_colored(ui, Icon::Terminal, "follow the terminal's directory", fc).clicked() {
                    self.side.follow = !self.side.follow;
                    self.settings.follow_cwd = self.side.follow;
                    if self.side.follow {
                        self.on_active_changed();
                    }
                }
                // A new folder or file in the folder shown.
                ui.menu_button(RichText::new("+").size(15.0).color(theme::text_dim()), |ui| {
                    use crate::fileops::FileOpKind;
                    for (label, kind) in [("New folder…", FileOpKind::NewFolder), ("New file…", FileOpKind::NewFile)] {
                        if ui.button(label).clicked() {
                            self.ask_file_op(kind, &host, &root, true);
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("new folder or file here");
                // What to show: hidden files, what git ignores.
                let lit = self.settings.show_hidden || !self.settings.show_ignored;
                ui.menu_button(RichText::new("\u{f06e}").size(13.0).color(if lit { theme::accent() } else { theme::text_dim() }), |ui| {
                    let mut changed = ui.checkbox(&mut self.settings.show_hidden, "Hidden files (.name, hidden attribute)").changed();
                    changed |= ui.checkbox(&mut self.settings.show_ignored, "Git-ignored files (.gitignore)").changed();
                    if changed {
                        self.dir_cache.clear();
                        self.save_settings();
                    }
                })
                .response
                .on_hover_text(format!(
                    "show: hidden files {}, git-ignored files {}",
                    if self.settings.show_hidden { "on" } else { "off" },
                    if self.settings.show_ignored { "on" } else { "off" }
                ));
                ui.add_space(4.0);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    // Browse another host here (each side panel can show its
                    // own: two hosts side by side, drag files across to copy).
                    let mut pick: Option<String> = None;
                    let hosts: Vec<String> = self.settings.profiles.iter().map(|p| p.name.clone()).collect();
                    ui.menu_button(RichText::new(format!("{host} \u{25be}")).size(12.0).color(theme::text()), |ui| {
                        for h in &hosts {
                            if ui.selectable_label(*h == host, h).clicked() {
                                pick = Some(h.clone());
                                ui.close_menu();
                            }
                        }
                    })
                    .response
                    .on_hover_text(format!("{host} — browse another host in this panel"));
                    if let Some(h) = pick.filter(|h| *h != host) {
                        let home = match self.conns.get(&h).and_then(|c| c.home()) {
                            Some(d) => d,
                            None if h == "local" => std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| "/".into()).replace('\\', "/"),
                            None => "~".into(),
                        };
                        self.side.follow = false;
                        self.set_root(&h, &home);
                    }
                });
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut acc = String::new();
            let parts: Vec<&str> = root.split('/').collect();
            for (i, part) in parts.iter().enumerate() {
                if i == 0 {
                    acc = if part.is_empty() { "/".into() } else { format!("{part}/") };
                } else if !part.is_empty() {
                    acc = join(&acc, part);
                } else {
                    continue;
                }
                let label = if i == 0 && part.is_empty() { "/".to_string() } else { part.to_string() };
                let last = i == parts.len() - 1;
                let r = ui.add(egui::Label::new(RichText::new(weft_core::shown(&label)).size(11.5).color(if last { theme::text() } else { theme::text_dim() })).sense(Sense::click()));
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if r.clicked() {
                    self.set_root(&host, &acc.clone());
                }
                let seg = acc.clone();
                r.on_hover_text("click: browse here \u{00b7} right-click: cd the terminal").context_menu(|ui| {
                    if ui.add_enabled(cd_target.is_some(), egui::Button::new("cd terminal here")).clicked() {
                        cd_to = Some(seg.clone());
                        ui.close_menu();
                    }
                });
                if !last && i > 0 {
                    ui.label(RichText::new(" / ").size(11.5).color(theme::text_faint()));
                }
            }
        });
        if let (Some(dir), Some(id)) = (cd_to, cd_target) {
            self.actions.push(Action::Run(id, cd_prof.cd_command(&dir)));
        }
        // Filter over everything under the root: fuzzy / glob / regex.
        let mut open_first = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let w = (ui.available_width() - 72.0).max(60.0);
            let r = ui.add(TextEdit::singleline(&mut self.side.filter).hint_text(self.settings.find_mode.hint()).desired_width(w));
            open_first = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            // One dropdown for the match mode.
            let before = self.settings.find_mode;
            egui::ComboBox::from_id_salt("find-mode")
                .selected_text(RichText::new(before.label()).size(11.5))
                .width(64.0)
                .show_ui(ui, |ui| {
                    for m in FindMode::ALL {
                        ui.selectable_value(&mut self.settings.find_mode, m, m.label()).on_hover_text(m.hint());
                    }
                })
                .response
                .on_hover_text(before.hint());
        });
        ui.add_space(2.0);
        if !self.side.filter.trim().is_empty() {
            self.ui_find_results(ui, &host, &root, open_first);
            return;
        }

        // Flatten the visible tree from the local cache.
        struct TRow {
            depth: usize,
            name: String,
            path: String,
            dir: bool,
            link: bool,
            size: u64,
            open: bool,
        }
        let mut rows: Vec<TRow> = Vec::new();
        let mut want: Vec<String> = Vec::new();
        let filter = self.side.filter.to_lowercase();
        fn walk(app: &App, host: &str, dir: &str, depth: usize, filter: &str, rows: &mut Vec<TRow>, want: &mut Vec<String>) {
            let key = (host.to_string(), dir.to_string());
            match app.dir_cache.get(&key) {
                None => want.push(dir.to_string()),
                Some(l) => {
                    if l.at.elapsed() > Duration::from_secs(30) {
                        want.push(dir.to_string());
                    }
                    for e in &l.entries {
                        let path = join(dir, &e.name);
                        let open = e.dir && app.side.expanded.contains(&(host.to_string(), path.clone()));
                        let matches = filter.is_empty() || e.name.to_lowercase().contains(filter);
                        if matches || open {
                            rows.push(TRow { depth, name: e.name.clone(), path: path.clone(), dir: e.dir, link: e.link, size: e.size, open });
                        }
                        if open && depth < 12 {
                            walk(app, host, &path, depth + 1, filter, rows, want);
                        }
                    }
                }
            }
        }
        walk(self, &host, &root, 0, &filter, &mut rows, &mut want);
        for w in want {
            self.list_dir(&host, &w);
        }
        let loading = self.inflight.contains(&(host.clone(), root.clone())) && !self.dir_cache.contains_key(&(host.clone(), root.clone()));
        let root_error = self.dir_cache.get(&(host.clone(), root.clone())).and_then(|l| l.error.clone());
        let pane = self.active_pane_id();
        let at_prompt = pane.and_then(|id| self.panes.get(&id)).map(|p| p.term.at_prompt()).unwrap_or(false);
        let prof = self.profile(&host);
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("listing…").color(theme::text_faint()));
                });
            }
            if let Some(e) = &root_error {
                ui.label(RichText::new(e).size(12.0).color(theme::error()));
            } else if rows.is_empty() && !loading {
                ui.label(RichText::new("empty").color(theme::text_faint()));
            }
            let send_to = self.send_targets(&host);
            for r in &rows {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                // Dragged onto another Files panel, a folder or a terminal: copied there.
                if resp.drag_started() {
                    self.file_drag = Some((host.clone(), r.path.clone()));
                }
                if r.dir {
                    self.drop_dirs.push((rect.intersect(ui.clip_rect()), host.clone(), r.path.clone()));
                }
                if self.file_drag.as_ref().is_some_and(|d| d.0 == host && d.1 == r.path) {
                    ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0_f32, theme::accent()), egui::StrokeKind::Inside);
                }
                let p = ui.painter();
                let selected = self.side.selected.as_deref() == Some(r.path.as_str());
                if selected {
                    p.rect_filled(rect, CornerRadius::same(4), theme::tint(theme::accent(), 45));
                } else if resp.hovered() {
                    p.rect_filled(rect, CornerRadius::same(4), theme::bg_hover());
                }
                let x = rect.left() + 4.0 + r.depth as f32 * 14.0;
                if r.dir {
                    icons::paint(p, egui::Rect::from_min_size(pos2(x, rect.top() + 5.0), vec2(12.0, 12.0)), if r.open { Icon::ChevronDown } else { Icon::ChevronRight }, theme::text_faint());
                }
                icons::paint(p, egui::Rect::from_min_size(pos2(x + 14.0, rect.top() + 3.0), vec2(16.0, 16.0)), if r.dir { Icon::Folder } else { Icon::File }, if r.dir { theme::ansi(4) } else { theme::text_faint() });
                let name_col = if r.dir { theme::text() } else { theme::text_dim() };
                let label = if r.link { format!("{} ↪", r.name) } else { r.name.clone() };
                let mut name_end = rect.right() - 6.0;
                if !r.dir {
                    let size = human_size(r.size);
                    name_end -= text_width(p, &size, FontId::proportional(11.0)) + 8.0;
                    p.text(pos2(rect.right() - 6.0, rect.center().y), Align2::RIGHT_CENTER, size, FontId::proportional(11.0), theme::text_faint());
                }
                text_fit(p, pos2(x + 34.0, rect.center().y), &label, FontId::proportional(13.0), name_col, name_end - (x + 34.0));
                // One click selects (and expands folders); opening a file takes a
                // double-click or the context menu.
                if resp.clicked() || resp.secondary_clicked() {
                    self.side.selected = Some(r.path.clone());
                }
                if resp.clicked() && r.dir {
                    let key = (host.clone(), r.path.clone());
                    if !self.side.expanded.remove(&key) {
                        self.side.expanded.insert(key);
                        self.actions.push(Action::ListDir(host.clone(), r.path.clone()));
                    }
                }
                if resp.double_clicked() {
                    if r.dir {
                        match pane {
                            Some(id) if at_prompt => self.actions.push(Action::Run(id, prof.cd_command(&r.path))),
                            _ => self.actions.push(Action::SetRoot(host.clone(), r.path.clone())),
                        }
                    } else {
                        self.actions.push(Action::OpenFile(host.clone(), r.path.clone(), 1));
                    }
                }
                let (path, isdir) = (r.path.clone(), r.dir);
                resp.on_hover_text(if isdir { "click: expand · double-click: cd" } else { "double-click: open in editor" }).context_menu(|ui| {
                    if !isdir && ui.button("Open in editor").clicked() {
                        self.actions.push(Action::OpenFile(host.clone(), path.clone(), 1));
                        ui.close_menu();
                    }
                    if isdir {
                        if ui.add_enabled(at_prompt, egui::Button::new("cd in terminal")).clicked() {
                            if let Some(id) = pane {
                                self.actions.push(Action::Run(id, prof.cd_command(&path)));
                            }
                            ui.close_menu();
                        }
                        if ui.button("Make this the root").clicked() {
                            self.actions.push(Action::SetRoot(host.clone(), path.clone()));
                            ui.close_menu();
                        }
                        if ui.button("New tab here").clicked() {
                            self.actions.push(Action::NewTab(host.clone(), Some(path.clone())));
                            ui.close_menu();
                        }
                    }
                    if ui.button("Insert path at prompt").clicked() {
                        if let Some(id) = pane {
                            self.actions.push(Action::Send(id, format!("{} ", prof.quote_path(&path)).into_bytes()));
                        }
                        ui.close_menu();
                    }
                    if ui.button("Copy path").clicked() {
                        self.actions.push(Action::Copy(path.clone()));
                        ui.close_menu();
                    }
                    ui.separator();
                    {
                        use crate::fileops::{parent_of, FileOpKind};
                        // New ones go into a folder, or beside a file.
                        let into = if isdir { path.clone() } else { parent_of(&path) };
                        for (label, kind) in [("New folder…", FileOpKind::NewFolder), ("New file…", FileOpKind::NewFile)] {
                            if ui.button(label).clicked() {
                                if isdir {
                                    self.side.expanded.insert((host.clone(), path.clone()));
                                }
                                self.ask_file_op(kind, &host, &into, true);
                                ui.close_menu();
                            }
                        }
                        if ui.button("Rename…").clicked() {
                            self.ask_file_op(FileOpKind::Rename, &host, &path, isdir);
                            ui.close_menu();
                        }
                        if ui.button("Delete…").clicked() {
                            self.ask_file_op(FileOpKind::Delete, &host, &path, isdir);
                            ui.close_menu();
                        }
                    }
                    // Copies to another host or folder (also by dragging the row).
                    ui.separator();
                    if host != "local" && ui.button("Download").on_hover_text(format!("to {}", crate::transfer::downloads_dir())).clicked() {
                        self.start_transfer(&host, &path, "local", &crate::transfer::downloads_dir());
                        ui.close_menu();
                    }
                    ui.menu_button("Send to", |ui| {
                        if send_to.is_empty() {
                            ui.label(RichText::new("open another host's Files panel or terminal first").size(12.0).color(theme::text_faint()));
                        }
                        for (h, dir, why) in &send_to {
                            if ui.button(format!("{h}:{dir}")).on_hover_text(why).clicked() {
                                self.start_transfer(&host, &path, h, dir);
                                ui.close_menu();
                            }
                        }
                    });
                });
            }
        });
    }

    /// History panel: every recorded command (all hosts, all days), searched
    /// by command (fuzzy / glob / regex) and optionally by output.
    fn ui_history(&mut self, ui: &mut Ui) {
        use crate::records::{local_time, Record};
        if self.records.is_none() {
            self.records = Some(crate::records::load(&self.data.root));
        }
        let mut reload = false;
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 130.0).max(80.0);
            ui.add(TextEdit::singleline(&mut self.records_query).hint_text("search commands…").desired_width(w));
            egui::ComboBox::from_id_salt("history-mode")
                .width(58.0)
                .selected_text(RichText::new(self.settings.find_mode.label()).size(12.0))
                .show_ui(ui, |ui| {
                    for m in FindMode::ALL {
                        ui.selectable_value(&mut self.settings.find_mode, m, m.label()).on_hover_text(m.hint());
                    }
                });
            if icons::button(ui, Icon::Refresh, "read the records again").clicked() {
                reload = true;
            }
        });
        let all = self.records.take().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.records_in_output, RichText::new("in output too").size(12.0)).on_hover_text("also find the text in what the commands printed");
            let mut hosts: Vec<&str> = all.iter().map(|r| r.host.as_str()).collect();
            hosts.sort();
            hosts.dedup();
            egui::ComboBox::from_id_salt("history-host")
                .width(90.0)
                .selected_text(RichText::new(self.records_host.clone().unwrap_or_else(|| "all hosts".into())).size(12.0))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.records_host, None, "all hosts");
                    for h in hosts {
                        ui.selectable_value(&mut self.records_host, Some(h.to_string()), h);
                    }
                });
            if ui.small_button("Records…").on_hover_text("what is kept, for how long, how much").clicked() {
                self.open_records_window();
            }
        });
        if !self.settings.records.commands.on {
            ui.label(RichText::new("Recording commands is off (Records…).").size(11.5).color(theme::warn()));
        }
        // Matches, worked out again only when the search or the records change.
        let q = self.records_query.trim().to_string();
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (&q, self.settings.find_mode.label(), self.records_in_output, &self.records_host, all.len()).hash(&mut h);
            h.finish()
        };
        let cache_id = Id::new("history-hits");
        // The query's key, and the records it found (or why it can't be read).
        type Hits = (u64, Arc<Result<Vec<usize>, String>>);
        let cached: Option<Hits> = ui.data(|d| d.get_temp(cache_id));
        let hits = match cached {
            Some((k, hits)) if k == key => hits,
            _ => {
                let hits = Arc::new(history_matches(&all, &q, self.settings.find_mode, self.records_in_output, self.records_host.as_deref()));
                ui.data_mut(|d| d.insert_temp(cache_id, (key, hits.clone())));
                hits
            }
        };
        let hits: &[usize] = match hits.as_ref() {
            Ok(h) => h,
            Err(e) => {
                ui.colored_label(theme::error(), e);
                &[]
            }
        };
        ui.label(RichText::new(format!("{} of {} commands", hits.len(), all.len())).size(11.0).color(theme::text_faint()));
        let pane = self.active_pane_id();
        let mut open: Option<&Record> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 40.0, hits.len(), |ui, range| {
            for &i in &hits[range] {
                let r = &all[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                let pt = ui.painter();
                if resp.hovered() {
                    pt.rect_filled(rect, CornerRadius::same(5), theme::bg_hover());
                }
                let (icon, col) = match r.exit {
                    Some(0) => (Icon::Check, theme::success()),
                    None => (Icon::Dot, theme::text_faint()),
                    _ => (Icon::Cross, theme::error()),
                };
                icons::paint(pt, egui::Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(14.0, 14.0)), icon, col);
                let cmd: String = if r.cmd.is_empty() { "(output)".into() } else { r.cmd.replace('\n', " ") };
                let room = rect.width() - 34.0;
                text_fit(pt, rect.min + vec2(26.0, 12.0), &cmd, FontId::monospace(12.5), theme::text(), room);
                let exit = match r.exit {
                    Some(c) if c != 0 => format!(" · exit {c}"),
                    _ => String::new(),
                };
                let meta = format!("{} · {} · {}{exit} · {}", local_time(r.t), r.host, fmt_dur(Duration::from_secs_f64(r.dur)), r.cwd);
                text_fit(pt, rect.min + vec2(26.0, 29.0), &meta, FontId::proportional(11.0), theme::text_faint(), room);
                let resp = resp.on_hover_text("click: open with its output · right-click: more");
                if resp.clicked() {
                    open = Some(r);
                }
                resp.context_menu(|ui| {
                    if ui.button("Copy command").clicked() {
                        self.actions.push(Action::Copy(r.cmd.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Copy output").clicked() {
                        self.actions.push(Action::Copy(r.out.clone()));
                        ui.close_menu();
                    }
                    if let Some(id) = pane {
                        if ui.add_enabled(!r.cmd.is_empty(), egui::Button::new("Run again in this pane")).clicked() {
                            self.actions.push(Action::Run(id, r.cmd.clone()));
                            ui.close_menu();
                        }
                    }
                    if ui.button("Open in editor").clicked() {
                        open = Some(r);
                        ui.close_menu();
                    }
                });
            }
        });
        if let Some(r) = open {
            let exit = r.exit.map(|c| c.to_string()).unwrap_or_else(|| "not reported".into());
            let cut = if r.cut > 0 { format!("\n({} bytes of output left out in the middle)", r.cut) } else { String::new() };
            let text = format!("$ {}\n# {} · {} · {} · exit {exit} · {}{cut}\n\n{}", r.cmd, local_time(r.t), r.host, r.cwd, fmt_dur(Duration::from_secs_f64(r.dur)), r.out);
            self.actions.push(Action::OpenScratch(format!("{} {}", local_time(r.t), r.cmd.chars().take(30).collect::<String>()), text));
        }
        self.records = if reload { None } else { Some(all) };
    }

    /// Help: every command by group (click to run) with the other keys, or
    /// what each feature does. F1, `?` in the tab bar, or the ☰ menu.
    pub fn ui_help(&mut self, ctx: &egui::Context) {
        let Some(mut h) = self.help.take() else { return };
        let key_labels: Vec<String> = COMMANDS.iter().map(|c| self.keys_label(c.0)).collect();
        let mut open = true;
        let mut run: Option<Cmd> = None;
        crate::widgets::dialog(ctx, "Help", vec2(600.0, 620.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(!h.features, "Commands and keys").clicked() {
                    h.features = false;
                }
                if ui.selectable_label(h.features, "Features").clicked() {
                    h.features = true;
                }
                ui.add(TextEdit::singleline(&mut h.query).hint_text("search help…").desired_width(f32::INFINITY));
            });
            // Which weft this is (also `weft --version`, and the palette's
            // About), and the test screens to look at with one's own eyes.
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::version::line()).size(11.5).color(theme::text_faint()));
                if ui.link(RichText::new("Display test\u{2026}").size(11.5)).on_hover_text("test screens: how this font draws blocks, colours, columns and boxes").clicked() {
                    run = Some(Cmd::DisplayTest);
                }
            });
            ui.separator();
            let q = h.query.to_lowercase();
            let hit = |texts: &[&str]| q.is_empty() || texts.iter().any(|t| t.to_lowercase().contains(&q));
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if h.features {
                    for (title, text) in crate::help::FEATURES {
                        if hit(&[title, text]) {
                            egui::CollapsingHeader::new(RichText::new(*title).strong()).default_open(true).show(ui, |ui| {
                                ui.label(*text);
                            });
                        }
                    }
                    return;
                }
                for g in GROUPS {
                    let items: Vec<_> = COMMANDS.iter().zip(&key_labels).filter(|(c, k)| group(c.0) == g && hit(&[c.2, k])).collect();
                    if items.is_empty() {
                        continue;
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new(g).strong().color(theme::text_dim()));
                    for ((cmd, _, label, _), keys) in items {
                        let r = ui.add(egui::Button::new(*label).shortcut_text(keys.as_str()).frame(false).min_size(vec2(ui.available_width(), 0.0)));
                        if r.on_hover_text("click to run").clicked() {
                            run = Some(*cmd);
                        }
                    }
                }
                let keys: Vec<_> = crate::help::KEYS.iter().filter(|k| hit(&[k.0, k.1])).collect();
                if !keys.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Keys and mouse").strong().color(theme::text_dim()));
                    // The descriptions wrap: in one line they widened the window
                    // past a small screen's edge.
                    let key_w = (ui.available_width() * 0.4).min(220.0);
                    let what_w = (ui.available_width() - key_w - 16.0).max(120.0);
                    egui::Grid::new("help-keys").num_columns(2).spacing([16.0, 4.0]).max_col_width(what_w).show(ui, |ui| {
                        for (k, what) in keys {
                            ui.add(egui::Label::new(RichText::new(*k).monospace().color(theme::accent())).wrap());
                            ui.add(egui::Label::new(*what).wrap());
                            ui.end_row();
                        }
                    });
                }
            });
        });
        if let Some(c) = run {
            open = false;
            self.run_cmd(c);
        }
        if open {
            self.help = Some(h);
        }
    }

    /// The ☰ menu: every command under its group's heading, one level only
    /// (submenus opened on hover and switched while moving to them).
    fn commands_menu(&mut self, ui: &mut Ui) {
        let mut run = None;
        let key_labels: Vec<String> = COMMANDS.iter().map(|c| self.keys_label(c.0)).collect();
        ui.menu_button(RichText::new("☰").size(14.0).color(theme::text_dim()), |ui| {
            ui.set_min_width(340.0);
            let height = (ui.ctx().screen_rect().height() - 80.0).max(200.0);
            ScrollArea::vertical().max_height(height).show(ui, |ui| {
                for (gi, g) in GROUPS.iter().enumerate() {
                    if gi > 0 {
                        ui.separator();
                    }
                    ui.label(RichText::new(*g).size(11.5).strong().color(theme::text_dim()));
                    for ((cmd, _, label, _), keys) in COMMANDS.iter().zip(&key_labels).filter(|(c, _)| group(c.0) == *g) {
                        if ui.add(egui::Button::new(*label).shortcut_text(keys.as_str()).min_size(vec2(ui.available_width(), 0.0))).clicked() {
                            run = Some(*cmd);
                            ui.close_menu();
                        }
                    }
                }
            });
        })
        .response
        .on_hover_text("all commands");
        if let Some(c) = run {
            self.run_cmd(c);
        }
    }

    /// Keyboard shortcuts: every command with its keys; + binds the next key
    /// pressed, a key's × removes it, Reset goes back to the defaults.
    pub fn ui_keys_window(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.keys_window.take() else { return };
        let mut open = true;
        // Who uses each key, to flag a key bound twice.
        let mut uses: HashMap<String, Vec<&str>> = HashMap::new();
        for (c, keys) in &self.keymap {
            let label = COMMANDS.iter().find(|x| x.0 == *c).map(|x| x.2).unwrap_or("");
            for k in keys {
                uses.entry(k.label()).or_default().push(label);
            }
        }
        let mut change: Option<(&'static str, Option<Vec<String>>)> = None;
        let mut reset_all = false;
        let q = w.query.to_lowercase();
        let clashing = |keys: &[String], label: &str| keys.iter().any(|k| uses.get(k).is_some_and(|v| v.iter().any(|x| *x != label)));
        let changed_n = self.settings.keys.len();
        let clash_n = COMMANDS.iter().filter(|c| clashing(&self.keymap_labels(c.0), c.2)).count();
        crate::widgets::dialog(ctx, "Keyboard shortcuts", vec2(680.0, 600.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut w.query).hint_text("search commands or keys…").desired_width(ui.available_width() - 96.0));
                if ui.add_enabled(changed_n > 0, egui::Button::new("Reset all")).on_hover_text("every command back to its default keys").clicked() {
                    reset_all = true;
                }
            });
            ui.horizontal(|ui| {
                ui.toggle_value(&mut w.changed_only, RichText::new(format!("changed ({changed_n})")).size(12.0)).on_hover_text("only the commands whose keys you changed");
                ui.add_enabled_ui(clash_n > 0 || w.clashes_only, |ui| {
                    let label = RichText::new(format!("clashes ({clash_n})")).size(12.0);
                    let label = if clash_n > 0 { label.color(theme::error()) } else { label };
                    ui.toggle_value(&mut w.clashes_only, label).on_hover_text("only the commands sharing a key with another");
                });
                ui.label(RichText::new("+ then press the keys · click a key to remove it · bound keys never reach the terminal").size(11.0).color(theme::text_faint()));
            });
            ui.add_space(2.0);
            ui.separator();
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut stripe = false;
                for g in GROUPS {
                    let rows: Vec<_> = COMMANDS
                        .iter()
                        .filter(|c| group(c.0) == g)
                        .filter(|c| q.is_empty() || c.2.to_lowercase().contains(&q) || c.1.contains(&q) || self.keys_label(c.0).to_lowercase().contains(&q))
                        .filter(|c| !w.changed_only || self.settings.keys.contains_key(c.1))
                        .filter(|c| !w.clashes_only || clashing(&self.keymap_labels(c.0), c.2))
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new(g).size(11.5).strong().color(theme::text_dim()));
                    ui.add_space(2.0);
                    for (cmd, id, label, defaults) in rows {
                        stripe = !stripe;
                        let keys = self.keymap_labels(*cmd);
                        let custom = self.settings.keys.contains_key(*id);
                        let capturing = w.capture == Some(*id);
                        // One line per command: its name on the left; on the
                        // right its keys, the + to add one, and ↺ when changed.
                        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
                        let bg = if capturing {
                            theme::tint(theme::accent(), 40)
                        } else if resp.hovered() {
                            theme::bg_hover()
                        } else if stripe {
                            theme::tint(theme::text_faint(), 7)
                        } else {
                            Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(row, CornerRadius::same(5), bg);
                        if capturing {
                            ui.painter().rect_stroke(row, CornerRadius::same(5), Stroke::new(1.0_f32, theme::accent()), egui::StrokeKind::Inside);
                        }
                        let mut right = ui.new_child(egui::UiBuilder::new().max_rect(row.shrink2(vec2(6.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
                        right.spacing_mut().item_spacing.x = 4.0;
                        let def = if defaults.is_empty() { "no key".to_string() } else { defaults.join(", ") };
                        if custom {
                            if icons::button(&mut right, Icon::Refresh, &format!("back to the default: {def}")).clicked() {
                                change = Some((id, None));
                            }
                        } else {
                            right.add_space(22.0);
                        }
                        if capturing {
                            right.label(RichText::new("press keys… (Esc cancels)").size(12.0).color(theme::accent()));
                        } else if icons::button(&mut right, Icon::Plus, "add a key: click, then press it").clicked() {
                            w.capture = Some(id);
                        }
                        if keys.is_empty() && !capturing {
                            right.label(RichText::new("no key").size(11.5).italics().color(theme::text_faint()));
                        }
                        for k in keys.iter().rev() {
                            let others: Vec<&str> = uses.get(k).map(|v| v.iter().copied().filter(|x| x != label).collect()).unwrap_or_default();
                            if key_chip(&mut right, k, !others.is_empty(), &others).clicked() {
                                let rest: Vec<String> = keys.iter().filter(|x| *x != k).cloned().collect();
                                change = Some((id, Some(rest)));
                            }
                        }
                        // The name gets what the right side left.
                        let name_right = right.min_rect().left() - 10.0;
                        let x = row.left() + 10.0;
                        if custom {
                            ui.painter().circle_filled(pos2(row.left() + 6.0, row.center().y), 2.5, theme::accent());
                        }
                        let color = if custom { theme::text() } else { theme::text_dim() };
                        text_fit(ui.painter(), pos2(x + 4.0, row.center().y), label, FontId::proportional(13.0), color, name_right - x - 4.0);
                        if custom {
                            resp.on_hover_text(format!("changed · default: {def}"));
                        }
                    }
                }
            });
        });
        if reset_all {
            self.settings.keys.clear();
            self.rebuild_keys();
            self.save_settings();
        } else if let Some((id, keys)) = change {
            self.set_bindings(id, keys);
        }
        if open {
            self.keys_window = Some(w);
        }
    }

    /// The memo list (editor tab bar, palette, empty editor): newest first,
    /// filtered by `query`; returns what was clicked.
    fn memo_rows(&mut self, ui: &mut Ui, max: usize, search: bool) -> Option<MemoPick> {
        let mut pick = None;
        if search {
            ui.add(TextEdit::singleline(&mut self.memo_query).hint_text("search memos…").desired_width(f32::INFINITY));
        }
        let q = self.memo_query.to_lowercase();
        let memos: Vec<MemoInfo> = self.memos().into_iter().filter(|m| q.is_empty() || m.title.to_lowercase().contains(&q) || m.preview.to_lowercase().contains(&q)).take(max).collect();
        if memos.is_empty() {
            let msg = if q.is_empty() { "no memos yet" } else { "no memo matches" };
            ui.label(RichText::new(msg).size(12.0).color(theme::text_faint()));
        }
        for m in memos {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(260.0), 38.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(5), theme::bg_hover());
            }
            text_fit(ui.painter(), rect.min + vec2(8.0, 12.0), &m.title, FontId::proportional(13.0), theme::text(), rect.width() - 16.0);
            let sub = format!("{} · {}", crate::records::local_time(m.modified), m.preview);
            let g = ui.painter().layout_no_wrap(sub, FontId::proportional(11.0), theme::text_faint());
            ui.painter().with_clip_rect(rect.shrink2(vec2(8.0, 0.0))).galley(rect.min + vec2(8.0, 21.0), g, theme::text_faint());
            let resp = resp.on_hover_text("click: open · right-click: delete");
            if resp.clicked() {
                pick = Some(MemoPick::Open(m.path.clone()));
            }
            resp.context_menu(|ui| {
                if ui.button(RichText::new("Delete this memo").color(theme::error())).clicked() {
                    pick = Some(MemoPick::Delete(m.path.clone()));
                    ui.close_menu();
                }
            });
        }
        pick
    }

    fn apply_memo_pick(&mut self, pick: MemoPick) {
        match pick {
            MemoPick::Open(p) => {
                self.memo_picker = false;
                self.open_file("local", &p, 1);
            }
            MemoPick::Delete(p) => self.delete_memo_file(&p),
        }
    }

    /// "Open memo…" from the palette: the memo list in a window.
    pub fn ui_memo_picker(&mut self, ctx: &egui::Context) {
        if !self.memo_picker {
            return;
        }
        let mut open = true;
        let mut pick = None;
        let mut new = false;
        crate::widgets::dialog(ctx, "Memos", vec2(380.0, 460.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("New memo").on_hover_text("Ctrl+Shift+M").clicked() {
                    new = true;
                }
                ui.label(RichText::new("saved in data/memos").size(11.0).color(theme::text_faint()));
            });
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                pick = self.memo_rows(ui, usize::MAX, true);
            });
        });
        self.memo_picker = open && !new;
        if new {
            self.new_memo();
        }
        if let Some(p) = pick {
            self.apply_memo_pick(p);
        }
    }

    pub fn open_records_window(&mut self) {
        let root = self.data.root.clone();
        self.records_window = Some(crate::records::Item::ALL.iter().map(|&i| (i, i.size(&root))).collect());
    }

    /// What is kept on disk and for how long, item by item.
    pub fn ui_records_window(&mut self, ctx: &egui::Context) {
        use crate::records::Item;
        let Some(sizes) = self.records_window.take() else { return };
        let mut open = true;
        let mut changed = false;
        let mut delete: Option<Item> = None;
        let mut pw_changed = false;
        let dir = self.data.root.display().to_string();
        crate::widgets::dialog(ctx, "Privacy", vec2(560.0, 680.0)).open(&mut open).resizable(false).show(ctx, |ui| {
            // Taller than a small window: it scrolls.
            ScrollArea::vertical().show(ui, |ui| {
                ui.label(RichText::new(format!("Kept in {dir}")).size(11.5).color(theme::text_faint()));
                ui.add_space(4.0);
                egui::Grid::new("records-grid").num_columns(5).spacing([14.0, 8.0]).show(ui, |ui| {
                    for h in ["", "Keep", "Up to", "Now", ""] {
                        ui.label(RichText::new(h).size(11.5).color(theme::text_dim()));
                    }
                    ui.end_row();
                    for (item, size) in &sizes {
                        let rule = item.rule_mut(&mut self.settings.records);
                        changed |= ui.checkbox(&mut rule.on, item.label()).on_hover_text(item.hint()).changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut rule.keep_days).range(0..=3650).speed(0.5).custom_formatter(|v, _| if v == 0.0 { "forever".into() } else { format!("{v} days") }))
                            .on_hover_text("drag or type; 0 = forever")
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut rule.max_mb).range(0..=100_000).speed(1.0).custom_formatter(|v, _| if v == 0.0 { "no limit".into() } else { format!("{v} MB") }))
                            .on_hover_text(if *item == Item::History { "per host; drag or type; 0 = no limit" } else { "drag or type; 0 = no limit" })
                            .changed();
                        ui.label(RichText::new(human_size(*size)).size(12.0).color(theme::text_dim()));
                        ui.menu_button(RichText::new("Delete…").size(12.0), |ui| {
                            if ui.button(RichText::new(format!("Delete all {}", item.label().to_lowercase())).color(theme::error())).clicked() {
                                delete = Some(*item);
                                ui.close_menu();
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Output kept per command:");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.settings.records.output_kb).range(0..=1_000_000).speed(4.0).custom_formatter(|v, _| if v == 0.0 { "all".into() } else { format!("{v} KB") }))
                        .on_hover_text("longer output keeps its start and end; 0 = all of it")
                        .changed();
                });
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new("ssh passwords in memory").strong());
                ui.label(
                    RichText::new(format!(
                        "{} held now. Never written to disk; ssh gets one through a one-time ticket, not its environment.\n\
                         Keys + ssh-agent are better still: then weft holds no password at all.",
                        self.secrets.len()
                    ))
                    .size(11.0)
                    .color(theme::text_faint()),
                );
                let pw = &mut self.settings.passwords;
                let how = if cfg!(windows) { "Encrypt them (ChaCha20, key sealed by Windows)" } else { "Encrypt them (ChaCha20)" };
                pw_changed |= ui.checkbox(&mut pw.encrypt, how).changed();
                ui.add_enabled_ui(pw.encrypt, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("New key every");
                        pw_changed |= secs_input(ui, "pw-rotate", &mut pw.rotate_secs, "never");
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Forget a password unused for");
                    pw_changed |= secs_input(ui, "pw-forget", &mut pw.forget_secs, "never");
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Limits apply at start and every hour; dated files go a whole day at a time.\n\
                         Keystrokes are never written, so passwords typed at a prompt are in none of these.\n\
                         A command typed with a leading space is left out of commands and history.",
                    )
                    .size(11.0)
                    .color(theme::text_faint()),
                );
            });
        });
        let mut sizes = sizes;
        if let Some(item) = delete {
            let n = crate::records::delete_all(&self.data.root, item);
            if item == Item::Commands {
                self.records = None;
            }
            self.toast(format!("{}: deleted {n} file(s)", item.label()), ToastKind::Info);
            let root = self.data.root.clone();
            sizes = Item::ALL.iter().map(|&i| (i, i.size(&root))).collect();
        }
        if changed {
            self.records_changed();
        }
        if pw_changed {
            self.secrets.configure(self.settings.passwords);
            self.save_settings();
        }
        if open {
            self.records_window = Some(sizes);
        }
    }

    fn ui_blocks(&mut self, ui: &mut Ui) {
        let Some(id) = self.active_pane_id() else { return };
        let mut scroll_to: Option<u64> = None;
        {
            let p = &self.panes[&id];
            let blocks: Vec<_> = p.term.blocks.iter().filter(|b| b.state() != BlockState::Prompt).collect();
            if blocks.is_empty() {
                ui.label(RichText::new("Commands appear here once the shell integration reports them (bash via ssh, or local PowerShell).").color(theme::text_faint()));
            }
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for b in blocks.iter().rev() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let pt = ui.painter();
                    if resp.hovered() {
                        pt.rect_filled(rect, CornerRadius::same(5), theme::bg_hover());
                    }
                    let (icon, col) = match (b.state(), b.exit) {
                        (BlockState::Running, _) => (Icon::Dot, theme::accent()),
                        (_, Some(0)) => (Icon::Check, theme::success()),
                        (_, None) => (Icon::Dot, theme::text_faint()),
                        _ => (Icon::Cross, theme::error()),
                    };
                    icons::paint(pt, egui::Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(14.0, 14.0)), icon, col);
                    let room = rect.width() - 34.0;
                    text_fit(pt, rect.min + vec2(26.0, 12.0), &b.command, FontId::monospace(12.5), theme::text(), room);
                    let lines = match (b.out_line, b.end_line) {
                        (Some(o), Some(e)) => format!("{} lines", e.saturating_sub(o)),
                        _ => "running".into(),
                    };
                    let meta = match b.exit {
                        Some(c) if c != 0 => format!("exit {c} · {} · {lines}", b.elapsed().map(fmt_dur).unwrap_or_default()),
                        _ => format!("{} · {lines}", b.elapsed().map(fmt_dur).unwrap_or_default()),
                    };
                    text_fit(pt, rect.min + vec2(26.0, 29.0), &meta, FontId::proportional(11.0), theme::text_faint(), room);
                    if resp.clicked() {
                        scroll_to = Some(b.prompt_line);
                    }
                    // The output is copied out only when asked for: per row and
                    // per frame it built megabytes while a long output streamed.
                    let cmd = b.command.clone();
                    let out = || p.term.block_output(b);
                    resp.context_menu(|ui| {
                        if ui.button("Copy command").clicked() {
                            self.actions.push(Action::Copy(cmd.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Copy output").clicked() {
                            self.actions.push(Action::Copy(out()));
                            ui.close_menu();
                        }
                        if ui.button("Run again").clicked() {
                            self.actions.push(Action::Run(id, cmd.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Open output in editor").clicked() {
                            self.actions.push(Action::OpenScratch(format!("output: {cmd}"), out()));
                            ui.close_menu();
                        }
                    });
                }
            });
        }
        if let Some(line) = scroll_to {
            let p = self.panes.get_mut(&id).unwrap();
            let all = visual_rows(&p.term);
            if let Some(i) = all.iter().position(|v| *v == VRow::Line(line)) {
                p.scroll = all.len().saturating_sub(i + p.term.rows).min(all.len().saturating_sub(p.term.rows));
            }
            self.focus_term = true;
        }
    }

    fn ui_grep(&mut self, ui: &mut Ui) {
        let Some((host, root)) = self.side.root.clone() else {
            ui.label(RichText::new("open the Files tab first").color(theme::text_faint()));
            return;
        };
        ui.label(RichText::new(format!("in {host}:{}", weft_core::shown(&root))).size(11.5).color(theme::text_faint()));
        let r = ui.add(TextEdit::singleline(&mut self.side.grep_q).hint_text(format!("{} (Enter)", self.side.grep_mode.hint())).desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            self.actions.push(Action::Grep);
        }
        // Wrapped: in a panel as narrow as it starts out the Search button
        // was cut off at the edge.
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("grep-mode").width(62.0).selected_text(RichText::new(self.side.grep_mode.label()).size(12.0)).show_ui(ui, |ui| {
                for m in editfind::MatchMode::ALL {
                    ui.selectable_value(&mut self.side.grep_mode, m, m.label()).on_hover_text(m.hint());
                }
            });
            ui.checkbox(&mut self.side.grep_icase, RichText::new("ignore case").size(12.0));
            ui.checkbox(&mut self.side.grep_hidden, RichText::new("hidden").size(12.0)).on_hover_text("also in hidden files and folders (names starting with .)");
            // As set here, the Search panel opens next time (kept in settings).
            let s = &mut self.settings.search;
            (s.files_mode, s.files_icase, s.files_hidden) = (self.side.grep_mode, self.side.grep_icase, self.side.grep_hidden);
            if ui.button("Search").clicked() {
                self.actions.push(Action::Grep);
            }
            if self.side.grep_busy {
                ui.spinner();
            }
        });
        ui.separator();
        let Some(res) = self.side.grep.clone() else { return };
        let hits = res["hits"].as_array().cloned().unwrap_or_default();
        // The matched part of each line gets the find background.
        let marker = self.side.grep_ran.as_ref().and_then(|(m, q, icase)| editfind::line_matcher(*m, q, *icase));
        let base = res["root"].as_str().unwrap_or("").replace('\\', "/");
        ui.label(RichText::new(format!("{} matches{}", hits.len(), if res["truncated"].as_bool().unwrap_or(false) { " (truncated)" } else { "" })).size(11.5).color(theme::text_dim()));
        // The active terminal can be sent to a result's folder when it is on
        // this host and waiting at a prompt (as in the Files tab).
        let cd_target = self.active_pane_id().filter(|id| self.panes.get(id).map(|p| p.host == host && p.term.at_prompt()).unwrap_or(false));
        let mut chosen: Option<HitAction> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut last_file = String::new();
            for h in hits {
                let path = h[0].as_str().unwrap_or("").replace('\\', "/");
                let line = h[1].as_u64().unwrap_or(1) as usize;
                let text = h[2].as_str().unwrap_or("").trim();
                if path != last_file {
                    ui.add_space(4.0);
                    let rel = path.strip_prefix(&base).unwrap_or(&path).trim_start_matches('/');
                    let r = ui.add(egui::Label::new(RichText::new(weft_core::shown(rel)).size(12.0).color(theme::ansi(12))).sense(Sense::click()));
                    if r.clicked() {
                        chosen = Some(HitAction::Open(path.clone(), 1));
                    }
                    r.on_hover_text("click: open · right-click: cd the terminal, show in Files, copy").context_menu(|ui| {
                        hit_menu(ui, &path, None, cd_target.is_some(), &mut chosen);
                    });
                    last_file = path.clone();
                }
                let shown: String = text.chars().take(120).collect();
                let font = FontId::monospace(12.0);
                let plain = egui::TextFormat { font_id: font.clone(), color: theme::text_dim(), ..Default::default() };
                let mut job = LayoutJob::default();
                job.append(&format!("{line:>5}  "), 0.0, egui::TextFormat { color: theme::text_faint(), ..plain.clone() });
                let mut at = 0;
                for (a, b) in marker.as_ref().map(|re| editfind::line_marks(re, &shown)).unwrap_or_default() {
                    job.append(&shown[at..a], 0.0, plain.clone());
                    job.append(&shown[a..b], 0.0, egui::TextFormat { color: theme::text(), background: theme::search_hit(), ..plain.clone() });
                    at = b;
                }
                job.append(&shown[at..], 0.0, plain);
                let r = ui.add(egui::Label::new(job).sense(Sense::click()));
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if r.clicked() {
                    chosen = Some(HitAction::Open(path.clone(), line));
                }
                r.on_hover_text("click: open at this line · right-click: more").context_menu(|ui| {
                    hit_menu(ui, &path, Some((line, text)), cd_target.is_some(), &mut chosen);
                });
            }
        });
        match chosen {
            Some(HitAction::Open(path, line)) => self.actions.push(Action::OpenFile(host.clone(), path, line)),
            Some(HitAction::Cd(dir)) => {
                if let Some(id) = cd_target {
                    let cmd = self.profile(&host).cd_command(&dir);
                    self.actions.push(Action::Run(id, cmd));
                    self.actions.push(Action::Focus(id));
                }
            }
            Some(HitAction::Reveal(dir, path)) => {
                self.side.tab = SideTab::Files;
                self.side.follow = false;
                self.side.selected = Some(path);
                self.set_root(&host, &dir);
            }
            Some(HitAction::Copy(t)) => self.actions.push(Action::Copy(t)),
            None => {}
        }
    }

    fn ui_procs(&mut self, ui: &mut Ui) {
        let host = self.active_host();
        let stale = self.side.procs_host != host || self.side.procs_at.map(|t| t.elapsed() > Duration::from_secs(3)).unwrap_or(true);
        if stale {
            self.actions.push(Action::Procs);
        }
        ui.ctx().request_repaint_after(Duration::from_secs(3));
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut self.side.procs_filter).hint_text("filter").desired_width(ui.available_width() - 30.0));
            if icons::button(ui, Icon::Refresh, "refresh").clicked() {
                self.actions.push(Action::Procs);
            }
        });
        let rows = match &self.side.procs {
            None => {
                ui.spinner();
                return;
            }
            Some(Err(e)) => {
                ui.colored_label(theme::error(), e);
                return;
            }
            Some(Ok(r)) => r.clone(),
        };
        let f = self.side.procs_filter.to_lowercase();
        let mut rows: Vec<_> = rows.into_iter().filter(|r| f.is_empty() || r.to_string().to_lowercase().contains(&f)).collect();
        let (sc, asc) = self.side.procs_sort;
        rows.sort_by(|a, b| {
            let o = match (a[sc].as_f64(), b[sc].as_f64()) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                _ => a[sc].to_string().cmp(&b[sc].to_string()),
            };
            if asc {
                o
            } else {
                o.reverse()
            }
        });
        let cols = [(0usize, "PID", 52.0), (3, "CPU%", 48.0), (4, "MEM%", 48.0), (2, "USER", 70.0), (6, "COMMAND", 0.0)];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (c, name, w) in cols {
                let w = if w == 0.0 { ui.available_width() } else { w };
                let sel = sc == c;
                let txt = RichText::new(if sel { format!("{name}{}", if asc { " ↑" } else { " ↓" }) } else { name.to_string() }).size(11.5).color(if sel { theme::text() } else { theme::text_dim() });
                if ui.add_sized([w, 20.0], egui::Button::new(txt).frame(false)).clicked() {
                    self.side.procs_sort = if sel { (c, !asc) } else { (c, false) };
                }
            }
        });
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for r in rows.iter().take(400) {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, CornerRadius::same(3), theme::bg_hover());
                }
                let f = FontId::monospace(11.5);
                let cpu = r[3].as_f64().unwrap_or(0.0);
                let mut x = rect.left() + 2.0;
                for (c, _, w) in cols {
                    let text = match c {
                        3 | 4 => format!("{:.1}", r[c].as_f64().unwrap_or(0.0)),
                        _ => r[c].as_str().map(String::from).unwrap_or_else(|| r[c].to_string()),
                    };
                    let col = if c == 3 && cpu > 50.0 { theme::warn() } else if c == 6 { theme::text() } else { theme::text_dim() };
                    let max = if w == 0.0 { ((rect.right() - x) / 7.0) as usize } else { (w / 7.0) as usize };
                    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text.chars().take(max).collect::<String>(), f.clone(), col);
                    x += if w == 0.0 { 0.0 } else { w };
                }
                let pid = r[0].as_i64().unwrap_or(0);
                let cmd = r[6].as_str().unwrap_or("").to_string();
                resp.on_hover_text(cmd.clone()).context_menu(|ui| {
                    ui.label(RichText::new(format!("pid {pid}")).color(theme::text_dim()));
                    if ui.button("Terminate (TERM)").clicked() {
                        self.actions.push(Action::Kill(pid, "TERM"));
                        ui.close_menu();
                    }
                    if ui.button("Kill (KILL)").clicked() {
                        self.actions.push(Action::Kill(pid, "KILL"));
                        ui.close_menu();
                    }
                    if ui.button("Copy command line").clicked() {
                        self.actions.push(Action::Copy(cmd.clone()));
                        ui.close_menu();
                    }
                });
            }
        });
    }

    /// The filter's matches under the root (instead of the tree while typing).
    fn ui_find_results(&mut self, ui: &mut Ui, host: &str, root: &str, open_first: bool) {
        let key = (host.to_string(), root.to_string());
        let stale = self.find_index.get(&key).map(|i| i.at.elapsed() > Duration::from_secs(60)).unwrap_or(true);
        if stale && !self.find_inflight.contains(&key) {
            self.find_inflight.insert(key.clone());
            let req = serde_json::json!({"op": "tree", "path": root, "hidden": self.settings.show_hidden, "max": 20000});
            self.request(host, req, Pending::Tree { root: root.to_string() });
        }
        let Some(index) = self.find_index.get(&key) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("indexing names under this folder\u{2026}").size(12.0).color(theme::text_faint()));
            });
            return;
        };
        let query = self.side.filter.trim().to_string();
        let mode = self.settings.find_mode;
        let rkey = (host.to_string(), root.to_string(), query.clone(), mode, index.generation);
        if self.find_result.as_ref().map(|r| r.key != rkey).unwrap_or(true) {
            let (hits, total, error) = match find::Matcher::new(mode, &query) {
                Ok(m) => {
                    let (h, t) = find::search(&m, &index.entries, 500);
                    (h, t, None)
                }
                Err(e) => (Vec::new(), 0, Some(e)),
            };
            self.find_result = Some(FindResult { key: rkey, hits, total, error });
        }
        let index = &self.find_index[&key];
        let Some(res) = self.find_result.as_ref() else { return };
        let (status, color) = match &res.error {
            Some(e) => (e.lines().last().unwrap_or(e).to_string(), theme::error()),
            None => (
                format!(
                    "{} {} \u{00b7} {} names{}{}",
                    res.total,
                    if res.total == 1 { "match" } else { "matches" },
                    index.entries.len(),
                    if index.truncated { " (first 20000)" } else { "" },
                    if res.total > res.hits.len() { " \u{00b7} first 500 shown" } else { "" }
                ),
                theme::text_faint(),
            ),
        };
        ui.label(RichText::new(status).size(11.0).color(color));
        let selected = self.side.selected.clone();
        let mut select: Option<String> = None;
        let mut open: Option<(String, bool)> = None;
        let mut menu: Option<(String, bool, &'static str)> = None;
        if open_first {
            if let Some((i, _)) = res.hits.first() {
                let (rel, dir) = &index.entries[*i];
                open = Some((join(root, rel), *dir));
            }
        }
        ScrollArea::vertical().id_salt("find-results").auto_shrink([false, false]).show_rows(ui, 22.0, res.hits.len(), |ui, range| {
            for (i, pos) in &res.hits[range] {
                let (rel, dir) = &index.entries[*i];
                let abs = join(root, rel);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                let p = ui.painter();
                if selected.as_deref() == Some(abs.as_str()) {
                    p.rect_filled(rect, CornerRadius::same(4), theme::tint(theme::accent(), 45));
                } else if resp.hovered() {
                    p.rect_filled(rect, CornerRadius::same(4), theme::bg_hover());
                }
                icons::paint(p, egui::Rect::from_min_size(pos2(rect.left() + 4.0, rect.top() + 3.0), vec2(16.0, 16.0)), if *dir { Icon::Folder } else { Icon::File }, if *dir { theme::ansi(4) } else { theme::text_faint() });
                // Name first, then its folder; matched characters in the accent color.
                let chars: Vec<char> = rel.chars().collect();
                let name_at = chars.iter().rposition(|&c| c == '/').map(|k| k + 1).unwrap_or(0);
                let mut job = LayoutJob::default();
                let put = |job: &mut LayoutJob, range: std::ops::Range<usize>, base: Color32, size: f32| {
                    for (c, of) in weft_core::shown_chars(&chars[range.clone()]) {
                        let hit = (of.start + range.start..of.end + range.start).any(|k| pos.contains(&k));
                        let fmt = egui::TextFormat { font_id: FontId::proportional(size), color: if hit { theme::accent() } else { base }, underline: if hit { Stroke::new(1.0_f32, theme::accent()) } else { Stroke::NONE }, ..Default::default() };
                        job.append(&c.to_string(), 0.0, fmt);
                    }
                };
                put(&mut job, name_at..chars.len(), if *dir { theme::text() } else { theme::text_dim() }, 13.0);
                if name_at > 0 {
                    job.append("   ", 0.0, egui::TextFormat { font_id: FontId::proportional(11.0), ..Default::default() });
                    put(&mut job, 0..name_at - 1, theme::text_faint(), 11.0);
                }
                let galley = ui.fonts(|f| f.layout_job(job));
                ui.painter().with_clip_rect(rect).galley(pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0), galley, theme::text());
                if resp.clicked() {
                    select = Some(abs.clone());
                }
                if resp.double_clicked() {
                    open = Some((abs.clone(), *dir));
                }
                let (abs_m, dir_m) = (abs.clone(), *dir);
                resp.on_hover_text(if *dir { "double-click: show in the tree" } else { "double-click: open in editor" }).context_menu(|ui| {
                    for (label, act) in [("Open in editor", "open"), ("Show in the tree", "reveal"), ("Insert path at prompt", "insert"), ("Copy path", "copy")] {
                        if (act != "open" || !dir_m) && ui.button(label).clicked() {
                            menu = Some((abs_m.clone(), dir_m, act));
                            ui.close_menu();
                        }
                    }
                });
            }
        });
        if let Some(s) = select {
            self.side.selected = Some(s);
        }
        match open {
            Some((path, false)) => self.actions.push(Action::OpenFile(host.to_string(), path, 1)),
            Some((path, true)) => self.reveal(host, root, &path),
            None => {}
        }
        if let Some((path, dir, act)) = menu {
            match act {
                "open" => self.actions.push(Action::OpenFile(host.to_string(), path, 1)),
                "reveal" => self.reveal(host, root, &path),
                "insert" => {
                    if let Some(id) = self.active_pane_id() {
                        let q = self.profile(host).quote_path(&path);
                        self.actions.push(Action::Send(id, format!("{q} ").into_bytes()));
                    }
                }
                _ => self.actions.push(Action::Copy(path)),
            }
            let _ = dir;
        }
    }

    /// Clear the filter and show `path` in the tree: its folders expanded, it selected.
    fn reveal(&mut self, host: &str, root: &str, path: &str) {
        let rel = path.strip_prefix(root).unwrap_or(path).trim_start_matches('/');
        let mut acc = root.to_string();
        let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
        let is_dir = self.find_index.get(&(host.to_string(), root.to_string())).and_then(|i| i.entries.iter().find(|e| e.0 == rel)).map(|e| e.1).unwrap_or(false);
        let n = if is_dir { parts.len() } else { parts.len().saturating_sub(1) };
        for part in &parts[..n] {
            acc = join(&acc, part);
            self.side.expanded.insert((host.to_string(), acc.clone()));
            self.actions.push(Action::ListDir(host.to_string(), acc.clone()));
        }
        self.side.selected = Some(path.to_string());
        self.side.filter.clear();
    }

    // ---- editor -----------------------------------------------------------------------

    /// Drag handle that moves the panel being drawn to another window edge
    /// or beside another panel (drop zones are drawn by `ui_dock_drop`);
    /// right-click for the same choices, and to split or close the panel.
    fn dock_grip(&mut self, ui: &mut Ui) {
        let panel = self.drawing;
        let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::click_and_drag());
        let active = resp.hovered() || resp.dragged() || self.dock_drag == Some(panel);
        let c = if active { theme::text() } else { theme::text_faint() };
        for row in 0..3 {
            for col in 0..2 {
                let p = pos2(rect.center().x - 2.5 + col as f32 * 5.0, rect.center().y - 5.0 + row as f32 * 5.0);
                ui.painter().circle_filled(p, 1.3, c);
            }
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text("drag onto another panel or a window edge to move this panel (right-click: menu)");
        if resp.drag_started() {
            self.dock_drag = Some(panel);
        }
        resp.context_menu(|ui| {
            for d in Dock::ALL {
                if ui.button(format!("Move to the {} edge", d.label())).clicked() {
                    self.move_panel(panel, None, d);
                    ui.close_menu();
                }
            }
            if panel.region != Region::Terms {
                ui.separator();
                if ui.button("Split right").clicked() {
                    self.split_panel(panel, Dock::Right);
                    ui.close_menu();
                }
                if ui.button("Split down").clicked() {
                    self.split_panel(panel, Dock::Bottom);
                    ui.close_menu();
                }
                if self.panels_of(panel.region).len() > 1 && ui.button("Close this panel").clicked() {
                    self.close_panel(panel);
                    ui.close_menu();
                }
            }
        });
    }

    /// Split and (when there are several) close buttons for the side panel
    /// or editor being drawn, after its grip.
    fn panel_buttons(&mut self, ui: &mut Ui) {
        let panel = self.drawing;
        let what = panel.region.name().to_lowercase();
        let split = icons::button(ui, Icon::SplitDown, &format!("another {what} below (right-click: to the right)"));
        if split.clicked() {
            self.split_panel(panel, Dock::Bottom);
        } else if split.secondary_clicked() {
            self.split_panel(panel, Dock::Right);
        }
        if self.panels_of(panel.region).len() > 1 && icons::button(ui, Icon::Cross, &format!("close this {what} (the others stay)")).clicked() {
            self.close_panel(panel);
        }
    }

    /// The side panels or editors in the layout.
    pub fn panels_of(&self, region: Region) -> Vec<PanelId> {
        let mut all = Vec::new();
        self.settings.layout.panels(&mut all);
        all.retain(|p| p.region == region);
        all
    }

    /// Another side panel or editor beside `panel`, half its space: a side
    /// panel shows another tab, an editor the same file.
    pub fn split_panel(&mut self, panel: PanelId, side: Dock) {
        let new = PanelId { region: panel.region, n: self.settings.layout.free_n(panel.region) };
        let layout = std::mem::take(&mut self.settings.layout);
        self.settings.layout = layout.split(panel, new, side);
        match panel.region {
            Region::Side => {
                let tab = self.settings.side_tabs.get(&panel.n).copied().unwrap_or(self.side.tab);
                let other = if tab == SideTab::Files { SideTab::Search } else { SideTab::Files };
                self.settings.side_tabs.insert(new.n, other);
                // The new one starts where this one is, but stays put when the
                // terminal moves (pick another host in its header: two hosts
                // side by side, drag files across to copy them).
                let mut view = self.side_views.get(&panel.n).cloned().unwrap_or_else(|| self.side_view());
                view.follow = false;
                self.side_views.insert(new.n, view);
                self.settings.sidebar = true;
            }
            Region::Editor => {
                if let Some(shown) = self.editor_shows.get(&panel.n).cloned() {
                    self.editor_shows.insert(new.n, shown);
                }
                self.editor.visible = true;
            }
            Region::Terms => {}
        }
        self.save_settings();
    }

    /// Close one of several side panels or editors (the last one is hidden
    /// with its toggle instead, so it comes back where it was).
    pub fn close_panel(&mut self, panel: PanelId) {
        if self.panels_of(panel.region).len() < 2 {
            return;
        }
        if let Some(rest) = std::mem::take(&mut self.settings.layout).remove(panel) {
            self.settings.layout = rest;
        }
        match panel.region {
            Region::Side => {
                self.settings.side_tabs.remove(&panel.n);
            }
            Region::Editor => {
                self.editor_shows.remove(&panel.n);
            }
            Region::Terms => {}
        }
        let first = self.panels_of(panel.region).first().map(|p| p.n).unwrap_or(0);
        match panel.region {
            Region::Side if self.side_focus == panel.n => self.side_focus = first,
            Region::Editor if self.editor_focus == panel.n => self.editor_focus = first,
            _ => {}
        }
        self.save_settings();
    }

    /// Move a panel next to another one (or to a window edge) and remember it.
    pub fn move_panel(&mut self, panel: PanelId, target: Option<PanelId>, side: Dock) {
        let layout = std::mem::take(&mut self.settings.layout);
        self.settings.layout = layout.moved(panel, target, side);
        self.save_settings();
    }

    /// What changed since the last frame for "the" side panel or editor (a
    /// command picked a tab, a file was opened) belongs to the focused one;
    /// a focus on a panel that is gone moves to the first.
    fn panels_follow_focus(&mut self) {
        let sides = self.panels_of(Region::Side);
        if !sides.iter().any(|p| p.n == self.side_focus) {
            self.side_focus = sides.first().map_or(0, |p| p.n);
        }
        let editors = self.panels_of(Region::Editor);
        if !editors.iter().any(|p| p.n == self.editor_focus) {
            self.editor_focus = editors.first().map_or(0, |p| p.n);
        }
        self.settings.side_tabs.insert(self.side_focus, self.side.tab);
        let view = self.side_view();
        self.side_views.insert(self.side_focus, view);
        if let Some(f) = self.editor.files.get(self.editor.active) {
            self.editor_shows.insert(self.editor_focus, (f.host.clone(), f.path.clone()));
        }
    }

    /// Side panel `n`: the shared side state, showing its own tab.
    fn side_panel(&mut self, ui: &mut Ui, n: u32) {
        self.side.tab = self.settings.side_tabs.get(&n).copied().unwrap_or(self.side.tab);
        if let Some(v) = self.side_views.get(&n).cloned() {
            self.set_side_view(v);
        }
        self.ui_sidebar(ui);
        self.settings.side_tabs.insert(n, self.side.tab);
        let view = self.side_view();
        self.side_views.insert(n, view);
    }

    fn side_view(&self) -> SideView {
        SideView { root: self.side.root.clone(), follow: self.side.follow, selected: self.side.selected.clone() }
    }

    fn set_side_view(&mut self, v: SideView) {
        self.side.root = v.root;
        self.side.follow = v.follow;
        self.side.selected = v.selected;
    }

    /// Editor `n`: the open files (shared), showing its own one. The find
    /// bar and a selection waiting to be made belong to the focused editor.
    fn editor_panel(&mut self, ui: &mut Ui, n: u32) {
        let focused = n == self.editor_focus;
        let shows = self.editor_shows.get(&n).and_then(|(h, p)| self.editor.files.iter().position(|f| &f.host == h && &f.path == p));
        if let Some(i) = shows {
            self.editor.active = i;
        }
        let held = (!focused).then(|| (self.editor.find.take(), self.editor.select.take()));
        self.ui_editor(ui);
        if let Some((find, select)) = held {
            self.editor.find = find;
            self.editor.select = select;
        }
        if let Some(f) = self.editor.files.get(self.editor.active) {
            self.editor_shows.insert(n, (f.host.clone(), f.path.clone()));
        }
    }

    /// The panels where the layout tree puts them, with draggable dividers.
    /// Hidden panels (side panel, editor) keep their place in the tree.
    pub fn ui_layout(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        self.layout_rect = rect;
        let (side, editor) = (self.settings.sidebar, self.editor.visible);
        let shown = |p: PanelId| match p.region {
            Region::Side => side,
            Region::Editor => editor,
            Region::Terms => true,
        };
        if !self.settings.layout.contains(PanelId::TERMS) {
            self.settings.layout = crate::app::Layout::default();
        }
        self.panels_follow_focus();
        let Some(tree) = self.settings.layout.pruned(&shown) else { return };
        let mut panels = Vec::new();
        let mut dividers = Vec::new();
        tree.rects(rect, &mut panels, &mut dividers);
        self.panel_rects = panels.iter().copied().collect();
        let pressed = ui.input(|i| (i.pointer.primary_pressed() || i.pointer.secondary_pressed()).then(|| i.pointer.interact_pos()).flatten());
        for (panel, r) in panels {
            self.drawing = panel;
            // A click in a side panel or editor makes it the one commands go to.
            if pressed.is_some_and(|p| r.contains(p)) {
                match panel.region {
                    Region::Side => self.side_focus = panel.n,
                    Region::Editor => self.editor_focus = panel.n,
                    Region::Terms => {}
                }
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(r).id_salt(("panel", panel.region, panel.n)), |ui| {
                ui.set_clip_rect(r.intersect(ui.clip_rect()));
                match panel.region {
                    Region::Side | Region::Editor => {
                        let margin = if panel.region == Region::Side { egui::Margin::symmetric(8, 0) } else { egui::Margin::symmetric(8, 6) };
                        egui::Frame::new().fill(theme::bg_base()).inner_margin(margin).show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            if panel.region == Region::Side {
                                self.side_panel(ui, panel.n);
                            } else {
                                self.editor_panel(ui, panel.n);
                            }
                        });
                    }
                    Region::Terms => {
                        // Terminal panel: its tab bar, then the active tab's panes.
                        // Focus mode: the panes only, and a way out.
                        if self.focus_mode.is_some() {
                            // The label has a strip of its own above the
                            // panes: drawn over them it covered a split
                            // pane's zoom and close buttons.
                            let strip = egui::Rect::from_min_size(r.min, vec2(r.width(), 24.0));
                            let body = egui::Rect::from_min_max(pos2(r.left(), strip.bottom()), r.max);
                            ui.painter().rect_filled(strip, CornerRadius::ZERO, theme::bg_base());
                            ui.painter().rect_filled(body, CornerRadius::ZERO, theme::bg_term());
                            ui.scope_builder(egui::UiBuilder::new().max_rect(body).id_salt("panes"), |ui| self.ui_panes(ui));
                            let keys = self.keys_label(Cmd::FocusMode);
                            let label = if keys.is_empty() { "FOCUS · click to leave".to_string() } else { format!("FOCUS · {keys} or click to leave") };
                            let g = ui.painter().layout_no_wrap(label, FontId::proportional(11.0), theme::accent());
                            let pill = egui::Rect::from_min_size(pos2(r.right() - g.size().x - 28.0, strip.center().y - 9.0), vec2(g.size().x + 16.0, 18.0));
                            let resp = ui.interact(pill, Id::new("focus-pill"), Sense::click());
                            let alpha = if resp.hovered() { 90 } else { 28 };
                            ui.painter().rect_filled(pill, CornerRadius::same(9), theme::tint(theme::accent(), alpha));
                            let text_alpha = if resp.hovered() { 255 } else { 120 };
                            ui.painter().galley(pos2(pill.left() + 8.0, pill.center().y - g.size().y / 2.0), g, theme::tint(theme::accent(), text_alpha));
                            if resp.clicked() {
                                self.run_cmd(Cmd::FocusMode);
                            }
                            return;
                        }
                        let bar = egui::Rect::from_min_size(r.min, vec2(r.width(), 34.0));
                        let body = egui::Rect::from_min_max(pos2(r.left(), bar.bottom()), r.max);
                        ui.painter().rect_filled(bar, CornerRadius::ZERO, theme::bg_base());
                        ui.painter().rect_filled(body, CornerRadius::ZERO, theme::bg_term());
                        let inner = egui::Rect::from_min_max(pos2(bar.left() + 6.0, bar.top() + 4.0), pos2(bar.right() - 6.0, bar.bottom()));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).id_salt("tabbar"), |ui| self.ui_tabbar(ui));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(body).id_salt("panes"), |ui| self.ui_panes(ui));
                    }
                }
            });
        }
        // Back to the focused side panel's tab and editor's file, for commands.
        if let Some(&tab) = self.settings.side_tabs.get(&self.side_focus) {
            self.side.tab = tab;
        }
        if let Some(v) = self.side_views.get(&self.side_focus).cloned() {
            self.set_side_view(v);
        }
        let focused_file = self.editor_shows.get(&self.editor_focus).and_then(|(h, p)| self.editor.files.iter().position(|f| &f.host == h && &f.path == p));
        if let Some(i) = focused_file {
            self.editor.active = i;
        }
        self.drawing = PanelId::TERMS;
        // Where a vertical border meets a horizontal one (three panels
        // around a point): one grip moves both, laid over the borders.
        let junctions = crate::app::panel_junctions(&dividers);
        // Over a junction the two borders under it stand aside (unless one
        // is already being dragged): right on the point, the thin border
        // used to win and the junction only caught the pointer a little off it.
        const JUNCTION: f32 = 20.0;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let near = |i: usize| {
            pointer.is_some_and(|p| junctions.iter().any(|j| (j.0.contains(&i) || j.1.contains(&i)) && egui::Rect::from_center_size(j.2, vec2(JUNCTION, JUNCTION)).contains(p)))
        };
        for (i, &(id, handle, vertical, parent)) in dividers.iter().enumerate() {
            let did = Id::new(("panel-divider", id));
            if near(i) && !ui.ctx().is_being_dragged(did) {
                ui.painter().rect_filled(handle, CornerRadius::ZERO, theme::border());
                continue;
            }
            let resp = ui.interact(handle.expand(2.0), did, Sense::drag());
            let hot = resp.hovered() || resp.dragged();
            if hot {
                ui.ctx().set_cursor_icon(if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
            }
            ui.painter().rect_filled(handle, CornerRadius::ZERO, if hot { theme::tint(theme::accent(), 140) } else { theme::border() });
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let r = if vertical { (p.x - parent.left()) / parent.width() } else { (p.y - parent.top()) / parent.height() };
                    if let Some(ratio) = self.settings.layout.ratio_mut(id) {
                        *ratio = r.clamp(0.08, 0.92);
                    }
                }
            }
            if resp.drag_stopped() {
                self.save_settings();
            }
        }
        for (vs, hs, at) in junctions {
            let ids: Vec<u32> = vs.iter().chain(&hs).map(|&i| dividers[i].0).collect();
            let zone = egui::Rect::from_center_size(at, vec2(JUNCTION, JUNCTION));
            let resp = ui.interact(zone, Id::new(("panel-junction", ids)), Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                ui.painter().circle_filled(at, 5.0, theme::accent());
            }
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    // Every border there: verticals follow x, horizontals y.
                    for &i in &vs {
                        let (id, _, _, parent) = dividers[i];
                        if let Some(r) = self.settings.layout.ratio_mut(id) {
                            *r = ((p.x - parent.left()) / parent.width()).clamp(0.08, 0.92);
                        }
                    }
                    for &i in &hs {
                        let (id, _, _, parent) = dividers[i];
                        if let Some(r) = self.settings.layout.ratio_mut(id) {
                            *r = ((p.y - parent.top()) / parent.height()).clamp(0.08, 0.92);
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                self.save_settings();
            }
        }
    }

    /// While a panel is dragged by its grip: highlight where it would go —
    /// a half of the panel under the pointer, or a window edge near the
    /// border — and move it there on release.
    pub fn ui_dock_drop(&mut self, ctx: &egui::Context) {
        let Some(panel) = self.dock_drag else { return };
        let area = self.layout_rect;
        let (pos, released) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.any_released() || !i.pointer.primary_down()));
        let Some(p) = pos else { return };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("dock-drop")));
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let nearest = |r: egui::Rect| {
            let fx = (p.x - r.left()) / r.width();
            let fy = (p.y - r.top()) / r.height();
            [(Dock::Left, fx), (Dock::Right, 1.0 - fx), (Dock::Top, fy), (Dock::Bottom, 1.0 - fy)].into_iter().min_by(|a, b| a.1.total_cmp(&b.1)).map(|t| t.0).unwrap_or(Dock::Left)
        };
        let half_of = |r: egui::Rect, side: Dock, frac: f32| match side {
            Dock::Left => egui::Rect::from_min_max(r.min, pos2(r.left() + r.width() * frac, r.bottom())),
            Dock::Right => egui::Rect::from_min_max(pos2(r.right() - r.width() * frac, r.top()), r.max),
            Dock::Top => egui::Rect::from_min_max(r.min, pos2(r.right(), r.top() + r.height() * frac)),
            Dock::Bottom => egui::Rect::from_min_max(pos2(r.left(), r.bottom() - r.height() * frac), r.max),
        };
        // Within this distance of the window border: the window edge.
        let border = [p.x - area.left(), area.right() - p.x, p.y - area.top(), area.bottom() - p.y].into_iter().fold(f32::MAX, f32::min);
        let over = self.panel_rects.iter().find(|(r, rect)| **r != panel && rect.contains(p)).map(|(r, rect)| (*r, *rect));
        let (target, side, zone, text) = if border < 36.0 || !area.contains(p) {
            let side = nearest(area);
            (None, side, half_of(area, side, 0.3), format!("{} \u{2192} {} edge of the window", panel.name(), side.label()))
        } else if let Some((t, rect)) = over {
            let side = nearest(rect);
            (Some(t), side, half_of(rect, side, 0.5), format!("{} \u{2192} {} of {}", panel.name(), side.label(), t.name().to_lowercase()))
        } else {
            // Over its own panel: nothing to do.
            painter.text(p + vec2(16.0, 16.0), Align2::LEFT_TOP, "drop on another panel or near a window edge", FontId::proportional(12.0), theme::text_dim());
            if released {
                self.dock_drag = None;
            }
            return;
        };
        painter.rect(zone.shrink(4.0), CornerRadius::same(8), theme::tint(theme::accent(), 40), Stroke::new(2.0_f32, theme::accent()), egui::StrokeKind::Inside);
        painter.text(zone.center(), Align2::CENTER_CENTER, text, FontId::proportional(15.0), theme::text());
        if released {
            self.dock_drag = None;
            self.move_panel(panel, target, side);
        }
    }

    pub fn ui_editor(&mut self, ui: &mut Ui) {
        if self.editor.files.is_empty() {
            ui.horizontal(|ui| {
                self.dock_grip(ui);
                self.panel_buttons(ui);
            });
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No files open").color(theme::text_dim()));
                ui.label(RichText::new("Click a file in the side panel, Ctrl+click a path in the terminal,\nor run `wopen FILE` on a remote shell.").size(12.0).color(theme::text_faint()));
                ui.add_space(10.0);
                if ui.button(RichText::new("New memo").size(12.5)).on_hover_text("a markdown note, saved automatically (Ctrl+Shift+M)").clicked() {
                    self.new_memo();
                }
            });
            if !self.memos().is_empty() {
                ui.add_space(14.0);
                ui.label(RichText::new("Recent memos").size(12.0).color(theme::text_dim()));
                self.memo_query.clear();
                if let Some(p) = self.memo_rows(ui, 6, false) {
                    self.apply_memo_pick(p);
                }
            }
            return;
        }
        self.editor.active = self.editor.active.min(self.editor.files.len() - 1);
        // Tabs.
        let mut close = None;
        ui.horizontal(|ui| {
        self.dock_grip(ui);
        self.panel_buttons(ui);
        if icons::button(ui, Icon::Plus, "new memo — a markdown note, saved automatically (Ctrl+Shift+M)").clicked() {
            self.actions.push(Action::NewMemo);
        }
        let mut pick = None;
        ui.menu_button(RichText::new("Memos").size(12.0).color(theme::text_dim()), |ui| {
            ui.set_width(340.0);
            ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                pick = self.memo_rows(ui, usize::MAX, true);
            });
            if pick.is_some() {
                ui.close_menu();
            }
        })
        .response
        .on_hover_text("open a memo (Open memo… in the palette)");
        if let Some(p) = pick {
            self.apply_memo_pick(p);
        }
        ui.add_space(6.0);
        let memo_dir = format!("{}/", self.memo_dir());
        // A thin scrollbar of its own below the tabs: egui's default floats
        // over the content and hid the tab names. The plain (vertical) wheel
        // scrolls the strip sideways; egui only does that when asked to.
        ui.style_mut().always_scroll_the_only_direction = true;
        let scroll = &mut ui.style_mut().spacing.scroll;
        scroll.floating = false;
        scroll.bar_width = 4.0;
        scroll.bar_inner_margin = 2.0;
        scroll.bar_outer_margin = 0.0;
        ScrollArea::horizontal().id_salt("editor-tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let n = self.editor.files.len();
                for (i, f) in self.editor.files.iter().enumerate() {
                    let memo = f.host == "local" && f.path.starts_with(&memo_dir);
                    let name = if memo { memo_title(&f.text) } else { f.path.rsplit('/').next().unwrap_or(&f.path).to_string() };
                    let sel = i == self.editor.active;
                    // Memos save themselves, so no unsaved dot for them.
                    let name = weft_core::shown(&name).into_owned();
                    let label = RichText::new(if f.dirty && !memo { format!("● {name}") } else { name }).size(12.5).color(if sel { theme::text() } else { theme::text_dim() });
                    let label = if memo { label.italics() } else { label };
                    let r = ui.selectable_label(sel, label).on_hover_text(format!("{}:{}", f.host, f.path));
                    if r.clicked() {
                        self.editor.active = i;
                    }
                    if r.middle_clicked() {
                        close = Some((i, CloseWhich::This));
                    }
                    r.context_menu(|ui| {
                        if let Some(w) = close_menu(ui, i, n) {
                            close = Some((i, w));
                        }
                    });
                    if icons::button(ui, Icon::Cross, "close (middle-click also works; right-click the tab for more)").clicked() {
                        close = Some((i, CloseWhich::This));
                    }
                }
            });
        });
        });
        if let Some((i, which)) = close {
            self.ask_close_files(i, which);
            if self.editor.files.is_empty() {
                return;
            }
        }
        let idx = self.editor.active;
        // Toolbar.
        let mut save = false;
        let mut reload = false;
        let mut force = false;
        let mut settings_changed = false;
        let mut delete = false;
        let mut open_find = false;
        let mut copy = false;
        let memo = self.is_memo(&self.editor.files[idx]);
        {
            let f = &mut self.editor.files[idx];
            ui.horizontal(|ui| {
                ui.painter().circle_filled(ui.cursor().min + vec2(5.0, 11.0), 3.5, host_color(&f.host));
                ui.add_space(12.0);
                let path = format!("{}  {}", f.host, weft_core::shown(&f.path));
                ui.add(egui::Label::new(RichText::new(&path).size(11.5).color(theme::text_faint())).truncate()).on_hover_text(path);
            });
            // Wraps onto a second line in a narrow editor instead of losing its end.
            ui.horizontal_wrapped(|ui| {
                let can_save = f.dirty && !f.scratch && !f.binary && !f.loading;
                // A just-saved file says so for a moment, quietly (no toast).
                let just_saved = f.saved_at.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(1600));
                if just_saved {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                }
                if memo {
                    // Saved by itself; the button saves right now, for peace of mind.
                    if ui.button(RichText::new("Save").size(12.5)).on_hover_text("memos save themselves within a second; this saves now (Ctrl+S)").clicked() {
                        save = true;
                    }
                    let (state, color) = if f.dirty {
                        ("memo · editing…", theme::text_faint())
                    } else if just_saved {
                        ("memo · \u{2713} saved", theme::success())
                    } else {
                        ("memo · saved automatically", theme::text_faint())
                    };
                    ui.label(RichText::new(state).size(11.5).color(color));
                    ui.menu_button(RichText::new("Delete…").size(12.5), |ui| {
                        if ui.button(RichText::new("Delete this memo").color(theme::error())).clicked() {
                            delete = true;
                            ui.close_menu();
                        }
                    });
                } else {
                    // Unsaved changes: Save stands out, with a word next to it.
                    let save_btn = if can_save {
                        egui::Button::new(RichText::new("Save").size(12.5).color(theme::bg_base())).fill(theme::accent())
                    } else {
                        egui::Button::new(RichText::new("Save").size(12.5))
                    };
                    if ui.add_enabled(can_save, save_btn).on_hover_text("Ctrl+S — writes through the agent").clicked() {
                        save = true;
                    }
                    if f.saving {
                        ui.label(RichText::new("saving…").size(11.5).color(theme::text_faint()));
                    } else if f.dirty && !f.scratch {
                        ui.label(RichText::new("\u{25cf} unsaved").size(11.5).color(theme::warn())).on_hover_text("changed since it was opened or saved");
                    } else if just_saved {
                        ui.label(RichText::new("\u{2713} saved").size(11.5).color(theme::success()));
                    }
                }
                // Copy: the selection, or the whole file; says so for a moment.
                let just_copied = f.copied_at.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(1200));
                if just_copied {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                }
                let copy_label = if just_copied { RichText::new("\u{2713} copied").size(12.5).color(theme::success()) } else { RichText::new("Copy").size(12.5) };
                if !f.binary && ui.add(egui::Button::new(copy_label)).on_hover_text("copy the selection, or the whole file when nothing is selected").clicked() {
                    copy = true;
                }
                if !f.scratch && !memo && ui.add_enabled(!f.loading, egui::Button::new(RichText::new("Reload").size(12.5))).clicked() {
                    reload = true;
                }
                if !f.binary && ui.selectable_label(f.read, RichText::new("Read").size(12.5)).on_hover_text("read mode: select and copy, no editing (click again to edit)").clicked() {
                    f.read = !f.read;
                }
                if md::is_markdown(&f.path) {
                    // One button: the view shown, a click for the other.
                    let (label, tip) = if f.rich { ("Rich \u{21c4}", "rendered document · click: plain markdown source") } else { ("Source \u{21c4}", "plain markdown source · click: rendered document") };
                    if ui.button(RichText::new(label).size(12.5)).on_hover_text(tip).clicked() {
                        f.rich = !f.rich;
                    }
                    // Who draws ```mermaid blocks. The menu stays open while choosing.
                    let m = &mut self.settings.mermaid;
                    ui.menu_button(RichText::new(format!("Diagrams: {}", m.label())).size(12.5), |ui| {
                        use crate::mermaid::Renderer;
                        let before = m.clone();
                        if ui.radio(*m == Renderer::Builtin, "built-in (nothing to install)").clicked() {
                            *m = Renderer::Builtin;
                        }
                        if ui.radio(*m == Renderer::System, "mermaid-cli from PATH").clicked() {
                            *m = Renderer::System;
                        }
                        let is_path = matches!(m, Renderer::Path(_));
                        if ui.radio(is_path, "mermaid-cli at a path").clicked() && !is_path {
                            *m = Renderer::Path(String::new());
                        }
                        if let Renderer::Path(p) = m {
                            ui.add(TextEdit::singleline(p).hint_text("C:\\…\\npm\\mmdc.cmd").desired_width(280.0));
                            let ok = std::path::Path::new(p.trim()).is_file();
                            let (t, c) = if p.trim().is_empty() { ("", theme::text_faint()) } else if ok { ("found", theme::success()) } else { ("not found", theme::error()) };
                            if !t.is_empty() {
                                ui.label(RichText::new(t).size(11.5).color(c));
                            }
                        }
                        if *m != before {
                            settings_changed = true;
                        }
                    });
                }
                if !f.binary && !crate::media::is_image(&f.path) && !crate::media::is_pdf(&f.path)
                    && ui.button(RichText::new("Find").size(12.5)).on_hover_text("find (Ctrl+F) and replace (Ctrl+H)").clicked() {
                        open_find = true;
                    }
                if f.loading {
                    ui.spinner();
                    ui.label(RichText::new("reading…").size(11.5).color(theme::text_faint()));
                } else if f.saving {
                    ui.spinner();
                } else if f.scratch {
                    ui.label(RichText::new("scratch (local only)").size(11.5).color(theme::text_faint()));
                }
            });
            if f.conflict {
                egui::Frame::new().fill(theme::tint(theme::warn(), 28)).corner_radius(CornerRadius::same(5)).inner_margin(6.0).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("The file changed on the host since it was opened.").color(theme::warn()));
                        if ui.button("Overwrite").clicked() {
                            force = true;
                        }
                        if ui.button("Reload").clicked() {
                            reload = true;
                        }
                    });
                });
            }
            if let Some(e) = &f.error {
                ui.colored_label(theme::error(), e);
            }
        }
        if delete {
            self.delete_memo(idx);
            return;
        }
        ui.separator();
        let edit_id = Id::new(("editor", idx, self.editor.files[idx].path.clone()));
        if copy {
            let f = &mut self.editor.files[idx];
            let range = TextEdit::load_state(ui.ctx(), edit_id).and_then(|s| s.cursor.char_range()).map(|r| (r.primary.index.min(r.secondary.index), r.primary.index.max(r.secondary.index)));
            let text = match range.filter(|(a, b)| a != b) {
                Some((a, b)) => f.text.chars().skip(a).take(b - a).collect(),
                None => f.text.clone(),
            };
            f.copied_at = Some(std::time::Instant::now());
            self.actions.push(Action::Copy(text));
        }
        if ui.memory(|m| m.has_focus(edit_id)) && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S)) {
            save = true;
        }
        // Find / replace (Ctrl+F / Ctrl+H while the editor has the keyboard).
        let text_file = {
            let f = &self.editor.files[idx];
            !f.binary && !crate::media::is_image(&f.path) && !crate::media::is_pdf(&f.path)
        };
        let editing = ui.memory(|m| m.has_focus(edit_id)) || ui.memory(|m| m.has_focus(Id::new("editor-find-q")) || m.has_focus(Id::new("editor-find-r")));
        if text_file && editing {
            let (ctrl_f, ctrl_h) = ui.input_mut(|i| (i.consume_key(Modifiers::CTRL, Key::F), i.consume_key(Modifiers::CTRL, Key::H)));
            if ctrl_f || ctrl_h || open_find {
                let f = &self.editor.files[idx];
                // Start from the selected text when it is one line.
                let picked = TextEdit::load_state(ui.ctx(), edit_id).and_then(|s| s.cursor.char_range()).and_then(|r| {
                    let (a, b) = (r.primary.index.min(r.secondary.index), r.primary.index.max(r.secondary.index));
                    let s: String = f.text.chars().skip(a).take(b - a).collect();
                    (!s.is_empty() && !s.contains('\n')).then_some(s)
                });
                let last = self.settings.search;
                let bar = self.editor.find.get_or_insert_with(|| crate::editfind::FindBar { mode: last.editor_mode, case: last.editor_case, ..Default::default() });
                if let Some(s) = picked {
                    bar.query = s;
                    bar.cur = 0;
                }
                bar.replacing |= ctrl_h;
                bar.focus = true;
            }
        } else if open_find && text_file {
            let last = self.settings.search;
            let bar = self.editor.find.get_or_insert_with(|| crate::editfind::FindBar { mode: last.editor_mode, case: last.editor_case, ..Default::default() });
            bar.focus = true;
        }
        if !text_file {
            self.editor.find = None;
        }
        let mut replaced: Option<usize> = None;
        let mut find_error: Option<String> = None;
        if let Some(mut bar) = self.editor.find.take() {
            let f = &mut self.editor.files[idx];
            let ms = bar.matches(&f.text);
            let count = ms.as_ref().map(|m| m.len()).unwrap_or(0);
            bar.cur = if count == 0 { 0 } else { bar.cur.min(count - 1) };
            let mut nav: i32 = 0;
            let mut close = false;
            let mut rep_one = false;
            let mut rep_all = false;
            let shift = ui.input(|i| i.modifiers.shift);
            // Shift+F3 first: a key with Shift held also counts as the key
            // alone for egui, and F3 took it (Shift+F3 went forward).
            if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3)) {
                nav = -1;
            } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3)) {
                nav = 1;
            }
            // In a narrow editor the bar ran on past the edge, and its ×
            // with it: the field gives way first (the rest needs about 300),
            // and what still has no room goes on the next line.
            ui.horizontal_wrapped(|ui| {
                let q = ui.add(TextEdit::singleline(&mut bar.query).id(Id::new("editor-find-q")).hint_text("find…").desired_width((ui.available_width() - 300.0).clamp(100.0, 220.0)));
                if bar.focus {
                    q.request_focus();
                    bar.focus = false;
                }
                if q.changed() {
                    bar.cur = 0;
                    nav = 2;
                }
                if q.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    nav = if shift { -1 } else { 1 };
                    q.request_focus();
                }
                // Esc takes the focus first (egui, before the frame).
                if (q.has_focus() || q.lost_focus()) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                    close = true;
                }
                if ui.selectable_label(bar.case, RichText::new("Aa").size(12.0)).on_hover_text("match case").clicked() {
                    bar.case = !bar.case;
                    nav = 2;
                }
                let before = bar.mode;
                egui::ComboBox::from_id_salt("editor-find-mode").width(62.0).selected_text(RichText::new(bar.mode.label()).size(12.0)).show_ui(ui, |ui| {
                    for m in editfind::MatchMode::ALL {
                        ui.selectable_value(&mut bar.mode, m, m.label()).on_hover_text(m.hint());
                    }
                });
                if bar.mode != before {
                    nav = 2;
                }
                match &ms {
                    Ok(_) if count > 0 => ui.label(RichText::new(format!("{}/{count}", bar.cur + 1)).size(11.5).color(theme::text_dim())),
                    Ok(_) if !bar.query.is_empty() => ui.label(RichText::new("no match").size(11.5).color(theme::warn())),
                    Ok(_) => ui.label(""),
                    Err(e) => ui.label(RichText::new(e).size(11.5).color(theme::error())),
                };
                if ui.add_enabled(count > 0, egui::Button::new("↑").small()).on_hover_text("previous (Shift+Enter, Shift+F3)").clicked() {
                    nav = -1;
                }
                if ui.add_enabled(count > 0, egui::Button::new("↓").small()).on_hover_text("next (Enter, F3)").clicked() {
                    nav = 1;
                }
                if ui.selectable_label(bar.replacing, RichText::new("replace").size(12.0)).on_hover_text("Ctrl+H").clicked() {
                    bar.replacing = !bar.replacing;
                }
                if icons::button(ui, Icon::Cross, "close (Esc)").clicked() {
                    close = true;
                }
            });
            if bar.replacing {
                ui.horizontal_wrapped(|ui| {
                    let hint = if bar.mode == editfind::MatchMode::Regex { "replace with… ($1, $2: groups)" } else { "replace with…" };
                    ui.add(TextEdit::singleline(&mut bar.replace).id(Id::new("editor-find-r")).hint_text(hint).desired_width((ui.available_width() - 300.0).clamp(100.0, 220.0)));
                    if ui.add_enabled(count > 0 && !f.loading, egui::Button::new("Replace")).on_hover_text("this match, then on to the next").clicked() {
                        rep_one = true;
                    }
                    if ui.add_enabled(count > 0 && !f.loading, egui::Button::new("All")).on_hover_text("every match").clicked() {
                        rep_all = true;
                    }
                });
            }
            let ms = ms.unwrap_or_default();
            if (rep_one || rep_all) && count > 0 {
                // Ctrl+Z in the file brings back the text from before the replace.
                let mut st = TextEdit::load_state(ui.ctx(), edit_id).unwrap_or_default();
                let mut undoer = st.undoer();
                let range = st.cursor.char_range().unwrap_or_else(|| egui::text::CCursorRange::one(egui::text::CCursor::new(0)));
                undoer.add_undo(&(range, f.text.clone()));
                st.set_undoer(undoer);
                st.store(ui.ctx(), edit_id);
            }
            if rep_one && count > 0 {
                match bar.replace_at(&mut f.text, ms[bar.cur]) {
                    Ok(_) => {
                        replaced = Some(1);
                        nav = 2;
                    }
                    Err(e) => find_error = Some(e),
                }
            } else if rep_all {
                match bar.replace_all(&mut f.text) {
                    Ok(n) => replaced = Some(n),
                    Err(e) => find_error = Some(e),
                }
            }
            // Go to a match: select it (shown once the editor has the keyboard) and scroll to it.
            let ms = if replaced.is_some() { bar.matches(&f.text).unwrap_or_default() } else { ms };
            if nav != 0 && !ms.is_empty() {
                let n = ms.len();
                bar.cur = match nav {
                    1 => (bar.cur + 1) % n,
                    -1 => (bar.cur + n - 1) % n,
                    _ => bar.cur.min(n - 1),
                };
                let (a, b) = ms[bar.cur];
                self.editor.select = Some((editfind::char_index(&f.text, a), editfind::char_index(&f.text, b)));
                f.goto = Some(editfind::line_of(&f.text, a));
            }
            // Every match gets a background, the current one a stronger one.
            f.mark_cur = ms.get(bar.cur).copied();
            f.marks = Arc::new(ms);
            if replaced.is_some() && !f.scratch {
                f.dirty = true;
            }
            if close {
                f.marks = Default::default();
                f.mark_cur = None;
                ui.memory_mut(|m| m.request_focus(edit_id));
            } else {
                self.settings.search.editor_mode = bar.mode;
                self.settings.search.editor_case = bar.case;
                self.editor.find = Some(bar);
            }
        }
        if let Some(n) = replaced.filter(|n| *n != 1) {
            self.toast(format!("replaced {n}"), ToastKind::Info);
        }
        if let Some(e) = find_error {
            self.toast(format!("replace: {e}"), ToastKind::Error);
        }
        let font = FontId::monospace(self.settings.font_size - 1.0);
        let row_h = ui.fonts(|f| f.row_height(&font));
        if settings_changed {
            self.save_settings();
        }
        let f = &mut self.editor.files[idx];
        if crate::media::is_image(&f.path) || crate::media::is_pdf(&f.path) {
            if let Some(bytes) = f.bytes.clone() {
                if crate::media::is_pdf(&f.path) {
                    crate::media::pdf_view(ui, &mut f.pdf, &f.path, &bytes);
                } else {
                    crate::media::image_view(ui, &mut f.image, &f.path, &bytes);
                }
                if reload {
                    self.reload_file(idx);
                }
                return;
            }
        }
        if f.rich && md::is_markdown(&f.path) && !f.binary {
            let readonly = f.loading;
            let goto = f.goto.take();
            let size = self.settings.font_size + 0.5;
            let cache_dir = self.data.path("cache/mermaid");
            let breaks = self.settings.markdown_breaks;
            let renderer = self.settings.mermaid.clone();
            let changed = markdown_body(ui, f, edit_id, size, breaks, &renderer, readonly, goto, &cache_dir);
            if changed && !f.scratch {
                f.dirty = true;
            }
            // Typing again: the "✓ saved" / "✓ copied" of before no longer holds.
            if changed {
                f.saved_at = None;
                f.copied_at = None;
            }
        // A match picked in the find bar: select it.
        if let Some((a, b)) = self.editor.select.take() {
            let mut st = TextEdit::load_state(ui.ctx(), edit_id).unwrap_or_default();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b))));
            st.store(ui.ctx(), edit_id);
        }
            if save {
                self.actions.push(Action::Save(idx));
            }
            if force {
                self.save_file(idx, true);
            }
            if reload {
                self.reload_file(idx);
            }
            return;
        }
        let ext = highlight::ext_of(&f.path);
        let lines = f.text.matches('\n').count() + 1;
        let goto = f.goto.take();
        let mut sa = ScrollArea::both().id_salt(("editor-scroll", idx)).auto_shrink([false, false]);
        if let Some(l) = goto {
            sa = sa.vertical_scroll_offset(((l.saturating_sub(4)) as f32 * row_h).max(0.0));
        }
        let (marks, mark_cur) = (f.marks.clone(), f.mark_cur);
        let find = marks_key(&marks, mark_cur);
        let readonly = f.binary || f.loading;
        let mut changed = false;
        sa.show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let nums: String = (1..=lines).map(|n| format!("{n:>4}\n")).collect();
                ui.add(egui::Label::new(RichText::new(nums).font(font.clone()).color(theme::text_faint())).selectable(false));
                let cache_id = edit_id.with("hl");
                let mut layouter = |ui: &Ui, text: &str, _wrap: f32| {
                    let key = hash_str(text, &format!("{ext}|{find}|{}|{}|{}|{}", font.size, theme::active(), highlight::ready(), highlight::warmed()));
                    let kept_id = cache_id.with("galley");
                    if let Some(galley) = KeptGalley::get(ui, kept_id, key) {
                        return galley;
                    }
                    let cached: HlCache = ui.ctx().memory_mut(|m| m.data.get_temp_mut_or_default::<HlCache>(cache_id).clone());
                    let job = match cached.job {
                        Some(j) if cached.key == key => j,
                        _ => {
                            let state = cached.state.clone();
                            let j = Arc::new(highlight::layout(text, &ext, font.clone(), &marks, mark_cur, &state, ui.ctx()));
                            ui.ctx().memory_mut(|m| m.data.insert_temp(cache_id, HlCache { key, job: Some(j.clone()), state }));
                            j
                        }
                    };
                    KeptGalley::lay_out(ui, kept_id, key, (*job).clone())
                };
                let before = crate::widgets::cursor_before(ui, edit_id);
                // Read mode: the text as an immutable buffer, selectable and copyable.
                let mut shown: &str;
                let buf: &mut dyn egui::TextBuffer = if f.read {
                    shown = f.text.as_str();
                    &mut shown
                } else {
                    &mut f.text
                };
                let out = TextEdit::multiline(buf)
                    .id(edit_id)
                    .font(font.clone())
                    .code_editor()
                    .frame(false)
                    .interactive(!readonly)
                    .desired_width(f32::INFINITY)
                    .desired_rows(lines.max(30))
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui);
                crate::widgets::select_word_on_double_click(ui, &out, &f.text, before);
                let resp = out.response;
                if resp.changed() {
                    changed = true;
                }
                if let Some(l) = goto {
                    let idx_char: usize = f.text.split('\n').take(l.saturating_sub(1)).map(|s| s.chars().count() + 1).sum();
                    if let Some(mut st) = TextEdit::load_state(ui.ctx(), edit_id) {
                        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(idx_char))));
                        st.store(ui.ctx(), edit_id);
                    }
                    // Never steal keyboard focus: files are often opened from
                    // the terminal (wopen, Ctrl+click) while the user keeps typing.
                }
            });
        });
        if changed && !f.scratch {
            f.dirty = true;
        }
        // Typing again: the "✓ saved" / "✓ copied" of before no longer holds.
        if changed {
            f.saved_at = None;
            f.copied_at = None;
        }
        // A match picked in the find bar: select it.
        if let Some((a, b)) = self.editor.select.take() {
            let mut st = TextEdit::load_state(ui.ctx(), edit_id).unwrap_or_default();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b))));
            st.store(ui.ctx(), edit_id);
        }
        if save {
            self.actions.push(Action::Save(idx));
        }
        if force {
            self.save_file(idx, true);
        }
        if reload {
            self.reload_file(idx);
        }
    }

    // ---- overlays -----------------------------------------------------------------------

    pub fn ui_palette(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.palette.take() else { return };
        let theme_items: Vec<(Cmd, String, String)> =
            theme::themes().map(|(i, _, name, light)| (Cmd::Theme(i), format!("Theme: {name}"), if light { "light".into() } else { "dark".into() })).collect();
        let all: Vec<(Cmd, String, String)> = COMMANDS.iter().map(|(c, _, l, _)| (*c, l.to_string(), self.keys_label(*c))).chain(theme_items).collect();
        let labels: Vec<String> = all.iter().map(|i| i.1.clone()).collect();
        let items: Vec<(Cmd, String, String)> = palette_matches(self.settings.palette_mode, &st.query, &labels).into_iter().map(|i| all[i].clone()).collect();
        let mut mode = self.settings.palette_mode;
        let mut run = None;
        let mut keep = true;
        let area = egui::Area::new(Id::new("palette")).anchor(Align2::CENTER_TOP, vec2(0.0, 70.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(theme::bg_raised()).corner_radius(CornerRadius::same(8)).inner_margin(8.0).show(ui, |ui| {
                ui.set_width(460.0);
                let id = Id::new("palette-q");
                let mut moved = false;
                if take_key(ui, Key::ArrowDown, |_| true) {
                    st.sel = (st.sel + 1).min(items.len().saturating_sub(1));
                    moved = true;
                }
                if take_key(ui, Key::ArrowUp, |_| true) {
                    st.sel = st.sel.saturating_sub(1);
                    moved = true;
                }
                if take_key(ui, Key::PageDown, |_| true) {
                    st.sel = (st.sel + 10).min(items.len().saturating_sub(1));
                    moved = true;
                }
                if take_key(ui, Key::PageUp, |_| true) {
                    st.sel = st.sel.saturating_sub(10);
                    moved = true;
                }
                if take_key(ui, Key::Enter, |_| true) {
                    run = items.get(st.sel).map(|i| i.0);
                }
                if take_key(ui, Key::Escape, |_| true) {
                    keep = false;
                }
                let mut r = None;
                ui.horizontal(|ui| {
                    r = Some(ui.add(TextEdit::singleline(&mut st.query).id(id).hint_text("type a command").desired_width(ui.available_width() - 28.0 - 56.0)));
                    // How the query is read; a click takes the next one. (A
                    // drop-down would open outside the palette and close it.)
                    use crate::editfind::MatchMode;
                    let tip = "how to match: fuzzy (letters in order, e.g. spdn → Split down), text (as typed), glob (*, ?), regex — click for the next";
                    if ui.add_sized([50.0, 20.0], egui::Button::new(RichText::new(mode.label()).size(11.5).color(theme::text_dim()))).on_hover_text(tip).clicked() {
                        let order = [MatchMode::Fuzzy, MatchMode::Text, MatchMode::Glob, MatchMode::Regex];
                        mode = order[(order.iter().position(|m| *m == mode).unwrap_or(0) + 1) % order.len()];
                        st.sel = 0;
                    }
                    if icons::button(ui, Icon::Cross, "close (Esc)").clicked() {
                        keep = false;
                    }
                });
                let r = r.unwrap();
                r.request_focus();
                if r.changed() {
                    st.sel = 0;
                }
                ui.add_space(4.0);
                // As tall as the window allows below the palette (anchored 70
                // from the top, its search line above); the rest scrolls. From
                // the window size alone: the cursor is elsewhere while egui
                // sizes a newly opened popup, which left room for three rows.
                let room = (ctx.screen_rect().height() - 70.0 - 48.0 - 24.0).max(120.0);
                // egui keeps a popup's size from before and lets it grow only
                // when its content overflows, which a scroll list never does:
                // after a short result list (3 themes) the palette stayed that
                // small. Asking for the height the rows need makes it grow back.
                let want = room.min(items.len() as f32 * 26.0);
                // A new palette or a new query starts at the top, not where
                // the list was scrolled last time.
                let mut list = ScrollArea::vertical().id_salt("palette-list").max_height(room).min_scrolled_height(want).auto_shrink([false, true]);
                if r.changed() || std::mem::take(&mut st.fresh) {
                    list = list.vertical_scroll_offset(0.0);
                }
                list.show(ui, |ui| {
                    for (i, (cmd, label, keys)) in items.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                        if i == st.sel && moved {
                            ui.scroll_to_rect(rect, None);
                        }
                        if i == st.sel || resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(5), if i == st.sel { theme::tint(theme::accent(), 45) } else { theme::bg_hover() });
                        }
                        let keys_w = if keys.is_empty() { 0.0 } else { text_width(ui.painter(), keys, FontId::proportional(11.5)) + 16.0 };
                        // Faint grey vanished on the selected row's accent fill.
                        let keys_color = if i == st.sel { theme::text_dim() } else { theme::text_faint() };
                        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, keys, FontId::proportional(11.5), keys_color);
                        text_fit(ui.painter(), pos2(rect.left() + 10.0, rect.center().y), label, FontId::proportional(13.0), theme::text(), rect.width() - 20.0 - keys_w);
                        if resp.clicked() {
                            run = Some(*cmd);
                        }
                    }
                });
            });
        });
        if clicked_outside(ctx, area.response.rect) {
            keep = false;
        }
        if mode != self.settings.palette_mode {
            self.settings.palette_mode = mode;
            self.save_settings();
        }
        if let Some(c) = run.filter(|c| matches!(c, Cmd::FontUp | Cmd::FontDown | Cmd::FontReset | Cmd::UiUp | Cmd::UiDown | Cmd::UiReset)) {
            // Size steps keep the palette open so they can be clicked repeatedly.
            self.run_cmd(c);
            self.palette = Some(st);
        } else if let Some(c) = run {
            self.palette = None;
            self.focus_term = true;
            self.run_cmd(c);
        } else if keep {
            self.palette = Some(st);
        } else {
            self.focus_term = true;
        }
    }

    pub fn ui_connect(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.connect.take() else { return };
        let mut open: Option<String> = None;
        let mut keep = true;
        let area = egui::Area::new(Id::new("connect")).anchor(Align2::CENTER_TOP, vec2(0.0, 70.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(theme::bg_raised()).corner_radius(CornerRadius::same(8)).inner_margin(12.0).show(ui, |ui| {
                ui.set_width(480.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Connect").size(15.0).color(theme::text()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::button(ui, Icon::Cross, "close (Esc)").clicked() {
                            keep = false;
                        }
                    });
                });
                ui.label(RichText::new("Terminals run over `ssh -tt`. Files, editor, search and procs use a python agent fed through `ssh -T` stdin — nothing is copied to or installed on the host.").size(11.5).color(theme::text_faint()));
                ui.add_space(6.0);
                if take_key(ui, Key::Escape, |_| true) {
                    keep = false;
                }
                let r = ui.add(TextEdit::singleline(&mut st.target).hint_text("user@host  [-p 2222 …extra ssh args]").desired_width(f32::INFINITY));
                if ui.memory(|m| m.focused().is_none()) {
                    r.request_focus();
                }
                let pw = ui.add(
                    TextEdit::singleline(&mut st.password)
                        .password(true)
                        .hint_text("password (optional — leave blank to use keys / ssh-agent)")
                        .desired_width(f32::INFINITY),
                );
                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                if (r.lost_focus() || pw.lost_focus()) && enter && !st.target.trim().is_empty() {
                    open = Some(st.target.trim().to_string());
                }
                ui.label(
                    RichText::new("The password stays in memory for this session only. It is never saved or logged.")
                        .size(11.0)
                        .color(theme::text_faint()),
                );
                ui.horizontal(|ui| {
                    ui.checkbox(&mut st.split, "open as split");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                        if ui.add_enabled(!st.target.trim().is_empty(), egui::Button::new("Connect")).clicked() {
                            open = Some(st.target.trim().to_string());
                        }
                    });
                });
                ui.separator();
                let mut names: Vec<(String, &str)> = self
                    .settings
                    .profiles
                    .iter()
                    .map(|p| (p.name.clone(), match p.kind { HostKind::Local => "local shell", HostKind::Ssh { .. } => "ssh profile", HostKind::Custom { .. } => "custom transport", HostKind::Serial { .. } => "serial console" }))
                    .collect();
                for h in &self.ssh_hosts {
                    if !names.iter().any(|(n, _)| n == h) {
                        names.push((h.clone(), "~/.ssh/config"));
                    }
                }
                ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for (name, kind) in names {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(5), theme::bg_hover());
                        }
                        ui.painter().circle_filled(pos2(rect.left() + 12.0, rect.center().y), 4.0, host_color(&name));
                        let kind_w = text_width(ui.painter(), kind, FontId::proportional(11.5)) + 16.0;
                        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, kind, FontId::proportional(11.5), theme::text_faint());
                        text_fit(ui.painter(), pos2(rect.left() + 26.0, rect.center().y), &name, FontId::proportional(13.5), theme::text(), rect.width() - 36.0 - kind_w);
                        if resp.clicked() {
                            open = Some(name);
                        }
                    }
                });
            });
        });
        if clicked_outside(ctx, area.response.rect) {
            keep = false;
        }
        if let Some(target) = open {
            let host = if self.settings.profiles.iter().any(|p| p.name == target) || target == "local" {
                target
            } else {
                // Ad-hoc target with optional extra args: remember it as a profile.
                let prof = crate::host::Profile::ssh(&target);
                let name = prof.name.clone();
                if !self.settings.profiles.iter().any(|p| p.name == name) {
                    self.settings.profiles.push(prof);
                }
                name
            };
            if !st.password.is_empty() && host != "local" {
                // Must be set before the pane/agent spawn so both use it.
                self.set_password(&host, std::mem::take(&mut st.password));
            }
            if st.split {
                let cur = self.active_pane_id();
                let id = self.spawn_pane(&host, None);
                if let (Some(cur), Some(tab)) = (cur, self.tabs.get_mut(self.active_tab)) {
                    let root = std::mem::replace(&mut tab.root, Node::Leaf(0));
                    tab.root = replace_leaf(root, cur, id);
                    tab.active = id;
                    tab.zoomed = false;
                }
            } else {
                self.new_tab(&host, None);
            }
            self.focus_term = true;
        } else if keep {
            self.connect = Some(st);
        } else {
            self.focus_term = true;
        }
    }

    /// "The agent needs a password" dialog (opened automatically when keys /
    /// ssh-agent are refused, or from the status bar).
    pub fn ui_auth(&mut self, ctx: &egui::Context) {
        let Some((host, mut password)) = self.auth_prompt.take() else { return };
        let mut submit = false;
        let mut keep = true;
        let area = egui::Area::new(Id::new("auth")).anchor(Align2::CENTER_TOP, vec2(0.0, 90.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(theme::bg_raised()).corner_radius(CornerRadius::same(8)).inner_margin(12.0).show(ui, |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new(format!("Password for {host}")).size(15.0).color(theme::text()));
                ui.label(
                    RichText::new("Keys / ssh-agent were not accepted, so the file panel, editor, search and procs are offline. Enter the ssh password to use it instead (kept in memory for this session only).")
                        .size(11.5)
                        .color(theme::text_faint()),
                );
                ui.add_space(6.0);
                if take_key(ui, Key::Escape, |_| true) {
                    keep = false;
                }
                let r = ui.add(TextEdit::singleline(&mut password).password(true).hint_text("ssh password").desired_width(f32::INFINITY));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !password.is_empty() {
                    submit = true;
                }
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(!password.is_empty(), egui::Button::new("Connect")).clicked() {
                            submit = true;
                        }
                        if ui.button("Not now").clicked() {
                            keep = false;
                        }
                    });
                });
            });
        });
        if clicked_outside(ctx, area.response.rect) {
            keep = false;
        }
        if submit {
            self.set_password(&host, password);
            self.focus_term = true;
        } else if keep {
            self.auth_prompt = Some((host, password));
        } else {
            self.focus_term = true;
        }
    }

    /// Theme picker: every theme with a color swatch, plus "follow system".
    /// Font picker: terminal font and icon font, from the built-in fonts,
    /// data/fonts, the system's fonts or a path. A click applies at once.
    pub fn ui_font_menu(&mut self, ctx: &egui::Context) {
        let Some(mut m) = self.font_menu.take() else { return };
        let mut open = true;
        let mut chosen: Option<String> = None;
        let mut rescan = false;
        let current = if m.icons { self.settings.icon_font.clone() } else { self.settings.term_font.clone() };
        let fonts_dir = self.fonts_dir();
        let size = self.settings.font_size;
        crate::widgets::dialog(ctx, "Fonts", vec2(440.0, 520.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(!m.icons, "Terminal font").on_hover_text("text in the terminal and the editor").clicked() {
                    m.icons = false;
                }
                if ui.selectable_label(m.icons, "Icon font").on_hover_text("icons the terminal font lacks (pick a Nerd Font)").clicked() {
                    m.icons = true;
                }
            });
            // Sample in the font in use: Latin, look-alikes, Hangul and Nerd Font icons.
            let sample = "AaBb 0O 1lI {}=> 한글 \u{f1050} \u{e0b0} \u{f07c} \u{f418} \u{e7a8}";
            egui::Frame::new().fill(theme::bg_base()).corner_radius(CornerRadius::same(5)).inner_margin(8.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(sample).font(theme::term_font(size)).color(theme::text()));
            });
            ui.label(RichText::new(format!("now: {}", theme::font_label(&current))).size(11.5).color(theme::text_faint()));
            ui.add(TextEdit::singleline(&mut m.query).hint_text("filter fonts…").desired_width(f32::INFINITY));
            let q = m.query.to_lowercase();
            let shown = |name: &str| q.is_empty() || name.to_lowercase().contains(&q);
            let row = |ui: &mut Ui, name: &str, spec: &str, chosen: &mut Option<String>| {
                let sel = spec == current;
                if ui.selectable_label(sel, RichText::new(name).size(13.0)).on_hover_text(spec).clicked() && !sel {
                    *chosen = Some(spec.to_string());
                }
            };
            ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.label(RichText::new("Built in (Nerd Fonts)").size(11.5).color(theme::text_dim()));
                for (spec, name, _) in theme::BUILTIN_FONTS {
                    if shown(name) {
                        row(ui, name, spec, &mut chosen);
                    }
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Your fonts (data/fonts)").size(11.5).color(theme::text_dim()));
                let own: Vec<_> = m.files.iter().filter(|f| f.2).collect();
                if own.is_empty() {
                    ui.label(RichText::new("none yet: put .ttf / .otf files in the fonts folder (below), then Rescan").size(11.5).color(theme::text_faint()));
                }
                for (name, path, _) in own {
                    if shown(name) {
                        row(ui, name, path, &mut chosen);
                    }
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Installed on this computer").size(11.5).color(theme::text_dim()));
                for (name, path, own) in &m.files {
                    if !own && shown(name) {
                        row(ui, name, path, &mut chosen);
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Open fonts folder").on_hover_text(fonts_dir.display().to_string()).clicked() {
                    let _ = std::fs::create_dir_all(&fonts_dir);
                    let opener = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                    let _ = std::process::Command::new(opener).arg(&fonts_dir).spawn();
                }
                if ui.button("Rescan").on_hover_text("look again for fonts added since").clicked() {
                    rescan = true;
                }
            });
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut m.path).hint_text(r"or a font file: C:\…\MyFont.ttf").desired_width(ui.available_width() - 60.0));
                if ui.add_enabled(!m.path.trim().is_empty(), egui::Button::new("Use")).clicked() {
                    chosen = Some(m.path.trim().trim_matches('"').replace('\\', "/"));
                }
            });
            ui.label(RichText::new("Hangul comes from Malgun Gothic when the font has none.").size(11.0).color(theme::text_faint()));
        });
        if rescan {
            m.files = theme::font_files(&fonts_dir);
        }
        let icons = m.icons;
        if open {
            self.font_menu = Some(m);
        }
        if let Some(spec) = chosen {
            self.set_font(icons, spec);
        }
    }

    pub fn ui_theme_menu(&mut self, ctx: &egui::Context) {
        let Some(button) = self.theme_menu else { return };
        let width = 260.0;
        let screen = ctx.screen_rect();
        let pos = pos2((button.right() - width).clamp(8.0, screen.right() - width - 8.0), button.bottom() + 4.0);
        let mut pick = None;
        let mut system = false;
        let mut keep = true;
        let current = self.settings.theme.clone();
        let area = egui::Area::new(Id::new("theme-menu")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(theme::bg_raised()).corner_radius(CornerRadius::same(8)).inner_margin(6.0).show(ui, |ui| {
                ui.set_width(width - 12.0);
                if take_key(ui, Key::Escape, |_| true) {
                    keep = false;
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Theme").size(12.5).color(theme::text_dim()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::button(ui, Icon::Cross, "close (Esc)").clicked() {
                            keep = false;
                        }
                    });
                });
                let row = |ui: &mut Ui, swatch: Option<[Color32; 3]>, label: &str, selected: bool| -> bool {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    if selected || resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(5), if selected { theme::tint(theme::accent(), 40) } else { theme::bg_hover() });
                    }
                    let mut x = rect.left() + 8.0;
                    if let Some([bg, accent, fg]) = swatch {
                        let chip = egui::Rect::from_min_size(pos2(x, rect.center().y - 8.0), vec2(34.0, 16.0));
                        ui.painter().rect_filled(chip, CornerRadius::same(4), bg);
                        ui.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0_f32, theme::border()), egui::StrokeKind::Inside);
                        ui.painter().circle_filled(pos2(chip.left() + 10.0, chip.center().y), 4.0, accent);
                        ui.painter().circle_filled(pos2(chip.left() + 23.0, chip.center().y), 4.0, fg);
                        x = chip.right() + 10.0;
                    }
                    ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), theme::text());
                    if selected {
                        icons::paint(ui.painter(), egui::Rect::from_min_size(pos2(rect.right() - 22.0, rect.center().y - 7.0), vec2(14.0, 14.0)), Icon::Check, theme::accent());
                    }
                    resp.clicked()
                };
                for (i, id, name, _) in theme::themes() {
                    if row(ui, Some(theme::swatch(i)), name, current == id) {
                        pick = Some(i);
                    }
                }
                ui.separator();
                let dark = theme::themes().find(|t| t.1 == self.settings.theme_dark).map(|t| t.2).unwrap_or("?");
                let light = theme::themes().find(|t| t.1 == self.settings.theme_light).map(|t| t.2).unwrap_or("?");
                if row(ui, None, "Follow system", current == "system") {
                    system = true;
                }
                ui.label(RichText::new(format!("dark: {dark} · light: {light}")).size(11.0).color(theme::text_faint()));
            });
        });
        // The opening button toggles the menu itself; don't count it as "outside".
        let on_button = ctx.input(|i| i.pointer.press_origin().map(|p| button.contains(p)).unwrap_or(false));
        if clicked_outside(ctx, area.response.rect) && !on_button {
            keep = false;
        }
        if let Some(i) = pick {
            self.pick_theme(i);
        }
        if system {
            self.settings.theme = "system".into();
        }
        if !keep {
            self.theme_menu = None;
        }
    }

    pub fn ui_toasts(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
        egui::Area::new(Id::new("toasts")).anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -36.0)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
            for t in &self.toasts {
                let col = match t.kind {
                    ToastKind::Info => theme::accent(),
                    ToastKind::Ok => theme::success(),
                    ToastKind::Error => theme::error(),
                };
                egui::Frame::new().fill(theme::bg_raised()).stroke(Stroke::new(1.0_f32, theme::border())).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 8 }).show(ui, |ui| {
                    ui.set_max_width(380.0);
                    let r = ui.label(RichText::new(&t.text).size(12.5).color(theme::text()));
                    ui.painter().rect_filled(egui::Rect::from_min_size(pos2(r.rect.left() - 8.0, r.rect.top()), vec2(3.0, r.rect.height())), CornerRadius::same(1), col);
                });
                ui.add_space(6.0);
            }
        });
    }
}

/// A mouse press this frame that started outside `rect` (closes popups).
/// A press released in the same frame is ignored: that is the click that
/// opened the popup (e.g. the toolbar button), not a click away from it.
fn clicked_outside(ctx: &egui::Context, rect: egui::Rect) -> bool {
    ctx.input(|i| {
        i.pointer.any_pressed() && !i.pointer.any_released() && i.pointer.press_origin().map(|p| !rect.contains(p)).unwrap_or(false)
    })
}

#[derive(Clone, Default)]
struct MdCache {
    key: u64,
    job: Option<Arc<LayoutJob>>,
    decos: Arc<Vec<md::Deco>>,
}

/// The markdown editor: the TextEdit edits the raw source while its layout
/// renders it (md::layout); bullets, checkboxes, quote bars, code
/// backgrounds and rules are painted at the galley positions afterwards.
/// Returns whether the text changed.
#[allow(clippy::too_many_arguments)]
/// History search: indices into `all`, newest first. The command is matched
/// whole with the filter's mode; with `in_output`, output is searched for the
/// text as typed (any case), or with the regex in regex mode.
fn history_matches(all: &[crate::records::Record], q: &str, mode: FindMode, in_output: bool, host: Option<&str>) -> Result<Vec<usize>, String> {
    let host_ok = |r: &crate::records::Record| host.map(|h| r.host == h).unwrap_or(true);
    if q.is_empty() {
        return Ok((0..all.len()).rev().filter(|&i| host_ok(&all[i])).collect());
    }
    let m = match find::Matcher::new(mode, q)? {
        find::Matcher::Pattern { re, .. } => find::Matcher::Pattern { re, full_path: true },
        m => m,
    };
    let out_re = if in_output {
        let (pat, icase) = if mode == FindMode::Regex { (q.to_string(), !q.chars().any(char::is_uppercase)) } else { (regex::escape(q), true) };
        Some(regex::RegexBuilder::new(&pat).case_insensitive(icase).build().map_err(|e| e.to_string())?)
    } else {
        None
    };
    Ok((0..all.len())
        .rev()
        .filter(|&i| {
            let r = &all[i];
            host_ok(r) && (m.matches(&r.cmd).is_some() || out_re.as_ref().map(|re| re.is_match(&r.out)).unwrap_or(false))
        })
        .collect())
}

/// Which tabs fit in `room` (by index, in order) and which go under a "more"
/// button `more` wide. The `active` one is always shown.
fn fit_tabs(widths: &[f32], room: f32, more: f32, active: usize) -> (Vec<usize>, Vec<usize>) {
    if widths.iter().sum::<f32>() <= room {
        return ((0..widths.len()).collect(), Vec::new());
    }
    let room = room - more;
    let mut shown: Vec<usize> = Vec::new();
    let mut used = 0.0;
    for (i, w) in widths.iter().enumerate() {
        if used + w > room {
            break;
        }
        shown.push(i);
        used += w;
    }
    if !shown.contains(&active) {
        while let Some(&last) = shown.last() {
            if used + widths[active] <= room {
                break;
            }
            used -= widths[last];
            shown.pop();
        }
        shown.push(active);
    }
    let hidden = (0..widths.len()).filter(|i| !shown.contains(i)).collect();
    (shown, hidden)
}

/// What the tab strip needs at least beside the tab bar's buttons: one tab
/// (90) and the + and connect buttons after it, with their margins.
const TAB_STRIP_LEAST: f32 = 90.0 + 4.0 + 2.0 * (22.0 + 2.0) + 2.0 + 12.0 + 16.0;

/// Where a dragged tab lands: the index of the tab it goes before (len: the end).
fn tab_drop_index(rects: &[egui::Rect], x: f32) -> usize {
    rects.iter().position(|r| r.is_positive() && x < r.center().x).unwrap_or(rects.len())
}

/// What a Search result's click or menu asked for.
enum HitAction {
    Open(String, usize),
    Cd(String),
    /// Folder to browse, file to select in it.
    Reveal(String, String),
    Copy(String),
}

/// Right-click menu of a Search result: a file (`line` None) or one match.
fn hit_menu(ui: &mut Ui, path: &str, line: Option<(usize, &str)>, can_cd: bool, chosen: &mut Option<HitAction>) {
    let dir = path.rsplit_once('/').map(|(d, _)| if d.is_empty() { "/" } else { d }).unwrap_or(path).to_string();
    let mut pick = |ui: &mut Ui, a: HitAction| {
        *chosen = Some(a);
        ui.close_menu();
    };
    let open_label = if line.is_some() { "Open at this line" } else { "Open" };
    if ui.button(open_label).clicked() {
        pick(ui, HitAction::Open(path.to_string(), line.map(|l| l.0).unwrap_or(1)));
    }
    let cd = ui.add_enabled(can_cd, egui::Button::new("cd terminal to its folder"));
    let cd = if can_cd { cd.on_hover_text(dir.as_str()) } else { cd.on_disabled_hover_text("needs a terminal on this host, waiting at a prompt") };
    if cd.clicked() {
        pick(ui, HitAction::Cd(dir.clone()));
    }
    if ui.button("Show in Files").clicked() {
        pick(ui, HitAction::Reveal(dir.clone(), path.to_string()));
    }
    ui.separator();
    if ui.button("Copy path").clicked() {
        pick(ui, HitAction::Copy(path.to_string()));
    }
    if let Some((n, text)) = line {
        if ui.button("Copy path:line").clicked() {
            pick(ui, HitAction::Copy(format!("{path}:{n}")));
        }
        if ui.button("Copy line text").clicked() {
            pick(ui, HitAction::Copy(text.to_string()));
        }
    }
}

/// Command groups, in the order the menu and help show them.
const GROUPS: [&str; 6] = ["Tabs and panes", "Terminal", "Panels, files and memos", "Look", "Privacy, settings and keys", "Help"];

fn group(c: Cmd) -> &'static str {
    match c {
        Cmd::NewTab
        | Cmd::Connect
        | Cmd::Serial
        | Cmd::X11
        | Cmd::SplitRight
        | Cmd::SplitDown
        | Cmd::ClosePane
        | Cmd::Zoom
        | Cmd::FocusMode
        | Cmd::NextTab
        | Cmd::PrevTab
        | Cmd::GoTab(_)
        | Cmd::FocusLeft
        | Cmd::FocusRight
        | Cmd::FocusUp
        | Cmd::FocusDown
        | Cmd::Resize(..) => GROUPS[0],
        Cmd::Composer
        | Cmd::Find
        | Cmd::CopyMode
        | Cmd::PrevBlock
        | Cmd::NextBlock
        | Cmd::FoldAll
        | Cmd::UnfoldAll
        | Cmd::ClearScrollback
        | Cmd::Reconnect
        | Cmd::CursorStyle
        | Cmd::LocalShell(_) => GROUPS[1],
        Cmd::Sidebar | Cmd::Editor | Cmd::PanelSplit(..) | Cmd::Hidden | Cmd::History | Cmd::NewMemo | Cmd::Memos | Cmd::OpenMemo => GROUPS[2],
        Cmd::FontUp
        | Cmd::FontDown
        | Cmd::FontReset
        | Cmd::UiUp
        | Cmd::UiDown
        | Cmd::UiReset
        | Cmd::ThemePicker
        | Cmd::ThemeSystem
        | Cmd::Theme(_)
        | Cmd::Fonts
        | Cmd::MarkdownBreaks => GROUPS[3],
        Cmd::Records | Cmd::Settings | Cmd::AgentLog | Cmd::Keys | Cmd::Restart | Cmd::ConfirmUnsaved | Cmd::ConfirmRunning => GROUPS[4],
        Cmd::Help | Cmd::Features | Cmd::Palette | Cmd::About | Cmd::DisplayTest | Cmd::Trace => GROUPS[5],
    }
}

/// A time setting in seconds, edited as a number and a unit (seconds,
/// minutes, hours). 0 shows `zero`.
/// A key binding drawn as keycaps ("Ctrl" "Shift" "T"); hovering shows the
/// × that a click acts on (remove). Red when another command has it too.
fn key_chip(ui: &mut Ui, label: &str, clash: bool, others: &[&str]) -> egui::Response {
    let font = FontId::monospace(11.5);
    let parts: Vec<&str> = if label.ends_with("++") { vec![label.trim_end_matches("++"), "+"] } else { label.split('+').collect() };
    let widths: Vec<f32> = parts.iter().map(|p| text_width(ui.painter(), p, font.clone()) + 10.0).collect();
    let gap = 3.0;
    let cross = 14.0;
    let w = widths.iter().sum::<f32>() + gap * (parts.len().saturating_sub(1)) as f32 + 8.0 + cross;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
    let hot = resp.hovered();
    let edge = if clash { theme::error() } else if hot { theme::text_dim() } else { theme::border() };
    ui.painter().rect(rect, CornerRadius::same(5), if hot { theme::bg_hover() } else { theme::bg_raised() }, Stroke::new(1.0_f32, edge), egui::StrokeKind::Inside);
    let mut x = rect.left() + 4.0;
    for (p, pw) in parts.iter().zip(&widths) {
        let cap = egui::Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(*pw, 16.0));
        ui.painter().rect_filled(cap, CornerRadius::same(3), theme::bg_base());
        ui.painter().text(cap.center(), Align2::CENTER_CENTER, *p, font.clone(), if clash { theme::error() } else { theme::text() });
        x += pw + gap;
    }
    let cx = rect.right() - cross / 2.0 - 3.0;
    ui.painter().text(pos2(cx, rect.center().y), Align2::CENTER_CENTER, "\u{00d7}", FontId::proportional(13.0), if hot { theme::error() } else { theme::text_faint() });
    let tip = if others.is_empty() { "click to remove this key".to_string() } else { format!("also bound to: {} · click to remove", others.join(", ")) };
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn secs_input(ui: &mut Ui, id: &str, secs: &mut u64, zero: &str) -> bool {
    use crate::secret::{split_secs, UNITS};
    let unit_id = Id::new(("secs-unit", id));
    let (mut v, mut u) = split_secs(*secs);
    // Keep the unit last chosen while it still fits the value.
    if let Some(saved) = ui.data(|d| d.get_temp::<usize>(unit_id)) {
        if (*secs).is_multiple_of(UNITS[saved].1) {
            (v, u) = (*secs / UNITS[saved].1, saved);
        }
    }
    // "never" while shown, but a plain 0 once clicked to type a number (the
    // DragValue's id is the next auto id; it has focus while being typed in).
    let editing = ui.memory(|m| m.has_focus(ui.next_auto_id()));
    let zero = if editing { "0".to_string() } else { zero.to_string() };
    let mut changed = ui.add(egui::DragValue::new(&mut v).range(0..=100_000).speed(0.25).custom_formatter(move |x, _| if x == 0.0 { zero.clone() } else { format!("{x}") })).changed();
    egui::ComboBox::from_id_salt(("secs-combo", id)).width(76.0).selected_text(UNITS[u].0).show_ui(ui, |ui| {
        for (i, (name, _)) in UNITS.iter().enumerate() {
            changed |= ui.selectable_value(&mut u, i, *name).changed();
        }
    });
    ui.data_mut(|d| d.insert_temp(unit_id, u));
    if changed {
        *secs = v * UNITS[u].1;
    }
    changed
}

/// The width to fit tables to: `now` once it has stayed the same for a
/// moment, until then the one before (resizing reuses the last layout).
fn settled_width(ctx: &egui::Context, id: Id, now: f32) -> f32 {
    const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);
    let (settled, pending) = ctx.memory(|m| m.data.get_temp::<(f32, Option<(f32, std::time::Instant)>)>(id)).unwrap_or((now, None));
    let state = if (now - settled).abs() < 1.0 {
        (settled, None)
    } else {
        match pending {
            Some((w, at)) if (w - now).abs() < 1.0 && at.elapsed() >= SETTLE => (now, None),
            Some((w, at)) if (w - now).abs() < 1.0 => {
                ctx.request_repaint_after(SETTLE.saturating_sub(at.elapsed()));
                (settled, Some((w, at)))
            }
            _ => {
                ctx.request_repaint_after(SETTLE);
                (settled, Some((now, std::time::Instant::now())))
            }
        }
    };
    ctx.memory_mut(|m| m.data.insert_temp(id, state));
    state.0
}

// One call site, the editor, which has each of these at hand.
#[allow(clippy::too_many_arguments)]
fn markdown_body(ui: &mut Ui, f: &mut crate::app::EditorFile, edit_id: Id, size: f32, breaks: bool, renderer: &crate::mermaid::Renderer, readonly: bool, goto: Option<usize>, cache_dir: &std::path::Path) -> bool {
    let ctx = ui.ctx().clone();
    let focused = ui.memory(|m| m.has_focus(edit_id));
    // The cursor's line shows its markup (from last frame's cursor); in read
    // mode everything stays rendered.
    let cursor_line = if focused && !f.read {
        TextEdit::load_state(&ctx, edit_id).and_then(|s| s.cursor.char_range()).map(|r| md::line_of_char(&f.text, r.primary.index))
    } else {
        None
    };
    let cache_id = edit_id.with("md");
    let (marks, mark_cur) = (f.marks.clone(), f.mark_cur);
    let style_key = format!("{cursor_line:?}|{size}|{breaks}|{renderer:?}|{}|{}|{}|{}", theme::active(), crate::mermaid::generation(), crate::highlight::ready(), marks_key(&marks, mark_cur));
    let dark = !theme::is_light();
    // Rendered diagram size in points: mmdc renders at scale 2; never wider than the editor.
    let diagram_size = |ctx: &egui::Context, source: &str, wrap: f32| -> Option<(egui::TextureId, egui::Vec2)> {
        match crate::mermaid::status(ctx, cache_dir, source, dark, renderer) {
            crate::mermaid::Status::Ready(tex, px) => {
                // Grows with the editor font (Ctrl+= / Ctrl+-), 1:1 at the default size.
                let pt = px / 2.0 * (size / 14.5);
                let scale = (wrap / pt.x).min(1.0);
                Some((tex, pt * scale))
            }
            _ => None,
        }
    };
    let mut changed = false;
    ScrollArea::vertical().id_salt(("md-scroll", edit_id)).auto_shrink([false, false]).show(ui, |ui| {
        let bg = ui.painter().add(egui::Shape::Noop);
        let width = ui.available_width();
        let mut layouter = |ui: &Ui, text: &str, wrap: f32| {
            // The width too: table columns are fitted to it. While the editor
            // is being resized the last layout stays (laying out the document
            // and every table cell again at each pixel made resizing crawl);
            // once the width holds still for a moment it is redone.
            let fit_w = settled_width(ui.ctx(), cache_id.with("width"), wrap);
            let key = hash_str(text, &format!("{style_key}|{}", fit_w.round()));
            // The galley is for this text, these styles and this wrap width.
            let kept_id = cache_id.with("galley");
            let kept_key = key ^ (wrap.to_bits() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            if let Some(galley) = KeptGalley::get(ui, kept_id, kept_key) {
                return galley;
            }
            let cached: MdCache = ui.ctx().memory_mut(|m| m.data.get_temp_mut_or_default::<MdCache>(cache_id).clone());
            let job = match cached.job {
                Some(j) if cached.key == key => j,
                _ => {
                    let mut measure = |t: &str, font: &FontId| ui.fonts(|f| f.layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE).size().x);
                    let mut fit = |j: egui::text::LayoutJob| ui.fonts(|f| f.layout_job(j).size());
                    let mut l = md::layout(text, cursor_line, size, breaks, fit_w, &mut |src: &str| diagram_size(ui.ctx(), src, fit_w).map(|(_, s)| s.y), &mut measure, &mut fit);
                    crate::highlight::mark_job(&mut l.job, &marks, mark_cur);
                    let j = Arc::new(l.job);
                    ui.ctx().memory_mut(|m| m.data.insert_temp(cache_id, MdCache { key, job: Some(j.clone()), decos: Arc::new(l.decos) }));
                    j
                }
            };
            let mut job = (*job).clone();
            job.wrap.max_width = wrap;
            KeptGalley::lay_out(ui, kept_id, kept_key, job)
        };
        let before = crate::widgets::cursor_before(ui, edit_id);
        // Read mode: the text as an immutable buffer, selectable and copyable.
        let read = f.read;
        let mut shown: &str;
        let buf: &mut dyn egui::TextBuffer = if read {
            shown = f.text.as_str();
            &mut shown
        } else {
            &mut f.text
        };
        let out = TextEdit::multiline(buf)
            .id(edit_id)
            .frame(false)
            .margin(egui::Margin::ZERO)
            .desired_width(width)
            .desired_rows(30)
            .interactive(!readonly)
            .lock_focus(true)
            .layouter(&mut layouter)
            .show(ui);
        crate::widgets::select_word_on_double_click(ui, &out, &f.text, before);
        changed = out.response.changed();
        let galley = &out.galley;
        let origin = out.galley_pos;
        let at = |i: usize| galley.pos_from_ccursor(egui::text::CCursor::new(i)).translate(origin.to_vec2());
        let decos = ctx.memory_mut(|m| m.data.get_temp_mut_or_default::<MdCache>(cache_id).decos.clone());
        let selection = out.cursor_range.map(|r| {
            let (a, b) = (r.primary.ccursor.index, r.secondary.ccursor.index);
            (a.min(b), a.max(b))
        }).filter(|(a, b)| a != b);
        let right = origin.x + width;
        let mut back = Vec::new();
        let painter = ui.painter().clone();
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let mut toggle: Option<usize> = None;
        let mut tables: Vec<TableView> = Vec::new();
        // Which table of the document this is, counting those scrolled out
        // of view too: a selection stays with its table.
        let mut table_no = 0;
        for d in decos.iter() {
            match *d {
                md::Deco::Code { start, end } => {
                    let r = egui::Rect::from_min_max(pos2(origin.x - 4.0, at(start).top() - 2.0), pos2(right, at(end).bottom() + 2.0));
                    back.push(egui::Shape::rect_filled(r, CornerRadius::same(5), theme::tint(theme::text_faint(), 30)));
                }
                md::Deco::Quote { start, end } => {
                    let r = egui::Rect::from_min_max(pos2(origin.x + 1.0, at(start).top()), pos2(origin.x + 4.0, at(end).bottom()));
                    back.push(egui::Shape::rect_filled(r, CornerRadius::same(1), theme::tint(theme::accent(), 160)));
                }
                md::Deco::Bullet { at: i, depth, col } => {
                    // In the middle of its column; by depth a filled dot, a
                    // hollow dot, a filled square, a hollow square, and
                    // again: next to each other, filled and hollow always
                    // differ (at a small size a dot and a square hardly do).
                    let r = at(i);
                    let c = pos2(r.left() - col * 0.5, r.center().y);
                    let (dot, square) = (size * 0.17, egui::Rect::from_center_size(c, vec2(size * 0.32, size * 0.32)));
                    let line = Stroke::new(1.3_f32, theme::text_dim());
                    match depth % 4 {
                        0 => {
                            painter.circle_filled(c, dot, theme::text_dim());
                        }
                        1 => {
                            painter.circle_stroke(c, dot, line);
                        }
                        2 => {
                            painter.rect_filled(square, CornerRadius::ZERO, theme::text_dim());
                        }
                        _ => {
                            painter.rect_stroke(square, CornerRadius::ZERO, line, egui::StrokeKind::Inside);
                        }
                    }
                }
                md::Deco::Number { at: i, ref label } => {
                    let r = at(i);
                    painter.text(pos2(r.left() - size * 0.3, r.center().y), egui::Align2::RIGHT_CENTER, label, FontId::proportional(size), theme::text_dim());
                }
                md::Deco::Check { at: i, byte, checked } => {
                    let r = at(i);
                    let b = egui::Rect::from_center_size(pos2(r.left() + size * 0.55, r.center().y), vec2(size * 0.85, size * 0.85));
                    if checked {
                        painter.rect_filled(b, CornerRadius::same(3), theme::accent());
                        icons::paint(&painter, b.shrink(1.5), Icon::Check, theme::bg_term());
                    } else {
                        painter.rect_stroke(b, CornerRadius::same(3), Stroke::new(1.2_f32, theme::text_dim()), egui::StrokeKind::Inside);
                    }
                    if out.response.clicked() && pointer.map(|p| b.expand(2.0).contains(p)).unwrap_or(false) && !readonly && !read {
                        toggle = Some(byte);
                    }
                }
                md::Deco::Rule { at: i } => {
                    let y = at(i).center().y;
                    painter.hline(origin.x..=right, y, Stroke::new(1.0_f32, theme::border()));
                }
                md::Deco::Underline { at: i } => {
                    let y = at(i).bottom() + 3.0;
                    painter.hline(origin.x..=right, y, Stroke::new(1.0_f32, theme::border()));
                }
                md::Deco::Table { at: i, grid: (ref cells, ref cols, ref rows_y, vpad), ref rows } => {
                    let no = table_no;
                    table_no += 1;
                    // A table that is scrolled out of view: none of its cells
                    // is laid out or drawn (a long document has hundreds).
                    if let (Some(&w), Some(&h)) = (cols.last(), rows_y.last()) {
                        if !ui.is_rect_visible(egui::Rect::from_min_size(pos2(origin.x, at(i).top()), vec2(w, h))) {
                            continue;
                        }
                    }
                    // A selection over the table: the rows it takes, whole (it
                    // copies their markdown source).
                    if let (Some(&last_x), Some((lo, hi))) = (cols.last(), selection) {
                        let top = at(i).top();
                        for (r, &(s, e)) in rows.iter().enumerate() {
                            if s < hi && e > lo {
                                if let (Some(&y0), Some(&y1)) = (rows_y.get(r), rows_y.get(r + 1)) {
                                    let band = egui::Rect::from_min_max(pos2(origin.x, top + y0), pos2(origin.x + last_x, top + y1));
                                    painter.rect_filled(band, CornerRadius::ZERO, theme::tint(ui.visuals().selection.bg_fill, 90));
                                }
                            }
                        }
                    }
                    let (Some(&last_x), Some(&height)) = (cols.last(), rows_y.last()) else { continue };
                    let top = at(i).top();
                    let x = |c: f32| origin.x + c;
                    let stroke = Stroke::new(1.0_f32, theme::border());
                    if let Some(&header_bottom) = rows_y.get(1) {
                        let header = egui::Rect::from_min_max(pos2(x(0.0), top), pos2(x(last_x), top + header_bottom));
                        back.push(egui::Shape::rect_filled(header, CornerRadius::ZERO, theme::tint(theme::text_faint(), 30)));
                    }
                    for &y in rows_y.iter() {
                        painter.hline(x(0.0)..=x(last_x), top + y, stroke);
                    }
                    for &c in cols.iter() {
                        painter.vline(x(c), top..=top + height, stroke);
                    }
                    let mut laid = Vec::new();
                    for (row, &y) in cells.iter().zip(rows_y.iter()) {
                        let mut line = Vec::new();
                        for (job, cx) in row {
                            let g = painter.layout_job(job.clone());
                            let at = pos2(x(*cx), top + y + vpad);
                            painter.galley(at, g.clone(), theme::text());
                            line.push((g, at));
                        }
                        laid.push(line);
                    }
                    tables.push(TableView { no, rect: egui::Rect::from_min_size(pos2(x(0.0), top), vec2(last_x, height)), xs: cols.clone(), ys: rows_y.clone(), rows: rows.clone(), cells: laid });
                }
                md::Deco::Diagram { start, end, ref source, collapsed } => {
                    let top = at(start).top();
                    if collapsed {
                        if let Some((tex, sz)) = diagram_size(&ctx, source, width) {
                            let r = egui::Rect::from_min_size(pos2(origin.x, top), sz);
                            painter.image(tex, r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                    } else {
                        let note = match crate::mermaid::status(&ctx, cache_dir, source, dark, renderer) {
                            crate::mermaid::Status::Rendering => Some(("rendering diagram…".to_string(), theme::text_faint())),
                            crate::mermaid::Status::Unavailable(hint) => Some((hint, theme::text_faint())),
                            crate::mermaid::Status::Failed(e) => Some((e, theme::error())),
                            crate::mermaid::Status::Ready(..) => None,
                        };
                        if let Some((text, color)) = note {
                            let _ = end;
                            painter.text(pos2(right - 6.0, top + 2.0), Align2::RIGHT_TOP, text, FontId::proportional(11.0), color);
                        }
                    }
                }
            }
        }
        // Read mode: a rendered table is selected like a spreadsheet. A drag
        // inside one cell takes some of its letters, a drag across cells the
        // block of cells between, a double click one cell; Ctrl+C copies it
        // (cells tab-separated, rows on lines). Its menu copies that, or the
        // whole table as markdown or tab-separated. (In edit mode a click
        // shows the table's source instead, to edit it.)
        if read {
            let sel_id = edit_id.with("table-sel");
            let mut sel: Option<TableSel> = ctx.memory(|m| m.data.get_temp::<Option<TableSel>>(sel_id)).flatten();
            let (press, at_now, down, pressed, copy) = ctx.input(|i| (i.pointer.press_origin(), i.pointer.interact_pos(), i.pointer.primary_down(), i.pointer.primary_pressed(), i.events.iter().any(|e| matches!(e, egui::Event::Copy))));
            // Text selected in the document (by keyboard, say) takes its place.
            if pressed || selection.is_some() {
                sel = None;
            }
            let has_keyboard = ctx.memory(|m| m.has_focus(edit_id));
            let double = ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary) || i.pointer.button_triple_clicked(egui::PointerButton::Primary));
            for t in tables.iter() {
                let k = t.no;
                let (Some(&(first, _)), Some(&(_, last))) = (t.rows.first(), t.rows.last()) else { continue };
                if let (true, Some(o), Some(p)) = (down, press, at_now) {
                    if t.rect.contains(o) && out.response.dragged() {
                        let (from, to) = (t.cell_at(o), t.cell_at(p));
                        sel = (from != to).then_some(TableSel { table: k, from, to, whole: false });
                        // The text under the table keeps no selection of its own.
                        let mut st = out.state.clone();
                        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(first))));
                        st.store(&ctx, edit_id);
                    }
                }
                let hit = ui.interact(t.rect, edit_id.with(("table", k)), Sense::click());
                if let (true, Some(p)) = (double && hit.contains_pointer(), at_now) {
                    let c = t.cell_at(p);
                    sel = Some(TableSel { table: k, from: c, to: c, whole: true });
                    // Ctrl+C is the editor's from here (the click went to the table).
                    ctx.memory_mut(|m| m.request_focus(edit_id));
                }
                let texts = t.texts();
                let mine = sel.filter(|s| s.table == k);
                let mut copied = false;
                hit.context_menu(|ui| {
                    if let Some(s) = mine {
                        if ui.button("Copy selection").clicked() {
                            ctx.copy_text(table_sel_text(&texts, &s));
                            copied = true;
                            ui.close_menu();
                        }
                    }
                    if ui.button("Copy table (markdown)").clicked() {
                        ctx.copy_text(f.text.chars().skip(first).take(last - first).collect());
                        copied = true;
                        ui.close_menu();
                    }
                    if ui.button("Copy as TSV (for spreadsheets)").clicked() {
                        ctx.copy_text(texts.iter().map(|row| row.join("\t")).collect::<Vec<_>>().join("\n"));
                        copied = true;
                        ui.close_menu();
                    }
                });
                if let Some(s) = mine {
                    let fill = theme::tint(ui.visuals().selection.bg_fill, 110);
                    for r in t.sel_rects(&s) {
                        back.push(egui::Shape::rect_filled(r, CornerRadius::ZERO, fill));
                    }
                    // Only while the editor has the keyboard: Ctrl+C in the
                    // terminal or another field is theirs.
                    if copy && has_keyboard {
                        ctx.copy_text(table_sel_text(&texts, &s));
                        copied = true;
                    }
                }
                if copied {
                    f.copied_at = Some(std::time::Instant::now());
                }
            }
            ctx.memory_mut(|m| m.data.insert_temp(sel_id, sel));
        }
        ui.painter().set(bg, egui::Shape::Vec(back));
        if let Some(byte) = toggle {
            if let Some(c) = f.text.get(byte..byte + 1) {
                let new = if c == " " { "x" } else { " " };
                f.text.replace_range(byte..byte + 1, new);
                changed = true;
            }
        }
        if let Some(l) = goto {
            let idx: usize = f.text.split('\n').take(l.saturating_sub(1)).map(|s| s.chars().count() + 1).sum();
            ui.scroll_to_rect(at(idx), Some(Align::Center));
        }
    });
    changed
}

/// A rendered table on screen: where, its column borders' x and rows' y
/// (from its left / top), its rows' source, and each cell's text as laid out
/// and where.
struct TableView {
    /// Which table of the document (the first is 0).
    no: usize,
    rect: egui::Rect,
    xs: Vec<f32>,
    ys: Vec<f32>,
    rows: Vec<(usize, usize)>,
    cells: Vec<Vec<(Arc<egui::Galley>, egui::Pos2)>>,
}

/// What is selected in a rendered table: from one (row, column, letter) to
/// another. Within one cell it is the letters between, unless `whole` (a
/// double click); across cells it is the block of cells between.
#[derive(Clone, Copy, PartialEq, Debug)]
struct TableSel {
    table: usize,
    from: (usize, usize, usize),
    to: (usize, usize, usize),
    whole: bool,
}

impl TableSel {
    /// Some letters of one cell, not cells.
    fn within_cell(&self) -> bool {
        !self.whole && (self.from.0, self.from.1) == (self.to.0, self.to.1)
    }

    /// The block of cells: rows and columns, each first to last.
    fn block(&self) -> ((usize, usize), (usize, usize)) {
        ((self.from.0.min(self.to.0), self.from.0.max(self.to.0)), (self.from.1.min(self.to.1), self.from.1.max(self.to.1)))
    }
}

impl TableView {
    /// The (row, column, letter) at a point; a point outside is taken to
    /// the nearest cell.
    fn cell_at(&self, p: egui::Pos2) -> (usize, usize, usize) {
        let within = |edges: &[f32], v: f32| edges.windows(2).position(|w| v < w[1]).unwrap_or(edges.len().saturating_sub(2));
        let row = within(&self.ys, p.y - self.rect.top()).min(self.cells.len().saturating_sub(1));
        let col = within(&self.xs, p.x - self.rect.left());
        let letter = self.cells.get(row).and_then(|r| r.get(col)).map(|(g, at)| g.cursor_from_pos(p - *at).ccursor.index).unwrap_or(0);
        (row, col, letter)
    }

    fn texts(&self) -> Vec<Vec<String>> {
        self.cells.iter().map(|row| row.iter().map(|(g, _)| g.text().to_string()).collect()).collect()
    }

    /// What to highlight for a selection: the letters' rows inside a cell,
    /// or the cells of a block.
    fn sel_rects(&self, s: &TableSel) -> Vec<egui::Rect> {
        let mut out = Vec::new();
        if s.within_cell() {
            let Some((g, at)) = self.cells.get(s.from.0).and_then(|r| r.get(s.from.1)) else { return out };
            let (a, b) = (s.from.2.min(s.to.2), s.from.2.max(s.to.2));
            let mut seen = 0;
            for row in &g.rows {
                let n = row.char_count_excluding_newline();
                let (lo, hi) = (a.max(seen), b.min(seen + n));
                if lo < hi {
                    out.push(egui::Rect::from_min_max(pos2(at.x + row.x_offset(lo - seen), at.y + row.min_y()), pos2(at.x + row.x_offset(hi - seen), at.y + row.max_y())));
                }
                seen += row.char_count_including_newline();
            }
            return out;
        }
        let ((r0, r1), (c0, c1)) = s.block();
        for r in r0..=r1 {
            for c in c0..=c1 {
                if let (Some(y), Some(x)) = (self.ys.get(r..r + 2), self.xs.get(c..c + 2)) {
                    out.push(egui::Rect::from_min_max(self.rect.min + vec2(x[0], y[0]), self.rect.min + vec2(x[1], y[1])));
                }
            }
        }
        out
    }
}

/// What a selection in a table copies: some letters of a cell, a cell, or a
/// block of cells (rows on lines, cells tab-separated, as spreadsheets paste).
fn table_sel_text(cells: &[Vec<String>], s: &TableSel) -> String {
    if s.within_cell() {
        let (a, b) = (s.from.2.min(s.to.2), s.from.2.max(s.to.2));
        return cells.get(s.from.0).and_then(|r| r.get(s.from.1)).map(|t| t.chars().skip(a).take(b - a).collect()).unwrap_or_default();
    }
    let ((r0, r1), (c0, c1)) = s.block();
    let single = (r0, c0) == (r1, c1);
    let cell = |r: usize, c: usize| cells.get(r).and_then(|row| row.get(c)).map(|t| if single { t.clone() } else { t.replace(['\t', '\n'], " ") }).unwrap_or_default();
    (r0..=r1).map(|r| (c0..=c1).map(|c| cell(r, c)).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n")
}

/// The palette's rows for what is typed: which labels, in which order.
/// Fuzzy (letters in order, the best match first) unless the mode is text,
/// glob or regex, which keep the list's order. Nothing typed is everything;
/// a pattern that can't be read matches nothing.
pub fn palette_matches(mode: crate::editfind::MatchMode, query: &str, labels: &[String]) -> Vec<usize> {
    use crate::editfind::MatchMode;
    let q = query.trim();
    if q.is_empty() {
        return (0..labels.len()).collect();
    }
    if mode == MatchMode::Fuzzy {
        let Ok(m) = crate::find::Matcher::new(crate::find::FindMode::Fuzzy, q) else { return Vec::new() };
        let mut hits: Vec<(i64, usize)> = labels.iter().enumerate().filter_map(|(i, l)| m.matches(l).map(|(score, _)| (score, i))).collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        return hits.into_iter().map(|h| h.1).collect();
    }
    match crate::editfind::line_matcher(mode, q, true) {
        Some(re) => labels.iter().enumerate().filter(|(_, l)| re.is_match(l)).map(|(i, _)| i).collect(),
        None => Vec::new(),
    }
}

fn replace_leaf(n: Node, target: usize, new_id: usize) -> Node {
    match n {
        Node::Leaf(id) if id == target => Node::Split { vertical: true, ratio: 0.5, a: Box::new(Node::Leaf(id)), b: Box::new(Node::Leaf(new_id)) },
        Node::Leaf(id) => Node::Leaf(id),
        Node::Split { vertical, ratio, a, b } => Node::Split { vertical, ratio, a: Box::new(replace_leaf(*a, target, new_id)), b: Box::new(replace_leaf(*b, target, new_id)) },
    }
}

struct Rect2;
impl Rect2 {
    fn dot(r: egui::Rect) -> egui::Pos2 {
        pos2(r.left() + 11.0, r.center().y)
    }
}

/// The tab context menu: close this / others / to the right / to the left /
/// all. Items with nothing to close are disabled.
fn close_menu(ui: &mut Ui, at: usize, n: usize) -> Option<CloseWhich> {
    let mut chosen = None;
    for (which, label) in CloseWhich::MENU {
        let any = !tabs_to_close(n, at, which).is_empty();
        if ui.add_enabled(any, egui::Button::new(label)).clicked() {
            chosen = Some(which);
            ui.close_menu();
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_palette_matches_fuzzy_text_glob_or_regex() {
        use super::{palette_matches, COMMANDS};
        use crate::editfind::MatchMode;
        let labels: Vec<String> = COMMANDS.iter().map(|c| c.2.to_string()).collect();
        let found = |mode, q: &str| -> Vec<&str> { palette_matches(mode, q, &labels).into_iter().map(|i| labels[i].as_str()).collect() };
        // Nothing typed: everything, in order.
        assert_eq!(found(MatchMode::Fuzzy, "  ").len(), labels.len());
        assert_eq!(found(MatchMode::Fuzzy, "")[0], labels[0]);
        // Fuzzy: letters in order, the best first; text needs them together.
        assert_eq!(found(MatchMode::Fuzzy, "spldn").first(), Some(&"Split down"));
        assert_eq!(found(MatchMode::Fuzzy, "split down").first(), Some(&"Split down"));
        assert!(found(MatchMode::Fuzzy, "cmdpal").contains(&"Command palette"));
        assert!(found(MatchMode::Text, "spldn").is_empty());
        assert_eq!(found(MatchMode::Text, "split d"), ["Split down"]);
        assert!(found(MatchMode::Fuzzy, "zzzzqq").is_empty());
        // Glob and regex, any case, in the list's order.
        assert_eq!(found(MatchMode::Glob, "go to tab ?"), ["Go to tab 1", "Go to tab 2", "Go to tab 3", "Go to tab 4", "Go to tab 5", "Go to tab 6", "Go to tab 7", "Go to tab 8"]);
        assert_eq!(found(MatchMode::Regex, "^go to tab [1-3]$"), ["Go to tab 1", "Go to tab 2", "Go to tab 3"]);
        // A pattern that can't be read: nothing, and no panic.
        assert!(found(MatchMode::Regex, "(").is_empty());
    }

    #[test]
    fn a_table_selection_copies_letters_a_cell_or_a_block() {
        use super::{table_sel_text, TableSel};
        let cells: Vec<Vec<String>> = [["name", "size"], ["weft.exe", "12 MB"], ["notes.md", "3 KB"]].iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect();
        let sel = |from, to, whole| TableSel { table: 0, from, to, whole };
        // Letters of one cell, either direction.
        assert_eq!(table_sel_text(&cells, &sel((1, 0, 0), (1, 0, 4), false)), "weft");
        assert_eq!(table_sel_text(&cells, &sel((1, 0, 8), (1, 0, 5), false)), "exe");
        // A double-clicked cell.
        assert_eq!(table_sel_text(&cells, &sel((2, 1, 2), (2, 1, 2), true)), "3 KB");
        // A block, dragged from any corner; the letters no longer matter.
        assert_eq!(table_sel_text(&cells, &sel((1, 0, 3), (2, 1, 1), false)), "weft.exe\t12 MB\nnotes.md\t3 KB");
        assert_eq!(table_sel_text(&cells, &sel((2, 1, 0), (1, 0, 0), false)), "weft.exe\t12 MB\nnotes.md\t3 KB");
        // One column, one row.
        assert_eq!(table_sel_text(&cells, &sel((0, 1, 0), (2, 1, 0), false)), "size\n12 MB\n3 KB");
        assert_eq!(table_sel_text(&cells, &sel((0, 0, 0), (0, 1, 0), false)), "name\tsize");
    }

    use super::{history_matches, FindMode};
    use crate::app::memo_title;
    use crate::records::Record;

    #[test]
    fn default_keys_are_valid_and_distinct() {
        let mut ids = std::collections::HashSet::new();
        let mut keys = std::collections::HashMap::new();
        for (cmd, id, label, defaults) in super::COMMANDS {
            assert!(ids.insert(*id), "id {id} used twice");
            for d in *defaults {
                let k = crate::keys::Shortcut::parse(d).unwrap_or_else(|| panic!("{label}: {d} doesn't parse"));
                if let Some(other) = keys.insert(k.label(), *cmd) {
                    panic!("{d} is the default of both {other:?} and {cmd:?}");
                }
            }
        }
    }

    #[test]
    fn narrow_tab_rows_keep_the_open_tab() {
        let w = [40.0, 70.0, 55.0, 50.0, 45.0];
        assert_eq!(super::fit_tabs(&w, 300.0, 28.0, 0), (vec![0, 1, 2, 3, 4], vec![]), "all fit");
        assert_eq!(super::fit_tabs(&w, 200.0, 28.0, 0), (vec![0, 1, 2], vec![3, 4]));
        // The open tab is past the edge: it takes the last places shown.
        assert_eq!(super::fit_tabs(&w, 200.0, 28.0, 4), (vec![0, 1, 4], vec![2, 3]));
        assert_eq!(super::fit_tabs(&w, 60.0, 28.0, 3), (vec![3], vec![0, 1, 2, 4]), "only the open one");
    }

    #[test]
    fn dragged_tab_lands_between_tabs() {
        use egui::{pos2, Rect};
        let rects: Vec<Rect> = (0..3).map(|i| Rect::from_min_max(pos2(i as f32 * 100.0, 0.0), pos2(i as f32 * 100.0 + 98.0, 28.0))).collect();
        assert_eq!(super::tab_drop_index(&rects, 10.0), 0);
        assert_eq!(super::tab_drop_index(&rects, 60.0), 1, "past the middle of the first tab");
        assert_eq!(super::tab_drop_index(&rects, 160.0), 2);
        assert_eq!(super::tab_drop_index(&rects, 290.0), 3, "after the last");
    }

    #[test]
    fn history_search() {
        let rec = |host: &str, cmd: &str, out: &str| Record { t: 0.0, host: host.into(), cwd: String::new(), cmd: cmd.into(), exit: Some(0), dur: 0.0, out: out.into(), cut: 0 };
        let all = [rec("local", "git status", "On branch main"), rec("srv", "cargo build --release", "error[E0425]: not found"), rec("srv", "ls /var/log", "syslog")];
        let find = |q: &str, mode, out, host| history_matches(&all, q, mode, out, host).unwrap();
        // Newest first; everything for an empty query.
        assert_eq!(find("", FindMode::Fuzzy, false, None), [2, 1, 0]);
        assert_eq!(find("", FindMode::Fuzzy, false, Some("srv")), [2, 1]);
        assert_eq!(find("gst", FindMode::Fuzzy, false, None), [0]);
        // Glob over the whole command, slashes and all.
        assert_eq!(find("ls /var/*", FindMode::Glob, false, None), [2]);
        assert_eq!(find("^cargo .*release$", FindMode::Regex, false, None), [1]);
        // Output: only when asked, as typed in any case, or by regex.
        assert!(find("E0425", FindMode::Fuzzy, false, None).is_empty());
        assert_eq!(find("e0425", FindMode::Fuzzy, true, None), [1]);
        assert_eq!(find(r"E\d{4}", FindMode::Regex, true, None), [1]);
        assert!(history_matches(&all, "(", FindMode::Regex, false, None).is_err());
    }

    #[test]
    fn memo_tab_is_named_after_the_first_line() {
        assert_eq!(memo_title(""), "new memo");
        assert_eq!(memo_title("\n  \n## 회의 메모  \n본문"), "회의 메모");
        assert_eq!(memo_title("- [ ] buy milk"), "[ ] buy milk");
        assert_eq!(memo_title("> quoted"), "quoted");
        assert_eq!(memo_title(&"가".repeat(30)), format!("{}…", "가".repeat(24)));
    }

    /// A memo edited, saved by itself, edited and saved again: never "changed
    /// on disk, overwrite?" (its own write used to look like someone else's).
    #[test]
    fn memos_never_conflict_with_their_own_saves() {
        let dir = std::env::temp_dir().join(format!("weft-memo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = crate::app::App::new_in(egui::Context::default(), crate::host::DataDir::at(&dir));
        // An existing memo opened from the list (it is read, with its time).
        let path = format!("{}/memo-test.md", app.memo_dir());
        std::fs::create_dir_all(app.memo_dir()).unwrap();
        std::fs::write(&path, "# old").unwrap();
        app.open_file("local", &path, 1);
        // Read by the local backend's thread: a moment later.
        let asked = std::time::Instant::now();
        while asked.elapsed().as_secs() < 5 && !app.editor.files.iter().any(|f| f.path == path && f.mtime.is_some()) {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let i = app.editor.files.iter().position(|f| f.path == path).unwrap();
        assert!(app.editor.files[i].mtime.is_some(), "opened with its time");
        for (k, text) in ["# first", "# first\nmore", "# first\nmore\nand more"].iter().enumerate() {
            // The file system's clock is coarse: make each write land later.
            std::thread::sleep(std::time::Duration::from_millis(30));
            app.editor.files[i].text = text.to_string();
            app.editor.files[i].dirty = true;
            if k % 2 == 0 {
                app.flush_memos();
            } else {
                app.save_file(i, false);
            }
            for _ in 0..5 {
                app.poll();
            }
            let f = &app.editor.files[i];
            assert!(!f.conflict && !f.dirty, "write {k}: conflict {} dirty {}", f.conflict, f.dirty);
            assert_eq!(std::fs::read_to_string(&f.path).unwrap(), *text);
        }
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two side panels: the first follows the terminal, the one split off
    /// stays put. With the second clicked last, the first still follows.
    #[test]
    fn side_panels_that_follow_the_terminal_follow_it() {
        use crate::app::{PanelId, Region};
        let dir = std::env::temp_dir().join(format!("weft-follow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        let mut app = crate::app::App::new_in(ctx.clone(), crate::host::DataDir::at(&dir));
        app.settings.sidebar = true;
        app.side.follow = true;
        app.set_root("local", "C:/start");
        app.split_panel(PanelId::first(Region::Side), crate::app::Dock::Bottom);
        let frame = |app: &mut crate::app::App| {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.ui_layout(ui));
            });
        };
        frame(&mut app);
        // The second one clicked last: it becomes the focus, with its own view
        // (as a click in it does while the panels are drawn).
        app.side_focus = 1;
        let v = app.side_views[&1].clone();
        app.set_side_view(v);
        frame(&mut app);
        app.follow_terminal("local", "C:/moved");
        frame(&mut app);
        let root = |app: &crate::app::App, n: u32| app.side_views.get(&n).and_then(|v| v.root.clone()).map(|r| r.1);
        assert_eq!(root(&app, 0).as_deref(), Some("C:/moved"), "the first follows the terminal");
        assert_eq!(root(&app, 1).as_deref(), Some("C:/start"), "the split-off one stays");
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ctrl+Shift+Z is redo in a text field, the Zoom command elsewhere.
    #[test]
    fn redo_key_stays_with_a_text_field() {
        let dir = std::env::temp_dir().join(format!("weft-redo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = egui::Context::default();
        let mut app = crate::app::App::new_in(ctx.clone(), crate::host::DataDir::at(&dir));
        let mut text = String::from("abc");
        let field = egui::Id::new("a text field");
        let key = egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers { ctrl: true, shift: true, ..Default::default() } };
        // Returns whether the key was left for the widgets.
        let mut frame = |events: Vec<egui::Event>, focus: bool| {
            let mut kept = false;
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                app.handle_global_keys(ctx);
                kept = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Z, .. })));
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut text).id(field));
                    if focus {
                        r.request_focus();
                    } else {
                        r.surrender_focus();
                    }
                });
            });
            kept
        };
        frame(vec![], true);
        frame(vec![], true);
        assert!(frame(vec![key.clone()], true), "redo reaches the focused text field");
        frame(vec![], false);
        frame(vec![], false);
        assert!(!frame(vec![key], false), "elsewhere the Zoom command takes it");
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One frame of a seconds field; the texts drawn and where.
    fn secs_frame(ctx: &egui::Context, secs: &mut u64, events: Vec<egui::Event>) -> Vec<(String, egui::Pos2)> {
        let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))), ..Default::default() };
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    super::secs_input(ui, "t", secs, "never");
                });
            });
        });
        out.shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.pos + t.galley.rect.center().to_vec2())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn never_reads_zero_once_clicked() {
        let ctx = egui::Context::default();
        let mut secs = 0;
        secs_frame(&ctx, &mut secs, vec![]);
        let shown = secs_frame(&ctx, &mut secs, vec![]);
        let at = shown.iter().find(|t| t.0 == "never").expect("never shown").1;
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        secs_frame(&ctx, &mut secs, vec![egui::Event::PointerMoved(at)]);
        secs_frame(&ctx, &mut secs, vec![click(true)]);
        secs_frame(&ctx, &mut secs, vec![click(false)]);
        let editing = secs_frame(&ctx, &mut secs, vec![]);
        let texts: Vec<&str> = editing.iter().map(|t| t.0.as_str()).collect();
        assert!(texts.contains(&"0") && !texts.contains(&"never"), "while typing: {texts:?}");
        assert_eq!(secs, 0);
        // Clicking away shows "never" again.
        let away = egui::pos2(500.0, 250.0);
        secs_frame(&ctx, &mut secs, vec![egui::Event::PointerMoved(away)]);
        secs_frame(&ctx, &mut secs, vec![egui::Event::PointerButton { pos: away, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
        secs_frame(&ctx, &mut secs, vec![egui::Event::PointerButton { pos: away, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        let after = secs_frame(&ctx, &mut secs, vec![]);
        assert!(after.iter().any(|t| t.0 == "never"), "after: {:?}", after.iter().map(|t| &t.0).collect::<Vec<_>>());
    }
}
