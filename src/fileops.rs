//! Files: new folder, new file, rename, delete. Asked in a small dialog,
//! done by the host (this PC, or the remote agent), and the folder is
//! listed again.

use crate::app::*;
use crate::theme;
use crate::transfer::base_name;
use egui::{vec2, Key, Modifiers, RichText, TextEdit};
use serde_json::json;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FileOpKind {
    NewFolder,
    NewFile,
    Rename,
    Delete,
}

/// The dialog, while open.
pub struct FileOp {
    pub kind: FileOpKind,
    pub host: String,
    /// The folder to make it in (new folder / file), else the file or
    /// folder itself.
    pub path: String,
    /// Rename, delete: it is a folder.
    pub dir: bool,
    pub name: String,
    /// Put the keyboard in the name field next frame.
    pub focus: bool,
}

/// "/a/b" → "/a", "/a" → "/", "C:/x" → "C:/".
pub fn parent_of(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rsplit_once('/') {
        Some((dir, _)) if dir.is_empty() || dir.ends_with(':') => format!("{dir}/"),
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

pub fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

/// Why this can't be a file's name (one path component), if it can't.
/// `windows`: the file is on a Windows disk, which reads some names as
/// something else (`a:b` is a stream of `a`, `name.` is `name`).
pub fn name_problem(name: &str, windows: bool) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        Some("type a name")
    } else if n == "." || n == ".." {
        Some("that is not a name")
    } else if n.contains(['/', '\\']) {
        Some("a name, not a path: no / or \\")
    } else if windows && n.contains(['<', '>', ':', '"', '|', '?', '*']) {
        Some("Windows takes none of < > : \" | ? * in a name")
    } else if windows && n.ends_with('.') {
        Some("Windows drops a dot at the end of a name")
    } else {
        None
    }
}

impl App {
    /// Deleting here goes to the Recycle Bin (this PC, on Windows); anywhere
    /// else it is for good. (Tests delete for good: the Recycle Bin is the
    /// user's.) (Tests delete for good: the Recycle Bin is the
    /// user's.)
    fn deletes_to_trash(&self, host: &str) -> bool {
        cfg!(windows) && !cfg!(test) && self.profile(host).is_local()
    }

    pub fn ask_file_op(&mut self, kind: FileOpKind, host: &str, path: &str, dir: bool) {
        let name = if kind == FileOpKind::Rename { base_name(path) } else { String::new() };
        self.file_op = Some(FileOp { kind, host: host.into(), path: path.into(), dir, name, focus: true });
        // The dialog has the keyboard, not the shell.
        self.focus_term = false;
    }

    /// Ask the host to do it; the answer comes as a toast, and the folder
    /// is listed again.
    pub fn run_file_op(&mut self, op: &FileOp) {
        let name = op.name.trim().to_string();
        match op.kind {
            FileOpKind::NewFolder | FileOpKind::NewFile => {
                let path = join(&op.path, &name);
                let what = if op.kind == FileOpKind::NewFolder { "mkdir" } else { "create" };
                self.request(&op.host, json!({"op": what, "path": path}), Pending::FileOp { done: format!("created {name}"), refresh: vec![op.path.clone()], renamed: None });
            }
            FileOpKind::Rename => {
                let parent = parent_of(&op.path);
                let to = join(&parent, &name);
                if to == op.path {
                    return;
                }
                self.request(
                    &op.host,
                    json!({"op": "rename", "from": op.path, "to": to}),
                    Pending::FileOp { done: format!("renamed to {name}"), refresh: vec![parent], renamed: Some((op.path.clone(), to)) },
                );
            }
            FileOpKind::Delete => {
                let trash = self.deletes_to_trash(&op.host);
                let name = base_name(&op.path);
                let done = if trash { format!("{name} moved to the Recycle Bin") } else { format!("deleted {name}") };
                self.request(&op.host, json!({"op": "delete", "path": op.path, "trash": trash}), Pending::FileOp { done, refresh: vec![parent_of(&op.path)], renamed: None });
            }
        }
    }

