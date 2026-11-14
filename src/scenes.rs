//! weft's screens as pictures, one PNG each, to compare one version of
//! weft with another: the same scenes drawn by both, then diffed pixel by
//! pixel (what differs is either meant or a regression).
//!
//!     cargo test -p weft scenes -- --ignored
//!
//! writes them to WEFT_SCENES_DIR (default target/shots/scenes). It uses
//! only what the test harness has had from its first version, so the file
//! can be dropped into an older checkout as it is.

use crate::testkit::Harness;
use egui::{Key, Modifiers};
use serde_json::json;
use std::path::PathBuf;

fn out_dir() -> PathBuf {
    let dir = std::env::var_os("WEFT_SCENES_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("shots").join("scenes"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Files the scenes show: always at the same place, so paths on screen are
/// the same from run to run and version to version.
fn fixtures() -> String {
    let dir = std::env::temp_dir().join("weft-scenes-fixtures");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/ui")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "use std::collections::HashMap;\n\n/// Counts words.\nfn main() {\n    let mut seen: HashMap<String, usize> = HashMap::new();\n    for word in \"a b a\".split(' ') {\n        *seen.entry(word.to_string()).or_default() += 1;\n    }\n    println!(\"{seen:?}\"); // 한글 주석\n}\n").unwrap();
    std::fs::write(dir.join("src/ui/view.rs"), "pub fn view() {}\n").unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    std::fs::write(
        dir.join("docs/notes.md"),
        "# Notes\n\nSome *emphasis*, **strong**, `code` and a [link](https://example.com).\n\n## List\n\n- one\n- two\n  - nested\n- [ ] to do\n- [x] done\n\n> A quote\n> over two lines.\n\n| name | size | kind |\n|:--|--:|:-:|\n| weft.exe | 12 MB | app |\n| notes.md | 3 KB | 문서 |\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\n---\n\nLast line.\n",
    )
    .unwrap();
    dir.display().to_string().replace('\\', "/")
}

/// A terminal showing what a program printed: colours, attributes, wide
/// characters, box drawing. (No block characters: those are drawn
/// differently on purpose since 0.2.)
fn printing_profile() -> serde_json::Value {
    let script = std::env::temp_dir().join("weft-scenes-print.py");
    std::fs::write(
        &script,
        r#"import time
E = chr(27)
print("plain text, then " + E + "[1mbold" + E + "[0m, " + E + "[4munderline" + E + "[0m, " + E + "[7minverse" + E + "[0m, " + E + "[2mdim" + E + "[0m, " + E + "[9mstrike" + E + "[0m")
print(" ".join(E + "[%dm colour%d " % (30 + i, i) + E + "[0m" for i in range(8)))
print(" ".join(E + "[%dm  bg%d  " % (40 + i, i) + E + "[0m" for i in range(8)))
print("".join(E + "[38;5;%dm#" % i for i in range(16, 88)) + E + "[0m")
print(E + "[38;2;215;119;87mtrue colour" + E + "[0m and " + E + "[48;2;40;60;90m a background " + E + "[0m")
print("한글 두 칸: 가나다라마바사 | ABC | 日本語 | emoji-free")
print(chr(0x250c) + chr(0x2500) * 10 + chr(0x252c) + chr(0x2500) * 10 + chr(0x2510))
print(chr(0x2502) + " name     " + chr(0x2502) + " size     " + chr(0x2502))
print(chr(0x251c) + chr(0x2500) * 10 + chr(0x253c) + chr(0x2500) * 10 + chr(0x2524))
print(chr(0x2502) + " weft.exe " + chr(0x2502) + "    12 MB " + chr(0x2502))
print(chr(0x2514) + chr(0x2500) * 10 + chr(0x2534) + chr(0x2500) * 10 + chr(0x2518))
for i in range(12):
    print("line %2d  the quick brown fox jumps over the lazy dog 0123456789 !@#$%%^&*()" % i)
print("$ ", end="", flush=True)
print("ready-marker", flush=True)
time.sleep(600)
"#,
    )
    .unwrap();
    json!({"name": "t", "kind": "custom", "pty": ["python", "-u", script.display().to_string()], "agent": ["python", "-c", "0"]})
}

fn settle(h: &mut Harness) {
    h.wait_for(30.0, |h| h.shows("ready-marker"));
    h.frames(5);
}

fn save(h: &mut Harness, name: &str) {
    // The cursor blinks with the clock: a few frames more, then as it is.
    h.frames(3);
    h.shot().save(out_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore]
fn scenes() {
    let root = fixtures();
    let prof = printing_profile();
    let base = |more: serde_json::Value| {
        let mut s = json!({"profiles": [prof.clone()], "sidebar": false, "follow_cwd": false});
        for (k, v) in more.as_object().unwrap() {
            s[k] = v.clone();
        }
        s
    };

    // The terminal alone; with its search; with the composer; split.
    let mut h = Harness::new(base(json!({})));
    settle(&mut h);
    save(&mut h, "01-terminal");
    h.command("find");
    h.type_text("fox");
    h.frames(3);
    save(&mut h, "02-terminal-search");
    h.key(Key::Escape, Modifiers::NONE);
    h.app.panes.values_mut().for_each(|p| p.search = None);
    h.command("composer");
    h.type_text("git status");
    h.frames(3);
    save(&mut h, "03-composer");
    h.app.panes.values_mut().for_each(|p| p.composer = None);
    h.command("split_right");
    h.command("split_down");
    h.frames(20);
    save(&mut h, "04-split-panes");
    h.command("new_tab");
    h.command("new_tab");
    h.frames(20);
    save(&mut h, "05-three-tabs");
    drop(h);

    // The windows opened from the palette and the menu.
    for (n, cmd) in [("06-palette", "palette"), ("07-help", "help"), ("08-shortcuts", "keys"), ("09-privacy", "privacy"), ("10-serial", "serial"), ("11-fonts", "fonts"), ("12-connect", "connect")] {
        let mut h = Harness::new(base(json!({})));
        settle(&mut h);
        h.command(cmd);
        h.frames(6);
        save(&mut h, n);
    }

    // Files beside the terminal, a folder opened.
    let mut h = Harness::new(base(json!({"sidebar": true})));
    settle(&mut h);
    h.app.set_root("local", &root);
    h.wait_for(30.0, |h| h.shows("Cargo.toml"));
    let src = h.at("src");
    h.click(src);
    h.wait_for(30.0, |h| h.shows("main.rs"));
    h.frames(5);
    save(&mut h, "13-files");
    let row = h.at("Cargo.toml");
    h.right_click(row);
    h.frames(3);
    save(&mut h, "14-files-menu");
    h.key(Key::Escape, Modifiers::NONE);
    drop(h);

    // The editor: a source file; markdown rendered, as source, in read mode.
    let mut h = Harness::new(base(json!({})));
    settle(&mut h);
    h.app.open_file("local", &format!("{root}/src/main.rs"), 1);
    h.wait_for(30.0, |h| h.shows("HashMap") && h.app.editor.files.iter().all(|f| !f.loading));
    // Syntax colours arrive from a thread: give them a moment.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    h.frames(10);
    save(&mut h, "15-editor-source");
    h.app.open_file("local", &format!("{root}/docs/notes.md"), 1);
    h.wait_for(30.0, |h| h.shows("Notes") && h.app.editor.files.iter().all(|f| !f.loading));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    h.frames(10);
    save(&mut h, "16-markdown-rich");
    h.click_text("Read");
    h.frames(6);
    save(&mut h, "17-markdown-read");
    h.click_text("Read");
    h.click_text("Rich \u{21c4}");
    h.frames(6);
    save(&mut h, "18-markdown-source");
}
