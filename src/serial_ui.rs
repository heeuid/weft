//! "Serial console…": pick a host (this PC or a remote one), a port and a
//! speed, and open the port in a terminal tab. Ports show who has them open:
//! two programs reading one port split its data and both see garbage.
//!
//! Ports are not opened to find out (opening toggles DTR, which resets many
//! boards). Local ports: Windows lets one program have a port at a time, so
//! a busy one fails to open and the tab says so; on Linux and macOS the
//! programs that have it open are listed (/proc, else lsof) and Open asks
//! again before joining them. Remote ports: the agent lists them the same
//! way, and the bridge asks before joining them.

use crate::app::{App, Pending};
use crate::editfind::MatchMode;
use crate::host::{HostKind, Profile, BAUDS};
use crate::theme;
use egui::{vec2, FontId, RichText, ScrollArea, TextEdit};
use weft_core::serial_log::{self, Chunk, Dir, Line, Show};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct PortInfo {
    pub port: String,
    pub desc: String,
    /// (pid, user, command) of other processes that have it open.
    pub holders: Vec<(u64, String, String)>,
}

pub struct SerialDialog {
    pub host: String,
    /// None: being asked for.
    pub ports: Option<Result<Vec<PortInfo>, String>>,
    pub port: String,
    pub baud: u32,
    /// A port in use that Open was pressed for: pressed again, it opens.
    pub anyway: Option<String>,
    /// This PC's ports being listed on a thread (who has each open takes a
    /// moment: /proc, lsof).
    local: Option<std::sync::mpsc::Receiver<Result<Vec<PortInfo>, String>>>,
}

impl SerialDialog {
    pub fn new() -> SerialDialog {
        SerialDialog { host: "local".into(), ports: None, port: String::new(), baud: 115_200, anyway: None, local: None }
    }
}

/// Other processes that have a port of this PC open: Linux reads /proc
/// (and minicom's lock files), macOS and the rest ask lsof. Not on Windows,
/// where a port in use does not open at all.
#[cfg(not(windows))]
fn local_holders(dev: &str) -> Vec<(u64, String, String)> {
    use std::os::unix::fs::MetadataExt;
    let me = std::process::id() as u64;
    if !std::path::Path::new("/proc/self/fd").is_dir() {
        return lsof(dev).map(|out| parse_lsof(&out).into_iter().filter(|h| h.0 != me).collect()).unwrap_or_default();
    }
    let real = std::fs::canonicalize(dev).unwrap_or_else(|_| dev.into());
    let user = |uid: u32| weft_core::host::user_name(Some(uid)).unwrap_or_else(|| uid.to_string());
    let mut found = Vec::new();
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u64>() else { continue };
        if pid == me {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else { continue };
        if fds.flatten().any(|f| std::fs::read_link(f.path()).is_ok_and(|l| l == real)) {
            let cmd = std::fs::read(e.path().join("cmdline")).map(|b| String::from_utf8_lossy(&b).replace('\0', " ").trim().to_string()).unwrap_or_default();
            let uid = e.metadata().map(|m| m.uid()).unwrap_or(0);
            found.push((pid, user(uid), cmd));
        }
    }
    let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for lock in ["/var/lock/LCK..", "/run/lock/LCK.."] {
        let pid = std::fs::read_to_string(format!("{lock}{name}")).ok().and_then(|s| s.split_whitespace().next().and_then(|p| p.parse::<u64>().ok()));
        if let Some(pid) = pid.filter(|p| !found.iter().any(|h| h.0 == *p) && std::path::Path::new(&format!("/proc/{p}")).exists()) {
            found.push((pid, "?".into(), format!("(lock file {lock}{name})")));
        }
    }
    found
}

