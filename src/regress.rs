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
    // And on Windows which console host the terminals run on: Windows'
    // own, with nothing set and none beside the test binary.
    if cfg!(windows) {
        assert_eq!(h.app.console_host.what, "Windows' own (conhost.exe)");
        assert!(h.shows("console host: Windows' own"), "About names the console host");
        h.command("display_test");
        h.frames(2);
        assert!(h.shows("console host: Windows' own"), "and so does Display test");
    }
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
    h.app.after_save = Some(AfterSave { ask: Ask::Quit { restart: false }, files: vec![key(&a)], asked: true });
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
    let dialogs: [(&str, IsOpen); 10] = [
        ("help", |a| a.help.is_some()),
        ("keys", |a| a.keys_window.is_some()),
        ("privacy", |a| a.records_window.is_some()),
        ("serial", |a| a.serial_dialog.is_some()),
        ("palette", |a| a.palette.is_some()),
        ("connect", |a| a.connect.is_some()),
        ("open_memo", |a| a.memo_picker),
        ("fonts", |a| a.font_menu.is_some()),
        ("theme", |a| a.theme_menu.is_some()),
        ("display_test", |a| a.display_test.is_some()),
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
    assert!(h.wait_for(90.0, |h| h.shows("PID") && h.app.side.procs.as_ref().is_some_and(|p| p.as_ref().is_ok_and(|rows| rows.len() > 5))), "the list arrived: {:?}", h.app.side.procs.as_ref().map(|p| p.as_ref().map(|r| r.len())));
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

/// weft showing what a program printed: Claude Code's logo (orange, its eye
/// row on a black background), every kind of block character, and lines
/// whose last `|` must stand in one column.
fn logo_harness(font: &str) -> Harness {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let script = std::env::temp_dir().join(format!("weft-logo-{}-{}.py", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    std::fs::write(
        &script,
        r#"import time
E = chr(27)
O = E + "[38;2;215;119;87m"
K = E + "[48;2;0;0;0m"
R = E + "[0m"
print()
print(" " + O + chr(0x2590) + K + chr(0x259b) + chr(0x2588) * 3 + chr(0x259c) + R + O + chr(0x258c) + R + "   Claude Code v2.1.312")
print(O + chr(0x259d) + chr(0x259c) + chr(0x2588) * 5 + chr(0x259b) + chr(0x2598) + R + "  New sessions")
print(O + "  " + chr(0x2598) * 2 + " " + chr(0x259d) * 2 + R + "    2 awaiting input")
print()
print("eighths up   " + "".join(chr(c) for c in range(0x2581, 0x2589)) + "|")
print("eighths left " + "".join(chr(c) for c in range(0x258f, 0x2587, -1)) + "|")
print("shades       " + chr(0x2591) * 2 + chr(0x2592) * 2 + chr(0x2593) * 2 + chr(0x2588) * 2 + "|")
print("bar          " + E + "[48;2;200;0;0m" + chr(0x2588) * 8 + R + "|")
print("MMMMMMMMMMMMMMMMMMMMM|")
print("iiiiiiiiiiiiiiiiiiiii|")
print("0123456789abcdefghijk|")
print("v2.1.312  a.b,c;d:e!?|")
print(chr(0xd55c) * 10 + " |")
print("done", flush=True)
time.sleep(600)
"#,
    )
    .unwrap();
    let prof = json!({"name": "t", "kind": "custom", "pty": ["python", "-u", script.display().to_string()], "agent": ["python", "-c", "0"]});
    let mut h = Harness::new(json!({"profiles": [prof], "term_font": font, "icon_font": "builtin:d2coding"}));
    // (Letter by letter in a font that isn't fixed-width: joined up.)
    assert!(h.wait_for(30.0, |h| h.texts.iter().map(|t| t.0.as_str()).collect::<String>().contains("done")), "the program's output is on screen ({font})");
    h.frames(3);
    h
}

fn test_fonts() -> Vec<(&'static str, String)> {
    let mut fonts = vec![("d2coding", "builtin:d2coding".to_string()), ("hack", "builtin:hack".to_string()), ("jetbrains", "builtin:jetbrains-mono".to_string())];
    // One that is not fixed-width, where the system has it.
    for (name, file) in [("arial", "C:/Windows/Fonts/arial.ttf"), ("dejavu", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")] {
        if std::path::Path::new(file).is_file() {
            fonts.push((name, file.to_string()));
        }
    }
    fonts
}

/// In every font: the logo's only black is its two eyes (a quarter of a
/// cell each), not a band across it; a row of full blocks hides its
/// background entirely; and the lines' last `|` stand in one column. In
/// pixels, as a window would show them.
#[test]
fn the_logo_and_the_grid_look_right_in_every_font() {
    for (name, font) in test_fonts() {
        let h = logo_harness(&font);
        assert_eq!(h.app.settings.term_font, font, "{name} is the font in use");
        let img = h.shot();
        h.save_shot(&format!("logo-{name}"));
        let (cw, ch) = h.ctx.fonts(|f| (crate::theme::cell_width(f, &crate::theme::term_font(h.app.settings.font_size)), f.row_height(&crate::theme::term_font(h.app.settings.font_size)).ceil()));
        // Black: the eyes' cells' background, and nothing else on screen.
        let black: Vec<(u32, u32)> = img.enumerate_pixels().filter(|(_, _, p)| p.0[0] < 12 && p.0[1] < 12 && p.0[2] < 12).map(|(x, y, _)| (x, y)).collect();
        let quarter = (cw / 2.0) * (ch / 2.0);
        assert!(!black.is_empty(), "{name}: the eyes are there");
        let (x0, x1) = (black.iter().map(|p| p.0).min().unwrap(), black.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (black.iter().map(|p| p.1).min().unwrap(), black.iter().map(|p| p.1).max().unwrap());
        assert!((y1 - y0) as f32 <= ch / 2.0 + 1.0, "{name}: black is half a row high at most (a band was a whole row): {} px, row {ch}", y1 - y0 + 1);
        assert!((x1 - x0) as f32 <= cw * 4.5 + 1.0, "{name}: from one eye to the other: {} px, cell {cw}", x1 - x0 + 1);
        let area = black.len() as f32;
        assert!(area > quarter * 1.5 && area < quarter * 2.5, "{name}: two quarter cells of black, {area} px against {quarter} each");
        // Between the eyes it is the logo's orange, not black.
        let mid = img.get_pixel((x0 + x1) / 2, (y0 + y1) / 2).0;
        assert!(mid[0] > 180 && mid[1] > 80 && mid[2] < 140, "{name}: orange between the eyes: {mid:?}");
        // The bar's red background is covered completely.
        let red = img.pixels().filter(|p| p.0[0] > 150 && p.0[1] < 40 && p.0[2] < 40).count();
        assert_eq!(red, 0, "{name}: no red shows around the full blocks");
        // The last `|` of each line: one column.
        // (A run of letters is one text, each letter a cell wide; in a font
        // that isn't fixed-width every letter is a text of its own.)
        let bars: Vec<f32> = h.texts.iter().filter(|(t, r)| r.top() > 100.0 && t.contains('|')).map(|(t, r)| r.left() + t.chars().position(|c| c == '|').unwrap() as f32 * cw).collect();
        assert!(bars.len() >= 8, "{name}: the lines are there: {}", bars.len());
        let (lo, hi) = (bars.iter().cloned().fold(f32::MAX, f32::min), bars.iter().cloned().fold(f32::MIN, f32::max));
        assert!(hi - lo <= cw * 0.6, "{name}: the lines end in one column: {lo} to {hi}, cell {cw}");
    }
}

/// Display test: the window of test screens. Its first page shows the logo
/// with only its eyes black, in pixels; the other pages open and show
/// their lines; Help has a link to it.
#[test]
fn the_display_test_window_shows_its_pages() {
    let mut h = Harness::new(json!({}));
    h.command("help");
    h.frames(2);
    h.click_text("Display test\u{2026}");
    h.frames(3);
    assert_eq!(h.app.display_test, Some(0), "opened from Help, on its first page");
    h.app.help = None;
    h.frames(3);
    assert!(h.shows("only the two eyes may be black") && h.shows("font: "));
    let img = h.shot();
    h.save_shot("display-test-blocks");
    let (cw, ch) = h.ctx.fonts(|f| (crate::theme::cell_width(f, &crate::theme::term_font(h.app.settings.font_size)), f.row_height(&crate::theme::term_font(h.app.settings.font_size)).ceil()));
    let black = img.pixels().filter(|p| p.0[0] < 12 && p.0[1] < 12 && p.0[2] < 12).count() as f32;
    let quarter = (cw / 2.0) * (ch / 2.0);
    assert!(black > quarter * 1.5 && black < quarter * 2.5, "two quarter cells of black: {black} px against {quarter} each");
    assert_eq!(img.pixels().filter(|p| p.0[0] > 150 && p.0[1] < 40 && p.0[2] < 40).count(), 0, "no red around the full blocks");
    for (page, line) in [("Colours", "256 colours:"), ("Columns", "Every line ends with a bar"), ("Boxes", "a tree:")] {
        h.click_text(page);
        h.frames(2);
        assert!(h.shows(line), "{page} shows its lines");
        h.save_shot(&format!("display-test-{}", page.to_lowercase()));
    }
    // Columns: the last | of every line in one column.
    h.click_text("Columns");
    h.frames(2);
    let bars: Vec<f32> = h.texts.iter().filter(|(t, _)| t.trim_end().ends_with('|')).map(|(t, r)| r.left() + t.chars().position(|c| c == '|').unwrap() as f32 * cw).collect();
    assert!(bars.len() >= 12, "the lines are there: {}", bars.len());
    let (lo, hi) = (bars.iter().cloned().fold(f32::MAX, f32::min), bars.iter().cloned().fold(f32::MIN, f32::max));
    assert!(hi - lo <= cw * 0.6, "one column: {lo} to {hi}, cell {cw}");
}

/// Frames and trees drawn with box characters are whole, in pixels: a
/// vertical line runs unbroken through the rows it crosses, a horizontal
/// one through its cells, corners meet, and a double frame is two closed
/// lines. (A font's own lines stop short of the row above and below.)
#[test]
fn box_drawing_lines_join() {
    let mut h = Harness::new(json!({}));
    h.command("display_test");
    h.frames(2);
    h.click_text("Boxes");
    h.frames(3);
    let img = h.shot();
    h.save_shot("display-test-boxes");
    let (cw, ch) = h.ctx.fonts(|f| (crate::theme::cell_width(f, &crate::theme::term_font(h.app.settings.font_size)), f.row_height(&crate::theme::term_font(h.app.settings.font_size)).ceil()));
    // Where the test terminal starts: its first row's text is drawn at the
    // first cell's corner.
    let name = h.texts.iter().find(|(t, _)| t.contains("name")).expect("the table's first row").1;
    let (x0, row1) = (name.left() - cw, name.top());
    let ink = |x: f32, y: f32| {
        let p = img.get_pixel(x.round() as u32, y.round() as u32).0;
        p[0] as u32 + p[1] as u32 + p[2] as u32 > 300
    };
    // Any ink within a pixel or two across a line's width.
    let on_line_v = |x: f32, y: f32| (-2..=2).any(|dx| ink(x + dx as f32, y));
    let on_line_h = |x: f32, y: f32| (-2..=2).any(|dy| ink(x, y + dy as f32));
    // The table's left edge: column 0, from the middle of its top border
    // row (one above "name") to the middle of its bottom border (row 4).
    let left = x0 + cw / 2.0;
    // (To a pixel short of the bottom corner's middle: its line ends there.)
    let (top, bottom) = (row1 - ch / 2.0, row1 + 3.0 * ch + ch / 2.0 - 1.5);
    let mut y = top;
    while y <= bottom {
        assert!(on_line_v(left, y), "the table's left edge is unbroken at y {y} (top {top}, bottom {bottom})");
        y += 1.0;
    }
    // Its top border: from the first corner to the last (21 cells).
    let mut x = left;
    while x <= x0 + 20.0 * cw + cw / 2.0 {
        assert!(on_line_h(x, top), "the table's top border is unbroken at x {x}");
        x += 1.0;
    }
    // The double frame beside it (columns 23 to 35): its left edge is two
    // lines with a gap, each unbroken over the rows.
    // (In the rows of text: where the middle line crosses, the inner line
    // opens to let it in.)
    let dx = x0 + 23.0 * cw + cw / 2.0;
    for row in [row1, row1 + 2.0 * ch] {
        let mut y = row;
        while y < row + ch {
            let cols: Vec<bool> = (-4..=4).map(|o| ink(dx + o as f32, y)).collect();
            let runs = cols.windows(2).filter(|w| !w[0] && w[1]).count() + cols[0] as usize;
            assert_eq!(runs, 2, "the double frame's edge is two lines at y {y}: {cols:?}");
            y += 1.0;
        }
    }
    // And between the rows of text the outer line goes on while the inner
    // one opens: never no line at all.
    let mut y = row1;
    while y < row1 + 3.0 * ch {
        assert!((-4..=4).any(|o| ink(dx + o as f32, y)), "the double frame's outer edge is unbroken at y {y}");
        y += 1.0;
    }
}

/// With Help open beside it, a terminal that is clicked has the keyboard
/// again: Esc is the shell's (vim's), and Help stays. A composer on top
/// closes first, not the Help behind it.
#[test]
fn escape_goes_to_what_has_the_keyboard() {
    let mut h = Harness::new(json!({}));
    let pane = h.app.active_pane_id().unwrap();
    h.command("help");
    h.frames(2);
    let in_terminal = |h: &Harness| h.ctx.memory(|m| m.focused()) == Some(egui::Id::new(("pane", pane)));
    assert!(!in_terminal(&h), "Help has the keyboard when it opens, not the terminal");
    // A click in the terminal, away from the window.
    h.click(egui::pos2(40.0, 700.0));
    h.frames(2);
    assert!(in_terminal(&h), "the terminal has it back");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.help.is_some(), "Esc went to the shell: Help stays open");
    // A composer over it: Esc closes the composer, then Help is still there.
    h.command("composer");
    h.type_text("x");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.panes[&pane].composer.is_none() && h.app.help.is_some(), "the composer closed, not Help");
    // The palette over it likewise.
    h.command("palette");
    h.frames(2);
    h.key(Key::Escape, Modifiers::NONE);
    h.frames(2);
    assert!(h.app.palette.is_none() && h.app.help.is_some(), "the palette closed, not Help");
}

/// A selection in one table stays in that table when the table above it
/// scrolls out of view (it was counted among the tables on screen, and
/// jumped to the next one).
#[test]
fn a_table_selection_stays_with_its_table() {
    let mut h = Harness::new(json!({}));
    h.size = egui::vec2(1200.0, 1500.0);
    let file = h.dir.join("three.md");
    let gap = "text\n\n".repeat(14);
    std::fs::write(&file, format!("| a1 | a2 |\n|---|---|\n| a3 | a4 |\n\n{gap}| b1 | b2 |\n|---|---|\n| b3 | b4 |\n\n{gap}| c1 | c2 |\n|---|---|\n| c3 | c4 |\n\n{gap}end\n")).unwrap();
    let path = file.display().to_string().replace('\\', "/");
    h.app.open_file("local", &path, 1);
    assert!(h.wait_for(20.0, |h| h.shows("Read") && h.app.editor.files.iter().all(|f| !f.loading)), "opened");
    h.click_text("Read");
    assert!(h.wait_for(20.0, |h| h.has("a3") && h.has("b3")), "the first two tables are on screen");
    let cell = h.at_exact("b3");
    h.double_click(cell);
    h.frame(vec![Event::Copy]);
    assert_eq!(h.copied.take().as_deref(), Some("b3"));
    // A smaller window, scrolled so the first table is above it.
    h.size = egui::vec2(1200.0, 420.0);
    let i = h.app.editor.files.iter().position(|f| f.path == path).unwrap();
    let line = h.app.editor.files[i].text.lines().position(|l| l.contains("b1")).unwrap() + 3;
    h.app.editor.files[i].goto = Some(line);
    assert!(h.wait_for(20.0, |h| h.has("b3") && !h.has("a3")), "the first table is out of view, the second still there");
    h.frame(vec![Event::Copy]);
    assert_eq!(h.copied.take().as_deref(), Some("b3"), "still the second table's cell");
}

/// The editor keeps a few laid-out files, not every one ever opened.
#[test]
fn laid_out_files_are_not_kept_for_ever() {
    let mut h = Harness::new(json!({}));
    for n in 0..7 {
        let file = h.dir.join(format!("f{n}.rs"));
        std::fs::write(&file, format!("fn f{n}() {{}}\n")).unwrap();
        h.app.open_file("local", &file.display().to_string().replace('\\', "/"), 1);
        assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().all(|f| !f.loading)), "opened");
        h.frames(3);
    }
    let kept = crate::ui_panels::kept_galleys(&h.ctx);
    assert!((1..=4).contains(&kept), "seven files were shown, {kept} layouts are kept");
}

/// weft with a terminal whose program is a python script: it prints what
/// `body` says (with `weft(...)` to send weft an event, as the shell
/// helpers do), then "ready-marker", then writes every line it is sent to
/// a file. What weft types into a terminal can be read back from it.
fn script_harness(body: &str, more: serde_json::Value) -> (Harness, std::path::PathBuf) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let stem = std::env::temp_dir().join(format!("weft-script-{}-{n}", std::process::id()));
    let (script, received) = (stem.with_extension("py"), stem.with_extension("txt"));
    let _ = std::fs::remove_file(&received);
    let head = "import base64, json, sys, time\nE = chr(27)\ndef weft(**m):\n    sys.stdout.write(E + ']7777;' + base64.b64encode(json.dumps(m).encode()).decode() + chr(7))\n    sys.stdout.flush()\n";
    let tail = format!("print('ready-marker', flush=True)\nout = open(r'{}', 'a', encoding='utf-8')\nwhile True:\n    line = sys.stdin.readline()\n    if not line:\n        break\n    out.write(line)\n    out.flush()\n    after(line)\n", received.display());
    let after = if body.contains("def after(") { "" } else { "def after(line):\n    pass\n" };
    std::fs::write(&script, format!("{head}{after}{body}\n{tail}")).unwrap();
    let prof = json!({"name": "t", "kind": "custom", "pty": ["python", "-u", script.display().to_string()], "agent": ["python", "-c", "0"]});
    let mut settings = json!({"profiles": [prof]});
    for (k, v) in more.as_object().unwrap() {
        settings[k] = v.clone();
    }
    let mut h = Harness::new(settings);
    assert!(h.wait_for(30.0, |h| h.texts.iter().map(|t| t.0.as_str()).collect::<String>().contains("ready-marker")), "the script's output is on screen");
    // … and all of it: the line's end can come a moment after its text,
    // and moves every row up by one.
    let pane = h.app.active_pane_id().unwrap();
    let mut seen = (h.app.panes[&pane].term.generation, std::time::Instant::now());
    h.wait_for(10.0, |h| {
        let now = h.app.panes[&pane].term.generation;
        if now != seen.0 {
            seen = (now, std::time::Instant::now());
        }
        seen.1.elapsed().as_millis() > 400
    });
    (h, received)
}

