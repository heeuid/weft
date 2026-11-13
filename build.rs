// weft.exe carries its icon (assets/weft.ico) on Windows, for Explorer,
// shortcuts and Settings > Apps. The window icon is set at run time (main.rs).
fn main() {
    // Which commit this weft is built from, and its day: `weft --version`
    // and Help show them (src/version.rs). Empty without git.
    let git = |args: &[&str]| -> String {
        std::process::Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    };
    println!("cargo:rustc-env=WEFT_COMMIT={}", git(&["rev-parse", "--short=9", "HEAD"]));
    println!("cargo:rustc-env=WEFT_COMMIT_DATE={}", git(&["log", "-1", "--format=%cs"]));
    // Again after each commit, checkout or pull (HEAD's log gets a line).
    for path in ["HEAD", "logs/HEAD"] {
        let p = git(&["rev-parse", "--git-path", path]);
        if !p.is_empty() {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    println!("cargo:rerun-if-changed=assets/weft.rc");
    println!("cargo:rerun-if-changed=assets/weft.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/weft.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
