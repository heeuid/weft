//! Regression tests of what weft does on screen, through the real App (see
//! testkit.rs): each one is a feature or a bug that was fixed, so a later
//! change that breaks it fails here. docs/regression.md lists them all.

use crate::testkit::Harness;
use egui::{Event, Key, Modifiers};
use serde_json::json;

const ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: false, mac_cmd: false, command: false };
const ALT_SHIFT: Modifiers = Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };
const CTRL_SHIFT: Modifiers = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };

/// Alt+1…8 go to that tab, Alt+9 to the last; Alt+Shift+←/→ step, wrapping.
#[test]
fn tab_keys() {
    let mut h = Harness::new(json!({}));
    h.command("new_tab");
    h.command("new_tab");
    assert_eq!((h.app.tabs.len(), h.app.active_tab), (3, 2));
    h.key(Key::Num1, ALT);
    assert_eq!(h.app.active_tab, 0, "Alt+1");
    h.key(Key::Num9, ALT);
    assert_eq!(h.app.active_tab, 2, "Alt+9: the last");
    h.key(Key::Num2, ALT);
    assert_eq!(h.app.active_tab, 1, "Alt+2");
    h.key(Key::ArrowLeft, ALT_SHIFT);
    assert_eq!(h.app.active_tab, 0, "Alt+Shift+Left");
    h.key(Key::ArrowLeft, ALT_SHIFT);
    assert_eq!(h.app.active_tab, 2, "wraps to the last");
    h.key(Key::ArrowRight, ALT_SHIFT);
    assert_eq!(h.app.active_tab, 0, "wraps to the first");
}

/// Ctrl+Shift+X arrives as a Cut event (no key event): it still opens copy mode.
#[test]
fn copy_mode_key_arrives_as_cut() {
    let mut h = Harness::new(json!({}));
    let pane = h.app.active_pane_id().unwrap();
    // What egui-winit sends for it: a Cut, the modifiers held, no key event.
    let input = egui::RawInput { events: vec![Event::Cut], modifiers: CTRL_SHIFT, ..Default::default() };
    let app = &mut h.app;
    let _ = h.ctx.run(input, |ctx| app.frame(ctx));
    h.frames(1);
    assert!(h.app.panes[&pane].copy.is_some(), "copy mode is on");
}

/// The palette after a short result list ("theme"), closed and opened again:
/// full height again, and starting at the top (it reopened 3-8 rows tall,
/// scrolled to where it was).
#[test]
fn palette_reopens_full_height_at_the_top() {
    let mut h = Harness::new(json!({}));
    h.command("palette");
    h.type_text("theme");
    h.frames(2);
    h.key(Key::Escape, Modifiers::NONE);
    h.command("palette");
    h.frames(2);
    assert!(h.shows("New tab (same host)"), "starts at the top");
    for row in ["Split down", "Go to tab 3", "Go to tab 6"] {
        assert!(h.shows(row), "\"{row}\" fits: the list is tall again");
    }
}

/// Removing a key in Keyboard shortcuts leaves every row where it was.
#[test]
fn keys_window_rows_hold_still() {
    let mut h = Harness::new(json!({}));
    h.command("keys");
    h.frames(2);
    let next_before = h.at("Connect to host…").y;
    let row_y = h.at("New tab (same host)").y;
    // The × of New tab's key: a "×" drawn on that row.
    let cross = h.texts.iter().filter(|(t, r)| t == "\u{00d7}" && (r.center().y - row_y).abs() < 10.0).map(|(_, r)| r.center()).next().expect("a key chip with ×");
    h.click(cross);
    h.frames(2);
    assert!(h.app.settings.keys.get("new_tab").is_some_and(|k| k.is_empty()), "the key was removed");
    assert!(h.shows("no key"));
    assert_eq!(h.at("Connect to host…").y, next_before, "the next row did not move");
}

/// Dialogs fit a small window (Help ran off a 900 px window's edge).
#[test]
fn dialogs_fit_a_small_window() {
    let mut h = Harness::new(json!({}));
    h.size = egui::vec2(900.0, 520.0);
    for (cmd, title) in [("help", "Help"), ("keys", "Keyboard shortcuts"), ("privacy", "Privacy"), ("serial", "Serial console")] {
        h.command(cmd);
        h.frames(3);
        let r = h.find(title).unwrap_or_else(|| panic!("{title} is open"));
        assert!(r.left() >= 0.0 && r.right() <= 900.0 && r.top() >= -1.0, "{title} title inside the window: {r:?}");
        let widest = h.texts.iter().map(|(_, r)| r.right()).fold(0.0_f32, f32::max);
        assert!(widest <= 900.0 + 1.0, "{title}: nothing drawn past the right edge ({widest})");
        h.key(Key::Escape, Modifiers::NONE);
        // Windows close with their own button; a fresh harness per dialog is overkill.
        h.app.help = None;
        h.app.keys_window = None;
        h.app.records_window = None;
        h.app.serial_dialog = None;
        h.frames(2);
    }
}

/// A newer weft appearing lights the restart button in the status bar.
#[test]
fn a_newer_weft_offers_a_restart() {
    let mut h = Harness::new(json!({}));
    let built = h.dir.join("weft-new.exe");
    std::fs::write(&built, b"new").unwrap();
    h.app.newer_weft_at = Some(built);
    h.app.started_at = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    h.app.update_checked = std::time::Instant::now() - std::time::Duration::from_secs(60);
    h.frames(2);
    assert!(h.app.update_ready);
    assert!(h.shows("new weft"), "the status bar button");
}

