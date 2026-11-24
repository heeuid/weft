//! The Settings window: weft's settings with their names and a word on
//! each, changed and applied at once; the windows for the bigger ones
//! (theme, fonts, shells, privacy, keys) a button away; the hosts kept,
//! to forget; and settings.json itself under Advanced, applied on Apply.
//!
//! How settings.json and state.json are read and written: store.rs.

use crate::app::{App, CursorStyle, Settings, ToastKind};
use crate::host::{HostKind, LocalShell};
use crate::theme;
use egui::{vec2, RichText, ScrollArea, TextEdit};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Page {
    #[default]
    General,
    Hosts,
    Advanced,
}

#[derive(Default)]
pub struct SettingsWindow {
    pub page: Page,
    /// Advanced: settings.json as being edited, and as it was shown (an
    /// unedited text follows the settings as they change).
    pub text: String,
    pub shown: String,
    /// What Apply said: an error, or the notes (unknown names, read at start).
    pub error: Option<String>,
    pub notes: Vec<String>,
    /// A host's Forget pressed once: pressed again within 5 s, it goes.
    pub forget: Option<(String, Instant)>,
    /// Changed and not yet saved (saved once nothing is held).
    pub unsaved: bool,
}

use crate::store::{prefs_of, STATE_KEYS};

/// Settings as shown under Advanced: settings.json as weft has it.
pub fn editable_json(s: &Settings) -> String {
    serde_json::to_string_pretty(&Value::Object(prefs_of(s))).unwrap_or_default()
}

/// A settings text as weft reads it, per name (numbers as weft holds
/// them: a hand-written 1.1 and weft's 1.1 as f32 compare equal), with the
/// names weft does not know as they are. None: not valid.
fn as_weft_reads(text: &str) -> Option<serde_json::Map<String, Value>> {
    let raw: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let mut out = raw.as_object()?.clone();
    if let Ok(read) = serde_json::from_value::<Settings>(raw) {
        if let Ok(Value::Object(m)) = serde_json::to_value(read) {
            for (k, v) in m {
                if out.contains_key(&k) {
                    out.insert(k, v);
                }
            }
        }
    }
    Some(out)
}

/// `theirs` with what `mine` changed from `base` over it, name by name
/// (a name `mine` dropped is dropped); the session's names left to
/// `theirs`. No `base` to tell by: `theirs` as it is.
fn merge_changed(base: Option<&serde_json::Map<String, Value>>, mine: &serde_json::Map<String, Value>, theirs: &mut serde_json::Map<String, Value>) {
    let Some(base) = base else { return };
    let keys: Vec<String> = crate::store::differing(mine, Some(base)).into_iter().filter(|k| !STATE_KEYS.contains(&k.as_str())).collect();
    // (The hosts one by one, as weft saves them.)
    crate::store::put(theirs, mine, Some(base), &keys);
}

/// What a host is, in a few words.
fn host_kind(k: &HostKind) -> String {
    match k {
        HostKind::Local => "this PC".into(),
        HostKind::Ssh { target, .. } => format!("ssh {target}"),
        HostKind::Custom { pty, .. } => format!("custom: {}", pty.first().cloned().unwrap_or_default()),
        HostKind::Serial { via, port, baud } => {
            if via == "local" {
                format!("serial console {port} at {baud} on this PC")
            } else {
                format!("serial console {port} at {baud} on {via}")
            }
        }
    }
}

impl App {
    pub fn open_settings_window(&mut self, page: Page) {
        let text = self.advanced_text();
        self.settings_window = Some(SettingsWindow { page, shown: text.clone(), text, ..Default::default() });
    }

    /// What Advanced shows: settings.json without the session; while the
    /// file is not valid, the file as it is (to be fixed, session and all).
    fn advanced_text(&self) -> String {
        self.settings_bad.clone().unwrap_or_else(|| editable_json(&self.settings))
    }

    /// Settings from settings.json's text: what it names replaces what is
    /// set, what it leaves out is the default; weft's own part (state.json)
    /// stays as it is. Applied now; Ok with notes (names weft does not
    /// know, settings read only at the start).
    pub fn apply_settings_text(&mut self, text: &str) -> Result<Vec<String>, String> {
        let mut v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not valid JSON: {e}"))?;
        let Some(o) = v.as_object_mut() else { return Err("settings.json is one object: { … }".into()) };
        let mut notes = Vec::new();
        for k in STATE_KEYS {
            o.remove(k);
        }
        let given: Vec<String> = o.keys().cloned().collect();
        // weft's own part, as it is.
        if let Ok(Value::Object(now)) = serde_json::to_value(&self.settings) {
            for k in STATE_KEYS {
                if let Some(x) = now.get(k) {
                    o.insert(k.to_string(), x.clone());
                }
            }
        }
        let new: Settings = serde_json::from_value(v).map_err(|e| format!("a value is not of its kind: {e}"))?;
        // Names weft does not know: said, not silently dropped (a typo).
        let known = serde_json::to_value(&new).ok();
        for k in &given {
            if known.as_ref().is_none_or(|kv| kv.get(k).is_none()) {
                notes.push(format!("\"{k}\" is not a setting of weft's: left out"));
            }
        }
        if new.console_host != self.settings.console_host || new.renderer != self.settings.renderer {
            notes.push("the console host and the graphics backend are read as weft starts: from the next start".into());
        }
        let old = std::mem::replace(&mut self.settings, new);
        self.settings_changed(&old);
        Ok(notes)
    }

