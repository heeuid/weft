//! Files dragged out of weft into another app (a browser's upload, a file
//! manager, a mail): the system's own drag, started as the pointer leaves
//! the window with rows of Files. Files of a remote host are fetched to
//! this PC first (a folder of weft's under its data) and dragged from
//! there. The window library weft uses has no drag out, so each system's
//! own way: Windows' OLE drag and macOS' dragging session (the drag crate),
//! X11's XDND on Linux and the BSDs (crate::xdnd).

use std::path::PathBuf;

/// Start the system's drag of these files (absolute paths on this PC).
/// Windows: returns once dropped or given up; elsewhere at once.
#[cfg(any(windows, target_os = "macos"))]
pub fn start(frame: &eframe::Frame, paths: Vec<PathBuf>) -> Result<(), String> {
    let icon = drag::Image::Raw(include_bytes!("../assets/weft.png").to_vec());
    drag::start_drag(frame, drag::DragItem::Files(paths), icon, |_, _| {}, drag::Options::default()).map_err(|e| e.to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn start(frame: &eframe::Frame, paths: Vec<PathBuf>) -> Result<(), String> {
    crate::xdnd::start(frame, paths)
}

/// Whether the system's drag can be started from this window.
#[cfg(any(windows, target_os = "macos"))]
pub fn possible(_frame: &eframe::Frame) -> bool {
    true
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn possible(frame: &eframe::Frame) -> bool {
    crate::xdnd::possible(frame)
}

/// Of these paths, those not inside another of them (a folder taken
/// with what is in it once).
pub fn outermost(paths: &[String]) -> Vec<String> {
    let inside = |p: &str, q: &str| p != q && p.starts_with(&format!("{}/", q.trim_end_matches('/')));
    paths.iter().filter(|p| !paths.iter().any(|q| inside(p, q))).cloned().collect()
}

/// Remote files being fetched to be dragged out.
pub struct Fetch {
    pub host: String,
    pub paths: Vec<String>,
    /// Where they come to on this PC.
    pub dir: String,
    /// Their transfers, and where each came to once it has.
    pub ids: Vec<(u64, Option<PathBuf>)>,
    /// Let go before they came: they wait to be dragged again.
    pub let_go: bool,
}

/// Fetched copies are dragged again at once for this long.
const READY: std::time::Duration = std::time::Duration::from_secs(600);

/// A drop back on weft counts as the drag's own for this long.
pub const BACK: std::time::Duration = std::time::Duration::from_secs(60);

pub type Stamps = Vec<Option<(std::time::SystemTime, u64)>>;

/// Files of a host as handed to the system's drag (copies fetched to
/// this PC, or the files themselves on it).
pub struct Copies {
    pub host: String,
    /// Their paths there.
    pub paths: Vec<String>,
    /// Here.
    pub here: Vec<PathBuf>,
    pub at: std::time::Instant,
    /// As they were then.
    pub stamps: Stamps,
}

impl Copies {
    pub fn new(host: String, paths: Vec<String>, here: Vec<PathBuf>) -> Copies {
        let stamps = stamps(&here);
        Copies { host, paths, here, at: std::time::Instant::now(), stamps }
    }

    /// Younger than `max` and none changed since.
    pub fn fresh(&self, max: std::time::Duration) -> bool {
        self.at.elapsed() < max && stamps(&self.here) == self.stamps
    }
}

/// When a file was last changed and its size: a copy fetched here and
/// changed since (opened in an editor and saved) is no longer the
/// original.
pub fn stamps(paths: &[PathBuf]) -> Stamps {
    paths.iter().map(|p| std::fs::metadata(p).ok().and_then(|m| Some((m.modified().ok()?, m.len())))).collect()
}

impl crate::app::App {
    /// Rows of Files dragged out of the window: on this PC the system's
    /// drag takes them; a remote host's are fetched here first.
    pub fn drag_out_of(&mut self, host: &str, paths: &[String]) {
        use crate::app::ToastKind;
        self.drag_src = Some((host.to_string(), paths.to_vec()));
        if self.profile(host).is_local() {
            self.drag_out = Some(paths.iter().map(PathBuf::from).collect());
            return;
        }
        if let Some(c) = &self.drag_ready {
            if c.host == host && c.paths == paths && c.fresh(READY) {
                self.drag_out = Some(c.here.clone());
                return;
            }
        }
        if self.drag_fetch.is_some() {
            self.toast("still fetching the ones dragged before", ToastKind::Info);
            return;
        }
        // A folder of its own each time (the names as they are there);
        // those of days ago go (an app may read a dropped file later).
        let base = self.data.root.join("drag-out");
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > 86_400);
                if old {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let dir = base.join(stamp.to_string());
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast(format!("can't make {}: {e}", dir.display()), ToastKind::Error);
            return;
        }
        let dir = dir.display().to_string().replace('\\', "/");
        let first = self.next_transfer + 1;
        for p in paths {
            self.start_transfer(host, p, "local", &dir);
        }
        let ids = (first..=self.next_transfer).map(|id| (id, None)).collect();
        self.drag_fetch = Some(Fetch { host: host.into(), paths: paths.to_vec(), dir, ids, let_go: false });
        self.toast(format!("fetching {} from {host} to drag out: it goes on once here (still held), else drag again", if paths.len() == 1 { crate::transfer::base_name(&paths[0]) } else { format!("{} items", paths.len()) }), ToastKind::Info);
    }

    /// The fetch for a drag out: once all are here, the drag goes on if
    /// the button is still held, else they wait to be dragged again.
    pub fn poll_drag_fetch(&mut self, ctx: &egui::Context) {
        use crate::app::ToastKind;
        use crate::transfer::State;
        let Some(mut f) = self.drag_fetch.take() else { return };
        // Let go meanwhile: not started later by another press.
        f.let_go |= ctx.input(|i| i.pointer.any_released() || !i.pointer.primary_down());
        let mut failed = None;
        for (id, here) in f.ids.iter_mut().filter(|(_, h)| h.is_none()) {
            match self.transfers.iter().find(|t| t.id == *id) {
                Some(t) if t.state == State::Running => {}
                Some(t) if t.state == State::Done => *here = Some(PathBuf::from(t.dest())),
                Some(t) => failed = Some(format!("{}: {}", t.name, if let State::Failed(e) = &t.state { e.as_str() } else { "cancelled" })),
                None => failed = Some("a transfer went".into()),
            }
        }
        if let Some(why) = failed {
            // One failed: the others are not needed.
            for (id, here) in &f.ids {
                if here.is_none() {
                    self.cancel_transfer(*id);
                }
            }
            self.toast(format!("drag out: not fetched: {why}"), ToastKind::Error);
            return;
        }
        let here: Vec<PathBuf> = f.ids.iter().filter_map(|(_, h)| h.clone()).collect();
        if here.len() < f.ids.len() {
            self.drag_fetch = Some(f);
            return;
        }
        if !f.let_go {
            // Still held: goes on with the pointer's next move (the system's
            // drag starts from a move of the mouse).
            if !ctx.input(|i| i.pointer.is_moving()) {
                self.drag_fetch = Some(f);
                return;
            }
            self.drag_out = Some(here.clone());
            self.drag_src = Some((f.host.clone(), f.paths.clone()));
        } else {
            self.toast(format!("{} here: drag them out again ({})", if here.len() == 1 { "it is" } else { "they are" }, f.dir), ToastKind::Info);
        }
        self.drag_ready = Some(Copies::new(f.host, f.paths, here));
    }
}

#[cfg(test)]
mod tests {
    /// A folder picked with what is in it: the folder once.
    #[test]
    fn a_folder_once_with_what_is_in_it() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(super::outermost(&v(&["/d", "/d/x", "/d/e/y", "/de", "/z"])), v(&["/d", "/de", "/z"]));
        assert_eq!(super::outermost(&v(&["/", "/a"])), v(&["/"]));
        assert_eq!(super::outermost(&v(&["C:/a/b", "C:/a"])), v(&["C:/a"]));
    }
}
