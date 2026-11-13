//! File transfer between any two hosts: local and remote, or two remotes.
//! weft relays the bytes: chunks read through the source host's agent (or
//! the local file system) are written through the destination's, so the
//! hosts never connect to each other and nothing is installed on either.
//!
//! One request is in flight per transfer at a time: walk the source, find a
//! free name at the destination ("name (1)" when taken; nothing is ever
//! overwritten), make the folders, then for each file read a chunk and write
//! it into "<name>.weft-part", renamed into place when the file is complete.

use crate::app::{Action, App, Pending, ToastKind};
use serde_json::{json, Value};
use std::time::Instant;

/// Bytes read and written per request.
pub const CHUNK: u64 = 512 * 1024;

/// What a transfer is waiting for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Step {
    Walk,
    /// Is the k-th candidate name free?
    Name(u32),
    Mkdirs,
    Read,
    Write { n: u64, eof: bool },
    Finish,
    Abort,
}

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct Item {
    /// Path below the folder sent ("" for a single file).
    pub rel: String,
    pub size: u64,
    pub dir: bool,
}

pub struct Transfer {
    pub id: u64,
    pub src_host: String,
    pub src_path: String,
    pub dst_host: String,
    pub dst_dir: String,
    /// Name at the destination (the source's, or "name (1)" when taken).
    pub name: String,
    pub is_dir: bool,
    pub items: Vec<Item>,
    /// The file being copied (index into `items`) and how far it is.
    pub file: usize,
    pub off: u64,
    pub total: u64,
    pub done: u64,
    pub state: State,
    /// A request is in flight.
    pub busy: bool,
    pub started: Instant,
    pub ended: Option<Instant>,
}

impl Transfer {
    pub fn dest(&self) -> String {
        join(&self.dst_dir, &self.name)
    }

    fn src_of(&self, i: &Item) -> String {
        if self.is_dir { join(&self.src_path, &i.rel) } else { self.src_path.clone() }
    }

    fn dst_of(&self, i: &Item) -> String {
        if self.is_dir { join(&self.dest(), &i.rel) } else { self.dest() }
    }

    fn part(&self) -> Option<String> {
        self.items.get(self.file).map(|i| format!("{}.weft-part", self.dst_of(i)))
    }

    /// Bytes a second so far.
    pub fn speed(&self) -> f64 {
        let secs = self.ended.unwrap_or_else(Instant::now).duration_since(self.started).as_secs_f64();
        if secs > 0.05 { self.done as f64 / secs } else { 0.0 }
    }

    pub fn files(&self) -> usize {
        self.items.iter().filter(|i| !i.dir).count()
    }

    /// Files finished so far.
    pub fn files_done(&self) -> usize {
        self.items[..self.file.min(self.items.len())].iter().filter(|i| !i.dir).count()
    }
}

