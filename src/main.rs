#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! weft — a terminal that weaves GUI into the TUI grid.
//!
//! Thick local client (rendering, editing, search, history, completion
//! ranking all happen here); the remote side only ever needs ssh, POSIX
//! tools and python, and nothing is written to the remote disk.

mod app;
mod confirm;
mod copymode;
mod display_test;
#[cfg(test)]
mod doccheck;
mod dragout;
mod help;
mod fileops;
mod frame;
mod gitui;
#[cfg(test)]
mod testkit;
#[cfg(test)]
mod regress;
mod media;
mod mermaid;
mod restart;
#[cfg(test)]
mod scenes;
mod serial_ui;
mod settings_ui;
mod store;
mod instances;
#[cfg(test)]
mod shot;
mod transfer;
mod ui_panels;
mod ui_term;
mod version;
mod widgets;
mod x11;
#[cfg(not(any(windows, target_os = "macos")))]
mod xdnd;

// The parts that live in their own crates, under the names they always had.
use weft_core::{editfind, find, host, pty, records, secret, term};
use weft_draw::{highlight, icons, keys, md, theme};

use app::App;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Process start, for the startup timings in weft.log.
static STARTED: OnceLock<Instant> = OnceLock::new();

pub fn since_start() -> String {
    format!("{} ms", STARTED.get().map(|t| t.elapsed().as_millis()).unwrap_or(0))
}

/// The graphics backend: WEFT_RENDERER, else settings "renderer", else
/// "auto": the steadiest for the system. Windows: wgpu on DirectX 12 (an
/// OpenGL window, after the PC slept or locked a few times, showed black
/// frames each time it redrew until it was minimized or started again;
/// wgpu makes its surface again when the system says it is lost or out of
/// date). macOS: wgpu on Metal (OpenGL there is deprecated). Linux and the
/// rest: OpenGL, which runs where Vulkan has no driver (virtual machines,
/// older GPUs). Read before the window is made, so from settings.json
/// directly.
fn renderer_choice() -> eframe::Renderer {
    let from_settings = || renderer_setting(&std::fs::read_to_string(host::DataDir::locate().path("settings.json")).ok()?);
    let want = std::env::var("WEFT_RENDERER").ok().or_else(from_settings).unwrap_or_default().to_lowercase();
    renderer_for(&want, cfg!(windows) || cfg!(target_os = "macos"))
}

/// settings.json's "renderer", as Graphics backend… saved it.
fn renderer_setting(settings_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(settings_json).ok()?.get("renderer")?.as_str().map(String::from)
}

/// "wgpu" (or directx, dx12, vulkan, metal), "opengl" (or gl, glow), else
/// the system's own: wgpu where `wgpu_first`.
fn renderer_for(want: &str, wgpu_first: bool) -> eframe::Renderer {
    match want.trim() {
        "wgpu" | "directx" | "dx12" | "vulkan" | "metal" => eframe::Renderer::Wgpu,
        "opengl" | "gl" | "glow" => eframe::Renderer::Glow,
        _ if wgpu_first => eframe::Renderer::Wgpu,
        _ => eframe::Renderer::Glow,
    }
}

/// wgpu's backend: DirectX 12 on Windows (it could take Vulkan there),
/// unless WGPU_BACKEND says another; the system's own elsewhere.
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut o = eframe::egui_wgpu::WgpuConfiguration::default();
    if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() {
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(c) = &mut o.wgpu_setup {
            c.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        }
    }
    o
}

/// What draws: the backend and the GPU and driver it got, for the log.
fn graphics_info(cc: &eframe::CreationContext) -> String {
    if let Some(gl) = cc.gl.as_ref() {
        use eframe::glow::HasContext;
        // SAFETY: plain string queries on the context eframe made current.
        let (renderer, vendor, version) = unsafe { (gl.get_parameter_string(eframe::glow::RENDERER), gl.get_parameter_string(eframe::glow::VENDOR), gl.get_parameter_string(eframe::glow::VERSION)) };
        return format!("OpenGL: {renderer} ({vendor}), {version}");
    }
    if let Some(rs) = cc.wgpu_render_state.as_ref() {
        let i = rs.adapter.get_info();
        return format!("wgpu {:?}: {} ({:?}), driver {} {}", i.backend, i.name, i.device_type, i.driver, i.driver_info);
    }
    "unknown".into()
}

