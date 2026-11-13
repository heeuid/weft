//! Which weft this is: its version (Cargo.toml, raised with every release
//! and tagged `v<version>` in git; CHANGELOG.md says what changed), and the
//! commit it was built from.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit built from (short), and its day; empty when built without git.
/// Changes not committed yet don't show here.
pub const COMMIT: &str = env!("WEFT_COMMIT");
pub const COMMIT_DATE: &str = env!("WEFT_COMMIT_DATE");

/// "weft 0.2.0 (1a2b3c4d5, 2026-11-14)", as `weft --version` prints it.
pub fn line() -> String {
    describe(VERSION, COMMIT, COMMIT_DATE)
}

fn describe(version: &str, commit: &str, date: &str) -> String {
    match (commit.is_empty(), date.is_empty()) {
        (true, _) => format!("weft {version}"),
        (false, true) => format!("weft {version} ({commit})"),
        (false, false) => format!("weft {version} ({commit}, {date})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line() {
        assert_eq!(describe("0.2.0", "1a2b3c4d5", "2026-11-14"), "weft 0.2.0 (1a2b3c4d5, 2026-11-14)");
        assert_eq!(describe("0.2.0", "1a2b3c4d5", ""), "weft 0.2.0 (1a2b3c4d5)");
        assert_eq!(describe("0.2.0", "", ""), "weft 0.2.0");
        // This build: Cargo.toml's version, three numbers.
        assert!(line().starts_with(&format!("weft {VERSION}")));
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert!(parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok()), "{VERSION}");
        // CHANGELOG.md has this version's section: a release is written up.
        let log = include_str!("../CHANGELOG.md");
        assert!(log.contains(&format!("## {VERSION}")), "CHANGELOG.md has no section for {VERSION}");
    }
}
