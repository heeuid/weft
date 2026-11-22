//! Regression tests drive the real weft, headless: the same `App::frame` the
//! window runs, fed pointer and key events frame by frame, checked by its
//! state and by what it drew (the texts on screen and where).
//!
//! Each harness has its own data folder (settings, memos), handed to its
//! App: nothing is shared through the process's environment, so the tests
//! run side by side.

use crate::app::App;
use egui::{pos2, vec2, Event, Key, Modifiers, Pos2, Rect};
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct Harness {
    pub app: App,
    pub ctx: egui::Context,
    pub dir: PathBuf,
    pub size: egui::Vec2,
    /// What the last frame drew: each text and its rect.
    pub texts: Vec<(String, Rect)>,
    /// The icon buttons the last frame drew: each one's tip and its rect.
    pub icons: Vec<(String, Rect)>,
    /// The last frame's shapes and scale, and the textures so far: what
    /// `shot` draws from.
    shapes: Vec<egui::epaint::ClippedShape>,
    scale: f32,
    textures: crate::shot::Textures,
    /// The last text weft put on the clipboard.
    pub copied: Option<String>,
    /// What weft asked of its window in the last frame (Close, CancelClose…),
    /// and in every frame so far.
    pub window: Vec<egui::ViewportCommand>,
    pub window_log: Vec<egui::ViewportCommand>,
    close_window: bool,
    /// The modifier keys held down across frames (Alt for an Alt+drag).
    pub held: Modifiers,
    pointer: Pos2,
    /// The display's own scale (a Mac's Retina is 2): egui's pixels per
    /// point are this times the app's zoom, which the app sets every frame
    /// from settings ("ui_scale"), so a zoom set by a test lasted one frame.
    pub native_scale: f32,
    /// The clock the app is shown: a frame is 1/60 s, and a hand pauses
    /// before each click or drag (else clicks far apart count as double).
    pub now: f64,
}

/// The Python the tests run: "python" on Windows ("python3" there is the
/// Store's stand-in, which only prints "Python"); elsewhere "python3",
/// which macOS and most Linuxes have (and often no "python").
pub fn py() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

/// A terminal that just sits there (python), so no real shell is needed.
pub fn idle_profile(name: &str) -> Value {
    json!({"name": name, "kind": "custom", "pty": [py(), "-c", "import time; time.sleep(120)"], "agent": [py(), "-c", "0"]})
}