struct WeftApp {
    app: App,
    /// The window's scale and monitor size last seen: a change is logged
    /// (moved to another monitor, a monitor plugged or woken).
    display: Option<(f32, Option<egui::Vec2>)>,
    first_frame: bool,
    /// WEFT_DEBUG_REPAINT=1: frames per 5 s and what asked for them, to the log.
    repaint_debug: Option<(Instant, u32, std::collections::BTreeMap<String, u32>)>,
    /// The window title last sent.
    title: String,
    /// For the health line in the log: since when, frames, font atlas fill.
    health: (Instant, u64, f32),
    /// Flipped to make egui rebuild its fonts (see raw_input_hook).
    atlas_flip: bool,
}

impl eframe::App for WeftApp {
    /// Keep egui's glyph atlas from overflowing in the middle of a frame.
    /// egui only rebuilds its fonts as a pass starts, once the atlas is over
    /// 80 % full; a pass that needs more new glyphs than are left overwrites
    /// glyphs part way through, and the text drawn after that (weft's
    /// panels; the status bar comes first) shows blank for a frame. Over
    /// hours of varied text (Hangul, icons, sizes) that became a flicker.
    /// Past half full, rebuild as the next pass starts: egui does so when
    /// max_texture_side changes, so it is flipped by one.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // The system's drag kept the button's release from the window: said
        // here, else egui would take the button as still held.
        if std::mem::take(&mut self.app.release_pointer) {
            let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap_or_default();
            // First, before a press of this frame.
            raw.events.insert(0, egui::Event::PointerGone);
            raw.events.insert(0, egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: raw.modifiers });
        }
        // The glyph atlas: 4096 px at 100 % scale, following the scale up
        // and down (5120 at 125 %, 6144 at 150 %, 8192 from 200 %), since
        // glyphs grow with it: about 40 000 different Hangul glyphs before a
        // rebuild at any scale. Never over 8192: egui makes the atlas as wide
        // as this, and with the GPU's own limit (16384 and more) it grew into
        // textures of hundreds of MB, reallocated and uploaded whole each time
        // it doubled. A scale change rebuilds the fonts anyway, so changing
        // the size with it costs nothing more. Images, PDF pages and diagrams
        // don't go by this value (media::load_texture): they keep their size.
        // WEFT_MAX_TEXTURE sets another (to try things).
        let scaled = ((4096.0 * ctx.pixels_per_point() / 1024.0).ceil() as usize * 1024).clamp(4096, 8192);
        let cap = std::env::var("WEFT_MAX_TEXTURE").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(scaled);
        let side = raw.max_texture_side.unwrap_or(cap).min(cap).max(1024);
        let off = std::env::var("WEFT_ATLAS_REBUILD").is_ok_and(|v| v == "off");
        if !off && !self.first_frame && ctx.fonts(|f| f.font_atlas_fill_ratio()) > 0.5 {
            self.atlas_flip = !self.atlas_flip;
        }
        raw.max_texture_side = Some(side - self.atlas_flip as usize);
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some((since, frames, causes)) = self.repaint_debug.as_mut() {
            *frames += 1;
            for c in ctx.repaint_causes() {
                *causes.entry(format!("{}:{} {}", c.file, c.line, c.reason)).or_default() += 1;
            }
            if since.elapsed() >= Duration::from_secs(5) {
                let list: Vec<String> = causes.iter().map(|(k, n)| format!("{n}× {k}")).collect();
                self.app.data.log(&format!("repaint: {frames} frames in 5 s; {}", list.join("; ")));
                *since = Instant::now();
                *frames = 0;
                causes.clear();
            }
        }
        // Health, to find what makes a long-running weft misbehave (it
        // flickered after hours): egui rebuilds every font when the glyph
        // atlas is 80 % full, a frame that uploads everything again.
        {
            let (since, frames, last_fill) = &mut self.health;
            *frames += 1;
            let fill = ctx.fonts(|f| f.font_atlas_fill_ratio());
            if *last_fill > 0.5 && fill < *last_fill * 0.5 {
                // Past 50 % is weft's own rebuild; 80 % and up means egui had to.
                self.app.data.log(&format!("health: font atlas rebuilt at {:.0}%", *last_fill * 100.0));
            }
            *last_fill = fill;
            let every = std::env::var("WEFT_HEALTH_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
            if since.elapsed() >= Duration::from_secs(every) {
                let textures = ctx.tex_manager().read().num_allocated();
                let side = ctx.fonts(|f| f.max_texture_side());
                let lines: usize = self.app.panes.values().map(|p| p.term.history.len()).sum();
                self.app.data.log(&format!(
                    "health: {frames} frames in {every} s, font atlas {:.0}% of {side}px, {textures} textures, {} panes, {lines} scrollback lines",
                    fill * 100.0,
                    self.app.panes.len()
                ));
                *since = Instant::now();
                *frames = 0;
            }
        }
        // The display: logged when it changes, to tell what went before a
        // window misbehaving (moved, a monitor woken or plugged).
        {
            let now = ctx.input(|i| (i.viewport().native_pixels_per_point.unwrap_or(1.0), i.viewport().monitor_size));
            if self.display != Some(now) {
                let was = self.display.map(|(s, m)| format!("{:.2}x on {}", s, m.map(|m| format!("{:.0}x{:.0}", m.x, m.y)).unwrap_or("?".into()))).unwrap_or("start".into());
                self.app.data.log(&format!("display: {was} -> {:.2}x on {}", now.0, now.1.map(|m| format!("{:.0}x{:.0}", m.x, m.y)).unwrap_or("?".into())));
                self.display = Some(now);
            }
        }
        let app = &mut self.app;
        app.frame(ctx);
        // Files dragged out of the window: the system's drag takes them.
        if self.first_frame {
            app.drag_out_ok = dragout::possible(frame);
        }
        if let Some(paths) = app.drag_out.take() {
            app.data.log(&format!("drag out: {} items", paths.len()));
            let (host, src) = app.drag_src.take().unwrap_or_default();
            match dragout::start(frame, paths.clone()) {
                Ok(()) => {
                    app.dragged_out = Some(dragout::Copies::new(host, src, paths));
                    // The system's drag took the release (Windows: it is over
                    // now; elsewhere it goes on without weft).
                    app.release_pointer = true;
                    ctx.request_repaint();
                }
                Err(e) => {
                    app.data.log(&format!("drag out: {e}"));
                    app.toast(&e, app::ToastKind::Error);
                }
            }
        }
        let title = app.active_pane().map(|p| format!("weft — {}", p.title())).unwrap_or_else(|| "weft".into());
        // Only on change: every viewport command asks for another frame, so
        // sending it each frame kept weft redrawing ~100 times a second.
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.first_frame {
            self.first_frame = false;
            self.app.data.log(&format!("startup: first frame built at {}", since_start()));
        }
    }
}