/// The editor: unsaved mark, Save → "✓ saved", typing clears it; Copy → "✓
/// copied"; Rich / Source is one button; Read mode keeps markdown rendered.
#[test]
fn editor_marks_buttons_and_read_mode() {
    let mut h = Harness::new(json!({"sidebar": false}));
    let file = h.dir.join("note.md");
    std::fs::write(&file, "# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nsome *text*\n").unwrap();
    let path = file.display().to_string().replace('\\', "/");
    h.app.open_file("local", &path, 1);
    assert!(h.wait_for(20.0, |h| h.shows("Rich") && h.app.editor.files.iter().all(|f| !f.loading)), "opened and read");
    h.frames(2);
    let i = h.app.editor.files.iter().position(|f| f.path == path).unwrap();
    // One view button.
    assert!(h.shows("Rich \u{21c4}") && !h.shows("Source \u{21c4}"));
    h.click_text("Rich \u{21c4}");
    assert!(h.shows("Source \u{21c4}") && !h.app.editor.files[i].rich);
    h.click_text("Source \u{21c4}");
    assert!(h.app.editor.files[i].rich);
    // Unsaved, saved, typing again.
    h.app.editor.files[i].text.push_str("more\n");
    h.app.editor.files[i].dirty = true;
    h.frames(2);
    assert!(h.shows("\u{25cf} unsaved"));
    h.click_text("Save");
    assert!(h.wait_for(20.0, |h| h.shows("\u{2713} saved")), "saved mark");
    assert!(std::fs::read_to_string(&file).unwrap().ends_with("more\n"));
    // Copy: the whole file when nothing is selected.
    h.click_text("Copy");
    assert!(h.shows("\u{2713} copied"));
    // Read mode: a click in the table leaves it rendered (its cells are
    // drawn one by one; with the cursor on it, it would show its source).
    let cell = |h: &Harness| h.texts.iter().find(|(t, _)| t == "2").map(|(_, r)| r.center());
    h.click_text("Read");
    assert!(h.app.editor.files[i].read);
    let at = cell(&h).unwrap_or_else(|| panic!("the table is rendered, a cell of its own: {:?}", h.texts));
    h.click(at);
    h.frames(2);
    assert!(cell(&h).is_some(), "the table stays rendered");
    // And nothing typed changes the file.
    let before = h.app.editor.files[i].text.clone();
    h.type_text("zzz");
    assert_eq!(h.app.editor.files[i].text, before, "read mode takes no typing");
}

/// A serial port that isn't there: the tab says so; the status bar names it.
#[test]
fn serial_tab_explains_a_missing_port() {
    let mut h = Harness::new(json!({}));
    h.app.open_serial("local", "COM99", 115_200);
    h.frames(3);
    let id = h.app.active_pane_id().unwrap();
    let err = h.app.panes[&id].spawn_error.clone().unwrap_or_default();
    assert!(err.contains("COM99") && (err.contains("no such port") || err.contains("in use")), "{err}");
    assert!(h.shows("serial COM99"), "the status bar");
    assert!(h.app.settings.profiles.iter().any(|p| p.name == "COM99"), "kept as a host, for the session");
}

/// An image in a tab behind another gives its texture back.
#[test]
fn hidden_images_leave_the_gpu() {
    let mut h = Harness::new(json!({}));
    let png = h.dir.join("dot.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255])).save(&png).unwrap();
    let txt = h.dir.join("a.txt");
    std::fs::write(&txt, "hello").unwrap();
    let (png, txt) = (png.display().to_string().replace('\\', "/"), txt.display().to_string().replace('\\', "/"));
    h.app.open_file("local", &png, 1);
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().any(|f| f.path == png && f.image.has_texture())), "shown: on the GPU");
    h.app.open_file("local", &txt, 1);
    h.frames(3);
    let img = h.app.editor.files.iter().find(|f| f.path == png).unwrap();
    assert!(!img.image.has_texture(), "behind another tab: unloaded");
}

/// Esc closes the composer, the scrollback search and the editor's find bar.
/// (egui takes the focus away on Esc before the frame; they waited for a
/// focus they no longer had, and stayed open.)
#[test]
fn escape_closes_composer_search_and_find() {
    let mut h = Harness::new(json!({}));
    let pane = h.app.active_pane_id().unwrap();
    h.command("composer");
    h.type_text("ls");
    assert_eq!(h.app.panes[&pane].composer.as_ref().map(|c| c.text.as_str()), Some("ls"), "typed into the composer");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.panes[&pane].composer.is_none(), "the composer closed");

    h.command("find");
    h.type_text("x");
    assert_eq!(h.app.panes[&pane].search.as_ref().map(|s| s.query.as_str()), Some("x"), "typed into the search");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.panes[&pane].search.is_none(), "the search closed");

    let file = h.dir.join("a.txt");
    std::fs::write(&file, "one two\n").unwrap();
    let path = file.display().to_string().replace('\\', "/");
    h.app.open_file("local", &path, 1);
    assert!(h.wait_for(20.0, |h| h.shows("Find") && h.app.editor.files.iter().all(|f| !f.loading)), "opened and read");
    h.frames(2);
    h.click_text("Find");
    h.type_text("two");
    assert_eq!(h.app.editor.find.as_ref().map(|f| f.query.as_str()), Some("two"), "typed into find");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.editor.find.is_none(), "the find bar closed");
}

