//! docs/regression.md names every test: a test added without a row there
//! fails this one. (The document is how one finds out what is covered;
//! it is only worth reading if it can't fall behind.)

use std::path::Path;

/// The tests in a source file: (module, function), the module empty for a
/// test at the file's top level.
fn tests_in(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let (mut module, mut marked) = (String::new(), false);
    for line in source.lines() {
        if let Some(rest) = line.strip_prefix("mod ").or_else(|| line.strip_prefix("pub mod ")) {
            if let Some(name) = rest.strip_suffix(" {") {
                module = name.to_string();
            }
        }
        if line.trim() == "#[test]" {
            marked = true;
            continue;
        }
        let code = line.trim_start();
        if marked && (code.starts_with("fn ") || code.starts_with("pub fn ")) {
            marked = false;
            let name = code.trim_start_matches("pub ").trim_start_matches("fn ").split('(').next().unwrap_or("").to_string();
            // Inside a module the test is indented.
            found.push((if line.starts_with(' ') { module.clone() } else { String::new() }, name));
        }
    }
    found
}

/// Is `name` in `doc` as a whole word?
fn names(doc: &str, name: &str) -> bool {
    doc.match_indices(name).any(|(i, _)| {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        !doc[..i].chars().next_back().is_some_and(word) && !doc[i + name.len()..].chars().next().is_some_and(word)
    })
}

#[test]
fn every_test_is_in_the_document() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(root.join("docs/regression.md")).unwrap();
    let mut missing = Vec::new();
    let mut count = 0;
    for dir in ["src", "crates/weft-core/src", "crates/weft-draw/src"] {
        let mut files: Vec<_> = std::fs::read_dir(root.join(dir)).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "rs")).collect();
        files.sort();
        for file in files {
            let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
            for (module, name) in tests_in(&std::fs::read_to_string(&file).unwrap()) {
                count += 1;
                // By its name, or by "file::module::*" for a module listed whole.
                let whole_module = !module.is_empty() && doc.contains(&format!("{stem}::{module}::*"));
                if !whole_module && !names(&doc, &name) {
                    missing.push(if module.is_empty() { format!("{stem}::{name}") } else { format!("{stem}::{module}::{name}") });
                }
            }
        }
    }
    assert!(count > 150, "the tests were found: {count}");
    assert!(missing.is_empty(), "docs/regression.md has no row for these tests (add one saying what each covers):\n  {}", missing.join("\n  "));
}

#[test]
fn tests_are_found_in_source() {
    let source = "fn helper() {}\n\n#[test]\nfn at_the_top() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn inside() {}\n\n    #[test]\n    #[ignore]\n    fn ignored_too() {}\n\n    fn not_a_test() {}\n}\n";
    let t = |m: &str, n: &str| (m.to_string(), n.to_string());
    assert_eq!(tests_in(source), [t("", "at_the_top"), t("tests", "inside"), t("tests", "ignored_too")]);
    assert!(names("`a::b::inside`, and", "inside") && !names("insider trading", "inside") && !names("x_inside", "inside"));
}