/// `lsof -FpcL`: a p line starts each process, then its c (command) and L
/// (user) lines.
#[cfg_attr(windows, allow(dead_code))]
pub fn parse_lsof(out: &str) -> Vec<(u64, String, String)> {
    let mut found: Vec<(u64, String, String)> = Vec::new();
    let mut cur: Option<usize> = None;
    for line in out.lines() {
        let (k, v) = line.split_at(line.len().min(1));
        match k {
            "p" => {
                cur = v.parse::<u64>().ok().map(|pid| {
                    found.push((pid, "?".into(), String::new()));
                    found.len() - 1
                });
            }
            "c" => {
                if let Some(i) = cur {
                    found[i].2 = v.to_string();
                }
            }
            "L" => {
                if let Some(i) = cur {
                    found[i].1 = v.to_string();
                }
            }
            _ => {}
        }
    }
    found
}

/// `lsof -FpcL dev`, given five seconds (as the agent's).
#[cfg(not(windows))]
fn lsof(dev: &str) -> Option<String> {
    use std::io::Read;
    let mut c = std::process::Command::new("lsof").args(["-FpcL", dev]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while c.try_wait().ok()?.is_none() {
        if std::time::Instant::now() > end {
            let _ = c.kill();
            let _ = c.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut out = String::new();
    c.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out)
}

/// The ports of this PC (no opening, see above), but macOS' own that are
/// no serial line (as the agent leaves them out).
fn local_ports() -> Result<Vec<PortInfo>, String> {
    let mut v: Vec<PortInfo> = serialport::available_ports()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| {
            let desc = match p.port_type {
                serialport::SerialPortType::UsbPort(u) => [u.manufacturer, u.product].into_iter().flatten().collect::<Vec<_>>().join(" "),
                serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
                serialport::SerialPortType::PciPort => "PCI".into(),
                serialport::SerialPortType::Unknown => String::new(),
            };
            #[cfg(not(windows))]
            let holders = local_holders(&p.port_name);
            #[cfg(windows)]
            let holders = Vec::new();
            PortInfo { port: p.port_name, desc, holders }
        })
        .collect();
    v.retain(|p| !p.port.ends_with("Bluetooth-Incoming-Port") && !p.port.ends_with("debug-console"));
    v.sort_by(|a, b| a.port.len().cmp(&b.port.len()).then_with(|| a.port.cmp(&b.port)));
    Ok(v)
}

/// A port of this PC as weft keeps it: on Windows "COM3" however it was
/// typed (com3, \\.\COM3), one name for its tab, its log and the system's
/// list; elsewhere as it is.
pub fn port_name(port: &str) -> String {
    let p = port.trim();
    if cfg!(windows) {
        let p = p.strip_prefix(r"\\.\").unwrap_or(p);
        if p.len() > 3 && p[..3].eq_ignore_ascii_case("com") && p[3..].chars().all(|c| c.is_ascii_digit()) {
            return p.to_ascii_uppercase();
        }
        return p.to_string();
    }
    p.to_string()
}

/// The agent's answer to "serial_ports".
pub fn parse_ports(r: &Value) -> Vec<PortInfo> {
    r["ports"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| PortInfo {
                    port: p["port"].as_str().unwrap_or("").into(),
                    desc: p["desc"].as_str().unwrap_or("").into(),
                    holders: p["holders"]
                        .as_array()
                        .map(|h| h.iter().map(|x| (x["pid"].as_u64().unwrap_or(0), x["user"].as_str().unwrap_or("").into(), x["cmd"].as_str().unwrap_or("").into())).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

impl App {
    /// On the host of the pane that has the keyboard: the session's own
    /// UARTs (a serial tab's: the host its port is on).
    pub fn open_serial_dialog(&mut self) {
        let host = self.active_pane_id().and_then(|id| self.panes.get(&id)).map(|p| p.host.clone());
        self.open_serial_dialog_on(host.as_deref());
    }

    pub fn open_serial_dialog_on(&mut self, host: Option<&str>) {
        let mut d = SerialDialog::new();
        if let Some(h) = host {
            d.host = match self.profile(h).kind {
                HostKind::Serial { via, .. } => via,
                HostKind::Ssh { .. } | HostKind::Custom { .. } => h.to_string(),
                _ => "local".into(),
            };
        }
        self.serial_dialog = Some(d);
        self.ask_serial_ports();
    }

    fn ask_serial_ports(&mut self) {
        let Some(d) = self.serial_dialog.as_mut() else { return };
        if d.host == "local" {
            d.ports = None;
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(local_ports());
                ctx.request_repaint();
            });
            d.local = Some(rx);
        } else {
            d.local = None;
            d.ports = None;
            let host = d.host.clone();
            self.request(&host, json!({"op": "serial_ports"}), Pending::SerialPorts { host: host.clone() });
        }
    }

    /// The tab for a port: a host profile kept in settings, so the tab comes
    /// back with the session (and a restart) like any other.
    pub fn open_serial(&mut self, via: &str, port: &str, baud: u32) {
        let port = if via == "local" { port_name(port) } else { port.to_string() };
        let port = port.as_str();
        let mut name = if via == "local" { port.to_string() } else { format!("{via}:{port}") };
        // Open in a tab at another speed (a remote port opened again,
        // anyway): a name of its own, the open tab's speed not changed under
        // it (its status line, and Enter after it ended, would use the new).
        let other_speed = |a: &Self, n: &str| a.panes.values().any(|p| p.host == n && !p.exited) && a.settings.profiles.iter().any(|p| p.name == n && !matches!(p.kind, HostKind::Serial { baud: b, .. } if b == baud));
        if other_speed(self, &name) {
            name = format!("{name} @{baud}");
        }
        let kind = HostKind::Serial { via: via.into(), port: port.into(), baud };
        match self.settings.profiles.iter_mut().find(|p| p.name == name) {
            Some(p) => p.kind = kind,
            None => self.settings.profiles.push(Profile { name: name.clone(), kind }),
        }
        self.save_settings();
        self.new_tab(&name, None);
    }

    /// Tabs of this weft that have `port` on `via` open.
    fn serial_tab(&self, via: &str, port: &str) -> Option<String> {
        self.serial_tab_id(via, port).map(|id| self.panes[&id].title())
    }

    fn serial_tab_id(&self, via: &str, port: &str) -> Option<usize> {
        let port = port_name(port);
        self.panes.iter().find_map(|(id, p)| match self.profile(&p.host).kind {
            HostKind::Serial { via: v, port: q, .. } if v == via && port_name(&q) == port && !p.exited => Some(*id),
            _ => None,
        })
    }

    pub fn ui_serial_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.serial_dialog.take() else { return };
        let mut go_to: Option<usize> = None;
        if let Some(ports) = d.local.as_ref().and_then(|rx| rx.try_recv().ok()) {
            d.ports = Some(ports);
            d.local = None;
        }
        let mut open = true;
        let mut ask = false;
        let mut go: Option<(String, String, u32)> = None;
        let hosts: Vec<String> = std::iter::once("local".to_string())
            .chain(self.settings.profiles.iter().filter(|p| matches!(p.kind, HostKind::Ssh { .. } | HostKind::Custom { .. })).map(|p| p.name.clone()))
            .collect();
        crate::widgets::dialog(ctx, "Serial console", vec2(520.0, 460.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Host");
                let before = d.host.clone();
                egui::ComboBox::from_id_salt("serial-host").selected_text(&d.host).show_ui(ui, |ui| {
                    for h in &hosts {
                        ui.selectable_value(&mut d.host, h.clone(), if h == "local" { "local (this PC)" } else { h.as_str() });
                    }
                });
                if d.host != before {
                    ask = true;
                    d.port.clear();
                }
                if crate::icons::button(ui, crate::icons::Icon::Refresh, "list the ports again").clicked() {
                    ask = true;
                }
            });
            ui.add_space(4.0);
            ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| match &d.ports {
                None => {
                    ui.label(RichText::new(format!("asking {}…", d.host)).color(theme::text_faint()));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(theme::error()));
                }
                Some(Ok(ports)) if ports.is_empty() => {
                    ui.label(RichText::new("no serial ports found — type one below").color(theme::text_faint()));
                }
                Some(Ok(ports)) => {
                    for p in ports {
                        let ours = self.serial_tab(&d.host, &p.port);
                        let busy = !p.holders.is_empty() || ours.is_some();
                        let sel = d.port == p.port;
                        let r = ui.horizontal(|ui| {
                            let r = ui.selectable_label(sel, RichText::new(&p.port).monospace().color(if busy { theme::warn() } else { theme::text() }));
                            if !p.desc.is_empty() {
                                ui.label(RichText::new(&p.desc).size(11.5).color(theme::text_dim()));
                            }
                            r
                        });
                        let r = r.inner;
                        if let Some(t) = &ours {
                            ui.label(RichText::new(format!("   open in this weft: \"{t}\"")).size(11.5).color(theme::warn()));
                        }
                        for (pid, user, cmd) in &p.holders {
                            ui.label(RichText::new(format!("   in use: pid {pid} · {user} · {cmd}")).size(11.5).color(theme::warn()));
                        }
                        if r.clicked() {
                            d.port = p.port.clone();
                        }
                        if r.double_clicked() {
                            go = Some((d.host.clone(), p.port.clone(), d.baud));
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Port");
                // The first port listed as the example (each system names its
                // own: COM3, /dev/ttyUSB0, /dev/cu.usbserial-…).
                let hint = d.ports.as_ref().and_then(|r| r.as_ref().ok()).and_then(|v| v.first()).map(|p| p.port.clone()).unwrap_or_default();
                ui.add(TextEdit::singleline(&mut d.port).hint_text(hint).desired_width(170.0));
                ui.label("Speed");
                // Any speed typed (74880 for an ESP8266's boot), or one of
                // the list.
                ui.add(egui::DragValue::new(&mut d.baud).range(50..=20_000_000).speed(0.0)).on_hover_text("click to type a speed, or pick one from the list");
                egui::ComboBox::from_id_salt("serial-baud").selected_text("").width(18.0).show_ui(ui, |ui| {
                    for b in BAUDS {
                        ui.selectable_value(&mut d.baud, *b, b.to_string());
                    }
                });
                if ui.add_enabled(!d.port.trim().is_empty(), egui::Button::new("Open")).on_hover_text("in a new tab; Ctrl+] ends it").clicked() {
                    go = Some((d.host.clone(), d.port.trim().to_string(), d.baud));
                }
            });
            if let Some(p) = d.anyway.clone().filter(|p| *p == d.port.trim()) {
                match self.serial_tab_id(&d.host, &p) {
                    // A port of this PC opens once (the system holds it for
                    // the tab that has it): that tab, not a second.
                    Some(id) if d.host == "local" => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{p} is open in this weft already, in the tab \"{}\" (a port of this PC opens once).", self.panes[&id].title())).color(theme::warn()));
                            if ui.button("Go to that tab").clicked() {
                                go_to = Some(id);
                            }
                        });
                    }
                    Some(id) => {
                        ui.label(RichText::new(format!("{p} is open in this weft already, in the tab \"{}\". Two readers split the data and both see garbage: press Open again to open it anyway.", self.panes[&id].title())).color(theme::warn()));
                    }
                    None => {
                        ui.label(RichText::new(format!("{p} is open in another program (above). Two readers split the data and both see garbage: press Open again to open it anyway.")).color(theme::warn()));
                    }
                }
            }
            ui.label(
                RichText::new("8 data bits, no parity, 1 stop bit. A port in use: Windows refuses it; on Linux and macOS, and on a remote host, you are asked first (two readers garble the data).")
                    .size(11.0)
                    .color(theme::text_faint()),
            );
        });
        self.serial_dialog = Some(d);
        if ask {
            self.ask_serial_ports();
        }
        // A port a tab of this weft has open, or (here) another program:
        // Open again to join it (another program's on a remote host: the
        // bridge asks).
        if let Some(id) = go_to {
            self.serial_dialog = None;
            self.focus_pane(id);
            return;
        }
        let ours: Option<usize> = go.as_ref().and_then(|(via, port, _)| self.serial_tab_id(via, port));
        let busy = |d: &SerialDialog, port: &str| ours.is_some() || (d.host == "local" && matches!(&d.ports, Some(Ok(v)) if v.iter().any(|p| p.port == port && !p.holders.is_empty())));
        if let Some((via, port, baud)) = go.take_if(|(via, port, _)| {
            let d = self.serial_dialog.as_mut().unwrap();
            // A port of this PC a tab has: never a second (it can't open).
            let again_ok = !(ours.is_some() && via == "local");
            if busy(d, port) && (d.anyway.as_deref() != Some(port.as_str()) || !again_ok) {
                d.anyway = Some(port.clone());
                d.port = port.clone();
                return false;
            }
            true
        }) {
            self.serial_dialog = None;
            self.open_serial(&via, &port, baud);
        } else if !open {
            self.serial_dialog = None;
        }
    }
}