/// A rendered table in read mode is selected like a spreadsheet: a drag
/// inside a cell copies those letters, a double click the cell, a drag
/// across cells the block (tab-separated), and its menu the whole table.
#[test]
fn table_cells_select_and_copy() {
    let mut h = Harness::new(json!({}));
    let file = h.dir.join("t.md");
    std::fs::write(&file, "| name | size |\n|---|---|\n| weft.exe | 12 MB |\n| notes.md | 3 KB |\n\nend\n").unwrap();
    let path = file.display().to_string().replace('\\', "/");
    h.app.open_file("local", &path, 1);
    assert!(h.wait_for(20.0, |h| h.shows("Read") && h.app.editor.files.iter().all(|f| !f.loading)), "opened and read");
    h.frames(2);
    h.click_text("Read");
    let cell = |h: &Harness, t: &str| h.texts.iter().find(|(s, _)| s == t).map(|(_, r)| *r).unwrap_or_else(|| panic!("cell {t:?} is drawn"));
    // Some letters of one cell: from its left edge to its middle.
    let r = cell(&h, "weft.exe");
    h.drag(r.left_center(), r.center());
    h.frame(vec![Event::Copy]);
    let got = h.copied.take().expect("Ctrl+C copied");
    assert!(!got.is_empty() && got.len() < 8 && "weft.exe".starts_with(&got), "letters of the cell: {got:?}");
    // A block of cells.
    let (a, b) = (cell(&h, "weft.exe").center(), cell(&h, "3 KB").center());
    h.drag(a, b);
    h.frame(vec![Event::Copy]);
    assert_eq!(h.copied.take().as_deref(), Some("weft.exe\t12 MB\nnotes.md\t3 KB"));
    // The mark is drawn by the toolbar, above: from the next frame.
    h.frames(1);
    assert!(h.shows("\u{2713} copied"), "the copied mark");
    // With the keyboard elsewhere (the terminal, another field), Ctrl+C is
    // not the table's.
    h.ctx.memory_mut(|m| m.request_focus(egui::Id::new("some other field")));
    h.frame(vec![Event::Copy]);
    assert_eq!(h.copied.take(), None, "Ctrl+C elsewhere copies no cells");
    // One cell, by a double click.
    let c = cell(&h, "12 MB").center();
    h.double_click(c);
    h.frame(vec![Event::Copy]);
    assert_eq!(h.copied.take().as_deref(), Some("12 MB"));
    // A click lets go of it.
    let c = cell(&h, "size").center();
    h.click(c);
    h.frame(vec![Event::Copy]);
    assert_eq!(h.copied.take(), None, "nothing selected, nothing copied");
    // The menu: the whole table, tab-separated.
    let c = cell(&h, "name").center();
    h.right_click(c);
    h.click_text("Copy as TSV");
    assert_eq!(h.copied.take().as_deref(), Some("name\tsize\nweft.exe\t12 MB\nnotes.md\t3 KB"));
    // And as it is written in the file.
    let c = cell(&h, "name").center();
    h.right_click(c);
    h.click_text("Copy table (markdown)");
    assert_eq!(h.copied.take().as_deref(), Some("| name | size |\n|---|---|\n| weft.exe | 12 MB |\n| notes.md | 3 KB |"));
}

/// Files: new folder, rename and delete from a row's menu; Esc leaves a
/// dialog without doing anything.
#[test]
fn files_new_folder_rename_delete() {
    let mut h = Harness::new(json!({"sidebar": true, "follow_cwd": false}));
    let root = h.dir.join("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), "hello").unwrap();
    let dir = root.display().to_string().replace('\\', "/");
    h.app.set_root("local", &dir);
    assert!(h.wait_for(20.0, |h| h.shows("a.txt") && h.shows("sub")), "the folder is listed");

    // Rename a.txt: the name is there to change; Enter does it.
    let row = h.at("a.txt");
    h.right_click(row);
    h.click_text("Rename\u{2026}");
    assert_eq!(h.app.file_op.as_ref().map(|o| o.name.as_str()), Some("a.txt"));
    h.key(Key::A, Modifiers::COMMAND);
    h.type_text("b.txt");
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_for(20.0, |h| root.join("b.txt").exists() && h.has("b.txt") && !h.has("a.txt")), "renamed, and listed again (its row, not the message)");
    assert!(!root.join("a.txt").exists());
    assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "hello");

    // A new folder inside sub.
    let row = h.at("sub");
    h.right_click(row);
    h.click_text("New folder\u{2026}");
    h.type_text("made");
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_for(20.0, |h| root.join("sub/made").is_dir() && h.has("made")), "made, and its row shown in the opened folder");

    // Delete, cancelled by Esc: nothing happens.
    let row = h.at("b.txt");
    h.right_click(row);
    h.click_text("Delete\u{2026}");
    assert!(h.shows("This cannot be undone."));
    h.key(Key::Escape, Modifiers::NONE);
    h.frames(5);
    assert!(h.app.file_op.is_none() && root.join("b.txt").exists(), "Esc keeps the file");

    // Delete, for real.
    let row = h.at("b.txt");
    h.right_click(row);
    h.click_text("Delete\u{2026}");
    h.click_text("Delete for good");
    assert!(h.wait_for(20.0, |h| !root.join("b.txt").exists() && !h.has("b.txt")), "deleted, and its row gone from the list");
}

