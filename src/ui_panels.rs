//! Chrome around the terminals: tabs, status bar, side panel, editor,
//! command palette, connect dialog, toasts and global shortcuts.

use crate::app::*;
use crate::find::{self, FindMode};
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

#[derive(Clone, Copy, PartialEq)]
enum Cmd {
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
}

const COMMANDS: &[(Cmd, &str, &str)] = &[
    (Cmd::NewTab, "New tab (same host)", "Ctrl+Shift+T"),
    (Cmd::Connect, "Connect to host…", "Ctrl+Shift+N"),
    (Cmd::SplitRight, "Split right", "Ctrl+Shift+D"),
    (Cmd::SplitDown, "Split down", "Ctrl+Shift+E"),
    (Cmd::ClosePane, "Close pane", "Ctrl+Shift+W"),
    (Cmd::Zoom, "Zoom / restore pane", "Ctrl+Shift+Z"),
    (Cmd::Composer, "Open composer (GUI command line)", "Ctrl+Space"),
    (Cmd::Find, "Find in scrollback", "Ctrl+Shift+F"),
    (Cmd::PrevBlock, "Jump to previous command", "Ctrl+Shift+Up"),
    (Cmd::NextBlock, "Jump to next command", "Ctrl+Shift+Down"),
    (Cmd::FoldAll, "Fold all command outputs", ""),
    (Cmd::UnfoldAll, "Unfold all command outputs", ""),
    (Cmd::ClearScrollback, "Clear scrollback", ""),
    (Cmd::Sidebar, "Toggle side panel", "Ctrl+Shift+B"),
    (Cmd::Editor, "Toggle editor", "Ctrl+Shift+J"),
    (Cmd::Hidden, "Toggle hidden files", ""),
    (Cmd::Reconnect, "Reconnect agent for this host", ""),
    (Cmd::FontUp, "Bigger font", "Ctrl+="),
    (Cmd::FontDown, "Smaller font", "Ctrl+-"),
    (Cmd::FontReset, "Default font size", "Ctrl+0"),
    (Cmd::UiUp, "Bigger UI (icons, tabs, panels)", "Ctrl+Shift+wheel"),
    (Cmd::UiDown, "Smaller UI (icons, tabs, panels)", "Ctrl+Shift+wheel"),
    (Cmd::UiReset, "UI scale 100%", ""),
    (Cmd::ThemePicker, "Theme…", ""),
    (Cmd::ThemeSystem, "Theme: follow system (dark/light)", ""),
    (Cmd::CursorStyle, "Terminal cursor: bar / block / underline", ""),
    (Cmd::MarkdownBreaks, "Markdown: toggle line breaks (GitHub PR style / CommonMark)", ""),
    (Cmd::Settings, "Open settings.json", ""),
    (Cmd::AgentLog, "Open agent log for this host", ""),
];

#[derive(Clone, Default)]
struct HlCache {
    key: u64,
    job: Option<Arc<LayoutJob>>,
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
            Cmd::CursorStyle => {
                self.settings.cursor_style = match self.settings.cursor_style {
                    CursorStyle::Bar => CursorStyle::Block,
                    CursorStyle::Block => CursorStyle::Underline,
                    CursorStyle::Underline => CursorStyle::Bar,
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
            Cmd::AgentLog => {
                let h = self.active_host();
                let p = self.data.path(&format!("logs/agent-{}.log", crate::host::safe_name(&h))).display().to_string().replace('\\', "/");
                self.open_file("local", &p, 1);
            }
        }
    }