fn received(file: &std::path::Path) -> String {
    std::fs::read_to_string(file).unwrap_or_default()
}

/// What a shell helper prints becomes a widget in the terminal: a table
/// (sorted by a click on a column), a progress bar (updated in place), a
/// choice (a click types its number), a file list (open, show in Files),
/// and a note pops up.
#[test]
fn shell_events_become_widgets() {
    use crate::app::WidgetKind;
    let body = r#"
def after(line):
    weft(t="widget", id="p1", kind="progress", rows=1, pct=100, label="done")
# (A pause after each, as when each is a command of its own; without one:
# widgets_printed_in_one_go_each_get_their_lines.)
weft(t="widget", id="t1", kind="table", rows=3, columns=["item", "count"], data=[["beta", "20"], ["alpha", "3"]])
print("item count"); print("beta 20"); print("alpha 3"); time.sleep(0.3)
weft(t="widget", id="p1", kind="progress", rows=1, pct=40, label="copying")
print("40% copying"); time.sleep(0.3)
weft(t="widget", id="c1", kind="choose", rows=3, options=["yes-please", "no-thanks"])
print("choose"); print("1) yes-please"); print("2) no-thanks"); time.sleep(0.3)
weft(t="widget", id="f1", kind="files", rows=3, dir="/d", entries=[
    {"name": "subdir", "path": "/d/subdir", "dir": True, "size": 0, "mtime": 0, "mode": "drwxr-xr-x"},
    {"name": "notes.txt", "path": "/d/notes.txt", "dir": False, "size": 2048, "mtime": 0, "mode": "-rw-r--r--"}])
print("Name"); print("subdir"); print("notes.txt"); time.sleep(0.3)
weft(t="notify", msg="hello from the shell")
"#;
    let (mut h, file) = script_harness(body, json!({}));
    let pane = h.app.active_pane_id().unwrap();
    let kinds = |h: &Harness| -> Vec<&'static str> {
        h.app.panes[&pane].widgets.iter().map(|w| match w.kind {
            WidgetKind::Table { .. } => "table",
            WidgetKind::Progress { .. } => "progress",
            WidgetKind::Choose { .. } => "choose",
            WidgetKind::Files { .. } => "files",
        }).collect()
    };
    assert_eq!(kinds(&h), ["table", "progress", "choose", "files"]);
    // Each where its own text was printed, one below the other.
    let lines: Vec<(u64, usize)> = h.app.panes[&pane].widgets.iter().map(|w| (w.anchor, w.rows)).collect();
    assert_eq!(lines, [(0, 3), (3, 1), (4, 3), (7, 3)], "the widgets' lines");
    assert!(h.app.toasts.iter().any(|t| t.text.contains("hello from the shell")), "the note pops up");
    assert!(h.shows("40%  copying"), "the progress bar says how far");
    // The table: a click on a column sorts by it.
    assert!(h.has("item") && h.has("count") && h.has("beta") && h.has("alpha"));
    let before = (h.at_exact("beta").y, h.at_exact("alpha").y);
    assert!(before.0 < before.1, "as printed: beta first");
    let col = h.at_exact("item");
    h.click(col);
    h.frames(2);
    assert!(h.app.panes[&pane].widgets[0].sort.is_some_and(|s| s.0 == 0), "sorted by the first column");
    assert!(h.at_exact("alpha").y < h.at_exact("beta").y, "alpha now comes first");
    // The choice: a click types its number, and the row is marked.
    let opt = h.at_exact("no-thanks");
    h.click(opt);
    assert_eq!(h.app.panes[&pane].widgets[2].chosen, Some(1), "clicked at {opt:?}; texts there: {:?}", h.texts.iter().filter(|(_, r)| r.expand(4.0).contains(opt)).map(|t| &t.0).collect::<Vec<_>>());
    assert!(h.wait_for(20.0, |_| received(&file).lines().any(|l| l.trim() == "2")), "the program was sent \"2\": {:?}", received(&file));
    assert!(h.shows("chosen"));
    // The file list: sizes are readable, a double click opens a file, the
    // menu shows a folder in Files.
    assert!(h.has("Name") && h.has("Size") && h.has("Mode") && h.has("2.0K") && h.has("drwxr-xr-x"));
    let row = h.at_exact("notes.txt");
    h.double_click(row);
    h.frames(2);
    assert!(h.app.editor.files.iter().any(|f| f.host == "t" && f.path == "/d/notes.txt"), "opened in the editor");
    let row = h.at_exact("subdir");
    h.right_click(row);
    h.click_text("Show in Files panel");
    assert_eq!(h.app.side.root, Some(("t".to_string(), "/d/subdir".to_string())));
    // The program answers by updating its progress bar: the same widget.
    assert!(h.wait_for(20.0, |h| h.shows("100%  done")), "updated in place");
    assert_eq!(kinds(&h).len(), 4, "not a fifth widget");
}