/// The palette matches fuzzily by default (letters in order), runs the best
/// match on Enter, and its mode button changes how the query is read and
/// is remembered.
#[test]
fn the_palette_is_fuzzy_and_its_mode_is_kept() {
    use crate::editfind::MatchMode;
    let mut h = Harness::new(json!({}));
    assert_eq!(h.app.settings.palette_mode, MatchMode::Fuzzy);
    h.command("palette");
    h.type_text("spldn");
    h.frames(2);
    assert!(h.shows("Split down"), "found by its letters");
    assert_eq!(h.app.pane_rects.len(), 1);
    h.key(Key::Enter, Modifiers::NONE);
    h.frames(3);
    assert_eq!(h.app.pane_rects.len(), 2, "Enter ran the best match: the pane was split");
    // As typed: the same letters find nothing.
    h.command("palette");
    h.click_text("fuzzy");
    assert_eq!(h.app.settings.palette_mode, MatchMode::Text);
    assert!(h.app.palette.is_some(), "the palette stays open");
    h.type_text("spldn");
    h.frames(2);
    assert!(!h.shows("Split down"));
    h.key(Key::Escape, Modifiers::NONE);
    // Kept in settings.json.
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(h.dir.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["palette_mode"], json!("Text"));
}

/// What was open comes back after a restart: the editor's files as they
/// were shown (source or rendered, read mode), the one in front, and the
/// folders opened in Files. Settings from before this was kept still load.
#[test]
fn a_restart_brings_back_what_was_open() {
    let mut h = Harness::new(json!({}));
    assert!(h.app.editor.files.is_empty() && !h.app.editor.visible, "old settings: nothing to restore");
    let (md, txt) = (h.dir.join("note.md"), h.dir.join("b.txt"));
    std::fs::write(&md, "# Title\n\ntext\n").unwrap();
    std::fs::write(&txt, "plain\n").unwrap();
    let p = |f: &std::path::Path| f.display().to_string().replace('\\', "/");
    h.app.open_file("local", &p(&md), 1);
    h.app.open_file("local", &p(&txt), 1);
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().all(|f| !f.loading)), "both loaded");
    h.app.editor.files[0].read = true;
    h.app.editor.files[0].rich = false;
    h.app.editor.active = 0;
    let dir = p(&h.dir);
    h.app.side.expanded.insert(("local".into(), format!("{dir}/sub")));

    h.restart();
    let files: Vec<(String, bool, bool)> = h.app.editor.files.iter().map(|f| (f.path.clone(), f.rich, f.read)).collect();
    assert_eq!(files, [(p(&md), false, true), (p(&txt), false, false)]);
    assert_eq!(h.app.editor.active, 0, "the one in front");
    assert!(h.app.editor.visible);
    assert!(h.app.side.expanded.contains(&("local".to_string(), format!("{dir}/sub"))));
    assert!(h.wait_for(20.0, |h| h.app.editor.files[0].text.starts_with("# Title") && h.app.editor.files[1].text == "plain\n"), "their text is read again");

    // A file that is gone meanwhile: its tab says so, the rest is there.
    std::fs::remove_file(&txt).unwrap();
    h.restart();
    assert_eq!(h.app.editor.files.len(), 2);
    assert!(h.wait_for(20.0, |h| h.app.editor.files[1].error.is_some() && h.app.editor.files[0].text.starts_with("# Title")), "the missing one explains itself");
}

fn dirty_file(h: &mut Harness, name: &str) -> (std::path::PathBuf, usize) {
    let file = h.dir.join(name);
    std::fs::write(&file, "old\n").unwrap();
    let path = file.display().to_string().replace('\\', "/");
    h.app.open_file("local", &path, 1);
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().any(|f| f.path == path && !f.loading)), "opened");
    let i = h.app.editor.files.iter().position(|f| f.path == path).unwrap();
    h.app.editor.files[i].text = "new\n".into();
    h.app.editor.files[i].dirty = true;
    h.frames(2);
    (file, i)
}

/// Closing a file with unsaved changes asks: Cancel keeps it, "without
/// saving" drops the changes, "Save and close" writes them first. With the
/// setting off it closes at once, as before.
#[test]
fn closing_an_unsaved_file_asks() {
    use crate::app::CloseWhich;
    let mut h = Harness::new(json!({}));
    let (file, i) = dirty_file(&mut h, "a.txt");
    h.app.ask_close_files(i, CloseWhich::This);
    h.frames(2);
    assert!(h.shows("Unsaved changes") && h.shows("a.txt"), "asked, naming the file");
    assert_eq!(h.app.editor.files.len(), 1, "nothing closed yet");
    // Esc: as Cancel.
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.confirm.is_none() && h.app.editor.files.len() == 1 && h.app.editor.files[0].dirty);
    // Save and close.
    h.app.ask_close_files(i, CloseWhich::This);
    h.frames(2);
    h.click_text("Save and close");
    assert!(h.wait_for(20.0, |h| h.app.editor.files.is_empty()), "closed once saved");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
    // Without saving.
    let (file, i) = dirty_file(&mut h, "b.txt");
    h.app.ask_close_files(i, CloseWhich::This);
    h.frames(2);
    h.click_text("Close without saving");
    assert!(h.app.editor.files.is_empty());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "old\n", "the changes were dropped");
    // A saved file never asks; nor does anything with the setting off.
    let (_, i) = dirty_file(&mut h, "c.txt");
    h.app.editor.files[i].dirty = false;
    h.app.ask_close_files(i, CloseWhich::This);
    assert!(h.app.confirm.is_none() && h.app.editor.files.is_empty());
    let (_, i) = dirty_file(&mut h, "d.txt");
    h.command("confirm_unsaved");
    assert!(!h.app.settings.confirm_unsaved);
    h.app.ask_close_files(i, CloseWhich::This);
    assert!(h.app.confirm.is_none() && h.app.editor.files.is_empty(), "off: closes at once");
}