/// `dir` joined with `name` with one '/', whatever the host (Windows paths
/// in weft use '/', and its file APIs take them).
pub fn join(dir: &str, name: &str) -> String {
    if name.is_empty() {
        dir.to_string()
    } else if dir.ends_with('/') || dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The last component of a path.
pub fn base_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Candidate `k` for a name at the destination: the name itself, then
/// "name (1).ext", "name (2).ext"… (a folder: "name (1)").
pub fn candidate(name: &str, dir: bool, k: u32) -> String {
    if k == 0 {
        return name.to_string();
    }
    match name.rfind('.').filter(|&i| i > 0 && !dir) {
        Some(i) => format!("{} ({k}){}", &name[..i], &name[i..]),
        None => format!("{name} ({k})"),
    }
}

/// The local folder downloads go to.
pub fn downloads_dir() -> String {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    format!("{}/Downloads", home.replace('\\', "/"))
}

impl App {
    /// Copy `src_path` (a file or a folder) from `src_host` into the folder
    /// `dst_dir` on `dst_host`.
    pub fn start_transfer(&mut self, src_host: &str, src_path: &str, dst_host: &str, dst_dir: &str) {
        self.next_transfer += 1;
        let id = self.next_transfer;
        self.transfers.push(Transfer {
            id,
            src_host: src_host.into(),
            src_path: src_path.into(),
            dst_host: dst_host.into(),
            dst_dir: dst_dir.into(),
            name: base_name(src_path),
            is_dir: false,
            items: Vec::new(),
            file: 0,
            off: 0,
            total: 0,
            done: 0,
            state: State::Running,
            busy: false,
            started: Instant::now(),
            ended: None,
        });
        self.data.log(&format!("transfer {id}: {src_host}:{src_path} -> {dst_host}:{dst_dir}"));
        self.xfer_request(id, true, json!({"op": "walk", "path": src_path}), Step::Walk);
    }

    pub fn cancel_transfer(&mut self, id: u64) {
        let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) else { return };
        if t.state != State::Running {
            return;
        }
        t.state = State::Cancelled;
        t.ended = Some(Instant::now());
        // A write still in flight would bring the part file back: remove it
        // once that request is answered (see on_transfer).
        if !t.busy {
            self.abort_part(id);
        }
    }

    fn abort_part(&mut self, id: u64) {
        let Some(t) = self.transfers.iter().find(|t| t.id == id) else { return };
        if t.items.is_empty() {
            return;
        }
        if let Some(part) = t.part() {
            self.xfer_request(id, false, json!({"op": "finish", "part": part, "abort": true}), Step::Abort);
        }
    }

    fn xfer_request(&mut self, id: u64, on_source: bool, req: Value, step: Step) {
        let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) else { return };
        t.busy = true;
        let host = if on_source { t.src_host.clone() } else { t.dst_host.clone() };
        self.request(&host, req, Pending::Xfer { t: id, step });
    }

    fn fail_transfer(&mut self, id: u64, err: String) {
        let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) else { return };
        t.state = State::Failed(err.clone());
        t.ended = Some(Instant::now());
        let name = t.name.clone();
        self.data.log(&format!("transfer {id} failed: {err}"));
        self.toast(format!("copying {name} failed: {err}"), ToastKind::Error);
        self.abort_part(id);
    }

    /// The next file to copy, or the end.
    fn next_file(&mut self, id: u64) {
        let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) else { return };
        while t.items.get(t.file).is_some_and(|i| i.dir) {
            t.file += 1;
        }
        t.off = 0;
        let Some(item) = t.items.get(t.file).cloned() else {
            t.state = State::Done;
            t.ended = Some(Instant::now());
            let (name, host, dir, n) = (t.name.clone(), t.dst_host.clone(), t.dst_dir.clone(), t.files());
            let what = if t.is_dir { format!("{name} ({n} files)") } else { name };
            self.data.log(&format!("transfer {id} done: {} bytes", t.done));
            self.toast(format!("copied {what} to {host}:{dir}"), ToastKind::Ok);
            self.actions.push(Action::ListDir(host, dir));
            return;
        };
        let src = t.src_of(&item);
        self.xfer_request(id, true, json!({"op": "rchunk", "path": src, "off": 0, "len": CHUNK}), Step::Read);
    }

    /// An answer to one of a transfer's requests.
    pub fn on_transfer(&mut self, id: u64, step: Step, ok: bool, r: &Value, err: String) {
        let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) else { return };
        t.busy = false;
        if step == Step::Abort {
            return;
        }
        if t.state != State::Running {
            // Cancelled while this was in flight: now the part can go.
            if t.state == State::Cancelled {
                self.abort_part(id);
            }
            return;
        }
        if !ok {
            self.fail_transfer(id, err);
            return;
        }
        match step {
            Step::Walk => {
                t.is_dir = r["dir"].as_bool().unwrap_or(false);
                t.items = r["items"]
                    .as_array()
                    .map(|a| a.iter().map(|i| Item { rel: i["p"].as_str().unwrap_or("").into(), size: i["s"].as_u64().unwrap_or(0), dir: i["d"].as_bool().unwrap_or(false) }).collect())
                    .unwrap_or_default();
                t.total = t.items.iter().map(|i| i.size).sum();
                let first = join(&t.dst_dir, &t.name);
                self.xfer_request(id, false, json!({"op": "stat", "path": first}), Step::Name(0));
            }
            Step::Name(k) => {
                if r["exists"].as_bool().unwrap_or(false) {
                    if k >= 999 {
                        self.fail_transfer(id, "no free name at the destination".into());
                        return;
                    }
                    let next = join(&t.dst_dir, &candidate(&base_name(&t.src_path), t.is_dir, k + 1));
                    self.xfer_request(id, false, json!({"op": "stat", "path": next}), Step::Name(k + 1));
                    return;
                }
                t.name = candidate(&base_name(&t.src_path), t.is_dir, k);
                if t.is_dir {
                    let dirs: Vec<String> = t.items.iter().filter(|i| i.dir).map(|i| i.rel.clone()).collect();
                    let root = t.dest();
                    self.xfer_request(id, false, json!({"op": "mkdirs", "root": root, "dirs": dirs}), Step::Mkdirs);
                } else {
                    self.next_file(id);
                }
            }
            Step::Mkdirs => self.next_file(id),
            Step::Read => {
                let (n, eof) = (r["n"].as_u64().unwrap_or(0), r["eof"].as_bool().unwrap_or(true));
                let Some(part) = t.part() else { return };
                let off = t.off;
                let req = json!({"op": "wchunk", "path": part, "off": off, "b64": r["b64"].as_str().unwrap_or("")});
                self.xfer_request(id, false, req, Step::Write { n, eof });
            }
            Step::Write { n, eof } => {
                t.off += n;
                t.done += n;
                let item = t.items[t.file].clone();
                if eof {
                    let (part, path) = (t.part().unwrap_or_default(), t.dst_of(&item));
                    self.xfer_request(id, false, json!({"op": "finish", "part": part, "path": path}), Step::Finish);
                } else {
                    let (src, off) = (t.src_of(&item), t.off);
                    self.xfer_request(id, true, json!({"op": "rchunk", "path": src, "off": off, "len": CHUNK}), Step::Read);
                }
            }
            Step::Finish => {
                t.file += 1;
                self.next_file(id);
            }
            Step::Abort => {}
        }
    }

    /// Transfers still copying.
    pub fn transfers_running(&self) -> bool {
        self.transfers.iter().any(|t| t.state == State::Running)
    }
}