/// `weft --askpass PROMPT`: ssh runs this (SSH_ASKPASS) when weft supplied a
/// password. Host key questions are shown to the user, never auto-accepted.
fn askpass(prompt: &str) -> i32 {
    // The ticket is spent only on a password prompt, not on a host key question.
    let secret = if prompt.contains("(yes/no") { None } else { secret::fetch_password().map(|p| p.to_string()) };
    match host::askpass_answer(prompt, secret) {
        host::AskpassAnswer::Secret(s) => {
            println!("{s}");
            0
        }
        host::AskpassAnswer::ConfirmHostKey => {
            // Asked in weft's window (on every system); no weft to ask: no.
            println!("{}", if secret::ask_host_key(prompt).unwrap_or(false) { "yes" } else { "no" });
            0
        }
        host::AskpassAnswer::Refuse => 1,
    }
}

/// Started from a terminal (`cargo run --release`, `weft.exe` in a shell),
/// weft inherits that terminal's console handles. A window app never uses
/// them, but while any process holds them the terminal can't close its tab
/// (it waits for them), and the processes weft spawns would inherit them
/// too. Let go of them at once.
#[cfg(all(windows, not(debug_assertions)))]
fn release_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> isize;
        fn SetStdHandle(which: u32, handle: isize) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn FreeConsole() -> i32;
    }
    // STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE
    let which = [-10i32 as u32, -11i32 as u32, -12i32 as u32];
    unsafe {
        let mut seen: Vec<isize> = Vec::new();
        for w in which {
            let h = GetStdHandle(w);
            // 0: none; -1: INVALID_HANDLE_VALUE. stdout and stderr may share one.
            if h != 0 && h != -1 && !seen.contains(&h) {
                seen.push(h);
                CloseHandle(h);
            }
            SetStdHandle(w, 0);
        }
        FreeConsole();
    }
}

/// The window and taskbar icon (assets/weft.png, 256 px; made by
/// assets/make_icon.py, which also writes the .ico built into weft.exe).
fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/weft.png")).expect("assets/weft.png").to_rgba8();
    egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
}

