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

/// PATH first, then npm's global folder and the Node.js folder: a weft started
/// with an old PATH (opened before the install) would otherwise miss mmdc.
fn mmdc_candidates() -> Vec<PathBuf> {
    let names = ["mmdc.cmd", "mmdc.exe", "mmdc"];
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    if let Some(node) = crate::host::which("node.exe").or_else(|| crate::host::which("node")) {
        if let Some(d) = node.parent() {
            dirs.push(d.to_path_buf());
        }
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        dirs.push(PathBuf::from(pf).join("nodejs"));
    }
    let mut out: Vec<PathBuf> = names.iter().filter_map(|n| crate::host::which(n)).collect();
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
    let mut cmd = if mmdc.extension().map(|e| e.eq_ignore_ascii_case("cmd")).unwrap_or(false) {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(mmdc);
        c
    } else {
        std::process::Command::new(mmdc)
    };
    cmd.arg("-i").arg(&input).arg("-o").arg(&output).args(["-b", "transparent", "-s", "2", "-t"]).arg(if dark { "dark" } else { "default" });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("mmdc: {e}"))?;
    let _ = std::fs::remove_file(&input);
    if !out.status.success() || !output.is_file() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("render failed");
        return Err(format!("mermaid: {}", line.trim()));
    }
    Ok(output)
}