    /// A file or folder got a new name: what was open or expanded under the
    /// old one follows.
    pub fn file_renamed(&mut self, host: &str, from: &str, to: &str) {
        let moved = |p: &str| -> Option<String> {
            if p == from {
                Some(to.to_string())
            } else {
                p.strip_prefix(from).filter(|rest| rest.starts_with('/')).map(|rest| format!("{to}{rest}"))
            }
        };
        // A save or a read on its way finds the file by its path.
        for ((h, _), pending) in self.pending.iter_mut() {
            if let (true, Pending::Save { path } | Pending::Open { path, .. }) = (h == host, pending) {
                if let Some(p) = moved(path) {
                    *path = p;
                }
            }
        }
        for f in self.editor.files.iter_mut().filter(|f| f.host == host) {
            if let Some(p) = moved(&f.path) {
                f.path = p;
            }
        }
        let open: Vec<(String, String)> = self.side.expanded.iter().filter(|(h, _)| h == host).cloned().collect();
        for key in open {
            if let Some(p) = moved(&key.1) {
                self.side.expanded.remove(&key);
                self.side.expanded.insert((host.to_string(), p));
            }
        }
        if let Some(p) = self.side.selected.as_deref().and_then(moved) {
            self.side.selected = Some(p);
        }
    }

    pub fn ui_file_op(&mut self, ctx: &egui::Context) {
        let Some(mut op) = self.file_op.take() else { return };
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        let title = match op.kind {
            FileOpKind::NewFolder => "New folder",
            FileOpKind::NewFile => "New file",
            FileOpKind::Rename => "Rename",
            FileOpKind::Delete => "Delete",
        };
        let trash = self.deletes_to_trash(&op.host);
        let on_windows = cfg!(windows) && self.profile(&op.host).is_local();
        crate::widgets::dialog(ctx, title, vec2(400.0, 130.0)).open(&mut open).resizable(false).show(ctx, |ui| {
            let small = |s: String| RichText::new(s).size(11.5).color(theme::text_faint());
            if op.kind == FileOpKind::Delete {
                ui.label(format!("{}{}", weft_core::shown(&base_name(&op.path)), if op.dir { " and everything in it" } else { "" }));
                ui.label(small(format!("{}:{}", op.host, weft_core::shown(&op.path))));
                ui.add_space(4.0);
                if trash {
                    ui.label(RichText::new("It goes to the Recycle Bin.").color(theme::text_dim()));
                } else {
                    ui.label(RichText::new("This cannot be undone.").color(theme::warn()));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(if trash { "Move to Recycle Bin" } else { "Delete for good" }).clicked() {
                        go = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                return;
            }
            let place = if op.kind == FileOpKind::Rename { parent_of(&op.path) } else { op.path.clone() };
            ui.label(small(format!("in {}:{}", op.host, weft_core::shown(&place))));
            let r = ui.add(TextEdit::singleline(&mut op.name).id(egui::Id::new("file-op-name")).desired_width(f32::INFINITY).hint_text("name"));
            if op.focus {
                r.request_focus();
                op.focus = false;
            }
            let problem = name_problem(&op.name, on_windows);
            if let Some(p) = problem.filter(|_| !op.name.trim().is_empty()) {
                ui.label(RichText::new(p).size(11.5).color(theme::warn()));
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = if op.kind == FileOpKind::Rename { "Rename" } else { "Create" };
                let b = ui.add_enabled(problem.is_none(), egui::Button::new(label));
                if (b.clicked() || enter) && problem.is_none() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        // Esc closes it wherever the keyboard is (egui drops the focus on
        // Esc before the frame, so the field can't be asked).
        if cancel || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            open = false;
        }
        if go {
            self.run_file_op(&op);
            self.focus_term = true;
        } else if open {
            self.file_op = Some(op);
        } else {
            self.focus_term = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parents_and_names() {
        assert_eq!(parent_of("/home/me/a.txt"), "/home/me");
        assert_eq!(parent_of("/a"), "/");
        assert_eq!(parent_of("C:/x"), "C:/");
        assert_eq!(parent_of("C:/x/y/"), "C:/x");
        assert_eq!(join("/home/me", "b"), "/home/me/b");
        assert_eq!(join("C:/", "b"), "C:/b");
        assert_eq!(join("/", "b"), "/b");
        for windows in [false, true] {
            assert_eq!(name_problem("notes.md", windows), None);
            assert_eq!(name_problem("  with space  ", windows), None);
            assert_eq!(name_problem(".gitignore", windows), None);
            assert!(name_problem("", windows).is_some() && name_problem("   ", windows).is_some());
            assert!(name_problem("..", windows).is_some() && name_problem(".", windows).is_some());
            assert!(name_problem("a/b", windows).is_some() && name_problem("a\\b", windows).is_some());
        }
        // Names a Windows disk reads as something else; fine elsewhere.
        for name in ["a:b", "what?", "x*y", "a|b", "<tag>", "say \"hi\"", "ends."] {
            assert!(name_problem(name, true).is_some(), "{name}");
            assert_eq!(name_problem(name, false), None, "{name}");
        }
    }
}