/// The composer's line and a paste reach the program in the terminal as
/// typed; a file's path is inserted at the prompt from Files.
#[test]
fn what_weft_types_reaches_the_program() {
    let (mut h, file) = script_harness("", json!({}));
    h.command("composer");
    h.type_text("echo composed");
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_for(20.0, |_| received(&file).contains("echo composed")), "the composer's line: {:?}", received(&file));
    // Typed straight into the terminal.
    h.click(egui::pos2(300.0, 500.0));
    h.type_text("typed here");
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_for(20.0, |_| received(&file).contains("typed here")), "typing: {:?}", received(&file));
    // Pasted (Ctrl+V arrives as a paste event).
    h.frame(vec![Event::Paste("pasted text".into())]);
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_for(20.0, |_| received(&file).contains("pasted text")), "a paste: {:?}", received(&file));
}

/// The icon button with this tip on the row of the text `row`.
fn icon_on_row(h: &Harness, tip: &str, row: &str) -> egui::Pos2 {
    let y = h.at(row).y;
    h.icons.iter().find(|(t, r)| t.contains(tip) && (r.center().y - y).abs() < 12.0).unwrap_or_else(|| panic!("no \"{tip}\" on the row of \"{row}\"")).1.center()
}

/// Keyboard shortcuts: + takes the next key pressed as a key of that
/// command (Esc: none after all), the key then runs it, ↺ and Reset all
/// go back to the defaults.
#[test]
fn a_key_is_bound_by_pressing_it() {
    const CTRL_ALT: Modifiers = Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: true };
    let mut h = Harness::new(json!({}));
    h.command("keys");
    h.frames(2);
    let row = "New tab (same host)";
    // + then Esc: nothing bound, and the window stays.
    let plus = icon_on_row(&h, "add a key", row);
    h.click(plus);
    assert_eq!(h.app.keys_window.as_ref().and_then(|w| w.capture), Some("new_tab"));
    assert!(h.shows("press keys"), "the row says it is waiting");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.keys_window.as_ref().is_some_and(|w| w.capture.is_none()), "Esc ends the waiting, not the window");
    assert!(!h.app.settings.keys.contains_key("new_tab"));
    // + then Ctrl+Alt+K: bound beside the default; nothing reached the terminal.
    let plus = icon_on_row(&h, "add a key", row);
    h.click(plus);
    h.key(Key::K, CTRL_ALT);
    assert_eq!(h.app.settings.keys.get("new_tab").map(|k| k.as_slice()), Some(&["Ctrl+Shift+T".to_string(), "Ctrl+Alt+K".to_string()][..]));
    assert!(h.shows("changed (1)"));
    assert_eq!(h.app.tabs.len(), 1, "binding it did not run it");
    // The filter: only the changed ones.
    h.click_text("changed (1)");
    assert!(h.shows(row) && !h.shows("Split right"), "only the changed command is listed");
    h.click_text("changed (1)");
    // The search field finds commands by key.
    h.click_text("search commands or keys");
    h.type_text("ctrl+alt+k");
    assert!(h.shows(row) && !h.shows("Split right"), "found by its key");
    h.key(Key::A, Modifiers::COMMAND);
    h.key(Key::Backspace, Modifiers::NONE);
    // The new key works once the window is closed.
    h.key(Key::Escape, Modifiers::NONE);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.keys_window.is_none(), "Esc closes the window");
    h.key(Key::K, CTRL_ALT);
    assert_eq!(h.app.tabs.len(), 2, "Ctrl+Alt+K opens a tab");
    // ↺: back to the default; the key no longer does it.
    h.command("keys");
    h.frames(2);
    let back = icon_on_row(&h, "back to the default", row);
    h.click(back);
    assert!(!h.app.settings.keys.contains_key("new_tab"));
    h.key(Key::Escape, Modifiers::NONE);
    h.key(Key::K, CTRL_ALT);
    assert_eq!(h.app.tabs.len(), 2, "the key is no longer bound");
    // A key two commands share is counted as a clash; Reset all clears it.
    h.command("keys");
    h.frames(2);
    let plus = icon_on_row(&h, "add a key", row);
    h.click(plus);
    h.key(Key::D, CTRL_SHIFT);
    assert!(h.shows("clashes (2)"), "New tab and Split right share Ctrl+Shift+D");
    h.click_text("Reset all");
    assert!(h.app.settings.keys.is_empty());
    assert!(h.shows("clashes (0)") && h.shows("changed (0)"));
}

fn tab_hosts(h: &Harness) -> Vec<String> {
    h.app.tabs.iter().map(|t| h.app.panes[&t.active].host.clone()).collect()
}

/// Where the tab with this title is (the status bar names the host too).
fn tab_at(h: &Harness, title: &str) -> egui::Pos2 {
    h.texts.iter().find(|(t, r)| t == title && r.top() < 40.0).unwrap_or_else(|| panic!("no tab \"{title}\"")).1.center()
}