// ---- on screen ------------------------------------------------------------

use crate::app::{PanelId, Region, SideTab};
use crate::theme;
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Id, Rect, RichText, Stroke};

impl App {
    /// Where a file on `host` can be sent from its menu: the local Downloads
    /// folder, what the other side panels show, and the folders terminals on
    /// other hosts are in. (host, folder, why) without repeats.
    pub fn send_targets(&self, host: &str) -> Vec<(String, String, String)> {
        let mut out: Vec<(String, String, String)> = Vec::new();
        let mut add = |h: &str, d: &str, why: String| {
            if !out.iter().any(|o| o.0 == h && o.1 == d) {
                out.push((h.to_string(), d.to_string(), why));
            }
        };
        if host != "local" {
            add("local", &downloads_dir(), "your Downloads folder".into());
        }
        for (n, v) in &self.side_views {
            if let Some((h, d)) = &v.root {
                if h != host || self.side_views.len() > 1 {
                    add(h, d, format!("what side panel {} shows", n + 1));
                }
            }
        }
        for t in &self.tabs {
            let mut ids = Vec::new();
            t.root.leaves(&mut ids);
            for id in ids {
                if let Some(p) = self.panes.get(&id).filter(|p| p.host != host) {
                    if let Some(cwd) = p.term.cwd.clone().or(p.initial_cwd.clone()) {
                        add(&p.host, &cwd, format!("where the terminal \"{}\" is", p.title()));
                    }
                }
            }
        }
        out
    }

