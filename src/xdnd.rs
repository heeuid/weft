//! Files dragged out of weft on X11 (Linux, the BSDs): the XDND protocol
//! (version 5) as a drag source, on a connection and a hidden window of
//! its own, on a thread of its own. The pointer is followed by asking the
//! server (the button is held by weft's window, not grabbed here); the
//! window under it that says it takes drops (XdndAware) is told the drag
//! entered, moved and left; at the release it is told to drop, asks for
//! the files as text/uri-list, and says it has finished. Under Wayland
//! itself (no X window) there is none of this: said.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, Atom, AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, PropMode, Window};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

/// Whether this window is an X11 one (not Wayland's).
pub fn possible(frame: &eframe::Frame) -> bool {
    matches!(frame.window_handle().map(|h| h.as_raw()), Ok(RawWindowHandle::Xlib(_)) | Ok(RawWindowHandle::Xcb(_)))
}

pub fn start(frame: &eframe::Frame, paths: Vec<PathBuf>) -> Result<(), String> {
    match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(_)) | Ok(RawWindowHandle::Xcb(_)) => {}
        Ok(RawWindowHandle::Wayland(_)) => return Err("dragging files out of weft is not done under Wayland yet (it is under X11)".into()),
        _ => return Err("no window to drag from".into()),
    }
    // One at a time: the drag before may still be giving its files.
    if BUSY.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err("the drag before is still being dropped: a moment".into());
    }
    std::thread::Builder::new()
        .name("weft-xdnd".into())
        .spawn(move || {
            if let Err(e) = run(None, &paths) {
                eprintln!("weft: drag out: {e}");
            }
            BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
        })
        .map_err(|e| {
            BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
            e.to_string()
        })?;
    Ok(())
}

static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A file's path as a URI (file:///…, every byte but the plain ones
/// percent-encoded).
pub fn file_uri(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    let mut out = String::from("file://");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The files as text/uri-list: a URI a line, CRLF.
pub fn uri_list(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| format!("{}\r\n", file_uri(p))).collect()
}

struct Atoms {
    aware: Atom,
    proxy: Atom,
    enter: Atom,
    position: Atom,
    status: Atom,
    leave: Atom,
    drop: Atom,
    finished: Atom,
    selection: Atom,
    type_list: Atom,
    copy: Atom,
    uri_list: Atom,
}

fn atom(c: &impl Connection, name: &str) -> Result<Atom, String> {
    Ok(c.intern_atom(false, name.as_bytes()).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.atom)
}

