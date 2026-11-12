#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! weft — a terminal that weaves GUI into the TUI grid.
//!
//! Thick local client (rendering, editing, search, history, completion
//! ranking all happen here); the remote side only ever needs ssh, POSIX
//! tools and python, and nothing is written to the remote disk.

mod app;
mod copymode;
mod editfind;
mod help;
mod highlight;
mod host;
mod find;
mod icons;
mod keys;
mod md;
mod media;
mod mermaid;
mod pty;
mod records;
mod secret;
mod term;
mod theme;
mod ui_panels;
mod ui_term;
mod widgets;

use app::App;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Process start, for the startup timings in weft.log.
static STARTED: OnceLock<Instant> = OnceLock::new();

pub fn since_start() -> String {
    format!("{} ms", STARTED.get().map(|t| t.elapsed().as_millis()).unwrap_or(0))
}

struct WeftApp {
    app: App,
    first_frame: bool,
    /// WEFT_DEBUG_REPAINT=1: frames per 5 s and what asked for them, to the log.
    repaint_debug: Option<(Instant, u32, std::collections::BTreeMap<String, u32>)>,
    /// The window title last sent.
    title: String,
}

impl eframe::App for WeftApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
        let app = &mut self.app;
        let wanted = theme::resolve(&app.settings.theme, &app.settings.theme_dark, &app.settings.theme_light, ctx);
        if wanted != theme::active() {
            theme::apply(ctx, wanted);
        }
        if (ctx.zoom_factor() - app.settings.ui_scale).abs() > 0.001 {
            ctx.set_zoom_factor(app.settings.ui_scale);
        }
        app.poll();
        app.handle_global_keys(ctx);

        // Focus mode: no status bar either.
        if app.focus_mode.is_none() {
            egui::TopBottomPanel::bottom("status")
                .exact_height(24.0)
                .frame(egui::Frame::new().fill(theme::bg_base()).inner_margin(egui::Margin::symmetric(10, 0)))
                .show(ctx, |ui| app.ui_status(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::bg_base())).show(ctx, |ui| app.ui_layout(ui));

        app.ui_palette(ctx);
        app.ui_connect(ctx);
        app.ui_auth(ctx);
        app.ui_theme_menu(ctx);
        app.ui_font_menu(ctx);
        app.ui_records_window(ctx);
        app.ui_help(ctx);
        app.ui_memo_picker(ctx);
        app.ui_keys_window(ctx);
        app.ui_toasts(ctx);
        app.ui_dock_drop(ctx);
        app.apply_actions();

        let title = app.active_pane().map(|p| format!("weft — {}", p.title())).unwrap_or_else(|| "weft".into());
        // Only on change: every viewport command asks for another frame, so
        // sending it each frame kept weft redrawing ~100 times a second.
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        // A running command's time badge counts seconds: once a second is
        // enough, and not at all under a full-screen app (no badge there).
        if app.panes.values().any(|p| !p.term.alt_active && p.term.blocks.last().map(|b| b.started.is_some() && b.end_line.is_none()).unwrap_or(false)) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            app.flush_memos();
            app.save_settings();
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
            println!("{}", if confirm_host_key(prompt) { "yes" } else { "no" });
            0
        }
        host::AskpassAnswer::Refuse => 1,
    }
}

#[cfg(windows)]
fn confirm_host_key(text: &str) -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, flags: u32) -> i32;
    }
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (t, c) = (wide(text), wide("weft — ssh host key"));
    // MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND | MB_TOPMOST; IDYES = 6
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), 0x4 | 0x20 | 0x10000 | 0x40000) == 6 }
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

#[cfg(not(windows))]
fn confirm_host_key(_text: &str) -> bool {
    false
}

fn main() -> eframe::Result<()> {
    STARTED.get_or_init(Instant::now);
    let args: Vec<String> = std::env::args().collect();
    // ssh invokes SSH_ASKPASS as `weft.exe "<prompt>"` with a ticket in the environment.
    let as_askpass = args.get(1).map(|a| a == "--askpass").unwrap_or(false)
        || (std::env::var_os(secret::TICKET_VAR).is_some() && args.len() == 2);
    if as_askpass {
        std::process::exit(askpass(args.last().map(|s| s.as_str()).unwrap_or("")));
    }
    secret::harden_process();
    #[cfg(all(windows, not(debug_assertions)))]
    release_console();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("weft")
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([720.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "weft",
        options,
        Box::new(|cc| {
            let window = since_start();
            theme::apply(&cc.egui_ctx, 0);
            highlight::preload(&cc.egui_ctx);
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            let mut app = App::new(cc.egui_ctx.clone());
            // Fonts come from the settings, which App::new reads.
            let before_fonts = since_start();
            app.apply_fonts();
            let fonts = format!("{} (from {before_fonts})", since_start());
            app.data.log(&format!("startup: window+GL {window}, fonts read {fonts}, app+panes {}", since_start()));
            let repaint_debug = std::env::var_os("WEFT_DEBUG_REPAINT").map(|_| (Instant::now(), 0, Default::default()));
            Ok(Box::new(WeftApp { app, first_frame: true, repaint_debug, title: String::new() }))
        }),
    )
}
