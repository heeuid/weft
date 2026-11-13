// weft.exe carries its icon (assets/weft.ico) on Windows, for Explorer,
// shortcuts and Settings > Apps. The window icon is set at run time (main.rs).
fn main() {
    println!("cargo:rerun-if-changed=assets/weft.rc");
    println!("cargo:rerun-if-changed=assets/weft.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/weft.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