/// Closing the window with unsaved changes is held back and asked about;
/// the answer closes it. With nothing to lose it closes at once.
#[test]
fn closing_weft_with_unsaved_files_asks() {
    let cancels = |h: &Harness| h.window.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose));
    let mut h = Harness::new(json!({}));
    h.close_window();
    assert!(!cancels(&h) && h.app.confirm.is_none(), "nothing to lose: it just closes");
    let (file, _) = dirty_file(&mut h, "a.txt");
    h.close_window();
    assert!(cancels(&h), "the close is held back");
    h.frames(2);
    assert!(h.shows("Close weft?") && h.shows("a.txt"));
    h.click_text("Cancel");
    assert!(h.app.confirm.is_none() && !h.app.quit_confirmed);
    // Asked again the next time; saving closes it.
    h.close_window();
    h.frames(2);
    // (The save can be done within the click's own frames.)
    h.window_log.clear();
    h.click_text("Save and close");
    assert!(h.wait_for(20.0, |h| h.app.quit_confirmed), "saved");
    h.frames(2);
    assert!(h.window_log.iter().any(|c| matches!(c, egui::ViewportCommand::Close)), "weft asks its window to close once saved");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
    h.close_window();
    assert!(!cancels(&h), "and the close goes through");
}

/// Closing a terminal where a command is still running asks; an idle one
/// closes at once.
#[test]
fn closing_a_busy_terminal_asks() {
    let mut h = Harness::new(json!({}));
    h.command("split_right");
    assert_eq!(h.app.panes.len(), 2);
    let id = h.app.active_pane_id().unwrap();
    assert_eq!(h.app.running_in(id), None, "nothing known to run");
    // The shell says a command started (OSC 133 C after a prompt and input).
    let osc = "\x1b]133;A\x07$ \x1b]133;B\x07make -j8\r\n\x1b]133;C\x07building\r\n";
    {
        let p = h.app.panes.get_mut(&id).unwrap();
        for b in osc.bytes() {
            p.parser.advance(&mut p.term, &[b]);
        }
    }
    assert!(h.app.running_in(id).is_some(), "a command is running");
    h.command("close_pane");
    assert_eq!(h.app.panes.len(), 2, "not closed yet");
    assert!(h.shows("Still running"));
    h.click_text("Cancel");
    assert_eq!(h.app.panes.len(), 2);
    h.command("close_pane");
    h.click_text("Close anyway");
    assert_eq!(h.app.panes.len(), 1, "closed on the answer");
    // The other one is idle: no question.
    h.command("split_right");
    h.command("close_pane");
    assert!(h.app.confirm.is_none());
    assert_eq!(h.app.panes.len(), 1);
}

/// weft says which version it is: in Help, and from the palette's About.
#[test]
fn the_version_is_shown() {
    let mut h = Harness::new(json!({}));
    let line = crate::version::line();
    assert!(line.contains(env!("CARGO_PKG_VERSION")));
    h.command("help");
    h.frames(2);
    assert!(h.shows(&line), "Help shows {line:?}");
    h.app.help = None;
    h.command("about");
    assert!(h.shows(&line), "About says it");
}

/// "Save and close", and while the save is on its way another file is
/// changed: weft asks about that one instead of closing over it. Cancel
/// then closes nothing, also not later.
#[test]
fn saving_then_closing_asks_about_what_changed_meanwhile() {
    use crate::confirm::{AfterSave, Ask};
    let mut h = Harness::new(json!({}));
    let (a, _) = dirty_file(&mut h, "a.txt");
    let (b, _) = dirty_file(&mut h, "b.txt");
    let key = |f: &std::path::Path| ("local".to_string(), f.display().to_string().replace('\\', "/"));
    // As after "Save and close" with only a.txt unsaved: a.txt is saved by
    // now, b.txt was changed since.
    h.app.editor.files[0].dirty = false;
    h.app.after_save = Some(AfterSave { ask: Ask::Quit { restart: false }, files: vec![key(&a)] });
    h.window_log.clear();
    h.frames(3);
    assert!(!h.window_log.iter().any(|c| matches!(c, egui::ViewportCommand::Close)), "not closed over b.txt");
    assert!(h.shows("Close weft?") && h.shows("b.txt"), "asked about b.txt");
    assert!(!h.app.quit_confirmed);
    h.click_text("Cancel");
    h.frames(5);
    assert!(h.app.after_save.is_none() && h.app.confirm.is_none() && !h.app.quit_confirmed, "Cancel: nothing waits to close");
    assert!(!h.window_log.iter().any(|c| matches!(c, egui::ViewportCommand::Close)));
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "old\n");
    assert_eq!(h.app.editor.files.len(), 2);
}

/// Esc closes whatever is open on top: every dialog, menu and picker.
#[test]
fn every_dialog_closes_on_escape() {
    use crate::app::App;
    type IsOpen = fn(&App) -> bool;
    let dialogs: [(&str, IsOpen); 9] = [
        ("help", |a| a.help.is_some()),
        ("keys", |a| a.keys_window.is_some()),
        ("privacy", |a| a.records_window.is_some()),
        ("serial", |a| a.serial_dialog.is_some()),
        ("palette", |a| a.palette.is_some()),
        ("connect", |a| a.connect.is_some()),
        ("open_memo", |a| a.memo_picker),
        ("fonts", |a| a.font_menu.is_some()),
        ("theme", |a| a.theme_menu.is_some()),
    ];
    let mut stayed = Vec::new();
    for (cmd, open) in dialogs {
        let mut h = Harness::new(json!({}));
        h.command(cmd);
        h.frames(2);
        assert!(open(&h.app), "{cmd} opens");
        h.key(Key::Escape, Modifiers::NONE);
        h.frames(2);
        if open(&h.app) {
            stayed.push(cmd);
        }
    }
    assert!(stayed.is_empty(), "Esc left these open: {stayed:?}");
}

