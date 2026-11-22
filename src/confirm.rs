//! Asking before something is lost: closing an editor file with unsaved
//! changes, closing a terminal where a command is still running, and
//! closing or restarting weft with either. Each can be switched off (the
//! box in the dialog, or the palette's "Ask before…" commands).

use crate::app::*;
use crate::theme;
use egui::{vec2, Key, Modifiers, RichText};

/// What is waiting for an answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Close editor tabs around this file (host, path).
    Files { target: (String, String), which: CloseWhich },
    /// Close these terminal panes.
    Panes(Vec<usize>),
    /// Close weft, or restart it.
    Quit { restart: bool },
}

pub struct Confirm {
    pub ask: Ask,
    /// The files with unsaved changes this would lose (host, path).
    pub unsaved: Vec<(String, String)>,
    /// The commands this would end.
    pub running: Vec<String>,
}

/// After "Save": what goes on once these files are saved.
pub struct AfterSave {
    pub ask: Ask,
    pub files: Vec<(String, String)>,
    /// The question was asked and answered (else weft is only waiting for
    /// saves that were under way, and still has to ask).
    pub asked: bool,
}

impl App {
    /// The file would lose changes when closed (memos save themselves,
    /// scratch buffers are not files).
    fn unsaved(&self, f: &EditorFile) -> bool {
        f.dirty && !f.scratch && !self.is_memo(f)
    }

    /// What is running in a pane that closing it would end: a command the
    /// shell marked as started and not finished, or a full-screen program.
    /// (Without shell integration only the latter can be told.)
    pub fn running_in(&self, id: usize) -> Option<String> {
        let p = self.panes.get(&id)?;
        if p.exited || p.pty.is_none() {
            return None;
        }
        let block = p.term.blocks.last().filter(|b| b.started.is_some() && b.end_line.is_none());
        // One an Enter started in a shell that marks no start (dash, ksh);
        // one guessed out of sight (tcsh, a plain ssh).
        if let (None, Some(cmd)) = (block, p.term.pending_command()) {
            return Some(if cmd.trim().is_empty() { "a command".to_string() } else { cmd.trim().to_string() });
        }
        if block.is_none() && !p.term.alt_active && p.term.running() {
            return Some("a command".to_string());
        }
        match block {
            Some(b) => Some(if b.command.trim().is_empty() { "a command".to_string() } else { b.command.trim().to_string() }),
            None if p.term.alt_active => Some("a full-screen program".to_string()),
            None => None,
        }
    }

    fn unsaved_among(&self, at: usize, which: CloseWhich) -> Vec<(String, String)> {
        let mut found: Vec<(String, String)> = tabs_to_close(self.editor.files.len(), at, which).into_iter().map(|i| &self.editor.files[i]).filter(|f| self.unsaved(f)).map(|f| (f.host.clone(), f.path.clone())).collect();
        found.reverse();
        found
    }

    /// Close editor tabs, asking first when changes would be lost.
    pub fn ask_close_files(&mut self, at: usize, which: CloseWhich) {
        let unsaved = if self.settings.confirm_unsaved { self.unsaved_among(at, which) } else { Vec::new() };
        let Some(f) = self.editor.files.get(at) else { return };
        if unsaved.is_empty() {
            self.close_files(at, which);
        } else {
            self.confirm = Some(Confirm { ask: Ask::Files { target: (f.host.clone(), f.path.clone()), which }, unsaved, running: Vec::new() });
            self.focus_term = false;
        }
    }

    /// Close terminal panes, asking first when a command is still running.
    pub fn ask_close_panes(&mut self, ids: Vec<usize>) {
        let running: Vec<String> = if self.settings.confirm_running { ids.iter().filter_map(|id| self.running_in(*id)).collect() } else { Vec::new() };
        if running.is_empty() {
            for id in ids {
                self.close_pane(id);
            }
        } else {
            self.confirm = Some(Confirm { ask: Ask::Panes(ids), unsaved: Vec::new(), running });
            self.focus_term = false;
        }
    }