    pub fn handle_global_keys(&mut self, ctx: &egui::Context) {
        let cs = Modifiers::CTRL | Modifiers::SHIFT;
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        let mut cmds = Vec::new();
        let table = [
            (Key::T, Cmd::NewTab),
            (Key::N, Cmd::Connect),
            (Key::W, Cmd::ClosePane),
            (Key::Z, Cmd::Zoom),
            (Key::D, Cmd::SplitRight),
            (Key::E, Cmd::SplitDown),
            (Key::F, Cmd::Find),
            (Key::B, Cmd::Sidebar),
            (Key::J, Cmd::Editor),
            (Key::ArrowUp, Cmd::PrevBlock),
            (Key::ArrowDown, Cmd::NextBlock),
        ];
        for (key, cmd) in table {
            if k(cs, key) {
                cmds.push(cmd);
            }
        }
        if k(cs, Key::P) {
            self.palette = if self.palette.is_some() { None } else { Some(PaletteState { query: String::new(), sel: 0 }) };
        }
        let mut tab_step = 0i32;
        if k(cs, Key::Tab) {
            tab_step = -1;
        } else if k(Modifiers::CTRL, Key::Tab) {
            tab_step = 1;
        }
        if k(Modifiers::CTRL, Key::Space) {
            cmds.push(Cmd::Composer);
        }
        if k(Modifiers::CTRL, Key::Equals) || k(Modifiers::CTRL, Key::Plus) {
            cmds.push(Cmd::FontUp);
        }
        if k(Modifiers::CTRL, Key::Minus) {
            cmds.push(Cmd::FontDown);
        }
        if k(Modifiers::CTRL, Key::Num0) {
            cmds.push(Cmd::FontReset);
        }
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
        let multi = self.pane_rects.len() > 1;
        let mut dir = None;
        if multi {
            for (key, d) in [(Key::ArrowLeft, (-1.0, 0.0)), (Key::ArrowRight, (1.0, 0.0)), (Key::ArrowUp, (0.0, -1.0)), (Key::ArrowDown, (0.0, 1.0))] {
                if k(Modifiers::ALT, key) {
                    dir = Some(d);
                }
            }
        }
        for c in cmds {
            self.run_cmd(c);
        }
        if tab_step != 0 && !self.tabs.is_empty() {
            let n = self.tabs.len() as i32;
            self.active_tab = (self.active_tab as i32 + tab_step).rem_euclid(n) as usize;
            self.focus_term = true;
            self.on_active_changed();
        }
        if let Some((dx, dy)) = dir {
            self.focus_dir(dx, dy);
        }
    }

    // ---- tab bar -----------------------------------------------------------------