/// The tab bar: a click switches, a drag moves the tab, + opens one, a
/// tab's × and a middle click close it, the menu closes the others.
#[test]
fn the_tab_bar_switches_moves_and_closes() {
    let mut profiles = Vec::new();
    let mut session = Vec::new();
    for name in ["aa", "bb", "cc"] {
        profiles.push(crate::testkit::idle_profile(name));
        session.push(json!({"t": "Pane", "host": name, "cwd": null}));
    }
    let mut h = Harness::new(json!({"profiles": profiles, "session": session, "confirm_running": false}));
    assert_eq!(tab_hosts(&h), ["aa", "bb", "cc"]);
    // A click switches.
    let b = tab_at(&h, "bb");
    h.click(b);
    assert_eq!(h.app.active_tab, 1);
    // A drag past the last tab moves it there; it stays the active one.
    let (a, c) = (tab_at(&h, "aa"), tab_at(&h, "cc"));
    h.drag(a, c + egui::vec2(70.0, 0.0));
    assert_eq!(tab_hosts(&h), ["bb", "cc", "aa"]);
    assert_eq!(h.app.active_tab, 2, "the dragged tab is the active one");
    // … and back in front of the first.
    let (a, b) = (tab_at(&h, "aa"), tab_at(&h, "bb"));
    h.drag(a, b - egui::vec2(30.0, 0.0));
    assert_eq!(tab_hosts(&h), ["aa", "bb", "cc"]);
    // + opens a tab on the active tab's host, and shows it.
    h.click_icon("new tab on the same host");
    assert_eq!(tab_hosts(&h), ["aa", "bb", "cc", "aa"]);
    assert_eq!(h.app.active_tab, 3);
    // The active tab's ×.
    let last = h.texts.iter().filter(|(t, r)| t == "aa" && r.top() < 40.0).map(|(_, r)| *r).fold(None, |m: Option<egui::Rect>, r| Some(m.map_or(r, |m| if r.left() > m.left() { r } else { m }))).unwrap();
    h.click(egui::pos2(last.left() - 22.0 + 110.0 - 15.0, last.center().y));
    assert_eq!(tab_hosts(&h), ["aa", "bb", "cc"], "the × closed it");
    // A middle click closes a tab that is not the active one.
    let b = tab_at(&h, "bb");
    let active_before = h.app.panes[&h.app.tabs[h.app.active_tab].active].host.clone();
    h.frame(vec![Event::PointerMoved(b)]);
    h.frame(vec![Event::PointerButton { pos: b, button: egui::PointerButton::Middle, pressed: true, modifiers: Modifiers::NONE }]);
    h.frame(vec![Event::PointerButton { pos: b, button: egui::PointerButton::Middle, pressed: false, modifiers: Modifiers::NONE }]);
    h.frames(2);
    assert_eq!(tab_hosts(&h), ["aa", "cc"]);
    assert_eq!(h.app.panes[&h.app.tabs[h.app.active_tab].active].host, active_before, "the active tab is still the same one");
    // The menu: with two tabs, "Close to the left" of the first has nothing to close.
    h.command("new_tab");
    let n = h.app.tabs.len();
    let c = tab_at(&h, "cc");
    h.right_click(c);
    assert!(h.shows("Close others") && h.shows("Close to the right") && h.shows("Close to the left") && h.shows("Close all"));
    h.click_text("Close others");
    h.frames(2);
    assert_eq!(tab_hosts(&h), ["cc"], "of {n} tabs only the clicked one is left");
}

/// The buttons at the right of the tab bar do what their tips say.
#[test]
fn the_top_bar_buttons() {
    let mut h = Harness::new(json!({}));
    assert!(!h.app.editor.visible && !h.app.settings.sidebar);
    h.click_icon("editor (");
    assert!(h.app.editor.visible);
    h.click_icon("editor (");
    assert!(!h.app.editor.visible);
    h.click_icon("side panel (");
    assert!(h.app.settings.sidebar);
    h.click_icon("side panel (");
    assert!(!h.app.settings.sidebar);
    h.click_icon("split right");
    assert_eq!(h.app.panes.len(), 2);
    h.click_icon("split down");
    assert_eq!(h.app.panes.len(), 3);
    h.click_icon("command palette");
    assert!(h.app.palette.is_some());
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.palette.is_none());
    let help = h.at_exact("?");
    h.click(help);
    assert!(h.app.help.is_some());
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.help.is_none());
    let fonts = h.at_exact("Aa");
    h.click(fonts);
    assert!(h.app.font_menu.is_some());
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.font_menu.is_none());
    h.click_icon("theme");
    assert!(h.app.theme_menu.is_some());
    h.click_icon("theme");
    assert!(h.app.theme_menu.is_none());
    // The ☰ menu lists every command with its keys; a click runs it.
    let menu = h.at_exact("\u{2630}");
    h.click(menu);
    assert!(h.shows("New tab (same host)") && h.shows("Ctrl+Shift+T"), "commands and their keys");
    h.click_text("Zoom / restore pane");
    h.frames(2);
    assert!(h.app.tabs[0].zoomed, "the menu ran the command");
    assert!(!h.shows("New tab (same host)"), "and closed");
}

/// Commands that change a setting or the layout, one by one.
#[test]
fn commands_do_what_they_say() {
    use crate::app::CursorStyle;
    let mut h = Harness::new(json!({"sidebar": true}));
    let toast = |h: &Harness, text: &str| h.app.toasts.iter().any(|t| t.text.contains(text));
    // Focus mode hides the side panel and the editor, and brings them back.
    h.command("editor");
    assert!(h.app.editor.visible && h.app.settings.sidebar);
    h.command("focus_mode");
    assert!(!h.app.editor.visible && !h.app.settings.sidebar);
    h.command("focus_mode");
    assert!(h.app.editor.visible && h.app.settings.sidebar, "as they were");
    h.command("editor");
    // The cursor goes round its three shapes.
    assert_eq!(h.app.settings.cursor_style, CursorStyle::Block);
    h.command("cursor_style");
    assert_eq!(h.app.settings.cursor_style, CursorStyle::Bar);
    assert!(toast(&h, "cursor: bar"));
    h.command("cursor_style");
    assert_eq!(h.app.settings.cursor_style, CursorStyle::Underline);
    h.command("cursor_style");
    assert_eq!(h.app.settings.cursor_style, CursorStyle::Block);
    // Font and UI size: a step each way, the default, and their ends.
    let size = h.app.settings.font_size;
    h.command("font_bigger");
    assert_eq!(h.app.settings.font_size, size + 1.0);
    h.command("font_smaller");
    h.command("font_smaller");
    assert_eq!(h.app.settings.font_size, size - 1.0);
    for _ in 0..40 {
        h.app.run_command_id("font_smaller");
    }
    assert_eq!(h.app.settings.font_size, 8.0, "no smaller than 8");
    for _ in 0..40 {
        h.app.run_command_id("font_bigger");
    }
    assert_eq!(h.app.settings.font_size, 32.0, "no bigger than 32");
    h.command("font_default");
    assert_eq!(h.app.settings.font_size, 14.0);
    h.command("ui_bigger");
    assert_eq!(h.app.settings.ui_scale, 1.1);
    h.command("ui_smaller");
    h.command("ui_smaller");
    assert_eq!(h.app.settings.ui_scale, 0.9);
    for _ in 0..40 {
        h.app.run_command_id("ui_smaller");
    }
    assert_eq!(h.app.settings.ui_scale, 0.6);
    for _ in 0..40 {
        h.app.run_command_id("ui_bigger");
    }
    assert_eq!(h.app.settings.ui_scale, 2.5);
    h.command("ui_default");
    assert_eq!(h.app.settings.ui_scale, 1.0);
    // Switches.
    let hidden = h.app.settings.show_hidden;
    h.command("hidden_files");
    assert_eq!(h.app.settings.show_hidden, !hidden);
    let breaks = h.app.settings.markdown_breaks;
    h.command("markdown_breaks");
    assert_eq!(h.app.settings.markdown_breaks, !breaks);
    assert!(toast(&h, "Markdown:"));
    h.command("theme_system");
    assert_eq!(h.app.settings.theme, "system");
    h.command("shell_cmd");
    assert_eq!(h.app.settings.local_shell, crate::host::LocalShell::Cmd);
    assert!(toast(&h, "local shell: "));
    // Zoom needs a split; then it shows one pane, and both again.
    h.command("zoom");
    assert!(toast(&h, "zoom needs a split tab"));
    assert!(!h.app.tabs[0].zoomed);
    h.command("split_right");
    let (left, right) = {
        let mut ids: Vec<usize> = h.app.panes.keys().copied().collect();
        ids.sort();
        (ids[0], ids[1])
    };
    assert_eq!(h.app.active_pane_id(), Some(right), "the new pane has the keyboard");
    h.command("zoom");
    assert!(h.app.tabs[0].zoomed);
    h.command("zoom");
    assert!(!h.app.tabs[0].zoomed);
    // Focus moves between the panes of a split.
    h.command("focus_left");
    assert_eq!(h.app.active_pane_id(), Some(left));
    h.command("focus_left");
    assert_eq!(h.app.active_pane_id(), Some(left), "nothing further left");
    h.command("focus_right");
    assert_eq!(h.app.active_pane_id(), Some(right));
    h.command("split_down");
    let below = h.app.active_pane_id().unwrap();
    h.command("focus_up");
    assert_eq!(h.app.active_pane_id(), Some(right));
    h.command("focus_down");
    assert_eq!(h.app.active_pane_id(), Some(below));
    // Memos shows the memo folder in Files; History its tab; Privacy, Help,
    // Features and the pickers their windows.
    h.command("history");
    assert!(h.app.side.tab == crate::app::SideTab::History);
    h.command("memos");
    assert!(h.app.side.tab == crate::app::SideTab::Files);
    assert!(h.app.side.root.as_ref().is_some_and(|(host, dir)| host == "local" && dir.contains("memo")), "{:?}", h.app.side.root);
    h.command("privacy");
    assert!(h.app.records_window.is_some());
    h.key(Key::Escape, Modifiers::NONE);
    h.command("features");
    assert!(h.app.help.as_ref().is_some_and(|w| w.features));
    h.key(Key::Escape, Modifiers::NONE);
    h.command("open_memo");
    assert!(h.app.memo_picker);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(!h.app.memo_picker);
    h.command("theme");
    assert!(h.app.theme_menu.is_some());
    h.command("fonts");
    assert!(h.app.font_menu.is_some());
    // settings.json opens in the editor, as it is on disk.
    h.command("settings_file");
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().any(|f| f.path.ends_with("settings.json") && !f.loading)), "settings.json is open");
    assert!(h.app.editor.visible);
}

/// A terminal with two commands run in it as a shell with marks would show
/// them (`echo hi`, then `seq` with a hundred lines of output), a few lines
/// before them and a path after.
fn shell_harness() -> (Harness, std::path::PathBuf) {
    shell_harness_in(json!({}))
}

/// … with these settings besides.
fn shell_harness_in(more: serde_json::Value) -> (Harness, std::path::PathBuf) {
    // (A pause after each mark: Windows' pseudo console hands a mark on at
    // once and the text when it next draws.)
    let body = r#"
def mark(s):
    sys.stdout.write(E + ']133;' + s + chr(7))
    sys.stdout.flush()
    time.sleep(0.25)
def say(s):
    sys.stdout.write(s)
    sys.stdout.flush()
    time.sleep(0.25)
say('motd-1\r\nmotd-2\r\nmotd-3\r\n')
mark('A'); say('$ '); mark('B'); say('echo hi\r\n'); mark('C'); say('hi\r\nthere\r\n'); mark('D;0')
mark('A'); say('$ '); mark('B'); say('seq\r\n'); mark('C')
say(''.join('line-%02d word-%02d\r\n' % (i, i) for i in range(100)))
mark('D;3')
say('see /proj/src/main.rs:12:3 for it\r\n')
mark('A'); say('$ '); mark('B')
"#;
    script_harness(body, more)
}