    /// What is under `pos` to copy into: a folder row, a side panel's Files
    /// tab (its folder), or a terminal (its directory). (host, folder).
    fn drop_target(&self, pos: egui::Pos2) -> Option<(String, String)> {
        if let Some((_, h, d)) = self.drop_dirs.iter().find(|(r, _, _)| r.contains(pos)) {
            return Some((h.clone(), d.clone()));
        }
        for (p, r) in &self.panel_rects {
            if p.region == Region::Side && r.contains(pos) && self.settings.side_tabs.get(&p.n).copied().unwrap_or(SideTab::Files) == SideTab::Files {
                if let Some((h, d)) = self.side_views.get(&p.n).and_then(|v| v.root.clone()) {
                    return Some((h, d));
                }
            }
        }
        let over_terms = self.panel_rects.get(&PanelId::TERMS).is_some_and(|r| r.contains(pos));
        if over_terms {
            for (id, r) in &self.pane_rects {
                if r.contains(pos) {
                    let p = self.panes.get(id)?;
                    return p.term.cwd.clone().or(p.initial_cwd.clone()).map(|c| (p.host.clone(), c));
                }
            }
        }
        None
    }

    /// The folder row under the pointer, if it is over one.
    fn drop_row(&self, pos: egui::Pos2) -> Option<Rect> {
        self.drop_dirs.iter().map(|(r, _, _)| *r).find(|r| r.contains(pos))
    }