    /// Close the terminal tabs around tab `at`, asking as above.
    pub fn ask_close_tabs(&mut self, at: usize, which: CloseWhich) {
        let mut ids = Vec::new();
        for ti in tabs_to_close(self.tabs.len(), at, which) {
            self.tabs[ti].root.leaves(&mut ids);
        }
        if self.settings.confirm_running && ids.iter().any(|id| self.running_in(*id).is_some()) {
            self.ask_close_panes(ids);
        } else {
            self.close_tabs(at, which);
        }
    }

    /// May weft close (or restart) now? When something would be lost it
    /// asks instead, and the answer closes it.
    pub fn may_quit(&mut self, restart: bool) -> bool {
        if self.quit_confirmed {
            return true;
        }
        // A save still on its way is waited for: closing now would leave
        // the file as it was before Ctrl+S.
        let saving: Vec<(String, String)> = self.editor.files.iter().filter(|f| f.saving).map(|f| (f.host.clone(), f.path.clone())).collect();
        if !saving.is_empty() {
            if self.after_save.is_none() {
                self.after_save = Some(AfterSave { ask: Ask::Quit { restart }, files: saving, asked: false });
            }
            return false;
        }
        let unsaved: Vec<(String, String)> = if self.settings.confirm_unsaved { self.editor.files.iter().filter(|f| self.unsaved(f)).map(|f| (f.host.clone(), f.path.clone())).collect() } else { Vec::new() };
        let mut ids: Vec<usize> = self.panes.keys().copied().collect();
        ids.sort();
        let running: Vec<String> = if self.settings.confirm_running { ids.iter().filter_map(|id| self.running_in(*id)).collect() } else { Vec::new() };
        if unsaved.is_empty() && running.is_empty() {
            return true;
        }
        self.confirm = Some(Confirm { ask: Ask::Quit { restart }, unsaved, running });
        self.focus_term = false;
        false
    }