/// "Serial console log": a serial console's traffic, read from its log on
/// this PC (weft_core::serial_log). Plain text or raw, output (from the
/// device), input (sent to it) or both, with or without times, searched
/// like the editor (text, fuzzy, glob, regex). The choices are kept
/// (settings "search"). Today's log grows while its tab is open: it is read
/// again when it changes.
pub struct SerialLogView {
    pub files: Vec<std::path::PathBuf>,
    pub file: Option<std::path::PathBuf>,
    pub query: String,
    chunks: Vec<Chunk>,
    /// How much of the file is read; a line not yet whole at its end.
    read_len: u64,
    partial: Vec<u8>,
    /// The tab's port whose log of today is followed (a new file each day),
    /// until another is picked.
    follow: Option<String>,
    /// That port has no log for today (nothing came yet, or logs are off).
    missing: bool,
    checked: std::time::Instant,
    lines: Vec<Line>,
    lines_raw: bool,
    /// The lines made so far (a growing log: only what is new is taken),
    /// how many pieces they took, how many of them are ended (the rest,
    /// at the end, may grow).
    builder: serial_log::Lines,
    built: usize,
    ended: usize,
    /// The lines shown, and what they were worked out for.
    hits: Vec<usize>,
    hits_key: Option<(String, MatchMode, bool, Show, bool)>,
    bad_query: bool,
}