/// The middle of the character cell where `needle` starts, in the terminal
/// row that shows it.
fn cell(h: &Harness, needle: &str) -> egui::Pos2 {
    let (text, rect) = h.texts.iter().find(|(t, _)| t.contains(needle)).unwrap_or_else(|| panic!("\"{needle}\" is not on screen"));
    let at = text[..text.find(needle).unwrap()].chars().count();
    let cw = rect.width() / text.chars().count() as f32;
    egui::pos2(rect.left() + (at as f32 + 0.5) * cw, rect.center().y)
}

/// The terminal's "Copy selection": what is selected, as text.
fn copy_selection(h: &mut Harness, at: egui::Pos2) -> String {
    h.copied = None;
    h.right_click(at);
    h.click_text("Copy selection");
    h.frames(2);
    h.copied.clone().unwrap_or_default()
}

/// The mouse in a terminal: the wheel scrolls back, a drag selects
/// characters, a double click a word, a triple click a line, Alt+drag a
/// block of columns; a click drops the selection; Ctrl+click opens a path.
#[test]
fn the_mouse_in_a_terminal() {
    const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
    let (mut h, _file) = shell_harness();
    let pane = h.app.active_pane_id().unwrap();
    let scroll = |h: &Harness| h.app.panes[&pane].scroll;
    // The wheel: back into what scrolled off, and down again to the end.
    let mid = egui::pos2(400.0, 400.0);
    assert_eq!(scroll(&h), 0);
    assert!(!h.shows("line-00"), "the first lines have scrolled off");
    h.wheel(mid, 3.0, Modifiers::NONE);
    let up = scroll(&h);
    assert!(up > 0, "scrolled back");
    h.wheel(mid, 3.0, Modifiers::NONE);
    assert!(scroll(&h) > up, "and further");
    h.wheel(mid, -1000.0, Modifiers::NONE);
    assert_eq!(scroll(&h), 0, "down stops at the end");
    assert!(h.shows("line-99"));
    // A double click: the word.
    let w = cell(&h, "word-98");
    h.clicks(w, 2);
    assert!(h.app.panes[&pane].sel.is_some(), "the word stays selected");
    assert_eq!(copy_selection(&mut h, w), "word-98");
    // A triple click: the line.
    let l = cell(&h, "line-97");
    h.clicks(l, 3);
    assert_eq!(copy_selection(&mut h, l), "line-97 word-97");
    // A drag: the characters from where it began to where it ended.
    let (a, b) = (cell(&h, "line-96"), cell(&h, "word-96"));
    h.drag(a, b);
    assert_eq!(copy_selection(&mut h, a), "line-96 w");
    // Over two lines: the rest of the first and the start of the second.
    let (a, b) = (cell(&h, "word-95"), cell(&h, "line-96"));
    h.drag(a, b + egui::vec2((b.x - cell(&h, "ine-96").x).abs() * 3.0, 0.0));
    assert_eq!(copy_selection(&mut h, a), "word-95\nline");
    // Alt+drag: a block of columns.
    let (a, b) = (cell(&h, "line-94"), cell(&h, "ne-96"));
    h.held = Modifiers::ALT;
    h.drag(a, b);
    h.held = Modifiers::NONE;
    assert_eq!(copy_selection(&mut h, a), "lin\nlin\nlin");
    // A plain click drops the selection.
    h.click(mid);
    assert!(h.app.panes[&pane].sel.is_none());
    // Ctrl+click on a path with a line number opens it there.
    let path = cell(&h, "/proj/src/main.rs");
    h.click_with(path + egui::vec2(20.0, 0.0), CTRL);
    h.frames(2);
    assert!(h.app.editor.files.iter().any(|f| f.host == "t" && f.path == "/proj/src/main.rs"), "opened: {:?}", h.app.editor.files.iter().map(|f| &f.path).collect::<Vec<_>>());
    // A plain word is not a path: a Ctrl+click on it opens nothing.
    let n = h.app.editor.files.len();
    let word = cell(&h, "see ");
    h.click_with(word, CTRL);
    h.frames(2);
    assert_eq!(h.app.editor.files.len(), n);
    // The terminal's menu splits and closes panes.
    h.command("editor");
    h.right_click(mid);
    h.click_text("Split right");
    h.frames(2);
    assert_eq!(h.app.panes.len(), 2);
}

/// A command and its output are a block: its buttons copy the command or
/// the output, fold it, run it again, put it in the composer, open the
/// output in the editor; commands fold them all and jump between them.
#[test]
fn a_command_block_has_its_buttons() {
    use crate::term::BlockState;
    let (mut h, file) = shell_harness();
    let pane = h.app.active_pane_id().unwrap();
    {
        let blocks = &h.app.panes[&pane].term.blocks;
        let done: Vec<(&str, Option<i32>)> = blocks.iter().filter(|b| b.state() == BlockState::Done).map(|b| (b.command.as_str(), b.exit)).collect();
        assert_eq!(done, [("echo hi", Some(0)), ("seq", Some(3))], "two commands, with how they ended");
    }
    // The first one has scrolled off: "previous command" goes to it.
    assert!(!h.shows("echo hi"));
    h.command("prev_command");
    h.command("prev_command");
    assert!(h.shows("echo hi"), "back at the first command");
    assert!(h.app.panes[&pane].scroll > 0);
    // Its buttons show when the pointer is on it.
    let on = cell(&h, "echo hi");
    h.hover(on);
    h.click_icon("copy command");
    assert_eq!(h.copied.as_deref(), Some("echo hi"));
    h.click_icon("copy output");
    assert_eq!(h.copied.as_deref().map(|s| s.trim_end()), Some("hi\nthere"));
    // Folding and unfolding: the command stays where it is on screen
    // (scrolled back, it used to slide away from under the pointer).
    h.click_icon("fold output");
    assert!(h.app.panes[&pane].term.blocks[0].folded);
    assert!(!h.texts.iter().any(|(t, _)| t.trim_end() == "there"), "the output is folded away");
    assert_eq!(cell(&h, "echo hi"), on, "folded: the command has not moved");
    h.hover(on);
    h.click_icon("unfold output");
    assert!(!h.app.panes[&pane].term.blocks[0].folded);
    assert!(h.texts.iter().any(|(t, _)| t.trim_end() == "there"));
    assert_eq!(cell(&h, "echo hi"), on, "unfolded: nor now");
    h.hover(on);
    h.click_icon("edit in composer");
    assert_eq!(h.app.panes[&pane].composer.as_ref().map(|c| c.text.as_str()), Some("echo hi"));
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.panes[&pane].composer.is_none());
    // (The composer is at the prompt: the terminal went back to its end.)
    h.command("prev_command");
    h.command("prev_command");
    h.hover(cell(&h, "echo hi"));
    h.click_icon("run again");
    assert!(h.wait_for(20.0, |_| received(&file).contains("echo hi")), "run again types the command: {:?}", received(&file));
    // Fold all, unfold all.
    h.command("fold_all");
    assert!(h.app.panes[&pane].term.blocks.iter().filter(|b| b.state() == BlockState::Done).all(|b| b.folded));
    assert!(!h.shows("line-50"));
    h.command("unfold_all");
    assert!(h.app.panes[&pane].term.blocks.iter().all(|b| !b.folded));
    // Next command: on to the second, then to the end.
    h.command("prev_command");
    h.command("prev_command");
    let at_first = h.app.panes[&pane].scroll;
    h.command("next_command");
    assert!(h.app.panes[&pane].scroll < at_first, "the second command is further down");
    h.command("next_command");
    assert_eq!(h.app.panes[&pane].scroll, 0, "past the last: the end");
    // The output in the editor, as a file of its own (last: the editor
    // takes room from the terminal, and the shell draws its screen again).
    h.command("prev_command");
    h.command("prev_command");
    h.hover(cell(&h, "echo hi"));
    h.click_icon("open output in editor");
    assert!(h.app.editor.files.iter().any(|f| f.scratch && f.path == "output: echo hi" && f.text.trim_end() == "hi\nthere"), "the output as a scratch file");
    // Clear scrollback: what scrolled off is gone.
    h.command("clear_scrollback");
    h.wheel(egui::pos2(400.0, 400.0), 50.0, Modifiers::NONE);
    assert!(!h.shows("motd-1") && !h.shows("line-00"), "nothing to scroll back to");
    assert_eq!(h.app.panes[&pane].scroll, 0);
}

/// Find in scrollback: the count, Enter and Shift+Enter step through the
/// hits (and bring them into view), Aa is case, × closes.
#[test]
fn find_in_scrollback_steps_through_the_hits() {
    let (mut h, _file) = shell_harness();
    let pane = h.app.active_pane_id().unwrap();
    let cur = |h: &Harness| h.app.panes[&pane].search.as_ref().map(|s| (s.cur, s.hits.len()));
    h.command("find");
    h.type_text("LINE-0");
    h.frames(2);
    assert_eq!(cur(&h), Some((9, 10)), "line-00 to line-09, any case; the last is the current one");
    assert!(h.shows("10/10"));
    assert!(!h.shows("line-00"));
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(cur(&h), Some((0, 10)), "Enter: the first hit");
    assert!(h.shows("1/10"));
    assert!(h.shows("line-00"), "brought into view");
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(cur(&h), Some((1, 10)));
    h.key(Key::Enter, Modifiers::SHIFT);
    assert_eq!(cur(&h), Some((0, 10)), "Shift+Enter: back");
    h.key(Key::Enter, Modifiers::SHIFT);
    assert_eq!(cur(&h), Some((9, 10)), "and round to the last");
    h.click_icon("next (Enter)");
    assert_eq!(cur(&h), Some((0, 10)), "the arrow steps too");
    // Aa: the case counts, and LINE-0 is nowhere.
    let aa = h.texts.iter().filter(|(t, _)| t == "Aa").map(|(_, r)| r.center()).max_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
    h.click(aa);
    h.frames(2);
    assert_eq!(cur(&h).map(|c| c.1), Some(0));
    assert!(h.shows("0/0"));
    h.click_icon("close (Esc)");
    assert!(h.app.panes[&pane].search.is_none());
}