    /// Files dragged: a row from a Files panel, or files from the system's
    /// file manager. Shows where they would go and starts the copies on drop.
    pub fn ui_file_drop(&mut self, ctx: &egui::Context) {
        let (hovered, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.clone()));
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("file-drop")));
        // From outside weft the system reports no position: ask for the cursor.
        if hovered || !dropped.is_empty() {
            ctx.request_repaint();
            let pos = cursor_in_window(ctx);
            let target = pos.and_then(|p| self.drop_target(p));
            if hovered {
                let text = match &target {
                    Some((h, d)) => format!("copy here: {h}:{d}"),
                    None => "drop on a Files panel, a folder or a terminal".to_string(),
                };
                if let Some(p) = pos {
                    drop_hint(&painter, p, self.drop_row(p), &text, target.is_some());
                }
            }
            if let Some((h, d)) = target.filter(|_| !dropped.is_empty()) {
                for f in &dropped {
                    if let Some(path) = &f.path {
                        self.start_transfer("local", &path.display().to_string().replace('\\', "/"), &h, &d);
                    }
                }
            } else if !dropped.is_empty() {
                self.toast("drop files on a Files panel, a folder or a terminal to copy them there", ToastKind::Info);
            }
            return;
        }
        let Some((src_host, src_path)) = self.file_drag.clone() else { return };
        let (pos, released) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.any_released() || !i.pointer.primary_down()));
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let parent = src_path.trim_end_matches('/').rsplit_once('/').map(|(p, _)| if p.is_empty() { "/".to_string() } else { p.to_string() }).unwrap_or_default();
        // Not into itself, nor where it already is.
        let target = pos.and_then(|p| self.drop_target(p)).filter(|(h, d)| !(h == &src_host && (d == &parent || d == &src_path || d.starts_with(&format!("{src_path}/")))));
        if let Some(p) = pos {
            let name = base_name(&src_path);
            let text = match &target {
                Some((h, d)) => format!("copy {name} to {h}:{d}"),
                None => format!("{name}: drop on another Files panel, a folder or a terminal"),
            };
            drop_hint(&painter, p, self.drop_row(p), &text, target.is_some());
        }
        if released {
            self.file_drag = None;
            if let Some((h, d)) = target {
                self.start_transfer(&src_host, &src_path, &h, &d);
            }
        }
    }

    /// The transfers: progress, speed, cancel; finished ones go after a while.
    pub fn ui_transfers(&mut self, ctx: &egui::Context) {
        let linger = std::time::Duration::from_secs(8);
        self.transfers.retain(|t| !(t.state == State::Done && t.ended.is_some_and(|e| e.elapsed() > linger)));
        if self.transfers.is_empty() {
            return;
        }
        if self.transfers_running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if let Some(left) = self.transfers.iter().filter(|t| t.state == State::Done).filter_map(|t| t.ended).map(|e| linger.saturating_sub(e.elapsed())).min() {
            // Only to take a finished one away; failed and cancelled ones stay
            // until cleared, with nothing to redraw meanwhile.
            ctx.request_repaint_after(left + std::time::Duration::from_millis(50));
        }
        let mut cancel = None;
        let mut clear = false;
        let screen = ctx.screen_rect();
        egui::Area::new(Id::new("transfers"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, vec2(-14.0, -36.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(theme::bg_raised()).corner_radius(CornerRadius::same(8)).inner_margin(10.0).show(ui, |ui| {
                    ui.set_width((screen.width() * 0.4).clamp(280.0, 420.0));
                    ui.horizontal(|ui| {
                        let running = self.transfers.iter().filter(|t| t.state == State::Running).count();
                        ui.label(RichText::new(if running > 0 { format!("Copying {running}") } else { "Copies".to_string() }).strong().size(12.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.transfers.iter().any(|t| t.state != State::Running) && ui.small_button("Clear").on_hover_text("remove the finished, failed and cancelled ones").clicked() {
                                clear = true;
                            }
                        });
                    });
                    egui::ScrollArea::vertical().max_height(screen.height() * 0.5).show(ui, |ui| {
                        for t in &self.transfers {
                            ui.add_space(4.0);
                            let (frac, color) = match &t.state {
                                State::Done => (1.0, theme::success()),
                                State::Failed(_) => (if t.total > 0 { t.done as f32 / t.total as f32 } else { 0.0 }, theme::error()),
                                State::Cancelled => (if t.total > 0 { t.done as f32 / t.total as f32 } else { 0.0 }, theme::text_faint()),
                                State::Running => (if t.total > 0 { t.done as f32 / t.total as f32 } else { 0.0 }, theme::accent()),
                            };
                            ui.horizontal(|ui| {
                                let w = ui.available_width() - 26.0;
                                let label = if t.is_dir { format!("{}/", t.name) } else { t.name.clone() };
                                ui.add_sized([w, 18.0], egui::Label::new(RichText::new(label).size(12.5)).truncate());
                                if t.state == State::Running && crate::icons::button(ui, crate::icons::Icon::Cross, "cancel (the part copied so far is removed)").clicked() {
                                    cancel = Some(t.id);
                                }
                            });
                            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
                            ui.painter().rect_filled(bar, CornerRadius::same(2), theme::tint(theme::text_faint(), 40));
                            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac.clamp(0.0, 1.0), bar.height())), CornerRadius::same(2), color);
                            let route = format!("{}:{} \u{2192} {}:{}", t.src_host, t.src_path, t.dst_host, t.dest());
                            let detail = match &t.state {
                                State::Running => {
                                    let files = if t.is_dir { format!(" · {}/{} files", t.files_done(), t.files()) } else { String::new() };
                                    format!("{} / {} · {}/s{files}", crate::widgets::human_size(t.done), crate::widgets::human_size(t.total), crate::widgets::human_size(t.speed() as u64))
                                }
                                State::Done => format!("done · {} at {}/s", crate::widgets::human_size(t.done), crate::widgets::human_size(t.speed() as u64)),
                                State::Failed(e) => format!("failed: {e}"),
                                State::Cancelled => "cancelled".to_string(),
                            };
                            let detail_color = if matches!(t.state, State::Failed(_)) { theme::error() } else { theme::text_faint() };
                            ui.add(egui::Label::new(RichText::new(detail).size(11.0).color(detail_color)).truncate()).on_hover_text(&route);
                        }
                    });
                });
            });
        if let Some(id) = cancel {
            self.cancel_transfer(id);
        }
        if clear {
            self.transfers.retain(|t| t.state == State::Running);
        }
    }
}