impl SerialLogView {
    fn new(logs: &std::path::Path, follow: Option<String>) -> SerialLogView {
        let files = serial_log::files(logs);
        // The tab's port's log of today; none yet: said, not another's.
        let want = follow.as_ref().map(|h| logs.join(serial_log::file_name(h, &crate::records::today())));
        let missing = want.as_ref().is_some_and(|w| !files.contains(w));
        let file = match &want {
            Some(w) => files.contains(w).then(|| w.clone()),
            None => files.first().cloned(),
        };
        let mut v = SerialLogView {
            files,
            file,
            query: String::new(),
            chunks: Vec::new(),
            read_len: 0,
            partial: Vec::new(),
            follow,
            missing,
            checked: std::time::Instant::now(),
            lines: Vec::new(),
            lines_raw: false,
            builder: Default::default(),
            built: 0,
            ended: 0,
            hits: Vec::new(),
            hits_key: None,
            bad_query: false,
        };
        v.read();
        v
    }

    /// The file read from the start.
    fn read(&mut self) {
        self.chunks.clear();
        self.read_len = 0;
        self.partial.clear();
        self.restart_lines();
        self.read_more();
    }

    fn restart_lines(&mut self) {
        self.builder = Default::default();
        self.built = 0;
        self.ended = 0;
        self.lines.clear();
        self.hits.clear();
        self.hits_key = None;
    }