    /// Do what was asked about, now.
    fn carry_out(&mut self, ask: Ask) {
        match ask {
            Ask::Files { target, which } => {
                if let Some(at) = self.editor.files.iter().position(|f| f.host == target.0 && f.path == target.1) {
                    self.close_files(at, which);
                }
            }
            Ask::Panes(ids) => {
                for id in ids {
                    if self.panes.contains_key(&id) {
                        self.close_pane(id);
                    }
                }
            }
            Ask::Quit { restart } => {
                self.quit_confirmed = true;
                if restart {
                    self.restart();
                } else {
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    /// "Save" was chosen: once every file is saved, go on; if one can't be
    /// (it changed on disk, the host is gone), stay, with the file as it is.
    fn after_save(&mut self) {
        let Some(wait) = self.after_save.as_ref() else { return };
        let files: Vec<&EditorFile> = wait.files.iter().filter_map(|k| self.editor.files.iter().find(|f| f.host == k.0 && f.path == k.1)).collect();
        if files.iter().any(|f| f.saving) {
            return;
        }
        let failed = files.iter().find(|f| f.dirty).map(|f| f.path.rsplit('/').next().unwrap_or(&f.path).to_string());
        let Some(wait) = self.after_save.take() else { return };
        if let Some(name) = failed {
            self.toast(format!("{name} could not be saved: nothing was closed"), ToastKind::Error);
            return;
        }
        // The saves took a while and something else was changed meanwhile:
        // that is asked about, not closed with the rest. (What was running
        // was shown and accepted with the first question.)
        if let (false, Ask::Quit { restart }) = (wait.asked, &wait.ask) {
            // Only waited for saves under way: now ask what there is to ask.
            let restart = *restart;
            if self.may_quit(restart) {
                self.carry_out(wait.ask);
            }
            return;
        }
        let unsaved = self.unsaved_for(&wait.ask);
        if unsaved.is_empty() {
            self.carry_out(wait.ask);
        } else {
            self.confirm = Some(Confirm { ask: wait.ask, unsaved, running: Vec::new() });
            self.focus_term = false;
        }
    }

    /// The files with unsaved changes that this would close, now.
    fn unsaved_for(&self, ask: &Ask) -> Vec<(String, String)> {
        if !self.settings.confirm_unsaved {
            return Vec::new();
        }
        match ask {
            Ask::Files { target, which } => match self.editor.files.iter().position(|f| f.host == target.0 && f.path == target.1) {
                Some(at) => self.unsaved_among(at, *which),
                None => Vec::new(),
            },
            Ask::Panes(_) => Vec::new(),
            Ask::Quit { .. } => self.editor.files.iter().filter(|f| self.unsaved(f)).map(|f| (f.host.clone(), f.path.clone())).collect(),
        }
    }

    pub fn ui_confirm(&mut self, ctx: &egui::Context) {
        self.after_save();
        let Some(c) = self.confirm.take() else { return };
        #[derive(PartialEq)]
        enum Answer {
            Save,
            Go,
            Cancel,
        }
        let mut answer = None;
        let mut open = true;
        let (title, verb) = match &c.ask {
            Ask::Files { .. } => ("Unsaved changes", "Close"),
            Ask::Panes(_) => ("Still running", "Close"),
            Ask::Quit { restart: false } => ("Close weft?", "Close"),
            Ask::Quit { restart: true } => ("Restart weft?", "Restart"),
        };
        let (mut ask_unsaved, mut ask_running) = (self.settings.confirm_unsaved, self.settings.confirm_running);
        crate::widgets::dialog(ctx, title, vec2(440.0, 200.0)).open(&mut open).resizable(false).show(ctx, |ui| {
            let small = |s: &str| RichText::new(s.to_owned()).size(11.5).color(theme::text_faint());
            let list = |ui: &mut egui::Ui, items: &[String]| {
                for it in items.iter().take(8) {
                    ui.label(RichText::new(format!("  {}", weft_core::shown(it))).monospace().size(12.0));
                }
                if items.len() > 8 {
                    ui.label(small(&format!("  and {} more", items.len() - 8)));
                }
            };
            if !c.unsaved.is_empty() {
                ui.label(if c.unsaved.len() == 1 { "This file has changes that are not saved:" } else { "These files have changes that are not saved:" });
                let names: Vec<String> = c.unsaved.iter().map(|(h, p)| if h == "local" { p.clone() } else { format!("{h}:{p}") }).collect();
                list(ui, &names);
                ui.add_space(4.0);
            }
            if !c.running.is_empty() {
                ui.label(if c.running.len() == 1 { "This is still running, and would be ended:" } else { "These are still running, and would be ended:" });
                list(ui, &c.running);
                ui.add_space(4.0);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if !c.unsaved.is_empty() {
                    if ui.button(format!("Save and {}", verb.to_lowercase())).clicked() {
                        answer = Some(Answer::Save);
                    }
                    if ui.button(format!("{verb} without saving")).clicked() {
                        answer = Some(Answer::Go);
                    }
                } else if ui.button(format!("{verb} anyway")).clicked() {
                    answer = Some(Answer::Go);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
            ui.add_space(6.0);
            // Asking can be switched off here, and on again from the palette.
            if !c.unsaved.is_empty() {
                ui.checkbox(&mut ask_unsaved, RichText::new("Ask before closing unsaved files").size(12.0));
            }
            if !c.running.is_empty() {
                ui.checkbox(&mut ask_running, RichText::new("Ask before ending running commands").size(12.0));
            }
        });
        if (ask_unsaved, ask_running) != (self.settings.confirm_unsaved, self.settings.confirm_running) {
            self.settings.confirm_unsaved = ask_unsaved;
            self.settings.confirm_running = ask_running;
            self.save_settings();
        }
        // Esc cancels wherever the keyboard is.
        if !open || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            answer = Some(Answer::Cancel);
        }
        match answer {
            None => self.confirm = Some(c),
            Some(Answer::Cancel) => {
                // Also what was waiting for files to be saved: nothing is
                // closed behind a Cancel.
                self.after_save = None;
                self.focus_term = true;
            }
            Some(Answer::Go) => self.carry_out(c.ask),
            Some(Answer::Save) => {
                for (host, path) in &c.unsaved {
                    if let Some(i) = self.editor.files.iter().position(|f| &f.host == host && &f.path == path) {
                        self.save_file(i, false);
                    }
                }
                self.after_save = Some(AfterSave { ask: c.ask, files: c.unsaved, asked: true });
            }
        }
    }
}