/// A label by the pointer saying where a drop would go.
/// Where the hint beside a dragged file goes: right of and below the
/// pointer, clear of the row it is over (`avoid`: the folder it would drop
/// into was hidden behind the hint) and inside the window.
fn hint_rect(p: egui::Pos2, size: egui::Vec2, avoid: Option<Rect>, screen: Rect) -> Rect {
    // Inside the window first, then off the row: below it, or above it
    // when there is no room below.
    let inside = |r: Rect| {
        let dx = (screen.right() - 4.0 - r.right()).min(0.0).max(screen.left() + 4.0 - r.left());
        let dy = (screen.bottom() - 4.0 - r.bottom()).min(0.0);
        r.translate(vec2(dx, dy))
    };
    let r = inside(Rect::from_min_size(p + vec2(16.0, 14.0), size));
    match avoid.filter(|a| a.intersects(r)) {
        Some(a) => {
            let below = a.bottom() + 4.0;
            let top = if below + size.y <= screen.bottom() - 4.0 { below } else { a.top() - 4.0 - size.y };
            Rect::from_min_size(egui::pos2(r.left(), top), size)
        }
        None => r,
    }
}

fn drop_hint(painter: &egui::Painter, p: egui::Pos2, avoid: Option<Rect>, text: &str, ok: bool) {
    let g = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.5), if ok { theme::text() } else { theme::text_dim() });
    let r = hint_rect(p, g.size() + vec2(16.0, 10.0), avoid, painter.ctx().screen_rect());
    painter.rect(r, CornerRadius::same(6), theme::bg_raised(), Stroke::new(1.0_f32, if ok { theme::accent() } else { theme::border() }), egui::StrokeKind::Inside);
    painter.galley(r.min + vec2(8.0, 5.0), g, Color32::WHITE);
}

/// Where the mouse is over weft's window, in points: files dragged in from
/// another program come with no position (winit 0.30), so ask the system.
fn cursor_in_window(ctx: &egui::Context) -> Option<egui::Pos2> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GetCursorPos, WindowFromPoint, GA_ROOT};
        unsafe {
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt) == 0 {
                return None;
            }
            let hwnd = GetAncestor(WindowFromPoint(pt), GA_ROOT);
            if hwnd.is_null() {
                return None;
            }
            ScreenToClient(hwnd, &mut pt);
            let ppp = ctx.pixels_per_point();
            let p = pos2(pt.x as f32 / ppp, pt.y as f32 / ppp);
            return ctx.screen_rect().contains(p).then_some(p);
        }
    }
    #[allow(unreachable_code)]
    ctx.input(|i| i.pointer.latest_pos())
}

#[cfg(test)]
mod tests {
    /// The hint beside a dragged file leaves the folder row under the
    /// pointer readable, and stays in the window.
    #[test]
    fn the_drop_hint_leaves_its_row_readable() {
        use super::hint_rect;
        use egui::{pos2, vec2, Rect};
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let size = vec2(260.0, 26.0);
        // Over a row: below it, not on it.
        let row = Rect::from_min_size(pos2(40.0, 100.0), vec2(300.0, 20.0));
        let r = hint_rect(pos2(60.0, 104.0), size, Some(row), screen);
        assert!(!r.intersects(row) && r.top() >= row.bottom(), "{r:?}");
        // The last row of the window: above it.
        let row = Rect::from_min_size(pos2(40.0, 578.0), vec2(300.0, 20.0));
        let r = hint_rect(pos2(60.0, 590.0), size, Some(row), screen);
        assert!(!r.intersects(row) && r.bottom() <= row.top() && screen.contains_rect(r), "{r:?}");
        // At the right edge: pulled back inside.
        let r = hint_rect(pos2(780.0, 300.0), size, None, screen);
        assert!(screen.contains_rect(r), "{r:?}");
        // Over no row: beside the pointer, as before.
        assert_eq!(hint_rect(pos2(100.0, 100.0), size, None, screen).min, pos2(116.0, 114.0));
    }

    use super::*;