    /// What a change of settings needs besides the value: fonts loaded,
    /// keys bound, the password vault told, Files told whether to follow.
    pub fn settings_changed(&mut self, old: &Settings) {
        let s = &self.settings;
        let fonts = s.term_font != old.term_font || s.icon_font != old.icon_font;
        let listed = (s.show_hidden, s.show_ignored) != (old.show_hidden, old.show_ignored);
        let keys = s.keys != old.keys;
        let vault = serde_json::to_value(s.passwords).ok() != serde_json::to_value(old.passwords).ok();
        let follow = (s.follow_cwd != old.follow_cwd).then_some(s.follow_cwd);
        crate::records::set_switches(&self.settings.records);
        crate::host::set_pythons(&self.settings.remote_python);
        if fonts {
            self.apply_fonts();
        }
        if listed {
            self.dir_cache.clear();
        }
        if keys {
            self.rebuild_keys();
        }
        if vault {
            self.secrets.configure(self.settings.passwords);
        }
        if let Some(f) = follow {
            self.side.follow = f;
            for v in self.side_views.values_mut() {
                v.follow = f;
            }
        }
    }

    pub fn ui_settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.settings_window.take() else { return };
        // An unedited Advanced text follows the settings as they change.
        let now = self.advanced_text();
        if w.text == w.shown && now != w.shown {
            w.text = now.clone();
            w.shown = now;
        }
        let mut open = true;
        let before = self.settings.clone();
        crate::widgets::dialog(ctx, "Settings", vec2(620.0, 560.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (page, label) in [(Page::General, "General"), (Page::Hosts, "Hosts"), (Page::Advanced, "Advanced (settings.json)")] {
                    if ui.selectable_label(w.page == page, label).clicked() {
                        w.page = page;
                    }
                }
            });
            ui.separator();
            // settings.json not valid: said, with the way out.
            if self.settings_bad.is_some() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("settings.json has an error: weft goes on with the settings it has and saves nothing to the file. Once it is fixed (under Advanced, or in any editor), the file's settings are used: what is changed here meanwhile is not kept. Or").color(theme::warn()));
                    if ui.button("write weft's settings over it").clicked() {
                        self.settings_take_over();
                    }
                });
                ui.separator();
            }
            match w.page {
                Page::General => self.settings_general(ui),
                Page::Hosts => self.settings_hosts(ui, &mut w),
                Page::Advanced => self.settings_advanced(ui, &mut w),
            }
        });
        // Changed here: taken up at once, saved when let go (a slider held
        // changes every frame).
        if editable_json(&self.settings) != editable_json(&before) {
            self.settings_changed(&before);
            w.unsaved = true;
        }
        if w.unsaved && !ctx.input(|i| i.pointer.primary_down()) {
            w.unsaved = false;
            self.save_settings();
        }
        if open {
            self.settings_window = Some(w);
        } else if w.unsaved {
            self.save_settings();
        }
    }

    fn settings_general(&mut self, ui: &mut egui::Ui) {
        let note = |ui: &mut egui::Ui, s: &str| {
            ui.label(RichText::new(s).size(11.5).color(theme::text_faint()));
        };
        let head = |ui: &mut egui::Ui, s: &str| {
            ui.add_space(6.0);
            ui.label(RichText::new(s).strong());
        };
        let mut open_cmd: Option<crate::ui_panels::Cmd> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let s = &mut self.settings;
            head(ui, "Look");
            egui::Grid::new("settings-look").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label("Theme");
                ui.horizontal(|ui| {
                    let name = theme::themes().find(|t| t.1 == s.theme).map(|t| t.2.to_string()).unwrap_or_else(|| if s.theme == "system" { "as the system (light / dark)".into() } else { s.theme.clone() });
                    ui.label(name);
                    if ui.button("Themes…").clicked() {
                        open_cmd = Some(crate::ui_panels::Cmd::ThemePicker);
                    }
                });
                ui.end_row();
                ui.label("Font");
                ui.horizontal(|ui| {
                    ui.label(s.term_font.trim_start_matches("builtin:"));
                    if ui.button("Fonts…").clicked() {
                        open_cmd = Some(crate::ui_panels::Cmd::Fonts);
                    }
                });
                ui.end_row();
                ui.label("Text size");
                ui.add(egui::Slider::new(&mut s.font_size, 8.0..=32.0).step_by(1.0)).on_hover_text("terminal and editor (Ctrl+wheel, Ctrl+= / Ctrl+-)");
                ui.end_row();
                ui.label("Window scale");
                ui.add(egui::Slider::new(&mut s.ui_scale, 0.6..=2.5).step_by(0.1).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).on_hover_text("icons, tabs, panels and text (Ctrl+Shift+wheel)");
                ui.end_row();
                ui.label("Cursor");
                ui.horizontal(|ui| {
                    for (c, label) in [(CursorStyle::Block, "block"), (CursorStyle::Bar, "bar"), (CursorStyle::Underline, "underline")] {
                        ui.radio_value(&mut s.cursor_style, c, label);
                    }
                });
                ui.end_row();
            });

            head(ui, "Terminal");
            egui::Grid::new("settings-term").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                ui.label("Shell here");
                ui.horizontal(|ui| {
                    if cfg!(windows) {
                        egui::ComboBox::from_id_salt("settings-shell").selected_text(s.local_shell.label()).show_ui(ui, |ui| {
                            for sh in [LocalShell::Auto, LocalShell::Pwsh, LocalShell::Powershell, LocalShell::Cmd, LocalShell::Nu] {
                                ui.selectable_value(&mut s.local_shell, sh, sh.label());
                            }
                        });
                    } else {
                        ui.label(if s.unix_shell.is_empty() { "the login shell" } else { s.unix_shell.as_str() });
                    }
                    if ui.button("Shells…").clicked() {
                        open_cmd = Some(crate::ui_panels::Cmd::Shells);
                    }
                });
                ui.end_row();
                ui.label("");
                note(ui, "for new tabs; a remote host's shell is chosen in Shells…");
                ui.end_row();
            });
            ui.checkbox(&mut s.confirm_running, "Ask before closing a terminal where a command is running");
            ui.checkbox(&mut s.guess_blocks, "Show command blocks guessed from the output (shells weft has no integration for)");
            ui.horizontal(|ui| {
                ui.label("Graphics backend:");
                ui.label(if s.renderer.is_empty() { "auto" } else { s.renderer.as_str() });
                if ui.button("Graphics…").clicked() {
                    open_cmd = Some(crate::ui_panels::Cmd::Graphics);
                }
            });

            head(ui, "Files and editor");
            ui.checkbox(&mut s.follow_cwd, "Files follows the terminal's folder");
            ui.checkbox(&mut s.show_hidden, "Show hidden files");
            ui.checkbox(&mut s.show_ignored, "Show files git ignores (.gitignore)");
            ui.checkbox(&mut s.confirm_unsaved, "Ask before closing a file with unsaved changes");
            ui.checkbox(&mut s.markdown_breaks, "Markdown: every newline breaks the line (as GitHub comments do)");
            ui.horizontal(|ui| {
                ui.label("Diagrams in markdown (mermaid) drawn by:");
                ui.label(s.mermaid.label());
            });
            note(ui, "changed in a markdown file's Diagrams menu (built-in, or mermaid-cli)");

            head(ui, "Windows");
            ui.horizontal(|ui| {
                ui.label("As weft starts, the windows left open:");
                for (v, label) in [("all", "all of them"), ("last", "the last one"), ("none", "none")] {
                    if ui.radio(s.restore_windows == v, label).clicked() {
                        s.restore_windows = v.into();
                    }
                }
            });
            note(ui, "windows closed one by one leave only the last; all of them are left when the system shuts down or weft stops at once");

            head(ui, "Help");
            ui.horizontal(|ui| {
                ui.label("Language of the help:");
                for (code, label) in [("", "as the system"), ("ko", "한국어"), ("en", "English")] {
                    if ui.radio(s.help_language == code, label).clicked() {
                        s.help_language = code.into();
                    }
                }
            });

            head(ui, "Privacy and keys");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Privacy…").on_hover_text("what is written to disk (commands, history, logs) and for how long; passwords in memory").clicked() {
                    open_cmd = Some(crate::ui_panels::Cmd::Records);
                }
                if ui.button("Keyboard shortcuts…").clicked() {
                    open_cmd = Some(crate::ui_panels::Cmd::Keys);
                }
                if ui.button("Python on remote hosts…").clicked() {
                    open_cmd = Some(crate::ui_panels::Cmd::RemotePython);
                }
            });
        });
        // What the window's own controls changed, taken up at once.
        if let Some(c) = open_cmd {
            self.run_cmd(c);
        }
    }

    fn settings_hosts(&mut self, ui: &mut egui::Ui, w: &mut SettingsWindow) {
        ui.label(RichText::new("Hosts weft keeps: ssh hosts connected to, serial consoles opened. Forget one to take it off the lists (one a tab uses: close the tab first).").size(12.0).color(theme::text_dim()));
        ui.add_space(6.0);
        if self.settings.profiles.is_empty() {
            ui.label(RichText::new("none yet").color(theme::text_faint()));
            return;
        }
        let in_use: Vec<String> = self.panes.values().map(|p| p.host.clone()).collect();
        let mut forget: Option<String> = None;
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("settings-hosts").num_columns(3).striped(true).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                for p in &self.settings.profiles {
                    ui.label(RichText::new(&p.name).strong());
                    let mut kind = host_kind(&p.kind);
                    if in_use.contains(&p.name) {
                        kind.push_str(" · a tab uses it");
                    }
                    ui.label(RichText::new(kind).size(12.0).color(theme::text_dim()));
                    let armed = w.forget.as_ref().is_some_and(|(n, at)| *n == p.name && at.elapsed() < Duration::from_secs(5));
                    let label = if armed { RichText::new("Forget? again").color(theme::error()) } else { RichText::new("Forget") };
                    // A tab using it would come back without it (an ssh
                    // host without its port and user, a serial console tried
                    // as ssh): its tab closed first.
                    let used = in_use.contains(&p.name);
                    if ui.add_enabled(!used, egui::Button::new(label)).on_disabled_hover_text("a tab uses it: close it first").clicked() {
                        if armed {
                            forget = Some(p.name.clone());
                        } else {
                            w.forget = Some((p.name.clone(), Instant::now()));
                        }
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(name) = forget {
            self.settings.profiles.retain(|p| p.name != name);
            self.settings.remote_shells.remove(&name);
            w.forget = None;
            self.toast(format!("{name} forgotten"), ToastKind::Info);
        }
        if w.forget.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
    }

    fn settings_advanced(&mut self, ui: &mut egui::Ui, w: &mut SettingsWindow) {
        ui.label(RichText::new("settings.json as it is (the tabs and files open are in state.json, which weft keeps). Names it leaves out are their default. Apply to use it; it is saved then.").size(12.0).color(theme::text_dim()));
        ui.horizontal(|ui| {
            let edited = w.text != w.shown;
            if ui.add_enabled(edited, egui::Button::new("Apply")).on_hover_text("Ctrl+Enter").clicked() || (edited && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))) {
                // What the text changed from what it showed, over the
                // settings as they are now (an edit outside taken first;
                // a host kept meanwhile, a theme picked: not undone).
                self.poll_prefs_now();
                let text = match (as_weft_reads(&w.shown), as_weft_reads(&w.text), self.settings_bad.is_none()) {
                    (Some(base), Some(mine), true) => {
                        let mut now = as_weft_reads(&editable_json(&self.settings)).unwrap_or_default();
                        merge_changed(Some(&base), &mine, &mut now);
                        serde_json::to_string(&Value::Object(now)).unwrap_or_default()
                    }
                    _ => w.text.clone(),
                };
                let old = self.settings.clone();
                match self.apply_settings_text(&text) {
                    Ok(notes) => {
                        // Saved: what changed, over the file as it is; over a
                        // file that was not valid, all of it (fixed here),
                        // with the hosts tabs use.
                        if self.settings_bad.is_some() {
                            self.keep_hosts_in_use(&old);
                            self.settings_take_over();
                        } else {
                            self.keep_hosts_in_use(&old);
                            self.save_prefs();
                        }
                        w.error = None;
                        w.notes = notes;
                        w.text = self.advanced_text();
                        w.shown = w.text.clone();
                        self.toast("settings applied", ToastKind::Ok);
                    }
                    Err(e) => w.error = Some(e),
                }
            }
            if ui.add_enabled(edited, egui::Button::new("Undo edits")).clicked() {
                w.text = w.shown.clone();
                w.error = None;
            }
            if ui.button("Open in the editor").on_hover_text("the whole file, in weft's editor; saved, it is applied").clicked() {
                self.run_cmd(crate::ui_panels::Cmd::Settings);
            }
        });
        if let Some(e) = &w.error {
            ui.label(RichText::new(e).color(theme::error()));
        }
        for n in &w.notes {
            ui.label(RichText::new(n).size(12.0).color(theme::warn()));
        }
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add(TextEdit::multiline(&mut w.text).code_editor().desired_width(f32::INFINITY).desired_rows(24));
        });
    }
}