/// The status bar: the text and UI size buttons, and for a host whose agent
/// is not there, what is wrong and a way to try again.
#[test]
fn the_status_bar() {
    let mut h = Harness::new(json!({}));
    let low = |h: &Harness, text: &str| h.texts.iter().find(|(t, r)| t == text && r.top() > 700.0).unwrap_or_else(|| panic!("no \"{text}\" in the status bar")).1.center();
    let p = low(&h, "A+");
    h.click(p);
    assert_eq!(h.app.settings.font_size, 15.0);
    let p = low(&h, "A\u{2212}");
    h.click(p);
    h.click(p);
    assert_eq!(h.app.settings.font_size, 13.0);
    let p = low(&h, "13");
    h.click(p);
    assert_eq!(h.app.settings.font_size, 14.0, "a click on the size: the default");
    // (The bar is at the bottom whatever the scale: found again each time.)
    let low = |h: &Harness, text: &str| h.texts.iter().filter(|(t, _)| t == text).map(|(_, r)| r.center()).max_by(|a, b| a.y.total_cmp(&b.y)).unwrap_or_else(|| panic!("no \"{text}\" in the status bar"));
    let p = low(&h, "+");
    h.click(p);
    h.frames(3);
    assert_eq!(h.app.settings.ui_scale, 1.1);
    let p = low(&h, "110%");
    h.click(p);
    h.frames(3);
    assert_eq!(h.app.settings.ui_scale, 1.0);
    let p = low(&h, "\u{2212}");
    h.click(p);
    h.frames(3);
    assert_eq!(h.app.settings.ui_scale, 0.9);
    h.command("ui_default");
    h.frames(3);
    // The size of the terminal, in characters.
    let pane = h.app.active_pane_id().unwrap();
    let (cols, rows) = (h.app.panes[&pane].term.cols, h.app.panes[&pane].term.rows);
    assert!(h.shows(&format!("{cols}\u{00d7}{rows}")));
    // The agent of this host ends at once: said, with a way to try again
    // and to give a password.
    assert!(h.wait_for(20.0, |h| h.shows("agent offline")), "the agent's state");
    h.click_text("password\u{2026}");
    assert!(h.app.auth_prompt.as_ref().is_some_and(|a| a.0 == "t"), "asks for this host's password");
    h.key(Key::Escape, Modifiers::NONE);
    h.app.auth_prompt = None;
    h.frames(2);
    h.click_text("reconnect");
    assert!(h.wait_for(20.0, |h| h.shows("agent connecting")) || h.shows("agent offline"), "it tried again");
}

/// weft with the side panel on a small source tree on this PC:
/// src/main.rs, src/lib.rs, docs/guide.md, readme.md.
fn tree_harness() -> (Harness, String) {
    let mut h = Harness::new(json!({"sidebar": true, "follow_cwd": false}));
    let root = h.dir.join("tree");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {\n    println!(\"needle here\");\n}\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "// Needle two\n").unwrap();
    std::fs::write(root.join("docs/guide.md"), "# Guide\n\nnothing to find\n").unwrap();
    std::fs::write(root.join("readme.md"), "read me\n").unwrap();
    let dir = root.display().to_string().replace('\\', "/");
    h.app.set_root("local", &dir);
    assert!(h.wait_for(20.0, |h| h.has("src") && h.has("readme.md")), "the folder is listed");
    (h, dir)
}

/// Go to a tab of the side panel: the ones a narrow panel has no room for
/// are under ».
fn side_tab(h: &mut Harness, name: &str) {
    if !h.has(name) {
        let more = h.at_exact("\u{00bb}");
        h.click(more);
    }
    let tab = h.at_exact(name);
    h.click(tab);
    h.frames(2);
}

/// Type into a text field that already has the keyboard, in place of what
/// is in it.
fn retype(h: &mut Harness, text: &str) {
    h.key(Key::A, Modifiers::COMMAND);
    h.type_text(text);
    h.frames(2);
}

/// The filter in Files finds names anywhere under the folder: fuzzy, or a
/// regex (said when it can't be read); a click selects, a double click
/// opens, Enter opens the first, the menu copies the path or shows the
/// file in the tree.
#[test]
fn the_files_filter_finds_names() {
    let (mut h, dir) = tree_harness();
    let field = h.at("fuzzy: letters in order");
    h.click(field);
    h.type_text("main");
    assert!(h.wait_for(20.0, |h| h.shows("1 match \u{00b7} 6 names")), "one of the six names matches");
    assert!(!h.has("readme.md"), "the tree gives way to the results");
    let row = h.at("main.rs   src");
    h.click(row);
    assert_eq!(h.app.side.selected.as_deref(), Some(format!("{dir}/src/main.rs").as_str()));
    h.right_click(row);
    h.click_text("Copy path");
    h.frames(2);
    assert_eq!(h.copied.as_deref(), Some(format!("{dir}/src/main.rs").as_str()));
    h.double_click(row);
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().any(|f| f.host == "local" && f.path.ends_with("src/main.rs"))), "a double click opens it");
    // Nothing matches; a regex that can't be read says so; one that can finds.
    h.click(field);
    retype(&mut h, "zzzz");
    assert!(h.shows("0 matches"));
    let mode = h.at_exact("fuzzy");
    h.click(mode);
    let regex = h.at_exact("regex");
    h.click(regex);
    assert_eq!(h.app.settings.find_mode, crate::find::FindMode::Regex);
    h.click(field);
    retype(&mut h, "(");
    assert!(!h.shows("matches") && !h.shows("1 match"), "no count for a pattern that can't be read");
    retype(&mut h, "^src/.*\\.rs$");
    assert!(h.shows("2 matches"), "src/main.rs and src/lib.rs");
    // Enter opens the first result.
    retype(&mut h, "guide");
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().any(|f| f.path.ends_with("docs/guide.md"))), "Enter opens the first");
    // Show in the tree: the filter goes, its folder opens, it is selected.
    let row = h.at("guide.md   docs");
    h.right_click(row);
    h.click_text("Show in the tree");
    assert!(h.app.side.filter.is_empty());
    assert_eq!(h.app.side.selected.as_deref(), Some(format!("{dir}/docs/guide.md").as_str()));
    assert!(h.app.side.expanded.contains(&("local".to_string(), format!("{dir}/docs"))));
    assert!(h.wait_for(20.0, |h| h.has("guide.md") && h.has("readme.md")), "the tree again, with docs open");
}

/// The Search tab finds text in the files under the folder: the matches by
/// file, a click opens at the line, the menu copies or shows in Files;
/// ignore case counts.
#[test]
fn the_search_tab_finds_text_in_files() {
    let (mut h, dir) = tree_harness();
    side_tab(&mut h, "Search");
    assert!(h.app.side.tab == crate::app::SideTab::Search, "from the » menu: the panel is too narrow for five tabs");
    assert!(h.shows(&format!("in local:{dir}")));
    let field = h.at("(Enter)");
    h.click(field);
    h.type_text("needle");
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_for(20.0, |h| h.app.side.grep.is_some() && !h.app.side.grep_busy), "the search came back");
    let n = |h: &Harness| h.app.side.grep.as_ref().and_then(|g| g["hits"].as_array().map(|a| a.len())).unwrap_or(0);
    let both = n(&h);
    assert!(h.shows(&format!("{both} matches")));
    // With the case counting only "needle here" is left (or, the other way
    // round, "Needle two" joins it).
    h.click_text("ignore case");
    h.click(field);
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_for(20.0, |h| !h.app.side.grep_busy && n(h) != both), "the case changes what is found");
    assert_eq!((both.min(n(&h)), both.max(n(&h))), (1, 2), "one with the case, two without");
    // A click on a line opens the file there.
    let line = h.at("needle here");
    h.click(line);
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().any(|f| f.host == "local" && f.path.ends_with("src/main.rs"))), "opened");
    // The menu of a line.
    h.right_click(line);
    assert!(h.shows("Open at this line") && h.shows("cd terminal to its folder"));
    h.click_text("Copy path:line");
    h.frames(2);
    assert_eq!(h.copied.as_deref(), Some(format!("{dir}/src/main.rs:2").as_str()));
    h.right_click(line);
    h.click_text("Copy line text");
    h.frames(2);
    assert_eq!(h.copied.as_deref(), Some("println!(\"needle here\");"));
    // The menu of a file: show it in Files.
    let file = h.at_exact("src/main.rs");
    h.right_click(file);
    h.click_text("Show in Files");
    h.frames(2);
    assert!(h.app.side.tab == crate::app::SideTab::Files);
    assert_eq!(h.app.side.selected.as_deref(), Some(format!("{dir}/src/main.rs").as_str()));
    assert_eq!(h.app.side.root, Some(("local".to_string(), format!("{dir}/src"))));
}

/// The Commands tab lists the terminal's commands, newest first, with how
/// they ended; a click goes to one, the menu copies it or its output.
/// History has them too, from the records: found by text, in the output
/// when asked, opened with their output.
#[test]
fn the_commands_and_history_tabs() {
    let (mut h, _file) = shell_harness_in(json!({"sidebar": true}));
    let pane = h.app.active_pane_id().unwrap();
    side_tab(&mut h, "Commands");
    assert!(h.has("echo hi") && h.has("seq"), "both commands are listed");
    assert!(h.at_exact("seq").y < h.at_exact("echo hi").y, "newest first");
    assert!(h.shows("exit 3") && h.shows("100 lines") && h.shows("2 lines"));
    let row = h.at_exact("echo hi");
    h.click(row);
    assert!(h.app.panes[&pane].scroll > 0, "the terminal went back to it");
    assert!(h.shows("$ echo hi"));
    h.right_click(row);
    h.click_text("Copy command");
    h.frames(2);
    assert_eq!(h.copied.as_deref(), Some("echo hi"));
    h.right_click(row);
    h.click_text("Copy output");
    h.frames(2);
    assert_eq!(h.copied.as_deref().map(|s| s.trim_end()), Some("hi\nthere"));
    // History: what was recorded.
    side_tab(&mut h, "History");
    assert!(h.wait_for(20.0, |h| h.shows("2 of 2 commands")), "both were recorded: {:?}", h.texts.iter().filter(|t| t.0.contains("commands")).collect::<Vec<_>>());
    let field = h.at("search commands");
    h.click(field);
    h.type_text("echo");
    h.frames(2);
    assert!(h.shows("1 of 2 commands"));
    retype(&mut h, "there");
    assert!(h.shows("0 of 2 commands"), "not in a command");
    h.click_text("in output too");
    h.frames(2);
    assert!(h.shows("1 of 2 commands"), "but in what one printed");
    let row = h.at_exact("echo hi");
    h.right_click(row);
    h.click_text("Copy output");
    h.frames(2);
    assert_eq!(h.copied.as_deref().map(|s| s.trim_end()), Some("hi\nthere"));
    h.click(row);
    h.frames(2);
    assert!(h.app.editor.files.iter().any(|f| f.scratch && f.text.starts_with("$ echo hi\n") && f.text.contains("exit 0") && f.text.trim_end().ends_with("hi\nthere")), "the command with its output, as a file");
    h.click_text("Records\u{2026}");
    assert!(h.app.records_window.is_some());
}