    /// What the file has gained since it was read (a log grows while its
    /// tab is open: not read whole again each time); read anew where it
    /// got shorter (another file by the same name).
    fn read_more(&mut self) {
        use std::io::{Read, Seek};
        let Some(path) = self.file.clone() else {
            self.restart_lines();
            return;
        };
        let len = path.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.read_len {
            self.chunks.clear();
            self.read_len = 0;
            self.partial.clear();
            self.restart_lines();
        }
        let mut more = Vec::new();
        if let Ok(mut f) = std::fs::File::open(&path) {
            if f.seek(std::io::SeekFrom::Start(self.read_len)).is_ok() {
                let _ = f.read_to_end(&mut more);
            }
        }
        self.read_len += more.len() as u64;
        // Whole lines only: the last may still be being written (a read
        // can end inside a character).
        self.partial.extend_from_slice(&more);
        if let Some(end) = self.partial.iter().rposition(|&b| b == b'\n') {
            let whole: Vec<u8> = self.partial.drain(..=end).collect();
            self.chunks.extend(serial_log::parse(&String::from_utf8_lossy(&whole)));
        }
    }

    /// The lines for these choices (worked out again only when they or the
    /// log change).
    fn update(&mut self, raw: bool, mode: MatchMode, case: bool, show: Show) {
        if self.lines_raw != raw {
            self.restart_lines();
            self.lines_raw = raw;
        }
        // What came since: its lines added (the lines not ended taken off
        // and made again with what is new).
        let mut from = None;
        if self.built < self.chunks.len() || (self.built == 0 && self.lines.is_empty() && !self.chunks.is_empty()) {
            self.lines.truncate(self.ended);
            let before = self.ended;
            self.builder.push(&self.chunks[self.built..], raw, &mut self.lines);
            self.built = self.chunks.len();
            // Out of order (a line sent ended after one that came later):
            // all sorted, and all looked at again.
            let sorted = self.lines.windows(2).skip(before.saturating_sub(1)).all(|w| w[0].t <= w[1].t);
            if !sorted {
                self.lines.sort_by(|a, b| a.t.total_cmp(&b.t));
            }
            self.ended = self.lines.len();
            // The lines not ended yet, in the order of their times (a
            // password typed after its prompt).
            let mut tail = self.builder.tail(raw);
            tail.sort_by(|a, b| a.t.total_cmp(&b.t));
            self.lines.extend(tail);
            from = Some(if sorted { before } else { 0 });
        }
        let key = (self.query.clone(), mode, case, show, raw);
        // Nothing new, the same choices: nothing to look at (the query is
        // not made again each frame).
        if from.is_none() && self.hits_key.as_ref() == Some(&key) {
            return;
        }
        // Matched as everywhere (crate::find): a line at a time.
        let m = if self.query.is_empty() { None } else { crate::find::Matcher::build(mode, &self.query, crate::find::Target::Line, Some(case)).ok() };
        self.bad_query = !self.query.is_empty() && m.is_none();
        let hit = |l: &Line| show.shows(l.d) && m.as_ref().is_none_or(|m| m.matches(&l.text).is_some());
        match (self.hits_key.as_ref() == Some(&key), from) {
            (true, None) => {}
            // The same choices: only the new lines looked at.
            (true, Some(from)) => {
                self.hits.retain(|&i| i < from);
                self.hits.extend((from..self.lines.len()).filter(|&i| hit(&self.lines[i])));
            }
            _ => {
                self.hits = (0..self.lines.len()).filter(|&i| hit(&self.lines[i])).collect();
                self.hits_key = Some(key);
            }
        }
    }
}