fn files_on(h: &mut Harness, names: &[&str]) -> (std::path::PathBuf, String) {
    let root = h.dir.join("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    for n in names {
        std::fs::write(root.join(n), format!("{n} text")).unwrap();
    }
    let dir = root.display().to_string().replace('\\', "/");
    h.app.set_root("local", &dir);
    assert!(h.wait_for(20.0, |h| h.has("sub")), "the folder is listed");
    (root, dir)
}

/// A file dragged onto a folder row is copied into it, and stays where it was.
#[test]
fn dragging_a_file_onto_a_folder_copies_it() {
    let mut h = Harness::new(json!({"sidebar": true, "follow_cwd": false}));
    let (root, _) = files_on(&mut h, &["a.txt"]);
    let (from, to) = (h.at_exact("a.txt"), h.at_exact("sub"));
    h.drag(from, to);
    assert!(h.wait_for(20.0, |_| root.join("sub/a.txt").is_file()), "copied into the folder");
    assert_eq!(std::fs::read_to_string(root.join("sub/a.txt")).unwrap(), "a.txt text");
    assert!(root.join("a.txt").is_file(), "the original stays");
    assert!(h.app.file_drag.is_none());
}

/// The eye menu in Files shows and hides hidden files, and is kept.
#[test]
fn files_show_hidden_from_the_eye_menu() {
    let mut h = Harness::new(json!({"sidebar": true, "follow_cwd": false}));
    files_on(&mut h, &[".secret", "seen.txt"]);
    assert!(h.has("seen.txt") && !h.has(".secret"), "hidden files are not listed at first");
    h.click_text("\u{f06e}");
    h.click_text("Hidden files");
    assert!(h.app.settings.show_hidden);
    assert!(h.wait_for(20.0, |h| h.has(".secret")), "now listed");
    h.click_text("Hidden files");
    assert!(h.wait_for(20.0, |h| !h.has(".secret") && h.has("seen.txt")), "and hidden again");
}

/// A new memo takes typing at once, and its Save button writes it.
#[test]
fn a_memo_is_typed_and_saved() {
    let mut h = Harness::new(json!({}));
    h.command("new_memo");
    h.frames(3);
    let path = h.app.editor.files.last().expect("a memo is open").path.clone();
    h.type_text("remember this");
    h.frames(2);
    assert_eq!(h.app.editor.files.last().unwrap().text, "remember this", "typed into the memo");
    h.click_text("Save");
    assert!(h.wait_for(20.0, |_| std::fs::read_to_string(&path).is_ok_and(|t| t == "remember this")), "on disk");
}

/// The composer sends its line on Enter and closes.
#[test]
fn the_composer_sends_on_enter() {
    let mut h = Harness::new(json!({}));
    let pane = h.app.active_pane_id().unwrap();
    h.command("composer");
    h.type_text("echo hi");
    h.key(Key::Enter, Modifiers::NONE);
    h.frames(2);
    assert!(h.app.panes[&pane].composer.is_none(), "closed after sending");
}

/// Side panels and editors can be split into more, each a panel of its own.
#[test]
fn panels_split() {
    use crate::app::Region;
    let mut h = Harness::new(json!({"sidebar": true, "follow_cwd": false}));
    let file = h.dir.join("a.txt");
    std::fs::write(&file, "x").unwrap();
    h.app.open_file("local", &file.display().to_string().replace('\\', "/"), 1);
    h.frames(3);
    let count = |h: &Harness, region: Region| h.app.panel_rects.keys().filter(|p| p.region == region).count();
    assert_eq!((count(&h, Region::Side), count(&h, Region::Editor)), (1, 1));
    h.command("split_side_down");
    h.command("split_editor_right");
    h.frames(2);
    assert_eq!((count(&h, Region::Side), count(&h, Region::Editor)), (2, 2));
    let rects: Vec<egui::Rect> = h.app.panel_rects.values().copied().collect();
    for (i, a) in rects.iter().enumerate() {
        assert!(a.width() > 40.0 && a.height() > 40.0, "every panel has room: {a:?}");
        for b in &rects[i + 1..] {
            assert!(a.intersect(*b).area() < 1.0, "panels don't overlap: {a:?} {b:?}");
        }
    }
}

/// Procs lists this PC's processes (it said "remote only" before).
#[test]
fn procs_lists_this_pc() {
    use crate::app::SideTab;
    let mut h = Harness::new(json!({"sidebar": true, "session": [{"t": "Pane", "host": "local", "cwd": null}]}));
    h.app.side.tab = SideTab::Procs;
    h.app.settings.side_tabs.insert(0, SideTab::Procs);
    assert!(h.wait_for(30.0, |h| h.shows("PID") && h.app.side.procs.as_ref().is_some_and(|p| p.as_ref().is_ok_and(|rows| rows.len() > 5))), "the list arrived: {:?}", h.app.side.procs.as_ref().map(|p| p.as_ref().map(|r| r.len())));
    assert!(!h.shows("remote agent"), "no \"connect to an ssh host\" note");
    let me = std::process::id().to_string();
    h.app.side.procs_filter = me.clone();
    h.frames(3);
    assert!(h.has(&me), "this very process is in the list, by its pid");
}