fn main() -> eframe::Result<()> {
    STARTED.get_or_init(Instant::now);
    let args: Vec<String> = std::env::args().collect();
    // The window to be (a restart's, through the launcher), not handed on
    // to what this weft starts (its shells, more wefts).
    let window_env = std::env::var("WEFT_WINDOW").ok();
    std::env::remove_var("WEFT_WINDOW");
    // `weft --version` (or -V): say which weft this is, and leave. Before
    // the console is let go of below, so a shell that started it sees it.
    if args.iter().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("{}", version::line());
        return Ok(());
    }
    // ssh invokes SSH_ASKPASS as `weft.exe "<prompt>"` with a ticket in the environment.
    let as_askpass = args.get(1).map(|a| a == "--askpass").unwrap_or(false)
        || (std::env::var_os(secret::TICKET_VAR).is_some() && args.len() == 2);
    if as_askpass {
        std::process::exit(askpass(args.last().map(|s| s.as_str()).unwrap_or("")));
    }
    secret::harden_process();
    #[cfg(all(windows, not(debug_assertions)))]
    release_console();
    let renderer = renderer_choice();
    let options = eframe::NativeOptions {
        renderer,
        wgpu_options: wgpu_options(),
        viewport: egui::ViewportBuilder::default()
            .with_title("weft")
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([720.0, 420.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    let ran = eframe::run_native(
        "weft",
        options,
        Box::new(|cc| {
            let window = since_start();
            theme::apply(&cc.egui_ctx, 0);
            highlight::preload(&cc.egui_ctx);
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            // The window to be (a restart's, or one opened to come back).
            let wanted = args.iter().position(|a| a == "--window").and_then(|i| args.get(i + 1).cloned()).or(window_env.clone());
            let reopening = args.iter().any(|a| a == "--reopen");
            let mut app = App::new_window(cc.egui_ctx.clone(), wanted, reopening);
            // Started to open a window another weft took meanwhile: not a
            // new empty one, nothing.
            if reopening && app.window_missed {
                app.data.log("windows: the window to open was taken by another weft");
                drop(app);
                std::process::exit(0);
            }
            // The other windows that come back: a weft each.
            if let Ok(me) = std::env::current_exe() {
                for id in std::mem::take(&mut app.windows_to_open) {
                    let _ = std::process::Command::new(&me).arg("--window").arg(&id).arg("--reopen").spawn();
                }
            }
            // Fonts come from the settings, which App::new reads.
            let before_fonts = since_start();
            app.apply_fonts();
            let fonts = format!("{} (from {before_fonts})", since_start());
            app.data.log(&format!("startup: window+GL {window}, fonts read {fonts}, app+panes {}", since_start()));
            app.graphics = graphics_info(cc);
            app.data.log(&format!("graphics: {}", app.graphics));
            let repaint_debug = std::env::var_os("WEFT_DEBUG_REPAINT").map(|_| (Instant::now(), 0, Default::default()));
            Ok(Box::new(WeftApp { app, display: None, first_frame: true, repaint_debug, title: String::new(), health: (Instant::now(), 0, 0.0), atlas_flip: false }))
        }),
    );
    // wgpu could not make the window (no DirectX 12 / Vulkan / Metal device,
    // a driver too old): weft starts again on OpenGL, once, and says so.
    if let Err(e) = &ran {
        if renderer == eframe::Renderer::Wgpu && std::env::var_os("WEFT_RENDERER_FELL_BACK").is_none() {
            host::DataDir::locate().log(&format!("graphics: wgpu failed ({e}); starting again on OpenGL"));
            // (The window asked for through the launcher, handed on too.)
            let again = std::env::current_exe().and_then(|me| {
                let mut c = std::process::Command::new(me);
                c.args(std::env::args_os().skip(1)).env("WEFT_RENDERER", "opengl").env("WEFT_RENDERER_FELL_BACK", "1");
                if let Some(w) = &window_env {
                    c.env("WEFT_WINDOW", w);
                }
                c.spawn()
            });
            if again.is_ok() {
                return Ok(());
            }
        }
    }
    ran
}

#[cfg(test)]
mod renderer_tests {
    use super::renderer_for;
    use eframe::Renderer::{Glow, Wgpu};

    /// The graphics backend: what is asked for, else the system's steadiest
    /// (wgpu on Windows and macOS, OpenGL elsewhere); wgpu on DirectX 12 on
    /// Windows.
    #[test]
    fn the_graphics_backend_by_system_and_setting() {
        assert_eq!(renderer_for("", true), Wgpu, "Windows, macOS");
        assert_eq!(renderer_for("auto", false), Glow, "Linux");
        assert_eq!(renderer_for("opengl", true), Glow);
        assert_eq!(renderer_for("glow", true), Glow);
        assert_eq!(renderer_for("wgpu", false), Wgpu);
        assert_eq!(renderer_for("dx12", false), Wgpu);
        assert_eq!(renderer_for(" metal ", false), Wgpu);
        assert_eq!(renderer_for("nonsense", true), Wgpu, "not known: the system's own");
        assert_eq!(renderer_for("nonsense", false), Glow);
        let o = super::wgpu_options();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(c) = &o.wgpu_setup {
            if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() {
                assert_eq!(c.instance_descriptor.backends, eframe::wgpu::Backends::DX12, "DirectX 12 on Windows");
            }
        }
    }
}
