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
    std::env::set_var("WEFT_BUILT", &built);
    h.app.started_at = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    h.app.update_checked = std::time::Instant::now() - std::time::Duration::from_secs(60);
    h.frames(2);
    std::env::remove_var("WEFT_BUILT");
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
    assert!(h.wait_for(5.0, |h| h.shows("Rich")), "opened");
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
    assert!(h.wait_for(5.0, |h| h.shows("\u{2713} saved")), "saved mark");
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
    assert!(h.wait_for(5.0, |h| h.app.editor.files.iter().any(|f| f.path == png && f.image.has_texture())), "shown: on the GPU");
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
    assert!(h.wait_for(5.0, |h| h.shows("Find")), "opened");
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
    assert!(h.wait_for(5.0, |h| h.shows("Read")), "opened");
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
    assert!(h.shows("\u{2713} copied"), "the copied mark");
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
}