/// Two Files panels side by side: a file's "Send to" offers the folder the
/// other panel shows, and copies it there.
#[test]
fn send_to_the_other_files_panel() {
    use crate::app::SideTab;
    let mut h = Harness::new(json!({"sidebar": true, "follow_cwd": false}));
    let (root, dir) = files_on(&mut h, &["a.txt"]);
    let other = format!("{dir}/sub");
    h.command("split_side_down");
    let n = *h.app.side_views.keys().max().expect("the new panel has a view of its own");
    h.app.settings.side_tabs.insert(n, SideTab::Files);
    h.app.side_views.get_mut(&n).unwrap().root = Some(("local".into(), other.clone()));
    h.frames(4);
    let row = h.at_exact("a.txt");
    h.right_click(row);
    h.click_text("Send to");
    h.frames(2);
    assert!(h.shows(&format!("local:{other}")), "the other panel's folder is offered; shown: {:?}", h.texts.iter().map(|t| &t.0).filter(|t| t.contains("local") || t.contains("open another")).collect::<Vec<_>>());
    h.click_text(&format!("local:{other}"));
    assert!(h.wait_for(20.0, |_| root.join("sub/a.txt").is_file()), "copied there");
    assert_eq!(std::fs::read_to_string(root.join("sub/a.txt")).unwrap(), "a.txt text");
}

/// Not a test of right or wrong: how long a frame takes in a few heavy
/// situations, to find what is worth making faster. Run with
/// `cargo test -p weft frame_times -- --ignored --nocapture`.
#[test]
#[ignore]
fn frame_times() {
    fn per_frame(h: &mut Harness, n: usize) -> f64 {
        h.frames(5);
        let t = std::time::Instant::now();
        h.frames(n);
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    }
    let p = |f: &std::path::Path| f.display().to_string().replace('\\', "/");
    let mut h = Harness::new(json!({"sidebar": false}));
    println!("idle terminal                  {:7.2} ms/frame", per_frame(&mut h, 60));

    // A long scrollback.
    let pane = h.app.active_pane_id().unwrap();
    {
        let pn = h.app.panes.get_mut(&pane).unwrap();
        let mut text = String::new();
        for i in 0..50_000 {
            text.push_str(&format!("line {i} of output, some words to fill the row and wrap now and then\r\n"));
        }
        pn.parser.advance(&mut pn.term, text.as_bytes());
    }
    println!("50 000 lines of scrollback     {:7.2} ms/frame", per_frame(&mut h, 60));
    h.command("find");
    h.type_text("line 4999");
    println!("  with a scrollback search     {:7.2} ms/frame", per_frame(&mut h, 30));
    h.key(Key::Escape, Modifiers::NONE);

    // A big source file, and a big markdown file with tables.
    let src = h.dir.join("big.rs");
    std::fs::write(&src, (0..8000).map(|i| format!("fn f{i}(a: u32) -> u32 {{ a + {i} }} // comment {i}\n")).collect::<String>()).unwrap();
    h.app.open_file("local", &p(&src), 1);
    assert!(h.wait_for(30.0, |h| h.app.editor.files.iter().all(|f| !f.loading)));
    println!("8 000-line source file         {:7.2} ms/frame", per_frame(&mut h, 30));
    {
        // Where that goes: the parts of laying the file out, one by one.
        let text = h.app.editor.files[0].text.clone();
        let font = egui::FontId::monospace(14.0);
        let state = Default::default();
        let job = std::sync::Arc::new(crate::highlight::layout(&text, "rs", font.clone(), &[], None, &state, &h.ctx));
        let time = |what: &str, f: &mut dyn FnMut()| {
            let t = std::time::Instant::now();
            for _ in 0..20 {
                f();
            }
            println!("    {what:34} {:7.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
        };
        println!("    sections in the job: {}", job.sections.len());
        time("hash of the text", &mut || {
            use std::hash::{Hash, Hasher};
            let mut s = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut s);
            std::hint::black_box(s.finish());
        });
        time("clone of the job", &mut || {
            std::hint::black_box((*job).clone());
        });
        time("layout_job (cached galley)", &mut || {
            std::hint::black_box(h.ctx.fonts(|f| f.layout_job((*job).clone())));
        });
        time("line numbers text", &mut || {
            let nums: String = (1..=8000).map(|n| format!("{n:>4}\n")).collect();
            std::hint::black_box(nums);
        });
        time("count of newlines", &mut || {
            std::hint::black_box(text.matches('\n').count());
        });
    }
    let md = h.dir.join("big.md");
    let mut text = String::new();
    for i in 0..300 {
        text.push_str(&format!("## Section {i}\n\nSome *text* with `code` and a [link](x).\n\n| a | b | c |\n|---|---|---|\n| {i} | two | three |\n| x | y | z |\n\n"));
    }
    std::fs::write(&md, text).unwrap();
    h.app.open_file("local", &p(&md), 1);
    assert!(h.wait_for(30.0, |h| h.app.editor.files.iter().all(|f| !f.loading)));
    println!("markdown, 300 tables           {:7.2} ms/frame", per_frame(&mut h, 30));

    // Files: a folder of 3 000 entries; then the other side tabs.
    let big = h.dir.join("many");
    std::fs::create_dir_all(&big).unwrap();
    for i in 0..3000 {
        std::fs::write(big.join(format!("file-{i:04}.txt")), "needle\n").unwrap();
    }
    h.app.settings.sidebar = true;
    h.app.side.follow = false;
    h.app.set_root("local", &p(&big));
    assert!(h.wait_for(30.0, |h| h.has("file-0000.txt")));
    println!("Files, 3 000 entries           {:7.2} ms/frame", per_frame(&mut h, 30));
    h.app.side.tab = crate::app::SideTab::History;
    h.app.settings.side_tabs.insert(0, crate::app::SideTab::History);
    println!("History tab                    {:7.2} ms/frame", per_frame(&mut h, 30));
    h.app.side.tab = crate::app::SideTab::Procs;
    h.app.settings.side_tabs.insert(0, crate::app::SideTab::Procs);
    println!("Procs tab (no agent)           {:7.2} ms/frame", per_frame(&mut h, 30));
    println!("    procs: {:?}; toasts {}; texts on screen {}", h.app.side.procs.as_ref().map(|p| p.as_ref().map(|r| r.len()).map_err(|e| e.clone())), h.app.toasts.len(), h.texts.len());
    h.app.editor.visible = false;
    println!("    the same, editor hidden    {:7.2} ms/frame", per_frame(&mut h, 30));
    h.app.settings.sidebar = false;
    println!("    and the side panel hidden  {:7.2} ms/frame", per_frame(&mut h, 30));
    drop(h);
    let mut h = Harness::new(json!({"sidebar": true, "session": [{"t": "Pane", "host": "local", "cwd": null}]}));
    h.app.side.tab = crate::app::SideTab::Procs;
    h.app.settings.side_tabs.insert(0, crate::app::SideTab::Procs);
    assert!(h.wait_for(30.0, |h| h.app.side.procs.as_ref().is_some_and(|p| p.is_ok())));
    println!("Procs tab, this PC ({} rows)  {:7.2} ms/frame", h.app.side.procs.as_ref().unwrap().as_ref().unwrap().len(), per_frame(&mut h, 60));
}

