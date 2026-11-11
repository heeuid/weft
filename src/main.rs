#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! weft — a terminal that weaves GUI into the TUI grid.
//!
//! Thick local client (rendering, editing, search, history, completion
//! ranking all happen here); the remote side only ever needs ssh, POSIX
//! tools and python, and nothing is written to the remote disk.

mod app;
mod highlight;
mod host;
mod icons;
mod md;
mod mermaid;
mod pty;
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
}

impl eframe::App for WeftApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

        egui::TopBottomPanel::top("tabs")
            .exact_height(34.0)
            .frame(egui::Frame::new().fill(theme::bg_base()).inner_margin(egui::Margin { left: 6, right: 6, top: 4, bottom: 0 }))
            .show(ctx, |ui| app.ui_tabbar(ui));
        egui::TopBottomPanel::bottom("status")
            .exact_height(24.0)
            .frame(egui::Frame::new().fill(theme::bg_base()).inner_margin(egui::Margin::symmetric(10, 0)))
            .show(ctx, |ui| app.ui_status(ui));
        let (w, h) = (ctx.screen_rect().width(), ctx.screen_rect().height());
        if app.settings.sidebar {
            let frame = egui::Frame::new().fill(theme::bg_base()).inner_margin(egui::Margin::symmetric(8, 0));
            let size = DockSize { width: 280.0, widths: 200.0..=520.0, height: 260.0, heights: 140.0..=h * 0.7 };
            dock_panel(ctx, "side", app.settings.side_dock, size, frame, |ui| app.ui_sidebar(ui));
        }
        if app.editor.visible {
            let frame = egui::Frame::new().fill(theme::bg_base()).inner_margin(egui::Margin::symmetric(8, 6));
            let size = DockSize { width: (w * 0.42).max(360.0), widths: 280.0..=w * 0.75, height: h * 0.45, heights: 160.0..=h * 0.8 };
            dock_panel(ctx, "editor", app.settings.editor_dock, size, frame, |ui| app.ui_editor(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::bg_term())).show(ctx, |ui| app.ui_panes(ui));

        app.ui_palette(ctx);
        app.ui_connect(ctx);
        app.ui_auth(ctx);
        app.ui_theme_menu(ctx);
        app.ui_toasts(ctx);
        app.ui_dock_drop(ctx);
        app.apply_actions();

        let title = app.active_pane().map(|p| format!("weft — {}", p.title())).unwrap_or_else(|| "weft".into());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        if app.panes.values().any(|p| p.term.blocks.last().map(|b| b.started.is_some() && b.end_line.is_none()).unwrap_or(false)) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            app.save_settings();
        }
        if self.first_frame {
            self.first_frame = false;
            self.app.data.log(&format!("startup: first frame built at {}", since_start()));
        }
    }
}

struct DockSize {
    width: f32,
    widths: std::ops::RangeInclusive<f32>,
    height: f32,
    heights: std::ops::RangeInclusive<f32>,
}

/// A resizable panel on the window edge `dock`. Each edge keeps its own size.
fn dock_panel(ctx: &egui::Context, id: &str, dock: app::Dock, size: DockSize, frame: egui::Frame, add: impl FnOnce(&mut egui::Ui)) {
    let id = format!("{id}-{}", dock.label());
    match dock {
        app::Dock::Left | app::Dock::Right => {
            let p = if dock == app::Dock::Left { egui::SidePanel::left(id) } else { egui::SidePanel::right(id) };
            p.resizable(true).default_width(size.width).width_range(size.widths).frame(frame).show(ctx, add);
        }
        app::Dock::Top | app::Dock::Bottom => {
            let p = if dock == app::Dock::Top { egui::TopBottomPanel::top(id) } else { egui::TopBottomPanel::bottom(id) };
            p.resizable(true).default_height(size.height).height_range(size.heights).frame(frame).show(ctx, add);
        }
    }
}

/// `weft --askpass PROMPT`: ssh runs this (SSH_ASKPASS) when weft supplied a
/// password. Host key questions are shown to the user, never auto-accepted.
fn askpass(prompt: &str) -> i32 {
    match host::askpass_answer(prompt, std::env::var(host::ASKPASS_SECRET_VAR).ok()) {
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

#[cfg(not(windows))]
fn confirm_host_key(_text: &str) -> bool {
    false
}

fn main() -> eframe::Result<()> {
    STARTED.get_or_init(Instant::now);
    let args: Vec<String> = std::env::args().collect();
    // ssh invokes SSH_ASKPASS as `weft.exe "<prompt>"` with our secret in the environment.
    let as_askpass = args.get(1).map(|a| a == "--askpass").unwrap_or(false)
        || (std::env::var_os(host::ASKPASS_SECRET_VAR).is_some() && args.len() == 2);
    if as_askpass {
        std::process::exit(askpass(args.last().map(|s| s.as_str()).unwrap_or("")));
    }
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
            theme::install_fonts(&cc.egui_ctx);
            let fonts = since_start();
            theme::apply(&cc.egui_ctx, 0);
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            let app = App::new(cc.egui_ctx.clone());
            app.data.log(&format!("startup: window+GL {window}, fonts read {fonts}, app+panes {}", since_start()));
            Ok(Box::new(WeftApp { app, first_frame: true }))
        }),
    )
}
