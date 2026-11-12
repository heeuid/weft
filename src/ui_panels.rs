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
use crate::widgets::human_size;
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
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    CopyMode,
}

/// Every command: (command, id for settings "keys", label, default keys).
const COMMANDS: &[(Cmd, &str, &str, &[&str])] = &[
    (Cmd::NewTab, "new_tab", "New tab (same host)", &["Ctrl+Shift+T"]),
    (Cmd::Connect, "connect", "Connect to host…", &["Ctrl+Shift+N"]),
    (Cmd::SplitRight, "split_right", "Split right", &["Ctrl+Shift+D"]),
    (Cmd::SplitDown, "split_down", "Split down", &["Ctrl+Shift+E"]),
    (Cmd::ClosePane, "close_pane", "Close pane", &["Ctrl+Shift+W"]),
    (Cmd::Zoom, "zoom", "Zoom / restore pane", &["Ctrl+Shift+Z"]),
    (Cmd::NextTab, "next_tab", "Next tab", &["Ctrl+Tab"]),
    (Cmd::PrevTab, "prev_tab", "Previous tab", &["Ctrl+Shift+Tab"]),
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
    (Cmd::ThemeSystem, "theme_system", "Theme: follow system (dark/light)", &[]),
    (Cmd::CursorStyle, "cursor_style", "Terminal cursor: block / bar / underline", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Auto), "shell_auto", "Local shell: auto (pwsh, else Windows PowerShell)", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Pwsh), "shell_pwsh", "Local shell: PowerShell 7 (pwsh)", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Powershell), "shell_powershell", "Local shell: Windows PowerShell 5.1", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Cmd), "shell_cmd", "Local shell: cmd", &[]),
    (Cmd::LocalShell(crate::host::LocalShell::Nu), "shell_nu", "Local shell: nushell", &[]),
    (Cmd::MarkdownBreaks, "markdown_breaks", "Markdown: toggle line breaks (GitHub PR style / CommonMark)", &[]),
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
                    self.close_pane(id)
                }
            }
            Cmd::Zoom => self.toggle_zoom(),
            Cmd::Sidebar => self.settings.sidebar = !self.settings.sidebar,
            Cmd::Editor => self.editor.visible = !self.editor.visible,
            Cmd::Find => {
                if let Some(p) = pane.and_then(|id| self.panes.get_mut(&id)) {
                    p.search = match p.search.take() {
                        Some(_) => None,
                        None => Some(SearchState { query: String::new(), icase: true, hits: Vec::new(), cur: 0, seen_generation: u64::MAX, focus: true }),
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
            Cmd::Palette => self.palette = if self.palette.is_some() { None } else { Some(PaletteState { query: String::new(), sel: 0 }) },
            Cmd::Keys => self.keys_window = Some(KeysWindow { query: String::new(), capture: None }),
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
            Cmd::FocusLeft => self.focus_dir(-1.0, 0.0),
            Cmd::FocusRight => self.focus_dir(1.0, 0.0),
            Cmd::FocusUp => self.focus_dir(0.0, -1.0),
            Cmd::FocusDown => self.focus_dir(0.0, 1.0),
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
        let usable = |c: Cmd| multi || !matches!(c, Cmd::FocusLeft | Cmd::FocusRight | Cmd::FocusUp | Cmd::FocusDown);
        // A bound key runs its command and never reaches the terminal.
        let keymap = &self.keymap;
        let mut cmds: Vec<Cmd> = ctx.input_mut(|i| {
            let mut out = Vec::new();
            i.events.retain(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => match crate::keys::lookup(keymap, *key, *modifiers, usable) {
                    Some(c) => {
                        out.push(c);
                        false
                    }
                    None => true,
                },
                _ => true,
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
    fn keys_label(&self, c: Cmd) -> String {
        let keys: Vec<String> = self.keymap.iter().find(|k| k.0 == c).map(|k| k.1.iter().map(|s| s.label()).collect()).unwrap_or_default();
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
            let got = i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
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
            self.dock_grip(ui, Region::Terms);
            // The buttons on the right get their room first; the tabs share
            // what is left (they used to run on under the buttons).
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.tabbar_buttons(ui);
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
        let room = (ui.available_width() - 4.0 - 2.0 * (22.0 + 2.0) - 2.0 - 12.0).max(80.0);
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
            self.close_tabs(ti, which);
        }
    }

    /// The buttons at the right of the tab bar, right to left.
    fn tabbar_buttons(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(6.0);
        self.commands_menu(ui);
        if ui.add(egui::Button::new(RichText::new("?").size(13.0).color(theme::text_dim())).frame(false)).on_hover_text("help (F1)").clicked() {
            self.help = if self.help.is_some() { None } else { Some(HelpState { features: false, query: String::new() }) };
        }
        let fr = ui.add(egui::Button::new(RichText::new("Aa").size(12.5).color(theme::text_dim())).frame(false)).on_hover_text("fonts");
        if fr.clicked() {
            if self.font_menu.is_some() {
                self.font_menu = None;
            } else {
                self.open_font_menu();
            }
        }
        let ti = if theme::is_light() { Icon::Sun } else { Icon::Moon };
        let tr = icons::button(ui, ti, "theme");
        if tr.clicked() {
            self.theme_menu = if self.theme_menu.is_some() { None } else { Some(tr.rect) };
        }
        let ec = if self.editor.visible { theme::accent() } else { theme::text_dim() };
        if icons::button_colored(ui, Icon::Editor, "editor (Ctrl+Shift+J)", ec).clicked() {
            self.editor.visible = !self.editor.visible;
        }
        let sc = if self.settings.sidebar { theme::accent() } else { theme::text_dim() };
        if icons::button_colored(ui, Icon::Sidebar, "side panel (Ctrl+Shift+B)", sc).clicked() {
            self.settings.sidebar = !self.settings.sidebar;
        }
        if icons::button(ui, Icon::SplitDown, "split down (Ctrl+Shift+E)").clicked() {
            self.split(false);
        }
        if icons::button(ui, Icon::SplitRight, "split right (Ctrl+Shift+D)").clicked() {
            self.split(true);
        }
        if icons::button(ui, Icon::Search, "command palette (Ctrl+Shift+P)").clicked() {
            self.palette = Some(PaletteState { query: String::new(), sel: 0 });
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
            let (dot, label) = match &status {
                ConnStatus::Ready => (theme::success(), if host == "local" { "native fs".to_string() } else { "agent ready".to_string() }),
                ConnStatus::Connecting => (theme::warn(), "agent connecting…".to_string()),
                ConnStatus::Failed(e) => (theme::error(), format!("agent offline: {}", e.chars().take(80).collect::<String>())),
            };
            ui.painter().circle_filled(rect.center(), 4.0, dot);
            ui.label(small(host.clone(), theme::text()));
            let r = ui.label(small(label, theme::text_dim()));
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
                ui.label(small(cwd, theme::text_faint()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Sizes, always at hand: clicking doesn't close anything; the value resets.
                let mut step: Option<Cmd> = None;
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
                ui.label(small("Ctrl+Space composer · Ctrl+Shift+P commands".into(), theme::text_faint()));
                if let Some(p) = self.active_pane() {
                    ui.label(small(format!("{}×{}", p.term.cols, p.term.rows), theme::text_faint()));
                }
                if let Some(s) = self.sys.get(&host) {
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
            });
        });
    }

    // ---- side panel -------------------------------------------------------------------

    pub fn ui_sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            self.dock_grip(ui, Region::Side);
            for (tab, label) in [(SideTab::Files, "Files"), (SideTab::Blocks, "Commands"), (SideTab::History, "History"), (SideTab::Search, "Search"), (SideTab::Procs, "Procs")] {
                let sel = self.side.tab == tab;
                let txt = RichText::new(label).size(12.5).color(if sel { theme::text() } else { theme::text_dim() });
                if ui.selectable_label(sel, txt).clicked() {
                    self.side.tab = tab;
                }
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
            ui.label(RichText::new(&host).size(12.0).color(theme::text()));
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
                let r = ui.add(egui::Label::new(RichText::new(label).size(11.5).color(if last { theme::text() } else { theme::text_dim() })).sense(Sense::click()));
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
            for r in &rows {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
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
                p.text(pos2(x + 34.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), name_col);
                if !r.dir {
                    p.text(pos2(rect.right() - 6.0, rect.center().y), Align2::RIGHT_CENTER, human_size(r.size), FontId::proportional(11.0), theme::text_faint());
                }
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
        let cached: Option<(u64, Arc<Result<Vec<usize>, String>>)> = ui.data(|d| d.get_temp(cache_id));
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
                let cmd: String = if r.cmd.is_empty() { "(output)".into() } else { r.cmd.chars().take(80).collect() };
                pt.text(rect.min + vec2(26.0, 5.0), Align2::LEFT_TOP, cmd, FontId::monospace(12.5), theme::text());
                let exit = match r.exit {
                    Some(c) if c != 0 => format!(" · exit {c}"),
                    _ => String::new(),
                };
                let meta = format!("{} · {} · {}{exit} · {}", local_time(r.t), r.host, fmt_dur(Duration::from_secs_f64(r.dur)), r.cwd);
                pt.text(rect.min + vec2(26.0, 23.0), Align2::LEFT_TOP, meta, FontId::proportional(11.0), theme::text_faint());
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
        egui::Window::new("Help").open(&mut open).collapsible(false).resizable(true).default_size(vec2(600.0, 620.0)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(!h.features, "Commands and keys").clicked() {
                    h.features = false;
                }
                if ui.selectable_label(h.features, "Features").clicked() {
                    h.features = true;
                }
                ui.add(TextEdit::singleline(&mut h.query).hint_text("search help…").desired_width(f32::INFINITY));
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
                    egui::Grid::new("help-keys").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                        for (k, what) in keys {
                            ui.label(RichText::new(*k).monospace().color(theme::accent()));
                            ui.label(*what);
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
        egui::Window::new("Keyboard shortcuts").open(&mut open).collapsible(false).resizable(true).default_size(vec2(640.0, 620.0)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut w.query).hint_text("search commands or keys…").desired_width(ui.available_width() - 90.0));
                if ui.add_enabled(!self.settings.keys.is_empty(), egui::Button::new("Reset all")).on_hover_text("every command back to its default keys").clicked() {
                    reset_all = true;
                }
            });
            ui.label(
                RichText::new("+ then press the keys (Esc cancels) · click a key to remove it · kept in settings.json under \"keys\" · bound keys never reach the terminal")
                    .size(11.0)
                    .color(theme::text_faint()),
            );
            ui.separator();
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for g in GROUPS {
                    let rows: Vec<_> = COMMANDS
                        .iter()
                        .filter(|c| group(c.0) == g)
                        .filter(|c| q.is_empty() || c.2.to_lowercase().contains(&q) || self.keys_label(c.0).to_lowercase().contains(&q))
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new(g).strong().color(theme::text_dim()));
                    egui::Grid::new(("keys-grid", g)).num_columns(3).striped(true).spacing([12.0, 4.0]).min_col_width(60.0).show(ui, |ui| {
                        for (cmd, id, label, defaults) in rows {
                            ui.label(*label);
                            ui.horizontal_wrapped(|ui| {
                                let keys = self.keymap.iter().find(|k| k.0 == *cmd).map(|k| k.1.clone()).unwrap_or_default();
                                for sc in &keys {
                                    let l = sc.label();
                                    let others: Vec<&str> = uses.get(&l).map(|v| v.iter().copied().filter(|x| x != label).collect()).unwrap_or_default();
                                    let color = if others.is_empty() { theme::text() } else { theme::error() };
                                    let tip = if others.is_empty() { "click to remove".to_string() } else { format!("also bound to: {} · click to remove", others.join(", ")) };
                                    if ui.add(egui::Button::new(RichText::new(format!("{l}  ×")).monospace().size(12.0).color(color))).on_hover_text(tip).clicked() {
                                        let rest: Vec<String> = keys.iter().map(|k| k.label()).filter(|k| *k != l).collect();
                                        change = Some((id, Some(rest)));
                                    }
                                }
                                if w.capture == Some(*id) {
                                    ui.label(RichText::new("press keys… (Esc: cancel)").color(theme::accent()));
                                } else if ui.small_button("+").on_hover_text("add a key: click, then press it").clicked() {
                                    w.capture = Some(id);
                                }
                            });
                            let custom = self.settings.keys.contains_key(*id);
                            let def = if defaults.is_empty() { "none".to_string() } else { defaults.join(" / ") };
                            if ui.add_enabled(custom, egui::Button::new("Reset").small()).on_hover_text(format!("default: {def}")).clicked() {
                                change = Some((id, None));
                            }
                            ui.end_row();
                        }
                    });
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
            ui.painter().text(rect.min + vec2(8.0, 4.0), Align2::LEFT_TOP, &m.title, FontId::proportional(13.0), theme::text());
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
        egui::Window::new("Memos").open(&mut open).collapsible(false).resizable(true).default_size(vec2(380.0, 460.0)).show(ctx, |ui| {
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
        egui::Window::new("Privacy").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
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
                    let cmd: String = b.command.chars().take(60).collect();
                    pt.text(rect.min + vec2(26.0, 5.0), Align2::LEFT_TOP, cmd, FontId::monospace(12.5), theme::text());
                    let lines = match (b.out_line, b.end_line) {
                        (Some(o), Some(e)) => format!("{} lines", e.saturating_sub(o)),
                        _ => "running".into(),
                    };
                    let meta = match b.exit {
                        Some(c) if c != 0 => format!("exit {c} · {} · {lines}", b.elapsed().map(fmt_dur).unwrap_or_default()),
                        _ => format!("{} · {lines}", b.elapsed().map(fmt_dur).unwrap_or_default()),
                    };
                    pt.text(rect.min + vec2(26.0, 23.0), Align2::LEFT_TOP, meta, FontId::proportional(11.0), theme::text_faint());
                    if resp.clicked() {
                        scroll_to = Some(b.prompt_line);
                    }
                    let (cmd, out) = (b.command.clone(), p.term.block_output(b));
                    resp.context_menu(|ui| {
                        if ui.button("Copy command").clicked() {
                            self.actions.push(Action::Copy(cmd.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Copy output").clicked() {
                            self.actions.push(Action::Copy(out.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Run again").clicked() {
                            self.actions.push(Action::Run(id, cmd.clone()));
                            ui.close_menu();
                        }
                        if ui.button("Open output in editor").clicked() {
                            self.actions.push(Action::OpenScratch(format!("output: {cmd}"), out.clone()));
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
        ui.label(RichText::new(format!("in {host}:{root}")).size(11.5).color(theme::text_faint()));
        let r = ui.add(TextEdit::singleline(&mut self.side.grep_q).hint_text(format!("{} (Enter)", self.side.grep_mode.hint())).desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            self.actions.push(Action::Grep);
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("grep-mode").width(62.0).selected_text(RichText::new(self.side.grep_mode.label()).size(12.0)).show_ui(ui, |ui| {
                for m in editfind::MatchMode::ALL {
                    ui.selectable_value(&mut self.side.grep_mode, m, m.label()).on_hover_text(m.hint());
                }
            });
            ui.checkbox(&mut self.side.grep_icase, RichText::new("ignore case").size(12.0));
            ui.checkbox(&mut self.side.grep_hidden, RichText::new("hidden").size(12.0)).on_hover_text("also in hidden files and folders (names starting with .)");
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
                    let r = ui.add(egui::Label::new(RichText::new(rel).size(12.0).color(theme::ansi(12))).sense(Sense::click()));
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
        if self.profile(&host).is_local() {
            ui.label(RichText::new("The process list comes from the remote agent (POSIX `ps`). Connect to an ssh host to use it.").color(theme::text_faint()));
            return;
        }
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
                    for k in range {
                        let hit = pos.contains(&k);
                        let fmt = egui::TextFormat { font_id: FontId::proportional(size), color: if hit { theme::accent() } else { base }, underline: if hit { Stroke::new(1.0_f32, theme::accent()) } else { Stroke::NONE }, ..Default::default() };
                        job.append(&chars[k].to_string(), 0.0, fmt);
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

    /// Drag handle that moves a panel to another window edge (drop zones are
    /// drawn by `ui_dock_drop`); right-click for the same choices as a menu.
    fn dock_grip(&mut self, ui: &mut Ui, region: Region) {
        let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::click_and_drag());
        let active = resp.hovered() || resp.dragged() || self.dock_drag == Some(region);
        let c = if active { theme::text() } else { theme::text_faint() };
        for row in 0..3 {
            for col in 0..2 {
                let p = pos2(rect.center().x - 2.5 + col as f32 * 5.0, rect.center().y - 5.0 + row as f32 * 5.0);
                ui.painter().circle_filled(p, 1.3, c);
            }
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text("drag onto another panel or a window edge to move this panel (right-click: menu)");
        if resp.drag_started() {
            self.dock_drag = Some(region);
        }
        resp.context_menu(|ui| {
            for d in Dock::ALL {
                if ui.button(format!("Move to the {} edge", d.label())).clicked() {
                    self.move_panel(region, None, d);
                    ui.close_menu();
                }
            }
        });
    }

    /// Move a panel next to another one (or to a window edge) and remember it.
    pub fn move_panel(&mut self, region: Region, target: Option<Region>, side: Dock) {
        let layout = std::mem::take(&mut self.settings.layout);
        self.settings.layout = layout.moved(region, target, side);
        self.save_settings();
    }

    /// The panels where the layout tree puts them, with draggable dividers.
    /// Hidden panels (side panel, editor) keep their place in the tree.
    pub fn ui_layout(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        self.layout_rect = rect;
        let (side, editor) = (self.settings.sidebar, self.editor.visible);
        let shown = |r: Region| match r {
            Region::Side => side,
            Region::Editor => editor,
            Region::Terms => true,
        };
        if !self.settings.layout.contains(Region::Terms) {
            self.settings.layout = crate::app::Layout::default();
        }
        let Some(tree) = self.settings.layout.pruned(&shown) else { return };
        let mut panels = Vec::new();
        let mut dividers = Vec::new();
        tree.rects(rect, &mut panels, &mut dividers);
        self.panel_rects = panels.iter().copied().collect();
        for (region, r) in panels {
            ui.scope_builder(egui::UiBuilder::new().max_rect(r).id_salt(("panel", region)), |ui| {
                ui.set_clip_rect(r.intersect(ui.clip_rect()));
                match region {
                    Region::Side | Region::Editor => {
                        let margin = if region == Region::Side { egui::Margin::symmetric(8, 0) } else { egui::Margin::symmetric(8, 6) };
                        egui::Frame::new().fill(theme::bg_base()).inner_margin(margin).show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            if region == Region::Side {
                                self.ui_sidebar(ui);
                            } else {
                                self.ui_editor(ui);
                            }
                        });
                    }
                    Region::Terms => {
                        // Terminal panel: its tab bar, then the active tab's panes.
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
        for (id, handle, vertical, parent) in dividers {
            let resp = ui.interact(handle.expand(2.0), Id::new(("panel-divider", id)), Sense::drag());
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
    }

    /// While a panel is dragged by its grip: highlight where it would go —
    /// a half of the panel under the pointer, or a window edge near the
    /// border — and move it there on release.
    pub fn ui_dock_drop(&mut self, ctx: &egui::Context) {
        let Some(region) = self.dock_drag else { return };
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
        let over = self.panel_rects.iter().find(|(r, rect)| **r != region && rect.contains(p)).map(|(r, rect)| (*r, *rect));
        let (target, side, zone, text) = if border < 36.0 || !area.contains(p) {
            let side = nearest(area);
            (None, side, half_of(area, side, 0.3), format!("{} \u{2192} {} edge of the window", region.name(), side.label()))
        } else if let Some((t, rect)) = over {
            let side = nearest(rect);
            (Some(t), side, half_of(rect, side, 0.5), format!("{} \u{2192} {} of {}", region.name(), side.label(), t.name().to_lowercase()))
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
            self.move_panel(region, target, side);
        }
    }

    pub fn ui_editor(&mut self, ui: &mut Ui) {
        if self.editor.files.is_empty() {
            ui.horizontal(|ui| self.dock_grip(ui, Region::Editor));
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
        self.dock_grip(ui, Region::Editor);
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
            self.close_files(i, which);
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
        let memo = self.is_memo(&self.editor.files[idx]);
        {
            let f = &mut self.editor.files[idx];
            ui.horizontal(|ui| {
                ui.painter().circle_filled(ui.cursor().min + vec2(5.0, 11.0), 3.5, host_color(&f.host));
                ui.add_space(12.0);
                ui.label(RichText::new(format!("{}  {}", f.host, f.path)).size(11.5).color(theme::text_faint()));
            });
            ui.horizontal(|ui| {
                let can_save = f.dirty && !f.scratch && !f.binary && !f.loading;
                if memo {
                    ui.label(RichText::new("memo · saved automatically").size(11.5).color(theme::text_faint()));
                    ui.menu_button(RichText::new("Delete…").size(12.5), |ui| {
                        if ui.button(RichText::new("Delete this memo").color(theme::error())).clicked() {
                            delete = true;
                            ui.close_menu();
                        }
                    });
                } else if ui.add_enabled(can_save, egui::Button::new(RichText::new("Save").size(12.5))).on_hover_text("Ctrl+S — writes through the agent").clicked() {
                    save = true;
                }
                if !f.scratch && !memo && ui.add_enabled(!f.loading, egui::Button::new(RichText::new("Reload").size(12.5))).clicked() {
                    reload = true;
                }
                if md::is_markdown(&f.path) {
                    if ui.selectable_label(f.rich, RichText::new("Rich").size(12.5)).on_hover_text("edit the rendered document").clicked() {
                        f.rich = true;
                    }
                    if ui.selectable_label(!f.rich, RichText::new("Source").size(12.5)).on_hover_text("plain markdown source").clicked() {
                        f.rich = false;
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
                if !f.binary && !crate::media::is_image(&f.path) && !crate::media::is_pdf(&f.path) {
                    if ui.button(RichText::new("Find").size(12.5)).on_hover_text("find (Ctrl+F) and replace (Ctrl+H)").clicked() {
                        open_find = true;
                    }
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
                let bar = self.editor.find.get_or_insert_with(Default::default);
                if let Some(s) = picked {
                    bar.query = s;
                    bar.cur = 0;
                }
                bar.replacing |= ctrl_h;
                bar.focus = true;
            }
        } else if open_find && text_file {
            let bar = self.editor.find.get_or_insert_with(Default::default);
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
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3)) {
                nav = 1;
            } else if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3)) {
                nav = -1;
            }
            ui.horizontal(|ui| {
                let q = ui.add(TextEdit::singleline(&mut bar.query).id(Id::new("editor-find-q")).hint_text("find…").desired_width(220.0));
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
                if q.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
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
                ui.horizontal(|ui| {
                    let hint = if bar.mode == editfind::MatchMode::Regex { "replace with… ($1, $2: groups)" } else { "replace with…" };
                    ui.add(TextEdit::singleline(&mut bar.replace).id(Id::new("editor-find-r")).hint_text(hint).desired_width(220.0));
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
                    ui.fonts(|f| f.layout_job((*job).clone()))
                };
                let resp = ui.add(
                    TextEdit::multiline(&mut f.text)
                        .id(edit_id)
                        .font(font.clone())
                        .code_editor()
                        .frame(false)
                        .interactive(!readonly)
                        .desired_width(f32::INFINITY)
                        .desired_rows(lines.max(30))
                        .lock_focus(true)
                        .layouter(&mut layouter),
                );
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
        let q = st.query.to_lowercase();
        let theme_items: Vec<(Cmd, String, String)> =
            theme::themes().map(|(i, _, name, light)| (Cmd::Theme(i), format!("Theme: {name}"), if light { "light".into() } else { "dark".into() })).collect();
        let items: Vec<(Cmd, String, String)> = COMMANDS
            .iter()
            .map(|(c, _, l, _)| (*c, l.to_string(), self.keys_label(*c)))
            .chain(theme_items)
            .filter(|(_, l, _)| q.is_empty() || l.to_lowercase().contains(&q))
            .collect();
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
                    r = Some(ui.add(TextEdit::singleline(&mut st.query).id(id).hint_text("type a command").desired_width(ui.available_width() - 28.0)));
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
                // As tall as the window allows below the palette; the rest scrolls.
                let room = (ctx.screen_rect().bottom() - ui.cursor().top() - 24.0).max(120.0);
                ScrollArea::vertical().max_height(room).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, (cmd, label, keys)) in items.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                        if i == st.sel && moved {
                            ui.scroll_to_rect(rect, None);
                        }
                        if i == st.sel || resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(5), if i == st.sel { theme::tint(theme::accent(), 45) } else { theme::bg_hover() });
                        }
                        ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), theme::text());
                        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, keys, FontId::proportional(11.5), theme::text_faint());
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
                    .map(|p| (p.name.clone(), match p.kind { HostKind::Local => "local shell", HostKind::Ssh { .. } => "ssh profile", HostKind::Custom { .. } => "custom transport" }))
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
                        ui.painter().text(pos2(rect.left() + 26.0, rect.center().y), Align2::LEFT_CENTER, &name, FontId::proportional(13.5), theme::text());
                        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, kind, FontId::proportional(11.5), theme::text_faint());
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
        egui::Window::new("Fonts").open(&mut open).collapsible(false).resizable(true).default_size(vec2(440.0, 520.0)).show(ctx, |ui| {
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
        | Cmd::SplitRight
        | Cmd::SplitDown
        | Cmd::ClosePane
        | Cmd::Zoom
        | Cmd::NextTab
        | Cmd::PrevTab
        | Cmd::FocusLeft
        | Cmd::FocusRight
        | Cmd::FocusUp
        | Cmd::FocusDown => GROUPS[0],
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
        Cmd::Sidebar | Cmd::Editor | Cmd::Hidden | Cmd::History | Cmd::NewMemo | Cmd::Memos | Cmd::OpenMemo => GROUPS[2],
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
        Cmd::Records | Cmd::Settings | Cmd::AgentLog | Cmd::Keys => GROUPS[4],
        Cmd::Help | Cmd::Features | Cmd::Palette => GROUPS[5],
    }
}

/// A time setting in seconds, edited as a number and a unit (seconds,
/// minutes, hours). 0 shows `zero`.
fn secs_input(ui: &mut Ui, id: &str, secs: &mut u64, zero: &str) -> bool {
    use crate::secret::{split_secs, UNITS};
    let unit_id = Id::new(("secs-unit", id));
    let (mut v, mut u) = split_secs(*secs);
    // Keep the unit last chosen while it still fits the value.
    if let Some(saved) = ui.data(|d| d.get_temp::<usize>(unit_id)) {
        if *secs % UNITS[saved].1 == 0 {
            (v, u) = (*secs / UNITS[saved].1, saved);
        }
    }
    let zero = zero.to_string();
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

fn markdown_body(ui: &mut Ui, f: &mut crate::app::EditorFile, edit_id: Id, size: f32, breaks: bool, renderer: &crate::mermaid::Renderer, readonly: bool, goto: Option<usize>, cache_dir: &std::path::Path) -> bool {
    let ctx = ui.ctx().clone();
    let focused = ui.memory(|m| m.has_focus(edit_id));
    // The cursor's line shows its markup (from last frame's cursor).
    let cursor_line = if focused {
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
            let key = hash_str(text, &style_key);
            let cached: MdCache = ui.ctx().memory_mut(|m| m.data.get_temp_mut_or_default::<MdCache>(cache_id).clone());
            let job = match cached.job {
                Some(j) if cached.key == key => j,
                _ => {
                    let mut measure = |t: &str, font: &FontId| ui.fonts(|f| f.layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE).size().x);
                    let mut l = md::layout(text, cursor_line, size, breaks, &mut |src: &str| diagram_size(ui.ctx(), src, wrap).map(|(_, s)| s.y), &mut measure);
                    crate::highlight::mark_job(&mut l.job, &marks, mark_cur);
                    let j = Arc::new(l.job);
                    ui.ctx().memory_mut(|m| m.data.insert_temp(cache_id, MdCache { key, job: Some(j.clone()), decos: Arc::new(l.decos) }));
                    j
                }
            };
            let mut job = (*job).clone();
            job.wrap.max_width = wrap;
            ui.fonts(|f| f.layout_job(job))
        };
        let out = TextEdit::multiline(&mut f.text)
            .id(edit_id)
            .frame(false)
            .margin(egui::Margin::ZERO)
            .desired_width(width)
            .desired_rows(30)
            .interactive(!readonly)
            .lock_focus(true)
            .layouter(&mut layouter)
            .show(ui);
        changed = out.response.changed();
        let galley = &out.galley;
        let origin = out.galley_pos;
        let at = |i: usize| galley.pos_from_ccursor(egui::text::CCursor::new(i)).translate(origin.to_vec2());
        let decos = ctx.memory_mut(|m| m.data.get_temp_mut_or_default::<MdCache>(cache_id).decos.clone());
        let right = origin.x + width;
        let mut back = Vec::new();
        let painter = ui.painter().clone();
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let mut toggle: Option<usize> = None;
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
                md::Deco::Bullet { at: i } => {
                    let r = at(i);
                    painter.circle_filled(pos2(r.left() + size * 0.35, r.center().y), size * 0.14, theme::text_dim());
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
                    if out.response.clicked() && pointer.map(|p| b.expand(2.0).contains(p)).unwrap_or(false) && !readonly {
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
                md::Deco::Table { ref rows, end, ref cols } => {
                    let (Some(&first), Some(&last_x)) = (rows.first(), cols.last()) else { continue };
                    let (top, bottom) = (at(first).top(), at(end).bottom());
                    let x = |c: f32| origin.x + c;
                    let stroke = Stroke::new(1.0_f32, theme::border());
                    if let Some(&body) = rows.get(1) {
                        let header = egui::Rect::from_min_max(pos2(x(0.0), top), pos2(x(last_x), at(body).top()));
                        back.push(egui::Shape::rect_filled(header, CornerRadius::ZERO, theme::tint(theme::text_faint(), 30)));
                    }
                    for &r in rows.iter() {
                        painter.hline(x(0.0)..=x(last_x), at(r).top(), stroke);
                    }
                    painter.hline(x(0.0)..=x(last_x), bottom, stroke);
                    for &c in cols.iter() {
                        painter.vline(x(c), top..=bottom, stroke);
                    }
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
}
