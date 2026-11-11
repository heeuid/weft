//! Mermaid diagrams for the markdown editor.  Rendered locally by
//! mermaid-cli (`mmdc`, the same tool nvim-editor uses) in a background
//! thread and cached as PNG files under data/cache/mermaid, so a diagram is
//! rendered once per source+theme.  Without mmdc the block stays code and the
//! editor shows how to enable diagrams.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub enum Status {
    /// Texture and its size in pixels.
    Ready(egui::TextureId, egui::Vec2),
    Rendering,
    Failed(String),
    /// mmdc is not installed.
    Unavailable,
}

enum Entry {
    Ready(egui::TextureHandle, egui::Vec2),
    Rendering,
    Failed(String),
}

#[derive(Default)]
struct Store {
    entries: HashMap<u64, Entry>,
    finished: Vec<(u64, Result<PathBuf, String>)>,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
/// Bumped when a diagram finishes, so cached layouts that reserved no space re-run.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn store() -> &'static Mutex<Store> {
    STORE.get_or_init(|| Mutex::new(Store::default()))
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub fn key(source: &str, dark: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.trim().hash(&mut h);
    dark.hash(&mut h);
    h.finish()
}

/// mermaid-cli on PATH (npm installs `mmdc.cmd` on Windows).
fn mmdc(ctx: &egui::Context) -> Option<PathBuf> {
    // Found once and kept. While missing (and only while a diagram asks for
    // it), look again in a background thread at most every 5 s, so installing
    // it takes effect without restarting weft and the UI never waits on disk.
    static FOUND: Mutex<(Option<PathBuf>, Option<std::time::Instant>)> = Mutex::new((None, None));
    let mut f = FOUND.lock().unwrap();
    if f.0.is_none() && f.1.map(|t| t.elapsed() > std::time::Duration::from_secs(5)).unwrap_or(true) {
        // Far future while the search runs, so only one runs at a time.
        f.1 = Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let found = mmdc_candidates().into_iter().next();
            let mut f = FOUND.lock().unwrap();
            f.1 = Some(std::time::Instant::now());
            if found.is_some() {
                f.0 = found;
                GENERATION.fetch_add(1, Ordering::Relaxed);
                ctx.request_repaint();
            }
        });
    }
    f.0.clone()
}

fn which_in(path: &std::ffi::OsStr, exe: &str) -> Option<PathBuf> {
    std::env::split_paths(path).map(|d| d.join(exe)).find(|p| p.is_file())
}

fn node_in(path: &std::ffi::OsStr) -> Option<PathBuf> {
    which_in(path, "node.exe").or_else(|| which_in(path, "node"))
}

/// PATH as the system and user environment have it now (registry), then our
/// own: weft may have been started before Node.js or mermaid-cli were installed.
fn fresh_path() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        for (root, key) in [(win::HKLM, "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"), (win::HKCU, "Environment")] {
            if let Some(v) = win::reg_string(root, key, "Path") {
                dirs.extend(std::env::split_paths(&v));
            }
        }
    }
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.to_string_lossy().to_lowercase()));
    std::env::join_paths(dirs).unwrap_or_default()
}

/// PATH first, then npm's global folder and the Node.js folder.
fn mmdc_candidates() -> Vec<PathBuf> {
    let names = ["mmdc.cmd", "mmdc.exe", "mmdc"];
    let path = fresh_path();
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    if let Some(d) = node_in(&path).and_then(|n| n.parent().map(|d| d.to_path_buf())) {
        dirs.push(d);
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        dirs.push(PathBuf::from(pf).join("nodejs"));
    }
    let mut out: Vec<PathBuf> = names.iter().filter_map(|n| which_in(&path, n)).collect();
    for d in dirs {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                out.push(p);
            }
        }
    }
    out
}

pub const INSTALL_HINT: &str = "need mermaid-cli";

fn load_png(ctx: &egui::Context, path: &Path, name: &str) -> Result<(egui::TextureHandle, egui::Vec2), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).map_err(|e| e.to_string())?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    let tex = ctx.load_texture(name, color, egui::TextureOptions::LINEAR);
    Ok((tex, egui::vec2(size[0] as f32, size[1] as f32)))
}

/// The diagram for `source`: starts rendering it on first use.
pub fn status(ctx: &egui::Context, cache_dir: &Path, source: &str, dark: bool) -> Status {
    let k = key(source, dark);
    let mut st = store().lock().unwrap();
    // Collect finished renders.
    for (fk, res) in std::mem::take(&mut st.finished) {
        let entry = match res.and_then(|p| load_png(ctx, &p, &format!("mermaid-{fk:x}"))) {
            Ok((tex, size)) => Entry::Ready(tex, size),
            Err(e) => Entry::Failed(e),
        };
        st.entries.insert(fk, entry);
    }
    if let Some(e) = st.entries.get(&k) {
        return match e {
            Entry::Ready(t, s) => Status::Ready(t.id(), *s),
            Entry::Rendering => Status::Rendering,
            Entry::Failed(m) => Status::Failed(m.clone()),
        };
    }
    let png = cache_dir.join(format!("{k:016x}.png"));
    if png.is_file() {
        let entry = match load_png(ctx, &png, &format!("mermaid-{k:x}")) {
            Ok((tex, size)) => Entry::Ready(tex, size),
            Err(e) => Entry::Failed(e),
        };
        let status = match &entry {
            Entry::Ready(t, s) => Status::Ready(t.id(), *s),
            Entry::Failed(m) => Status::Failed(m.clone()),
            Entry::Rendering => Status::Rendering,
        };
        st.entries.insert(k, entry);
        return status;
    }
    let Some(mmdc) = mmdc(ctx) else { return Status::Unavailable };
    st.entries.insert(k, Entry::Rendering);
    let (dir, src, ctx2) = (cache_dir.to_path_buf(), source.to_string(), ctx.clone());
    std::thread::spawn(move || {
        let res = render(&mmdc, &dir, k, &src, dark);
        store().lock().unwrap().finished.push((k, res));
        GENERATION.fetch_add(1, Ordering::Relaxed);
        ctx2.request_repaint();
    });
    Status::Rendering
}

