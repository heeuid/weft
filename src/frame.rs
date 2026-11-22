//! One frame of weft: everything drawn and handled, in order. The window
//! (main.rs) calls it, and so do the tests (testkit.rs), so they check the
//! app as it runs rather than pieces of it.

use crate::app::App;
use crate::theme;
use std::time::Duration;

impl App {
    pub fn frame(&mut self, ctx: &egui::Context) {
        // Fonts found for characters weft's lack, put in for this frame.
        weft_draw::fallback::poll(ctx);
        let wanted = theme::resolve(&self.settings.theme, &self.settings.theme_dark, &self.settings.theme_light, ctx);
        if wanted != theme::active() {
            theme::apply(ctx, wanted);
        }
        if (ctx.zoom_factor() - self.settings.ui_scale).abs() > 0.001 {
            ctx.set_zoom_factor(self.settings.ui_scale);
        }
        self.poll();
        self.handle_global_keys(ctx);
        self.escape_closes_windows(ctx);

        // Focus mode: no status bar either.
        if self.focus_mode.is_none() {
            egui::TopBottomPanel::bottom("status")
                .exact_height(24.0)
                .frame(egui::Frame::new().fill(theme::bg_base()).inner_margin(egui::Margin::symmetric(10, 0)))
                .show(ctx, |ui| self.ui_status(ui));
        }
        self.check_update();
        self.drop_dirs.clear();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::bg_base())).show(ctx, |ui| self.ui_layout(ui));
        self.ui_file_drop(ctx);
        self.ui_transfers(ctx);

        self.ui_palette(ctx);
        self.ui_connect(ctx);
        self.ui_auth(ctx);
        self.ui_theme_menu(ctx);
        self.ui_font_menu(ctx);
        self.ui_records_window(ctx);
        self.ui_graphics_window(ctx);
        self.ui_python_window(ctx);
        self.ui_shells_window(ctx);
        self.ui_help(ctx);
        self.ui_memo_picker(ctx);
        self.ui_keys_window(ctx);
        self.ui_serial_dialog(ctx);
        self.ui_serial_log(ctx);
        self.ui_settings_window(ctx);
        self.ui_file_op(ctx);
        self.ui_confirm(ctx);
        self.ui_host_key(ctx);
        self.ui_display_test(ctx);
        self.ui_toasts(ctx);
        self.ui_dock_drop(ctx);
        self.apply_actions();
        for f in &mut self.editor.files {
            f.image.unload_hidden();
            f.pdf.unload_hidden();
        }

        // A running command's time badge counts seconds: once a second is
        // enough, and not at all under a full-screen app (no badge there).
        if self.panes.values().any(|p| !p.term.alt_active && p.term.blocks.last().map(|b| b.started.is_some() && b.end_line.is_none()).unwrap_or(false)) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.may_quit(false) {
                self.flush_memos();
                self.save_settings();
                // (Its window's lock let go of at once, closing takes a
                // moment more: another weft closing now sees this one gone.)
                crate::instances::leave(&self.data, &self.window_id, self.handing_over, &mut self.window_lock);
            } else {
                // Unsaved files or running commands: asked first; the answer
                // closes the window.
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
    }
}