impl App {
    /// The serial log viewer, on `pane_host`'s log of today when given.
    pub fn open_serial_log(&mut self, pane_host: Option<&str>) {
        let logs = self.data.path("logs");
        self.serial_log = Some(SerialLogView::new(&logs, pane_host.map(String::from)));
    }

    pub fn ui_serial_log(&mut self, ctx: &egui::Context) {
        let Some(mut v) = self.serial_log.take() else { return };
        let mut open = true;
        let mut reread = false;
        let mut pick: Option<std::path::PathBuf> = None;
        let mut to_editor: Option<(String, String)> = None;
        let mut copy: Option<String> = None;
        // Today's log grows while its tab is open; a port followed goes on
        // to the next day's file (and to today's once it is there).
        let mut more = false;
        if v.checked.elapsed() >= std::time::Duration::from_millis(700) {
            v.checked = std::time::Instant::now();
            if let Some(h) = v.follow.clone() {
                let today = self.data.path("logs").join(serial_log::file_name(&h, &crate::records::today()));
                if v.file.as_ref() != Some(&today) && today.is_file() {
                    pick = Some(today);
                }
            }
            let len = v.file.as_deref().and_then(|p| p.metadata().ok()).map(|m| m.len()).unwrap_or(0);
            if len != v.read_len {
                more = true;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(700));
        let modes = &mut self.settings.search;
        let before = *modes;
        crate::widgets::dialog(ctx, "Serial console log", vec2(760.0, 560.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let name = match (&v.file, &v.follow) {
                    (Some(f), _) => serial_log::title(f),
                    (None, Some(h)) if v.missing => format!("{h}: no log today yet"),
                    _ => "no serial logs yet".into(),
                };
                egui::ComboBox::from_id_salt("serial-log-file").width(330.0).selected_text(name).show_ui(ui, |ui| {
                    for f in &v.files {
                        if ui.selectable_label(v.file.as_ref() == Some(f), serial_log::title(f)).clicked() {
                            pick = Some(f.clone());
                            // Another picked: that one is kept, not followed.
                            v.follow = None;
                        }
                    }
                });
                if crate::icons::button(ui, crate::icons::Icon::Refresh, "list the logs and read this one again").clicked() {
                    v.files = serial_log::files(&self.data.path("logs"));
                    // A port followed with no log today yet: still that one
                    // (taken up when its file comes), not another's.
                    pick = if v.file.is_none() && v.follow.is_some() { None } else { v.file.clone().or_else(|| v.files.first().cloned()) };
                    reread = pick.is_some();
                }
            });
            if v.missing && v.file.is_none() {
                ui.label(RichText::new("This tab's port has no log for today: nothing has come from it yet, or serial logs are off (Privacy…). Older logs are in the list.").size(11.5).color(theme::text_faint()));
            }
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut modes.serial_raw, false, "text").on_hover_text("as read: escape sequences left out, backspaces applied");
                ui.selectable_value(&mut modes.serial_raw, true, "raw").on_hover_text("as it came: escapes and control characters written out (\\e[1m, \\r, \\x03)");
                ui.separator();
                for s in Show::ALL {
                    let tip = match s {
                        Show::Both => "what the device sent and what was sent to it",
                        Show::Output => "only what the device sent",
                        Show::Input => "only what was sent to the device (typing, pastes)",
                    };
                    ui.selectable_value(&mut modes.serial_show, s, s.label()).on_hover_text(tip);
                }
                ui.separator();
                ui.checkbox(&mut modes.serial_times, "times").on_hover_text("when each line started, to the millisecond");
            });
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 200.0).max(120.0);
                ui.add(TextEdit::singleline(&mut v.query).hint_text("search lines…").desired_width(w));
                egui::ComboBox::from_id_salt("serial-log-mode").width(58.0).selected_text(RichText::new(modes.serial_mode.label()).size(12.0)).show_ui(ui, |ui| {
                    for m in MatchMode::ALL {
                        ui.selectable_value(&mut modes.serial_mode, m, m.label()).on_hover_text(m.hint());
                    }
                });
                ui.toggle_value(&mut modes.serial_case, "Aa").on_hover_text("match case");
            });
            let (raw, mode, case, show, times) = (modes.serial_raw, modes.serial_mode, modes.serial_case, modes.serial_show, modes.serial_times);
            v.update(raw, mode, case, show);
            ui.horizontal(|ui| {
                let note = if v.bad_query { "the query can't be read".to_string() } else { format!("{} of {} lines", v.hits.len(), v.lines.len()) };
                ui.label(RichText::new(note).size(11.0).color(if v.bad_query { theme::error() } else { theme::text_faint() }));
                if !self.settings.records.serial_log.on {
                    ui.label(RichText::new("· serial logs are off (Privacy…)").size(11.0).color(theme::warn()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let shown = || -> Vec<&Line> { v.hits.iter().map(|&i| &v.lines[i]).collect() };
                    if ui.small_button("Open in editor").on_hover_text("these lines as text, to read whole or save").clicked() {
                        let title = v.file.as_deref().map(serial_log::title).unwrap_or_default();
                        to_editor = Some((format!("serial {title}"), serial_log::as_text(&shown(), times, show == Show::Both)));
                    }
                    if ui.small_button("Copy").on_hover_text("these lines").clicked() {
                        copy = Some(serial_log::as_text(&shown(), times, show == Show::Both));
                    }
                });
            });
            ui.separator();
            let font = FontId::monospace(12.5);
            let row_h = ui.fonts(|f| f.row_height(&font)) + 3.0;
            let time_w = if times { crate::widgets::text_width(ui.painter(), "2026-11-17 14:03:21.482  ", font.clone()) } else { 0.0 };
            let mark_w = if show == Show::Both { crate::widgets::text_width(ui.painter(), "> ", font.clone()) } else { 0.0 };
            ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show_rows(ui, row_h, v.hits.len(), |ui, range| {
                // As wide as the longest line shown: a long line scrolls
                // sideways, not cut.
                let lead = 8.0 + time_w + mark_w;
                let widest = v.hits[range.clone()].iter().map(|&i| crate::widgets::text_width(ui.painter(), &v.lines[i].text, font.clone())).fold(0.0_f32, f32::max);
                let w = ui.available_width().max(lead + widest);
                for &i in &v.hits[range] {
                    let l = &v.lines[i];
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), egui::Sense::click());
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(rect, 3.0, theme::bg_hover());
                    }
                    let y = rect.center().y;
                    if times {
                        p.text(egui::pos2(rect.left() + 4.0, y), egui::Align2::LEFT_CENTER, serial_log::clock(l.t), font.clone(), theme::text_faint());
                    }
                    let sent = l.d == Dir::Tx;
                    let note = l.d == Dir::Note;
                    if sent && show == Show::Both {
                        p.text(egui::pos2(rect.left() + 4.0 + time_w, y), egui::Align2::LEFT_CENTER, ">", font.clone(), theme::accent());
                    }
                    let x = rect.left() + 4.0 + time_w + mark_w;
                    let col = if note { theme::text_faint() } else if sent { theme::accent() } else { theme::text() };
                    p.text(egui::pos2(x, y), egui::Align2::LEFT_CENTER, &l.text, font.clone(), col);
                    let resp = resp.on_hover_text(format!("{} · {}\n{}", serial_log::clock(l.t), if note { "weft's note" } else if sent { "sent" } else { "from the device" }, l.text));
                    resp.context_menu(|ui| {
                        if ui.button("Copy line").clicked() {
                            copy = Some(l.text.clone());
                            ui.close_menu();
                        }
                    });
                }
            });
        });
        if *modes != before {
            self.save_settings();
        }
        if let Some(f) = pick {
            v.files = serial_log::files(&self.data.path("logs"));
            v.missing = false;
            v.file = Some(f);
            reread = true;
        }
        if reread {
            v.read();
        } else if more {
            v.read_more();
        }
        if let Some((title, text)) = to_editor {
            self.actions.push(crate::app::Action::OpenScratch(title, text));
        }
        if let Some(s) = copy {
            self.actions.push(crate::app::Action::Copy(s));
        }
        if open {
            self.serial_log = Some(v);
        }
    }
}