    pub fn ui_tabbar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            self.dock_grip(ui, Region::Terms);
            let mut activate = None;
            let mut close = None;
            let n_tabs = self.tabs.len();
            for (ti, tab) in self.tabs.iter().enumerate() {
                let Some(p) = self.panes.get(&tab.active) else { continue };
                let title = p.title();
                let running = p.term.blocks.last().map(|b| b.state() == BlockState::Running).unwrap_or(false);
                let active = ti == self.active_tab;
                let font = FontId::proportional(12.5);
                let galley = ui.painter().layout_no_wrap(title.clone(), font, if active { theme::text() } else { theme::text_dim() });
                let w = (galley.size().x + 52.0).clamp(110.0, 260.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
                // contains_pointer, not hovered(): the close button drawn on hover
                // sits on top of the tab and would "steal" hovered() from it, hiding
                // itself again next frame -> endless flicker.
                let hovered = resp.contains_pointer();
                let bg = if active {
                    theme::bg_term()
                } else if hovered {
                    theme::bg_hover()
                } else {
                    theme::bg_base()
                };
                ui.painter().rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, bg);
                if active {
                    ui.painter().hline(rect.x_range().shrink(6.0), rect.top() + 1.0, Stroke::new(2.0_f32, host_color(&p.host)));
                }
                let dot = Rect2::dot(rect);
                if running {
                    let phase = (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
                    ui.painter().circle_filled(dot, 3.5, theme::tint(theme::accent(), (110.0 + 145.0 * phase) as u8));
                    ui.ctx().request_repaint_after(Duration::from_millis(60));
                } else {
                    ui.painter().circle_filled(dot, 3.5, host_color(&p.host));
                }
                let text_w = rect.width() - 52.0;
                let mut g = galley;
                if g.size().x > text_w {
                    let mut t: String = title.chars().take(((text_w / 7.0) as usize).max(4)).collect();
                    t.push('…');
                    g = ui.painter().layout_no_wrap(t, FontId::proportional(12.5), if active { theme::text() } else { theme::text_dim() });
                }
                ui.painter().galley(pos2(rect.left() + 22.0, rect.center().y - g.size().y / 2.0), g, theme::text());
                let cr = egui::Rect::from_center_size(pos2(rect.right() - 15.0, rect.center().y), vec2(18.0, 18.0));
                if hovered || active {
                    let cresp = ui.interact(cr, Id::new(("tabclose", ti)), Sense::click());
                    if cresp.hovered() {
                        ui.painter().rect_filled(cr, CornerRadius::same(4), theme::bg_hover());
                    }
                    icons::paint(ui.painter(), cr.shrink(4.5), Icon::Cross, if cresp.hovered() { theme::text() } else { theme::text_faint() });
                    if cresp.clicked() {
                        close = Some((ti, CloseWhich::This));
                    }
                }
                if resp.clicked() {
                    activate = Some(ti);
                }
                if resp.middle_clicked() {
                    close = Some((ti, CloseWhich::This));
                }
                resp.context_menu(|ui| {
                    if let Some(w) = close_menu(ui, ti, n_tabs) {
                        close = Some((ti, w));
                    }
                });
            }
            ui.add_space(4.0);
            if icons::button(ui, Icon::Plus, "new tab on the same host (Ctrl+Shift+T)").clicked() {
                self.run_cmd(Cmd::NewTab);
            }
            if icons::button(ui, Icon::Plug, "connect to a host (Ctrl+Shift+N)").clicked() {
                self.run_cmd(Cmd::Connect);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
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
            });
            if let Some(ti) = activate {
                self.active_tab = ti;
                self.focus_term = true;
                self.on_active_changed();
            }
            if let Some((ti, which)) = close {
                self.close_tabs(ti, which);
            }
        });
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
            for (tab, label) in [(SideTab::Files, "Files"), (SideTab::Blocks, "Commands"), (SideTab::Search, "Search"), (SideTab::Procs, "Procs")] {
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
        let r = ui.add(TextEdit::singleline(&mut self.side.grep_q).hint_text("text to find (Enter)").desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            self.actions.push(Action::Grep);
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.side.grep_icase, RichText::new("ignore case").size(12.0));
            ui.checkbox(&mut self.side.grep_regex, RichText::new("regex").size(12.0)).on_hover_text("remote only (grep -E style basic regex)");
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
        let base = res["root"].as_str().unwrap_or("").replace('\\', "/");
        ui.label(RichText::new(format!("{} matches{}", hits.len(), if res["truncated"].as_bool().unwrap_or(false) { " (truncated)" } else { "" })).size(11.5).color(theme::text_dim()));
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut last_file = String::new();
            for h in hits {
                let path = h[0].as_str().unwrap_or("").replace('\\', "/");
                let line = h[1].as_u64().unwrap_or(1) as usize;
                let text = h[2].as_str().unwrap_or("").trim();
                if path != last_file {
                    ui.add_space(4.0);
                    let rel = path.strip_prefix(&base).unwrap_or(&path).trim_start_matches('/');
                    ui.label(RichText::new(rel).size(12.0).color(theme::ansi(12)));
                    last_file = path.clone();
                }
                let r = ui.add(egui::Label::new(RichText::new(format!("{line:>5}  {}", text.chars().take(120).collect::<String>())).monospace().size(12.0).color(theme::text_dim())).sense(Sense::click()));
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if r.clicked() {
                    self.actions.push(Action::OpenFile(host.clone(), path.clone(), line));
                }
            }
        });
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
            });
            return;
        }
        self.editor.active = self.editor.active.min(self.editor.files.len() - 1);
        // Tabs.
        let mut close = None;
        ui.horizontal(|ui| {
        self.dock_grip(ui, Region::Editor);
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
                    let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
                    let sel = i == self.editor.active;
                    let label = RichText::new(if f.dirty { format!("● {name}") } else { name }).size(12.5).color(if sel { theme::text() } else { theme::text_dim() });
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
        {
            let f = &mut self.editor.files[idx];
            ui.horizontal(|ui| {
                ui.painter().circle_filled(ui.cursor().min + vec2(5.0, 11.0), 3.5, host_color(&f.host));
                ui.add_space(12.0);
                ui.label(RichText::new(format!("{}  {}", f.host, f.path)).size(11.5).color(theme::text_faint()));
            });
            ui.horizontal(|ui| {
                let can_save = f.dirty && !f.scratch && !f.binary && !f.loading;
                if ui.add_enabled(can_save, egui::Button::new(RichText::new("Save").size(12.5))).on_hover_text("Ctrl+S — writes through the agent").clicked() {
                    save = true;
                }
                if !f.scratch && ui.add_enabled(!f.loading, egui::Button::new(RichText::new("Reload").size(12.5))).clicked() {
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
                } else if !crate::media::is_image(&f.path) && !crate::media::is_pdf(&f.path) {
                    ui.add(TextEdit::singleline(&mut f.find).hint_text("highlight…").desired_width(140.0));
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
        ui.separator();
        let edit_id = Id::new(("editor", idx, self.editor.files[idx].path.clone()));
        if ui.memory(|m| m.has_focus(edit_id)) && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S)) {
            save = true;
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
        let find = f.find.clone();
        let readonly = f.binary || f.loading;
        let mut changed = false;
        sa.show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let nums: String = (1..=lines).map(|n| format!("{n:>4}\n")).collect();
                ui.add(egui::Label::new(RichText::new(nums).font(font.clone()).color(theme::text_faint())).selectable(false));
                let cache_id = edit_id.with("hl");
                let mut layouter = |ui: &Ui, text: &str, _wrap: f32| {
                    let key = hash_str(text, &format!("{ext}|{find}|{}|{}", font.size, theme::is_light()));
                    let cached: HlCache = ui.ctx().memory_mut(|m| m.data.get_temp_mut_or_default::<HlCache>(cache_id).clone());
                    let job = match cached.job {
                        Some(j) if cached.key == key => j,
                        _ => {
                            let j = Arc::new(highlight::layout(text, &ext, font.clone(), &find));
                            ui.ctx().memory_mut(|m| m.data.insert_temp(cache_id, HlCache { key, job: Some(j.clone()) }));
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
            .map(|(c, l, k)| (*c, l.to_string(), k.to_string()))
            .chain(theme_items)
            .filter(|(_, l, _)| q.is_empty() || l.to_lowercase().contains(&q))
            .collect();
        let mut run = None;
        let mut keep = true;
        let area = egui::Area::new(Id::new("palette")).anchor(Align2::CENTER_TOP, vec2(0.0, 70.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(theme::bg_raised()).corner_radius(CornerRadius::same(8)).inner_margin(8.0).show(ui, |ui| {
                ui.set_width(460.0);
                let id = Id::new("palette-q");
                if take_key(ui, Key::ArrowDown, |_| true) {
                    st.sel = (st.sel + 1).min(items.len().saturating_sub(1));
                }
                if take_key(ui, Key::ArrowUp, |_| true) {
                    st.sel = st.sel.saturating_sub(1);
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
                for (i, (cmd, label, keys)) in items.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
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
    let style_key = format!("{cursor_line:?}|{size}|{breaks}|{renderer:?}|{}|{}", theme::active(), crate::mermaid::generation());
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
                    let l = md::layout(text, cursor_line, size, breaks, &mut |src: &str| diagram_size(ui.ctx(), src, wrap).map(|(_, s)| s.y), &mut measure);
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