/// A file in the editor is laid out when it changes, not once a frame (a
/// big source file cost 12 ms every frame). New fonts (the scale changed)
/// lay it out again: what was kept points into the old font atlas.
#[test]
fn the_editor_lays_a_file_out_once() {
    use crate::ui_panels::editor_layouts;
    let mut h = Harness::new(json!({}));
    for (name, text) in [("big.rs", "fn main() {\n    let x = 1;\n}\n".repeat(400)), ("big.md", "# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\ntext\n".repeat(60))] {
        let file = h.dir.join(name);
        std::fs::write(&file, text).unwrap();
        let path = file.display().to_string().replace('\\', "/");
        h.app.open_file("local", &path, 1);
        assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().all(|f| !f.loading)), "opened");
        // Let the highlighting and the table widths settle.
        // (The colours arrive from a thread all tests share: wait until a
        // stretch of frames passes without them changing anything.)
        let mut settled = usize::MAX;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            h.frames(10);
            settled = editor_layouts();
            h.frames(30);
            if editor_layouts() == settled {
                break;
            }
        }
        assert_eq!(editor_layouts(), settled, "{name}: 30 frames with nothing changed lay nothing out");
        // A change is laid out, and shows.
        let i = h.app.editor.files.iter().position(|f| f.path == path).unwrap();
        h.app.editor.files[i].text.insert_str(0, "changed_here ");
        h.frames(3);
        let edited = editor_layouts();
        assert!(edited > settled, "{name}: an edit is laid out");
        assert!(h.shows("changed_here"), "{name}: and drawn");
        // New fonts: laid out again.
        let scale = h.ctx.pixels_per_point();
        h.ctx.set_pixels_per_point(scale * 1.25);
        h.frames(3);
        assert!(editor_layouts() > edited, "{name}: new fonts, a new layout");
        h.ctx.set_pixels_per_point(scale);
        h.frames(3);
        h.app.close_files(i, crate::app::CloseWhich::This);
    }
}

/// The timed save writes settings.json only when something changed (it
/// wrote it every ten seconds regardless, inside a frame); an asked-for
/// save always writes.
#[test]
fn the_timed_save_writes_only_changes() {
    let mut h = Harness::new(json!({}));
    let file = h.dir.join("settings.json");
    h.app.save_settings();
    std::fs::remove_file(&file).unwrap();
    h.app.autosave_settings();
    assert!(!file.exists(), "nothing changed: nothing written");
    h.command("new_tab");
    h.app.autosave_settings();
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).expect("the new tab was written")).unwrap();
    assert_eq!(saved["session"].as_array().map(|s| s.len()), Some(2));
    std::fs::remove_file(&file).unwrap();
    h.app.save_settings();
    assert!(file.exists(), "asked for: written");
}

/// A long markdown document: tables scrolled out of view are still there
/// when scrolled to (they are skipped while off screen).
#[test]
fn tables_far_down_a_document_are_drawn_when_reached() {
    let mut h = Harness::new(json!({}));
    let file = h.dir.join("long.md");
    let mut text = String::new();
    for i in 0..80 {
        text.push_str(&format!("## Section {i}\n\n| key | value |\n|---|---|\n| cell{i}a | cell{i}b |\n\n"));
    }
    std::fs::write(&file, text).unwrap();
    let path = file.display().to_string().replace('\\', "/");
    h.app.open_file("local", &path, 1);
    assert!(h.wait_for(20.0, |h| h.has("cell0a")), "the first table is drawn");
    assert!(!h.has("cell79a"), "the last one is far below");
    // To the end of the document.
    let i = h.app.editor.files.iter().position(|f| f.path == path).unwrap();
    let lines = h.app.editor.files[i].text.lines().count();
    h.app.editor.files[i].goto = Some(lines);
    assert!(h.wait_for(20.0, |h| h.has("cell79a") && h.has("cell79b")), "the last table, once scrolled to");
    assert!(!h.has("cell0a"), "and the first is out of view");
}
