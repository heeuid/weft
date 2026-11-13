//! Regression tests drive the real weft, headless: the same `App::frame` the
//! window runs, fed pointer and key events frame by frame, checked by its
//! state and by what it drew (the texts on screen and where).
//!
//! Each harness has its own data folder (settings, memos) and holds
//! `TEST_APP` while it lives, since WEFT_DATA_DIR is process-wide.

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
    pointer: Pos2,
    _guard: std::sync::MutexGuard<'static, ()>,
}

/// A terminal that just sits there (python), so no real shell is needed.
pub fn idle_profile(name: &str) -> Value {
    json!({"name": name, "kind": "custom", "pty": ["python", "-c", "import time; time.sleep(120)"], "agent": ["python", "-c", "0"]})
}

impl Harness {
    /// weft with these settings (merged over a quiet default: one idle tab,
    /// no side panel), a window of 1200×800.
    pub fn new(settings: Value) -> Harness {
        let guard = crate::app::TEST_APP.lock().unwrap_or_else(|e| e.into_inner());
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
        std::env::set_var("WEFT_DATA_DIR", &dir);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        let app = App::new(ctx.clone());
        let mut h = Harness { app, ctx, dir, size: vec2(1200.0, 800.0), texts: Vec::new(), pointer: pos2(600.0, 400.0), _guard: guard };
        h.frames(3);
        h
    }

    /// One frame with these events.
    pub fn frame(&mut self, events: Vec<Event>) {
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let input = egui::RawInput { events, modifiers, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)), ..Default::default() };
        let app = &mut self.app;
        let out = self.ctx.run(input, |ctx| app.frame(ctx));
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

    /// Where a drawn text containing `needle` is.
    pub fn find(&self, needle: &str) -> Option<Rect> {
        self.texts.iter().find(|(t, _)| t.contains(needle)).map(|(_, r)| *r)
    }

    pub fn at(&self, needle: &str) -> Pos2 {
        self.find(needle).unwrap_or_else(|| panic!("\"{needle}\" is not on screen; shown: {:?}", self.texts.iter().map(|t| &t.0).collect::<Vec<_>>())).center()
    }

    fn button(&self, pos: Pos2, button: egui::PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos, button, pressed, modifiers: Modifiers::default() }
    }

    /// Move there, press and release (over three frames, as a hand does).
    pub fn click(&mut self, pos: Pos2) {
        self.pointer = pos;
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![self.button(pos, egui::PointerButton::Primary, true)]);
        self.frame(vec![self.button(pos, egui::PointerButton::Primary, false)]);
        self.frames(1);
    }

    pub fn click_text(&mut self, needle: &str) {
        let p = self.at(needle);
        self.click(p);
    }

    pub fn right_click(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![self.button(pos, egui::PointerButton::Secondary, true)]);
        self.frame(vec![self.button(pos, egui::PointerButton::Secondary, false)]);
        self.frames(1);
    }

    pub fn double_click(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        for _ in 0..2 {
            self.frame(vec![self.button(pos, egui::PointerButton::Primary, true)]);
            self.frame(vec![self.button(pos, egui::PointerButton::Primary, false)]);
        }
        self.frames(1);
    }

    /// Press at `from`, move to `to` in steps, release there.
    pub fn drag(&mut self, from: Pos2, to: Pos2) {
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