/// "Open memo…": the memos by their first line, found by text, opened by a
/// click, deleted from the menu; New memo starts one.
#[test]
fn the_memo_picker_lists_finds_and_deletes() {
    let mut h = Harness::new(json!({}));
    let dir = h.app.memo_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(std::path::Path::new(&dir).join("one.md"), "# Shopping\n\nmilk, eggs\n").unwrap();
    std::fs::write(std::path::Path::new(&dir).join("two.md"), "Meeting notes\n\nbudget\n").unwrap();
    h.command("open_memo");
    assert!(h.has("Shopping") && h.has("Meeting notes"), "listed by their first line");
    let field = h.at("search memos");
    h.click(field);
    h.type_text("budget");
    h.frames(2);
    assert!(h.has("Meeting notes") && !h.has("Shopping"), "found by what is in it");
    retype(&mut h, "zzzz");
    assert!(h.shows("no memo matches"));
    retype(&mut h, "shop");
    // Delete from the menu.
    let row = h.at_exact("Shopping");
    h.right_click(row);
    h.click_text("Delete this memo");
    h.frames(3);
    assert!(!std::path::Path::new(&dir).join("one.md").exists(), "the file is gone");
    h.click(field);
    retype(&mut h, "meet");
    // A click opens it and closes the window.
    let row = h.at_exact("Meeting notes");
    h.click(row);
    assert!(!h.app.memo_picker);
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().any(|f| f.path.ends_with("two.md") && !f.loading)), "opened in the editor");
    // New memo: one more file in the editor, the window closed.
    h.command("open_memo");
    let n = h.app.editor.files.len();
    h.click_text("New memo");
    h.frames(2);
    assert!(!h.app.memo_picker);
    assert_eq!(h.app.editor.files.len(), n + 1);
}

/// The theme menu picks a theme (and marks it), follows the system, and
/// closes on its ×, on Esc and on a click elsewhere.
#[test]
fn the_theme_menu_picks_a_theme() {
    let mut h = Harness::new(json!({}));
    let themes: Vec<(usize, &str, &str, bool)> = crate::theme::themes().collect();
    assert!(themes.len() >= 2);
    h.click_icon("theme");
    for t in &themes {
        assert!(h.has(t.2), "{} is offered", t.2);
    }
    let other = themes.iter().find(|t| t.1 != h.app.settings.theme).unwrap();
    let row = h.at_exact(other.2);
    h.click(row);
    h.frames(2);
    assert_eq!(h.app.settings.theme, other.1);
    assert!(h.app.theme_menu.is_some(), "the menu stays, to try another");
    let row = h.at_exact("Follow system");
    h.click(row);
    h.frames(2);
    assert_eq!(h.app.settings.theme, "system");
    assert!(h.shows("dark: ") && h.shows("light: "));
    h.click_icon("close (Esc)");
    assert!(h.app.theme_menu.is_none());
    h.command("theme");
    assert!(h.app.theme_menu.is_some());
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.theme_menu.is_none(), "Esc closes it");
    h.command("theme");
    h.click(egui::pos2(300.0, 500.0));
    assert!(h.app.theme_menu.is_none(), "a click elsewhere closes it");
}

/// Fonts: the built-in ones and the filter; a click sets the terminal
/// font, the other tab the icon font; a file that is no font is said to be
/// none, and the built-in font is used.
#[test]
fn the_font_window_sets_fonts() {
    let mut h = Harness::new(json!({}));
    h.command("fonts");
    assert!(h.shows("Built in (Nerd Fonts)") && h.shows("Installed on this computer"));
    let builtin: Vec<(&str, &str)> = crate::theme::BUILTIN_FONTS.iter().map(|f| (f.0, f.1)).collect();
    let before = h.app.settings.term_font.clone();
    let other = builtin.iter().find(|f| f.0 != before).unwrap();
    assert!(h.shows(&format!("now: {}", crate::theme::font_label(&before))));
    // The filter leaves the fonts whose name has the text.
    let field = h.at("filter fonts");
    h.click(field);
    let part = |name: &str| name.chars().take(name.chars().count() - 1).collect::<String>().to_lowercase();
    h.type_text(&part(other.1));
    h.frames(2);
    for f in builtin.iter().filter(|f| !f.1.to_lowercase().contains(&part(other.1))) {
        assert!(!h.has(f.1), "{} is filtered out", f.1);
    }
    let row = h.at_exact(other.1);
    h.click(row);
    h.frames(3);
    assert_eq!(h.app.settings.term_font, other.0);
    assert!(h.shows(&format!("now: {}", crate::theme::font_label(other.0))));
    // The icon font is set from its own tab, and leaves the terminal font.
    h.click_text("Icon font");
    h.frames(2);
    let icon_before = h.app.settings.icon_font.clone();
    let icon = builtin.iter().find(|f| f.0 != icon_before).unwrap();
    // (The sample line is as tall as its font: the field has moved.)
    let field = h.at_exact(&part(other.1));
    h.click(field);
    retype(&mut h, &part(icon.1));
    let row = h.at_exact(icon.1);
    h.click(row);
    h.frames(3);
    assert_eq!(h.app.settings.icon_font, icon.0);
    assert_eq!(h.app.settings.term_font, other.0);
    // A file that is no font: said, and the font stays.
    h.click_text("Terminal font");
    let bad = h.dir.join("not-a-font.ttf");
    std::fs::write(&bad, "this is text").unwrap();
    let path = h.at("or a font file");
    h.click(path);
    h.type_text(&bad.display().to_string());
    h.frames(2);
    let use_it = h.at_exact("Use");
    h.click(use_it);
    h.frames(3);
    assert!(h.app.toasts.iter().any(|t| t.text.contains("using the built-in font")), "weft says it can't be read: {:?}", h.app.toasts.iter().map(|t| &t.text).collect::<Vec<_>>());
    assert!(h.shows("Built in (Nerd Fonts)"), "and still draws its text, in the built-in font");
    h.key(Key::Escape, Modifiers::NONE);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.font_menu.is_none());
}

/// Privacy: what is kept, switched off and on, deleted; the password rules.
#[test]
fn the_privacy_window_keeps_and_deletes() {
    use crate::records::Item;
    let (mut h, _file) = shell_harness_in(json!({}));
    h.command("privacy");
    assert!(h.shows("Kept in ") && h.shows("ssh passwords in memory") && h.shows("0 held now"));
    for item in Item::ALL {
        assert!(h.shows(item.label()), "{} has its row", item.label());
    }
    // Commands are being recorded; switched off, the History tab says so.
    assert!(h.app.settings.records.commands.on);
    h.click_text(Item::Commands.label());
    h.frames(2);
    assert!(!h.app.settings.records.commands.on, "a click on its name switches it off");
    h.click_text(Item::Commands.label());
    h.frames(2);
    assert!(h.app.settings.records.commands.on);
    // Delete all commands: the files go, and History is empty.
    let root = h.app.data.root.clone();
    assert!(Item::Commands.size(&root) > 0, "the two commands were recorded");
    let commands_y = h.at(Item::Commands.label()).y;
    let delete = h.texts.iter().filter(|(t, _)| t == "Delete\u{2026}").map(|(_, r)| r.center()).min_by(|a, b| (a.y - commands_y).abs().total_cmp(&(b.y - commands_y).abs())).unwrap();
    h.click(delete);
    h.click_text(&format!("Delete all {}", Item::Commands.label().to_lowercase()));
    h.frames(3);
    assert_eq!(Item::Commands.size(&root), 0, "deleted");
    assert!(h.app.toasts.iter().any(|t| t.text.contains("deleted")));
    // Encrypting passwords in memory is a switch here, kept in settings.
    let enc = h.app.settings.passwords.encrypt;
    h.click_text("Encrypt them");
    h.frames(2);
    assert_eq!(h.app.settings.passwords.encrypt, !enc);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.records_window.is_none());
}

/// Open a file of this PC in the editor, with this text in it.
fn open_text(h: &mut Harness, name: &str, text: &str) -> (std::path::PathBuf, String) {
    let file = h.dir.join(name);
    std::fs::write(&file, text).unwrap();
    let path = file.display().to_string().replace('\\', "/");
    h.app.open_file("local", &path, 1);
    assert!(h.wait_for(20.0, |h| h.app.editor.files.iter().any(|f| f.path == path && !f.loading)), "{name} is open");
    h.frames(2);
    (file, path)
}