    /// Real copies through real agents: local -> r1 -> r2 -> local, two
    /// "remote" hosts being agent.py run by python here.
    #[test]
    fn copies_between_any_two_hosts() {
        let Ok(out) = std::process::Command::new("python").arg("--version").output() else { return };
        if !out.status.success() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("weft-xfer-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["data", "src/folder/sub", "r1", "r2", "back"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        // Over two chunks, so reads continue at an offset.
        let big: Vec<u8> = (0..(CHUNK as u32 * 2 + 1234)).map(|i| (i % 253) as u8).collect();
        std::fs::write(dir.join("src/folder/big.bin"), &big).unwrap();
        std::fs::write(dir.join("src/folder/sub/메모.txt"), "안녕").unwrap();
        std::fs::write(dir.join("src/folder/empty"), "").unwrap();
        let agent = format!("{}/remote/agent.py", env!("CARGO_MANIFEST_DIR"));
        let prof = |name: &str| json!({"name": name, "kind": "custom", "pty": ["python", "-c", "import time; time.sleep(60)"], "agent": ["python", "-u", agent]});
        let settings = json!({"profiles": [prof("r1"), prof("r2")], "sidebar": false});
        std::fs::write(dir.join("data/settings.json"), settings.to_string()).unwrap();
        // Only tests building an App read WEFT_DATA_DIR; they run one at a time.
        let _guard = crate::app::TEST_APP.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("WEFT_DATA_DIR", dir.join("data"));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, crate::theme::DEFAULT_FONT, crate::theme::DEFAULT_ICON_FONT);
        let mut app = App::new(ctx.clone());
        let p = |s: &str| dir.join(s).display().to_string().replace('\\', "/");
        let run = |app: &mut App, src_host: &str, src: &str, dst_host: &str, dst: &str| {
            app.start_transfer(src_host, src, dst_host, dst);
            let end = Instant::now() + std::time::Duration::from_secs(60);
            while app.transfers_running() && Instant::now() < end {
                app.poll();
                // The list of copies is drawn meanwhile, as in the window.
                let _ = ctx.run(Default::default(), |ctx| app.ui_transfers(ctx));
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let t = app.transfers.last().unwrap();
            assert_eq!(t.state, State::Done, "{src_host}:{src} -> {dst_host}:{dst}");
        };
        run(&mut app, "local", &p("src/folder"), "r1", &p("r1"));
        run(&mut app, "r1", &p("r1/folder"), "r2", &p("r2"));
        run(&mut app, "r2", &p("r2/folder"), "local", &p("back"));
        assert_eq!(std::fs::read(dir.join("back/folder/big.bin")).unwrap(), big);
        assert_eq!(std::fs::read_to_string(dir.join("back/folder/sub/메모.txt")).unwrap(), "안녕");
        assert_eq!(std::fs::read(dir.join("back/folder/empty")).unwrap(), b"");
        // Sent again: a second copy beside it, nothing overwritten.
        run(&mut app, "local", &p("src/folder/sub/메모.txt"), "local", &p("back/folder/sub"));
        assert!(dir.join("back/folder/sub/메모 (1).txt").is_file());
        assert!(!std::fs::read_dir(dir.join("back/folder")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".weft-part")));
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_at_the_destination() {
        assert_eq!(candidate("a.tar.gz", false, 0), "a.tar.gz");
        assert_eq!(candidate("a.tar.gz", false, 2), "a.tar (2).gz");
        assert_eq!(candidate("notes", false, 1), "notes (1)");
        assert_eq!(candidate(".bashrc", false, 1), ".bashrc (1)");
        assert_eq!(candidate("src.d", true, 1), "src.d (1)");
        assert_eq!(join("/home/u", "x"), "/home/u/x");
        assert_eq!(join("/", "x"), "/x");
        assert_eq!(join("C:/Users/me/", "x"), "C:/Users/me/x");
        assert_eq!(base_name("/home/u/dir/"), "dir");
        assert_eq!(base_name("C:/a/b.txt"), "b.txt");
    }
}