fn render(mmdc: &Path, dir: &Path, k: u64, source: &str, dark: bool) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let input = dir.join(format!("{k:016x}.mmd"));
    let output = dir.join(format!("{k:016x}.png"));
    std::fs::write(&input, source).map_err(|e| e.to_string())?;
    // npm's mmdc.cmd runs `node`: find it the same way, next to mmdc as a fallback.
    let mut path = fresh_path();
    if node_in(&path).is_none() {
        let extra: Vec<PathBuf> = [mmdc.parent().map(|d| d.to_path_buf()), std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("nodejs"))].into_iter().flatten().collect();
        let joined = std::env::join_paths(extra.iter().cloned().chain(std::env::split_paths(&path))).unwrap_or(path.clone());
        if node_in(&joined).is_none() {
            let _ = std::fs::remove_file(&input);
            return Err("mermaid-cli needs Node.js: node not found".into());
        }
        path = joined;
    }
    let mut cmd = if mmdc.extension().map(|e| e.eq_ignore_ascii_case("cmd")).unwrap_or(false) {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(mmdc);
        c
    } else {
        std::process::Command::new(mmdc)
    };
    cmd.env("PATH", &path);
    cmd.arg("-i").arg(&input).arg("-o").arg(&output).args(["-b", "transparent", "-s", "2", "-t"]).arg(if dark { "dark" } else { "default" });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("mmdc: {e}"))?;
    let _ = std::fs::remove_file(&input);
    if !out.status.success() || !output.is_file() {
        let err = decode(if out.stderr.iter().any(|c| !c.is_ascii_whitespace()) { &out.stderr } else { &out.stdout });
        let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("render failed");
        return Err(format!("mermaid: {}", line.trim()));
    }
    Ok(output)
}

/// Console output: UTF-8, else the OEM code page (cmd.exe messages, e.g. CP949).
fn decode(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    #[cfg(windows)]
    if let Some(s) = win::oem_to_string(bytes) {
        return s;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(windows)]
mod win {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::OsStringExt;

    pub const HKLM: isize = 0x8000_0002_u32 as i32 as isize;
    pub const HKCU: isize = 0x8000_0001_u32 as i32 as isize;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, ty: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn MultiByteToWideChar(cp: u32, flags: u32, src: *const u8, src_len: i32, dst: *mut u16, dst_len: i32) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// A string value, REG_EXPAND_SZ expanded.
    pub fn reg_string(root: isize, key: &str, value: &str) -> Option<OsString> {
        const RRF_RT_REG_SZ: u32 = 0x2;
        let (k, v) = (wide(key), wide(value));
        let mut len = 0u32;
        // SAFETY: valid NUL-terminated strings; the first call only asks for the size.
        unsafe {
            if RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut len) != 0 {
                return None;
            }
            let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
            len = (buf.len() * 2) as u32;
            if RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) != 0 {
                return None;
            }
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(OsString::from_wide(&buf[..n]))
        }
    }

    pub fn oem_to_string(bytes: &[u8]) -> Option<String> {
        const CP_OEMCP: u32 = 1;
        let len = i32::try_from(bytes.len()).ok()?;
        // SAFETY: lengths match the buffers passed.
        unsafe {
            let n = MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
            if n <= 0 {
                return None;
            }
            let mut buf = vec![0u16; n as usize];
            MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, buf.as_mut_ptr(), n);
            Some(String::from_utf16_lossy(&buf))
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn reads_the_registry_path() {
            let p = super::reg_string(super::HKLM, "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment", "Path").expect("system PATH");
            assert!(p.to_string_lossy().to_lowercase().contains("system32"), "expanded: {p:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    /// A weft started before Node.js was installed (node missing from its own
    /// PATH) still runs npm's mmdc.cmd, which calls `node`: the PATH comes from
    /// the registry. Needs node on the system/user PATH; a fake mmdc.cmd stands
    /// in for mermaid-cli.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn stale_path_still_finds_node() {
        let dir = std::env::temp_dir().join(format!("weft-mmdc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("mmdc.cmd");
        // mmdc -i IN -o OUT ...: write OUT with node, as the real one would.
        std::fs::write(&fake, "@node -e \"require('fs').writeFileSync(process.argv[1], 'png')\" %4\r\n").unwrap();
        let old = std::env::var_os("PATH").unwrap_or_default();
        let without_node: Vec<_> = std::env::split_paths(&old).filter(|d| !d.join("node.exe").is_file()).collect();
        std::env::set_var("PATH", std::env::join_paths(without_node).unwrap());
        assert!(super::node_in(&std::env::var_os("PATH").unwrap()).is_none(), "node hidden from our PATH");
        let res = super::render(&fake, &dir.join("out"), 1, "graph TD; A-->B", true);
        std::env::set_var("PATH", old);
        let png = res.expect("render with the registry PATH");
        assert_eq!(std::fs::read_to_string(png).unwrap(), "png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(windows)]
    fn cmd_errors_are_not_mojibake() {
        let out = std::process::Command::new("cmd").args(["/C", "no-such-command-xyz"]).output().unwrap();
        let msg = super::decode(&out.stderr);
        assert!(!msg.contains('\u{fffd}'), "{msg}");
        assert!(msg.contains("no-such-command-xyz"), "{msg}");
    }
}