/// Find in the editor counts the matches (with or without the case), steps
/// through them and round, replaces one or all, reads a regex with groups,
/// says when nothing matches or the pattern can't be read.
#[test]
fn find_and_replace_in_the_editor() {
    let mut h = Harness::new(json!({}));
    let (_file, path) = open_text(&mut h, "words.txt", "cat dog cat\nCat bird\ncat\n");
    let text = |h: &Harness| h.app.editor.files.iter().find(|f| f.path == path).unwrap().text.clone();
    let cur = |h: &Harness| h.app.editor.find.as_ref().map(|b| b.cur);
    h.click_text("Find");
    h.type_text("cat");
    h.frames(2);
    assert!(h.shows("1/4"), "four, in any case");
    // Aa: the case counts.
    let aa = h.texts.iter().filter(|(t, r)| t == "Aa" && r.top() > 40.0).map(|(_, r)| r.center()).next().expect("the find bar's Aa");
    h.click(aa);
    h.frames(2);
    assert!(h.shows("1/3"), "three as typed");
    // Enter, Shift+Enter, F3 and the arrows step, and go round.
    let field = h.at_exact("cat");
    h.click(field);
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(cur(&h), Some(1));
    h.key(Key::Enter, Modifiers::SHIFT);
    assert_eq!(cur(&h), Some(0));
    h.key(Key::Enter, Modifiers::SHIFT);
    assert_eq!(cur(&h), Some(2), "round to the last");
    h.key(Key::F3, Modifiers::NONE);
    assert_eq!(cur(&h), Some(0), "F3: the next, round to the first");
    h.key(Key::F3, Modifiers::SHIFT);
    assert_eq!(cur(&h), Some(2));
    let down = h.at_exact("\u{2193}");
    h.click(down);
    assert_eq!(cur(&h), Some(0));
    let up = h.at_exact("\u{2191}");
    h.click(up);
    assert_eq!(cur(&h), Some(2));
    h.click(down);
    assert!(h.shows("1/3"));
    // Replace: this match, then all of them; the file is unsaved.
    let rep = h.at_exact("replace");
    h.click(rep);
    let with = h.at("replace with");
    h.click(with);
    h.type_text("fox");
    h.frames(2);
    let one = h.at_exact("Replace");
    h.click(one);
    h.frames(2);
    assert_eq!(text(&h), "fox dog cat\nCat bird\ncat\n", "the current match");
    assert!(h.shows("1/2"));
    let all = h.at_exact("All");
    h.click(all);
    h.frames(2);
    assert_eq!(text(&h), "fox dog fox\nCat bird\nfox\n", "every match; Cat is another word");
    assert!(h.shows("no match"));
    assert!(h.app.editor.files.iter().any(|f| f.path == path && f.dirty), "the file has changes to save");
    // A regex with a group; one that can't be read is said.
    h.app.editor.find.as_mut().unwrap().mode = crate::editfind::MatchMode::Regex;
    h.click(field);
    retype(&mut h, "(");
    let counted = h.texts.iter().any(|(t, _)| t.split_once('/').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && a.chars().chain(b.chars()).all(|c| c.is_ascii_digit())));
    assert!(!h.has("no match") && !counted, "a pattern that can't be read has no count");
    assert!(h.shows("unclosed group"), "it says what is wrong with it");
    retype(&mut h, "(\\w+) bird");
    assert!(h.shows("1/1"));
    h.app.editor.find.as_mut().unwrap().replace.clear();
    h.frames(2);
    assert!(h.shows("$1, $2: groups"), "the empty replace field says how groups are written");
    h.app.editor.find.as_mut().unwrap().replace = "$1 fish".into();
    h.frames(2);
    let all = h.at_exact("All");
    h.click(all);
    h.frames(2);
    assert_eq!(text(&h), "fox dog fox\nCat fish\nfox\n");
    // Every part of the bar is inside the window, in an editor this narrow
    // too (its × was past the edge, out of reach).
    let close = h.icons.iter().find(|(t, _)| t == "close (Esc)").unwrap().1;
    assert!(close.right() <= h.size.x && close.left() > h.size.x / 2.0, "the × is in the editor: {close:?}");
    assert!((close.center().y - h.at_exact("replace").y).abs() < 6.0, "and on the bar's line, not alone on the next");
    h.click_icon("close (Esc)");
    assert!(h.app.editor.find.is_none());
}

/// A file that changed on disk after it was opened is not written over
/// without a word: Reload takes what is there, Overwrite what is here.
#[test]
fn a_file_changed_on_disk_is_not_written_over() {
    let mut h = Harness::new(json!({}));
    let (file, path) = open_text(&mut h, "shared.txt", "as opened\n");
    let i = h.app.editor.files.iter().position(|f| f.path == path).unwrap();
    let change_both = |h: &mut Harness, there: &str, here: &str| {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&file, there).unwrap();
        h.app.editor.files[i].text = here.into();
        h.app.editor.files[i].dirty = true;
        h.frames(2);
        h.click_text("Save");
        assert!(h.wait_for(20.0, |h| h.app.editor.files[i].conflict), "the save is held back");
        assert!(h.shows("The file changed on the host since it was opened."));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), there, "nothing was written");
    };
    change_both(&mut h, "theirs\n", "mine\n");
    h.click_text("Reload");
    assert!(h.wait_for(20.0, |h| h.app.editor.files[i].text == "theirs\n" && !h.app.editor.files[i].conflict), "Reload: what is on disk");
    assert!(!h.app.editor.files[i].dirty);
    change_both(&mut h, "theirs again\n", "mine again\n");
    h.click_text("Overwrite");
    assert!(h.wait_for(20.0, |_| std::fs::read_to_string(&file).unwrap() == "mine again\n"), "Overwrite: what is here");
    assert!(h.wait_for(20.0, |h| !h.app.editor.files[i].conflict && !h.app.editor.files[i].dirty));
}

/// The editor's tabs: a click shows the file, × and a middle click close
/// it, the menu closes the ones to the right, the others, all.
#[test]
fn the_editor_tabs_switch_and_close() {
    let mut h = Harness::new(json!({}));
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        open_text(&mut h, name, &format!("this is {name}\n"));
    }
    let names = |h: &Harness| h.app.editor.files.iter().map(|f| f.path.rsplit('/').next().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(&h), ["a.txt", "b.txt", "c.txt", "d.txt"]);
    let b = h.at_exact("b.txt");
    h.click(b);
    assert_eq!(h.app.editor.active, 1);
    assert!(h.shows("this is b.txt"));
    // The menu: to the right of b.
    h.right_click(b);
    h.click_text("Close to the right");
    h.frames(2);
    assert_eq!(names(&h), ["a.txt", "b.txt"]);
    // A middle click closes a.
    let a = h.at_exact("a.txt");
    h.frame(vec![Event::PointerMoved(a)]);
    h.frame(vec![Event::PointerButton { pos: a, button: egui::PointerButton::Middle, pressed: true, modifiers: Modifiers::NONE }]);
    h.frame(vec![Event::PointerButton { pos: a, button: egui::PointerButton::Middle, pressed: false, modifiers: Modifiers::NONE }]);
    h.frames(2);
    assert_eq!(names(&h), ["b.txt"]);
    // The × of a tab.
    open_text(&mut h, "e.txt", "this is e\n");
    let e = h.at_exact("e.txt");
    let cross = h.icons.iter().filter(|(t, r)| t.starts_with("close (middle-click") && r.left() > e.x).map(|(_, r)| r.center()).min_by(|p, q| p.x.total_cmp(&q.x)).expect("the × after e.txt");
    h.click(cross);
    h.frames(2);
    assert_eq!(names(&h), ["b.txt"]);
    // Close others, close all.
    open_text(&mut h, "f.txt", "this is f\n");
    open_text(&mut h, "g.txt", "this is g\n");
    let f = h.at_exact("f.txt");
    h.right_click(f);
    h.click_text("Close others");
    h.frames(2);
    assert_eq!(names(&h), ["f.txt"]);
    let f = h.at_exact("f.txt");
    h.right_click(f);
    h.click_text("Close all");
    h.frames(2);
    assert!(h.app.editor.files.is_empty());
}

/// A program that shows several widgets in one go, without a pause (a
/// script, say): each is on its own lines, one below the other. On
/// Windows they all came with the cursor on the first one's line and were
/// drawn on top of each other: only the last could be seen.
#[test]
fn widgets_printed_in_one_go_each_get_their_lines() {
    let body = r#"
weft(t="widget", id="t1", kind="table", rows=3, columns=["item", "count"], data=[["beta", "20"], ["alpha", "3"]])
print("item count"); print("beta 20"); print("alpha 3")
weft(t="widget", id="p1", kind="progress", rows=1, pct=40, label="copying")
print("40% copying")
weft(t="widget", id="c1", kind="choose", rows=3, options=["yes-please", "no-thanks"])
print("choose"); print("1) yes-please"); print("2) no-thanks")
weft(t="widget", id="f1", kind="files", rows=3, dir="/d", entries=[
    {"name": "subdir", "path": "/d/subdir", "dir": True, "size": 0, "mtime": 0, "mode": "drwxr-xr-x"},
    {"name": "notes.txt", "path": "/d/notes.txt", "dir": False, "size": 2048, "mtime": 0, "mode": "-rw-r--r--"}])
print("Name"); print("subdir"); print("notes.txt")
weft(t="widget", id="p1", kind="progress", rows=1, pct=60, label="copying")
"#;
    let (mut h, file) = script_harness(body, json!({}));
    let pane = h.app.active_pane_id().unwrap();
    let lines: Vec<(u64, usize)> = h.app.panes[&pane].widgets.iter().map(|w| (w.anchor, w.rows)).collect();
    assert_eq!(lines, [(0, 3), (3, 1), (4, 3), (7, 3)], "one below the other");
    // Each can be seen, in that order down the screen, on the lines its
    // own text was printed on.
    let ys = [h.at_exact("beta").y, h.at("60%  copying").y, h.at_exact("no-thanks").y, h.at_exact("notes.txt").y];
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "table, progress, choice, files: {ys:?}");
    for (widget, text) in [("beta", "beta 20"), ("no-thanks", "2) no-thanks"), ("notes.txt", "notes.txt ")] {
        let under = h.texts.iter().find(|(t, _)| t.starts_with(text)).unwrap_or_else(|| panic!("the line \"{text}\"")).1;
        assert!((h.at_exact(widget).y - under.center().y).abs() < under.height(), "\"{widget}\" is on the line printed as \"{text}\"");
    }
    // And each takes its clicks: the choice types its number.
    let opt = h.at_exact("no-thanks");
    h.click(opt);
    assert!(h.wait_for(20.0, |_| received(&file).lines().any(|l| l.trim() == "2")), "the program was sent \"2\": {:?}", received(&file));
}