/// The drag, on the X server of `display` (None: $DISPLAY's), until the
/// button is let go (or two minutes).
fn run(display: Option<&str>, paths: &[PathBuf]) -> Result<(), String> {
    let (c, screen) = x11rb::connect(display).map_err(|e| e.to_string())?;
    let root = c.setup().roots[screen].root;
    let a = Atoms {
        aware: atom(&c, "XdndAware")?,
        proxy: atom(&c, "XdndProxy")?,
        enter: atom(&c, "XdndEnter")?,
        position: atom(&c, "XdndPosition")?,
        status: atom(&c, "XdndStatus")?,
        leave: atom(&c, "XdndLeave")?,
        drop: atom(&c, "XdndDrop")?,
        finished: atom(&c, "XdndFinished")?,
        selection: atom(&c, "XdndSelection")?,
        type_list: atom(&c, "XdndTypeList")?,
        copy: atom(&c, "XdndActionCopy")?,
        uri_list: atom(&c, "text/uri-list")?,
    };
    // The source: a window never shown, owning the drag's selection.
    let me = c.generate_id().map_err(|e| e.to_string())?;
    c.create_window(0, me, root, -10, -10, 1, 1, 0, xproto::WindowClass::INPUT_ONLY, 0, &xproto::CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE)).map_err(|e| e.to_string())?;
    c.change_property32(PropMode::REPLACE, me, a.type_list, AtomEnum::ATOM, &[a.uri_list]).map_err(|e| e.to_string())?;
    c.set_selection_owner(me, a.selection, x11rb::CURRENT_TIME).map_err(|e| e.to_string())?;
    c.flush().map_err(|e| e.to_string())?;
    let data = uri_list(paths);
    // To the target, or to the window it names as its proxy (the message
    // still naming the target).
    let send = |to: Target, kind: Atom, d: [u32; 5]| -> Result<(), String> {
        let ev = ClientMessageEvent::new(32, to.window, kind, d);
        c.send_event(false, to.proxy.unwrap_or(to.window), EventMask::NO_EVENT, ev).map_err(|e| e.to_string())?;
        c.flush().map_err(|e| e.to_string())
    };
    let mut target: Option<Target> = None;
    let mut accepted = false;
    // A position told and not yet answered (the next waits for it, as
    // the protocol asks; a second at most), and the last one told.
    let mut waiting: Option<Instant> = None;
    let mut told = (0, 0);
    let end = Instant::now() + Duration::from_secs(120);
    loop {
        if Instant::now() > end {
            break;
        }
        // Answers: a target's status, a request for the files.
        while let Some(ev) = c.poll_for_event().map_err(|e| e.to_string())? {
            match ev {
                // (Only the one under the pointer now: a late answer of the
                // one before is not this one's.)
                Event::ClientMessage(m) if m.type_ == a.status && target.is_some_and(|t| t.window == m.data.as_data32()[0]) => {
                    accepted = m.data.as_data32()[1] & 1 == 1;
                    waiting = None;
                }
                Event::SelectionRequest(r) => answer(&c, &r, &a, &data)?,
                _ => {}
            }
        }
        let q = c.query_pointer(root).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let held = u16::from(q.mask) & u16::from(xproto::KeyButMask::BUTTON1) != 0;
        let under = aware_under(&c, root, &a)?;
        if under != target {
            if let Some(t) = target {
                send(t, a.leave, [me, 0, 0, 0, 0])?;
            }
            accepted = false;
            waiting = None;
            told = (-1, -1);
            if let Some(t) = under {
                // The version both speak; one type, in the message.
                send(t, a.enter, [me, t.version.min(5) << 24, a.uri_list, 0, 0])?;
            }
            target = under;
        }
        if !held {
            // Let go before the target answered: its answer, a second at most.
            let until = Instant::now() + Duration::from_secs(1);
            while target.is_some() && waiting.is_some() && Instant::now() < until {
                match c.poll_for_event().map_err(|e| e.to_string())? {
                    Some(Event::ClientMessage(m)) if m.type_ == a.status && target.is_some_and(|t| t.window == m.data.as_data32()[0]) => {
                        accepted = m.data.as_data32()[1] & 1 == 1;
                        waiting = None;
                    }
                    Some(Event::SelectionRequest(r)) => answer(&c, &r, &a, &data)?,
                    Some(_) => {}
                    None => std::thread::sleep(Duration::from_millis(5)),
                }
            }
            // Let go: dropped where it is taken, else nothing.
            match target {
                Some(t) if accepted => {
                    send(t, a.drop, [me, 0, x11rb::CURRENT_TIME, 0, 0])?;
                    finish(&c, me, &a, &data)?;
                }
                Some(t) => send(t, a.leave, [me, 0, 0, 0, 0])?,
                None => {}
            }
            break;
        }
        let free = waiting.is_none_or(|w| w.elapsed() > Duration::from_secs(1));
        if let Some(t) = target.filter(|_| free && told != (q.root_x, q.root_y)) {
            let pos = ((q.root_x as u16 as u32) << 16) | (q.root_y as u16 as u32);
            send(t, a.position, [me, 0, pos, x11rb::CURRENT_TIME, a.copy])?;
            waiting = Some(Instant::now());
            told = (q.root_x, q.root_y);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = c.destroy_window(me);
    let _ = c.flush();
    Ok(())
}

/// After the drop: the files given when asked, until the target says it
/// has finished (or five seconds).
fn finish(c: &impl Connection, _me: Window, a: &Atoms, data: &str) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        while let Some(ev) = c.poll_for_event().map_err(|e| e.to_string())? {
            match ev {
                Event::SelectionRequest(r) => answer(c, &r, a, data)?,
                Event::ClientMessage(m) if m.type_ == a.finished => return Ok(()),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// The files, for a target that asks for them (text/uri-list; anything
/// else: refused).
fn answer(c: &impl Connection, r: &xproto::SelectionRequestEvent, a: &Atoms, data: &str) -> Result<(), String> {
    let property = if r.target == a.uri_list {
        let prop = if r.property == x11rb::NONE { r.target } else { r.property };
        c.change_property8(PropMode::REPLACE, r.requestor, prop, a.uri_list, data.as_bytes()).map_err(|e| e.to_string())?;
        prop
    } else {
        x11rb::NONE
    };
    let ev = xproto::SelectionNotifyEvent { response_type: xproto::SELECTION_NOTIFY_EVENT, sequence: 0, time: r.time, requestor: r.requestor, selection: r.selection, target: r.target, property };
    c.send_event(false, r.requestor, EventMask::NO_EVENT, ev).map_err(|e| e.to_string())?;
    c.flush().map_err(|e| e.to_string())
}

/// A window taking drops: the version of the protocol it speaks, and the
/// window its messages go to when it names a proxy.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Target {
    window: Window,
    version: u32,
    proxy: Option<Window>,
}

/// The window under the pointer that takes drops: the first one down from
/// the root, through the window manager's frames, that says XdndAware
/// (itself or through its proxy).
fn aware_under(c: &impl Connection, root: Window, a: &Atoms) -> Result<Option<Target>, String> {
    let e = |e: x11rb::errors::ReplyError| e.to_string();
    let mut w = root;
    for _ in 0..16 {
        let q = c.query_pointer(w).map_err(|e| e.to_string())?.reply().map_err(e)?;
        if q.child == x11rb::NONE {
            return Ok(None);
        }
        w = q.child;
        let proxy = c.get_property(false, w, a.proxy, AtomEnum::WINDOW, 0, 1).map_err(|e| e.to_string())?.reply().map_err(e)?.value32().and_then(|mut v| v.next());
        let on = proxy.unwrap_or(w);
        let p = c.get_property(false, on, a.aware, AtomEnum::ATOM, 0, 1).map_err(|e| e.to_string())?.reply().map_err(e)?;
        if let Some(version) = p.value32().and_then(|mut v| v.next()) {
            return Ok(Some(Target { window: w, version, proxy }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files as text/uri-list: file URIs, the bytes that are not plain
    /// percent-encoded, CRLF after each.
    #[test]
    fn files_as_a_uri_list() {
        let l = uri_list(&[PathBuf::from("/home/u/a b.txt"), PathBuf::from("/tmp/한.md")]);
        assert_eq!(l, "file:///home/u/a%20b.txt\r\nfile:///tmp/%ED%95%9C.md\r\n");
    }

    /// A drag out on an X server of its own (Xvfb), to a window that takes
    /// drops as a file manager or a browser does: entered, moved over and
    /// answered, dropped at the release, the files asked for as
    /// text/uri-list and given, finished. Without Xvfb: said, not run.
    #[test]
    fn files_dropped_on_an_x11_app() {
        use x11rb::protocol::xtest::ConnectionExt as _;
        let n = 90 + std::process::id() % 9;
        let display = format!(":{n}");
        let Ok(mut xvfb) = std::process::Command::new("Xvfb").args([display.as_str(), "-screen", "0", "800x600x24", "-nolisten", "tcp"]).stderr(std::process::Stdio::null()).spawn() else {
            eprintln!("no Xvfb: the drag out on X11 not tried");
            return;
        };
        struct Kill(std::process::Child);
        impl Drop for Kill {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let started = Instant::now();
        let c = loop {
            if let Ok((c, _)) = x11rb::connect(Some(&display)) {
                break c;
            }
            assert!(started.elapsed() < Duration::from_secs(10), "Xvfb did not start");
            if let Ok(Some(_)) = xvfb.try_wait() {
                panic!("Xvfb ended");
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let _xvfb = Kill(xvfb);
        let root = c.setup().roots[0].root;
        let a = |s: &str| atom(&c, s).unwrap();
        let (aware, enter, position, status, drop, finished, selection, uri) = (a("XdndAware"), a("XdndEnter"), a("XdndPosition"), a("XdndStatus"), a("XdndDrop"), a("XdndFinished"), a("XdndSelection"), a("text/uri-list"));
        // The app taking drops: a window at the top left.
        let w = c.generate_id().unwrap();
        c.create_window(0, w, root, 0, 0, 400, 400, 0, xproto::WindowClass::INPUT_OUTPUT, 0, &xproto::CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE)).unwrap();
        c.change_property32(PropMode::REPLACE, w, aware, AtomEnum::ATOM, &[5]).unwrap();
        c.map_window(w).unwrap();
        // The button held at the right, outside it, as when a row leaves weft.
        c.xtest_fake_input(xproto::MOTION_NOTIFY_EVENT, 0, x11rb::CURRENT_TIME, root, 600, 300, 0).unwrap();
        c.xtest_fake_input(xproto::BUTTON_PRESS_EVENT, 1, x11rb::CURRENT_TIME, root, 0, 0, 0).unwrap();
        c.sync().unwrap();
        let files = vec![PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/c")];
        let want = uri_list(&files);
        let d = display.clone();
        let source = std::thread::spawn(move || run(Some(&d), &files));
        std::thread::sleep(Duration::from_millis(200));
        // Over the app.
        c.xtest_fake_input(xproto::MOTION_NOTIFY_EVENT, 0, x11rb::CURRENT_TIME, root, 200, 200, 0).unwrap();
        c.flush().unwrap();
        let (mut entered, mut moved, mut got) = (false, 0, None);
        let end = Instant::now() + Duration::from_secs(20);
        while got.is_none() && Instant::now() < end {
            let Some(ev) = c.poll_for_event().unwrap() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            match ev {
                Event::ClientMessage(m) if m.type_ == enter => {
                    let d = m.data.as_data32();
                    assert_eq!(d[1] >> 24, 5, "version 5");
                    assert_eq!(d[2], uri, "the files as text/uri-list");
                    entered = true;
                }
                Event::ClientMessage(m) if m.type_ == position => {
                    let src = m.data.as_data32()[0];
                    let at = m.data.as_data32()[2];
                    assert!(at == (200 << 16) | 200 || at == (210 << 16) | 200, "where the pointer is: {at:#x}");
                    c.send_event(false, src, EventMask::NO_EVENT, ClientMessageEvent::new(32, src, status, [w, 1, 0, 0, m.data.as_data32()[4]])).unwrap();
                    moved += 1;
                    // Taken: a little further (told once answered), then let go.
                    if moved == 1 {
                        c.xtest_fake_input(xproto::MOTION_NOTIFY_EVENT, 0, x11rb::CURRENT_TIME, root, 210, 200, 0).unwrap();
                    } else if moved == 2 {
                        c.xtest_fake_input(xproto::BUTTON_RELEASE_EVENT, 1, x11rb::CURRENT_TIME, root, 0, 0, 0).unwrap();
                    }
                    c.flush().unwrap();
                }
                Event::ClientMessage(m) if m.type_ == drop => {
                    let prop = a("WEFT_TEST_DROP");
                    c.convert_selection(w, selection, uri, prop, m.data.as_data32()[2]).unwrap();
                    c.flush().unwrap();
                }
                Event::SelectionNotify(s) => {
                    assert_eq!(s.property, a("WEFT_TEST_DROP"), "given");
                    let v = c.get_property(true, w, s.property, AtomEnum::ANY, 0, 1 << 16).unwrap().reply().unwrap();
                    got = Some(String::from_utf8(v.value).unwrap());
                    let src = c.get_selection_owner(selection).unwrap().reply().unwrap().owner;
                    c.send_event(false, src, EventMask::NO_EVENT, ClientMessageEvent::new(32, src, finished, [w, 1, 0, 0, 0])).unwrap();
                    c.flush().unwrap();
                }
                _ => {}
            }
        }
        assert!(entered, "told it entered");
        assert!(moved >= 2, "told where it is");
        assert_eq!(got.as_deref(), Some(want.as_str()), "the files dropped");
        source.join().unwrap().unwrap();
    }
}