#[allow(dead_code)]
impl Harness {
    /// weft with these settings (merged over a quiet default: one idle tab,
    /// no side panel), a window of 1200×800.
    pub fn new(settings: Value) -> Harness {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("weft-kit-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut base = json!({"profiles": [idle_profile("t")], "session": [{"t": "Pane", "host": "t", "cwd": null}], "sidebar": false});
        if let (Some(b), Some(s)) = (base.as_object_mut(), settings.as_object()) {
            for (k, v) in s {
                b.insert(k.clone(), v.clone());
            }
        }
        std::fs::write(dir.join("settings.json"), base.to_string()).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        crate::icons::watch();
        let mut app = App::new_in(ctx.clone(), crate::host::DataDir::at(&dir));
        // As main does: the fonts the settings name (not only the default).
        app.apply_fonts();
        let mut h = Harness { app, ctx, dir, size: vec2(1200.0, 800.0), texts: Vec::new(), icons: Vec::new(), shapes: Vec::new(), scale: 1.0, textures: Default::default(), copied: None, window: Vec::new(), window_log: Vec::new(), close_window: false, held: Modifiers::NONE, now: 0.0, pointer: pos2(600.0, 400.0), native_scale: 1.0 };
        h.frames(3);
        h
    }

    /// The window's × (or Alt+F4): one frame with the close asked.
    pub fn close_window(&mut self) {
        self.close_window = true;
        self.frames(1);
    }

    /// Close weft and start it again on the same data folder, as a restart
    /// does: what it saved is what the new one starts from.
    pub fn restart(&mut self) {
        // As the window's × does: weft saves on its way out, by itself.
        self.close_window();
        assert!(!self.window.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose)), "weft closes without a question");
        self.app = App::new_in(self.ctx.clone(), crate::host::DataDir::at(&self.dir));
        self.app.apply_fonts();
        self.frames(3);
    }

    /// One frame with these events.
    pub fn frame(&mut self, events: Vec<Event>) {
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(self.held);
        self.now += 1.0 / 60.0;
        let mut input = egui::RawInput { events, modifiers, time: Some(self.now), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(self.native_scale);
        if std::mem::take(&mut self.close_window) {
            input.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        }
        let app = &mut self.app;
        crate::icons::seen();
        let out = self.ctx.run(input, |ctx| app.frame(ctx));
        self.icons = crate::icons::seen();
        self.window = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
        self.window_log.extend(self.window.iter().cloned());
        for c in &out.platform_output.commands {
            if let egui::OutputCommand::CopyText(t) = c {
                self.copied = Some(t.clone());
            }
        }
        let mut texts = Vec::new();
        fn collect(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2()))),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        for c in &out.shapes {
            collect(&c.shape, &mut texts);
        }
        self.texts = texts;
        self.textures.update(&out.textures_delta);
        self.scale = out.pixels_per_point;
        self.shapes = out.shapes;
    }

    pub fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(Vec::new());
        }
    }

    /// Is this text on screen (a drawn text containing it)?
    pub fn shows(&self, needle: &str) -> bool {
        self.texts.iter().any(|(t, _)| t.contains(needle))
    }

    /// The last frame as an image, pixel for pixel what a window would show.
    pub fn shot(&self) -> image::RgbaImage {
        let (w, h) = ((self.size.x * self.scale).round() as usize, (self.size.y * self.scale).round() as usize);
        self.textures.render(&self.ctx.tessellate(self.shapes.clone(), self.scale), self.scale, w, h)
    }

    /// The last frame saved as target/shots/<name>.png, to be looked at.
    pub fn save_shot(&self, name: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("shots");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.png"));
        self.shot().save(&path).unwrap();
        path
    }

    /// Is a text drawn that is exactly this (a row's name, say: not a
    /// message that mentions it)?
    pub fn has(&self, text: &str) -> bool {
        self.texts.iter().any(|(t, _)| t == text)
    }

    /// Where the text that is exactly this is drawn.
    pub fn at_exact(&self, text: &str) -> Pos2 {
        self.texts.iter().find(|(t, _)| t == text).unwrap_or_else(|| panic!("\"{text}\" is not on screen; shown: {:?}", self.texts.iter().map(|t| &t.0).collect::<Vec<_>>())).1.center()
    }

    /// Where a drawn text containing `needle` is.
    pub fn find(&self, needle: &str) -> Option<Rect> {
        self.texts.iter().find(|(t, _)| t.contains(needle)).map(|(_, r)| *r)
    }

    pub fn at(&self, needle: &str) -> Pos2 {
        self.find(needle).unwrap_or_else(|| panic!("\"{needle}\" is not on screen; shown: {:?}", self.texts.iter().map(|t| &t.0).collect::<Vec<_>>())).center()
    }

    fn button(&self, pos: Pos2, button: egui::PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos, button, pressed, modifiers: self.held }
    }

    /// Move there, press and release (over three frames, as a hand does).
    pub fn click(&mut self, pos: Pos2) {
        self.pointer = pos;
        self.now += 1.0;
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![self.button(pos, egui::PointerButton::Primary, true)]);
        self.frame(vec![self.button(pos, egui::PointerButton::Primary, false)]);
        self.frames(1);
    }

    pub fn click_text(&mut self, needle: &str) {
        let p = self.at(needle);
        self.click(p);
    }

    /// Where the icon button whose tip contains `tip` is.
    pub fn icon(&self, tip: &str) -> Pos2 {
        self.icons.iter().find(|(t, _)| t.contains(tip)).unwrap_or_else(|| panic!("no icon button \"{tip}\"; drawn: {:?}", self.icons.iter().map(|t| &t.0).collect::<Vec<_>>())).1.center()
    }

    pub fn click_icon(&mut self, tip: &str) {
        let p = self.icon(tip);
        self.click(p);
    }

    /// Turn the wheel at `pos` by `lines` (down is negative, as egui has it).
    pub fn wheel(&mut self, pos: Pos2, lines: f32, modifiers: Modifiers) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, lines), modifiers }]);
        self.frames(2);
    }

    /// A click with modifier keys held (Ctrl+click, Shift+click).
    pub fn click_with(&mut self, pos: Pos2, modifiers: Modifiers) {
        self.now += 1.0;
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers }]);
        self.frame(vec![Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers }]);
        self.frames(1);
    }

    /// Put the pointer there and let a frame or two see it.
    pub fn hover(&mut self, pos: Pos2) {
        self.pointer = pos;
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frames(2);
    }

    /// Press and release `times` in a row at one spot (2: a word, 3: a line).
    pub fn clicks(&mut self, pos: Pos2, times: usize) {
        self.now += 1.0;
        self.frame(vec![Event::PointerMoved(pos)]);
        for _ in 0..times {
            self.frame(vec![self.button(pos, egui::PointerButton::Primary, true)]);
            self.frame(vec![self.button(pos, egui::PointerButton::Primary, false)]);
        }
        self.frames(1);
    }

    pub fn right_click(&mut self, pos: Pos2) {
        self.now += 1.0;
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![self.button(pos, egui::PointerButton::Secondary, true)]);
        self.frame(vec![self.button(pos, egui::PointerButton::Secondary, false)]);
        self.frames(1);
    }

    pub fn double_click(&mut self, pos: Pos2) {
        self.now += 1.0;
        self.frame(vec![Event::PointerMoved(pos)]);
        for _ in 0..2 {
            self.frame(vec![self.button(pos, egui::PointerButton::Primary, true)]);
            self.frame(vec![self.button(pos, egui::PointerButton::Primary, false)]);
        }
        self.frames(1);
    }

    /// Press at `from`, move to `to` in steps, release there.
    pub fn drag(&mut self, from: Pos2, to: Pos2) {
        self.now += 1.0;
        self.frame(vec![Event::PointerMoved(from)]);
        self.frame(vec![self.button(from, egui::PointerButton::Primary, true)]);
        for k in 1..=8 {
            self.frame(vec![Event::PointerMoved(from + (to - from) * k as f32 / 8.0)]);
        }
        self.frame(vec![self.button(to, egui::PointerButton::Primary, false)]);
        self.frames(1);
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
        self.frame(vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }]);
        self.frames(1);
    }

    /// Run a command the way a key or the palette does.
    pub fn command(&mut self, id: &str) {
        self.app.run_command_id(id);
        self.frames(2);
    }

    pub fn type_text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.into())]);
        self.frames(1);
    }

    /// Keep drawing until `cond` holds (output, agents and timers take a
    /// moment), up to `secs`.
    pub fn wait_for(&mut self, secs: f32, mut cond: impl FnMut(&Harness) -> bool) -> bool {
        let end = std::time::Instant::now() + std::time::Duration::from_secs_f32(secs);
        while std::time::Instant::now() < end {
            self.frames(1);
            if cond(self) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        false
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
