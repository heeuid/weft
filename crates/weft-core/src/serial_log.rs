//! Serial console logs: `logs/serial-<name>-<date>.jsonl` on this PC, one
//! line for each piece of traffic of a serial console tab:
//! `{"t": unix seconds, "d": "rx" | "tx", "s": the text}`. rx is what came
//! from the device, tx what was sent to it (typing, pastes). Kept as it
//! came, escapes and all; `lines` turns it into what the viewer shows: the
//! plain text (escapes taken out, backspaces applied) or the raw text (the
//! escapes and control characters written out), a line at a time, each
//! with the time it started.

use crate::records::{self, today};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    /// From the device.
    Rx,
    /// To the device.
    Tx,
    /// weft's own note in the log (it reached its size limit).
    Note,
}

/// What the serial bridge says as it ends (remote/serial.py): what comes
/// after it is the bridge's, not the device's.
pub const BRIDGE_END: &str = "\x1b]7777;{\"t\":\"serial-end\"}\x07";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chunk {
    pub t: f64,
    pub d: Dir,
    pub s: String,
}

/// Which way of the traffic the viewer shows.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Show {
    #[default]
    Both,
    /// What came from the device.
    Output,
    /// What was sent to it.
    Input,
}

impl Show {
    pub const ALL: [Show; 3] = [Show::Both, Show::Output, Show::Input];

    pub fn label(self) -> &'static str {
        match self {
            Show::Both => "in + out",
            Show::Output => "output",
            Show::Input => "input",
        }
    }

    pub fn shows(self, d: Dir) -> bool {
        match self {
            _ if d == Dir::Note => true,
            Show::Both => true,
            Show::Output => d == Dir::Rx,
            Show::Input => d == Dir::Tx,
        }
    }
}

/// How big a port's log may grow in a day: a quarter of the serial logs'
/// limit, a megabyte at least (0: no limit).
pub fn per_day_max(total: u64) -> u64 {
    if total == 0 { 0 } else { (total / 4).max(1024 * 1024) }
}

/// Now, as the log writes it (unix seconds): a reader stamps what it
/// read with it as it reads, not when the window next draws.
pub fn now() -> f64 {
    records::now()
}

pub fn file_name(name: &str, day: &str) -> String {
    format!("serial-{}-{day}.jsonl", crate::host::safe_name(name))
}

/// Writes one tab's traffic to its log for the day (a new file each day).
/// Nothing is written while serial logs are off (settings "records").
pub struct Writer {
    dir: PathBuf,
    name: String,
    day: String,
    file: Option<std::fs::File>,
    /// The start of a character cut in two by a read, for each way.
    carry: [Vec<u8>; 3],
    /// What is sent is kept only once this has come (a remote console's
    /// "connected to"): before it, answers and a sudo password are typed.
    wait_for: Option<String>,
    /// The end of what came, to find `wait_for` and `BRIDGE_END` across
    /// reads.
    tail: Vec<u8>,
    /// `wait_for` came: what came is kept from the next line on.
    skip_line: bool,
    /// What came after the bridge's end is not the device's.
    ended: bool,
    remote: bool,
    /// What today's file holds: it stops at the serial logs' size limit
    /// (a device that never stops writing filled the disk in a day).
    size: u64,
    full: bool,
    /// It reached its limit just now (said once in the tab).
    filled: bool,
}

impl Writer {
    pub fn new(dir: PathBuf, name: &str) -> Writer {
        Writer { dir, name: name.to_string(), day: String::new(), file: None, carry: [Vec::new(), Vec::new(), Vec::new()], wait_for: None, tail: Vec::new(), skip_line: false, ended: false, remote: false, size: 0, full: false, filled: false }
    }

    /// A writer that keeps what is sent only after `ready` has come.
    pub fn after(dir: PathBuf, name: &str, ready: &str) -> Writer {
        Writer { wait_for: Some(ready.to_string()), remote: true, ..Writer::new(dir, name) }
    }

    /// It reached today's limit since last asked.
    pub fn take_filled(&mut self) -> bool {
        std::mem::take(&mut self.filled)
    }

    /// A remote console's (it waits for the bridge's line): its end is the
    /// bridge's.
    fn remote(&self) -> bool {
        self.remote
    }

    /// Today's file of this tab.
    pub fn path(&self) -> PathBuf {
        self.dir.join(file_name(&self.name, &today()))
    }

    pub fn write(&mut self, d: Dir, bytes: &[u8]) {
        self.write_at(d, records::now(), bytes);
    }

    /// `write`, of what came at `t`.
    pub fn write_at(&mut self, d: Dir, t: f64, bytes: &[u8]) {
        let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len().max(1)).position(|w| w == needle);
        let mut kept: Vec<u8> = bytes.to_vec();
        if self.ended {
            return;
        }
        // A remote console: neither way before the bridge's own "connected
        // to" line (ssh's banner and password prompt, its questions, a
        // sudo password), that line neither.
        if let Some(ready) = self.wait_for.clone() {
            if d != Dir::Rx {
                return;
            }
            self.tail.extend_from_slice(&kept);
            match find(&self.tail, ready.as_bytes()) {
                Some(i) => {
                    kept = self.tail[i + ready.len()..].to_vec();
                    self.tail.clear();
                    self.wait_for = None;
                    self.skip_line = true;
                }
                None => {
                    let keep = ready.len();
                    if self.tail.len() > keep {
                        self.tail.drain(..self.tail.len() - keep);
                    }
                    return;
                }
            }
        }
        if self.skip_line && d == Dir::Rx {
            match kept.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    kept.drain(..=nl);
                    self.skip_line = false;
                }
                None => return,
            }
        }
        // The bridge's end: what came before it is the device's. What may
        // be its start, at the end of a read, waits for the next (cut by a
        // read, its first bytes were logged).
        if self.remote() && d == Dir::Rx {
            let mut all = std::mem::take(&mut self.tail);
            all.extend_from_slice(&kept);
            if let Some(i) = find(&all, BRIDGE_END.as_bytes()) {
                all.truncate(i);
                self.ended = true;
                kept = all;
            } else {
                let end = BRIDGE_END.as_bytes();
                let hold = (1..end.len().min(all.len() + 1)).rev().find(|&n| all.ends_with(&end[..n])).unwrap_or(0);
                self.tail = all.split_off(all.len() - hold);
                kept = all;
            }
        }
        let bytes = &kept[..];
        if bytes.is_empty() || !records::serial_log_on() {
            return;
        }
        let carry = &mut self.carry[d as usize];
        carry.extend_from_slice(bytes);
        let s = take_text(carry);
        if s.is_empty() {
            return;
        }
        // The day it came on (a piece read just before midnight is that
        // day's).
        let day = records::local_time(t).get(..10).map(String::from).filter(|d| d.len() == 10).unwrap_or_else(today);
        if self.file.is_none() || day != self.day {
            let _ = std::fs::create_dir_all(&self.dir);
            self.file = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(file_name(&self.name, &day))).ok();
            self.size = self.file.as_ref().and_then(|f| f.metadata().ok()).map(|m| m.len()).unwrap_or(0);
            // Full already (another tab, or this one before a reopening):
            // its note is there once.
            let max = per_day_max(records::serial_log_max());
            self.full = max > 0 && self.size >= max;
            self.day = day;
        }
        if self.full {
            return;
        }
        // A port's day: a quarter of what all serial logs may take (a
        // device that never stops writing filled the disk in a day, and
        // the next pruning took every older log).
        let max = per_day_max(records::serial_log_max());
        let (d, s) = if max > 0 && self.size + s.len() as u64 > max {
            self.full = true;
            self.filled = true;
            (Dir::Note, format!("weft: this port's log for today reached its limit ({:.1} MB: a quarter of the serial logs' total, 1 MB at least; Privacy…) and stops here until tomorrow", max as f64 / 1024.0 / 1024.0))
        } else {
            (d, s)
        };
        if let Some(f) = self.file.as_mut() {
            // One write for the line and its end: two tabs on one port
            // append to one file, and their lines never mix.
            let mut line = serde_json::to_string(&Chunk { t: (t * 1000.0).round() / 1000.0, d, s }).unwrap_or_default();
            line.push('\n');
            if f.write_all(line.as_bytes()).is_ok() {
                self.size += line.len() as u64;
            }
        }
    }
}

/// A byte that is not UTF-8, kept in the text as a character of Unicode's
/// last private area (U+10FF00 and the byte): a binary protocol, a wrong
/// speed's garbage is in the log as it came (raw shows it \xNN, text as
/// U+FFFD).
const BYTE_BASE: u32 = 0x10_FF00;

/// The byte a character keeps, if it keeps one.
pub fn kept_byte(c: char) -> Option<u8> {
    (c as u32).checked_sub(BYTE_BASE).filter(|b| *b < 0x100).map(|b| b as u8)
}

/// The text in `buf`, leaving there the start of a character whose rest
/// has not come yet; each byte that is not UTF-8 is kept as a character
/// of its own (kept_byte; not the whole read spoilt: a character after
/// one is kept whole).
fn take_text(buf: &mut Vec<u8>) -> String {
    let mut out = String::new();
    let mut at = 0;
    loop {
        match std::str::from_utf8(&buf[at..]) {
            Ok(s) => {
                out.push_str(s);
                buf.clear();
                return out;
            }
            Err(e) => {
                let n = e.valid_up_to();
                out.push_str(std::str::from_utf8(&buf[at..at + n]).expect("valid"));
                match e.error_len() {
                    Some(bad) => {
                        for b in &buf[at + n..at + n + bad] {
                            out.push(char::from_u32(BYTE_BASE + *b as u32).expect("in range"));
                        }
                        at += n + bad;
                    }
                    // A character cut by the read: kept for the next.
                    None => {
                        buf.drain(..at + n);
                        return out;
                    }
                }
            }
        }
    }
}

/// A log's pieces, in order (lines that can't be read are left out).
pub fn read(path: &Path) -> Vec<Chunk> {
    std::fs::read_to_string(path).map(|s| parse(&s)).unwrap_or_default()
}

pub fn parse(text: &str) -> Vec<Chunk> {
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// The serial logs kept, newest first.
pub fn files(logs: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(logs) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| n.starts_with("serial-") && n.ends_with(".jsonl")))
        .collect();
    // By date (the end of the name), then by name.
    v.sort_by(|a, b| {
        let key = |p: &PathBuf| {
            let s = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let day = if s.len() >= 10 { s[s.len() - 10..].to_string() } else { String::new() };
            (day, s)
        };
        let (da, na) = key(a);
        let (db, nb) = key(b);
        db.cmp(&da).then(na.cmp(&nb))
    });
    v
}

/// "pi:/dev/ttyUSB0 · 2026-11-17" for a log's file name.
pub fn title(path: &Path) -> String {
    let s = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let s = s.strip_prefix("serial-").unwrap_or(&s);
    if s.len() > 11 {
        format!("{} · {}", &s[..s.len() - 11], &s[s.len() - 10..])
    } else {
        s.to_string()
    }
}

/// One line of a view.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// When its first character came (or was sent).
    pub t: f64,
    pub d: Dir,
    pub text: String,
}

/// The lines of a log, both ways, in the order they started. A line from
/// the device ends at a newline; one sent ends at Enter. `raw`: as it came,
/// with escapes and control characters written out (`\e[1m`, `\r`, `\x03`);
/// else plain text (escapes left out, backspaces applied, Ctrl keys sent as
/// ^C).
pub fn lines(chunks: &[Chunk], raw: bool) -> Vec<Line> {
    let mut b = Lines::default();
    let mut out = Vec::new();
    b.push(chunks, raw, &mut out);
    out.extend(b.tail(raw));
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

/// Lines put together as the pieces come (the viewer of a growing log
/// takes only what is new): the lines each way not ended yet, and whether
/// what came last ended in CRs.
#[derive(Default)]
pub struct Lines {
    open: [(f64, String); 2],
    cr: bool,
}

impl Lines {
    /// The lines these pieces end, onto `out` (in the order they end).
    pub fn push(&mut self, chunks: &[Chunk], raw: bool, out: &mut Vec<Line>) {
        for c in chunks {
            // weft's note: a line of its own.
            if c.d == Dir::Note {
                out.push(Line { t: c.t, d: c.d, text: c.s.trim().to_string() });
                continue;
            }
            let (start, buf) = &mut self.open[c.d as usize];
            for ch in c.s.chars() {
                // What came ended in CRs: a line of its own unless LF comes
                // next (a device that ends its lines with CR alone, a
                // progress line); CR CR LF is one end too.
                if c.d == Dir::Rx && self.cr && ch != '\n' && ch != '\r' {
                    out.push(Line { t: *start, d: c.d, text: shown(buf, c.d, raw) });
                    buf.clear();
                }
                if c.d == Dir::Rx {
                    self.cr = ch == '\r';
                }
                if buf.is_empty() {
                    *start = c.t;
                }
                buf.push(ch);
                let ends = match c.d {
                    Dir::Tx => ch == '\r' || ch == '\n',
                    _ => ch == '\n',
                };
                if ends {
                    out.push(Line { t: *start, d: c.d, text: shown(buf, c.d, raw) });
                    buf.clear();
                }
            }
        }
    }

    /// The lines not ended yet (the last of what came), as they are now.
    pub fn tail(&self, raw: bool) -> Vec<Line> {
        [Dir::Rx, Dir::Tx]
            .into_iter()
            .zip(&self.open)
            .filter(|(_, (_, buf))| !buf.is_empty())
            .map(|(d, (start, buf))| Line { t: *start, d, text: shown(buf, d, raw) })
            .collect()
    }
}

fn shown(s: &str, d: Dir, raw: bool) -> String {
    if raw {
        written_out(s)
    } else {
        let s: String = s.chars().map(|c| if kept_byte(c).is_some() { '\u{FFFD}' } else { c }).collect();
        plain(&s, d)
    }
}

/// Every control character written out, so nothing is hidden.
pub fn written_out(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\x1b' => out.push_str("\\e"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c if kept_byte(c).is_some() => out.push_str(&format!("\\x{:02x}", kept_byte(c).unwrap_or(0))),
            c if (c as u32) < 0x20 || c == '\x7f' || ('\u{80}'..='\u{9f}').contains(&c) => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// The text a person reads: escape sequences left out, backspaces applied.
/// Other control characters: left out of what came, shown as ^C in what
/// was sent (Ctrl+C was pressed).
pub fn plain(s: &str, d: Dir) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        match c {
            '\x1b' => match cs.get(i) {
                // CSI: to its final byte.
                Some('[') => {
                    i += 1;
                    while i < cs.len() && !('@'..='~').contains(&cs[i]) {
                        i += 1;
                    }
                    i += 1;
                }
                // OSC and the other strings: to BEL or ESC \.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    i += 1;
                    while i < cs.len() && cs[i] != '\x07' && !(cs[i] == '\x1b' && cs.get(i + 1) == Some(&'\\')) {
                        i += 1;
                    }
                    i += if cs.get(i) == Some(&'\x1b') { 2 } else { 1 };
                }
                // ESC O A (an arrow key in application mode), ESC ( B…
                Some('O' | '(' | ')' | '#') => i += 2,
                Some(_) => i += 1,
                None => {}
            },
            '\x08' | '\x7f' => {
                out.pop();
            }
            '\t' => out.push('\t'),
            '\r' | '\n' => {}
            c if (c as u32) < 0x20 => {
                if d == Dir::Tx {
                    out.push('^');
                    out.push(char::from(c as u8 + 0x40));
                }
            }
            c if ('\u{80}'..='\u{9f}').contains(&c) => {}
            c => out.push(c),
        }
    }
    out
}

/// "2026-11-17 14:03:21.482", local time.
pub fn clock(t: f64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt((t * 1000.0).round() as i64)
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}

/// A view as text, for the editor or the clipboard: a line each, with its
/// time and `>` before what was sent, as asked.
/// The lines as text; `mark`: what was sent marked "> " (when both ways
/// are shown).
pub fn as_text(lines: &[&Line], times: bool, mark: bool) -> String {
    let mut out = String::new();
    for l in lines {
        if times {
            out.push_str(&format!("[{}] ", clock(l.t)));
        }
        if mark && l.d == Dir::Tx {
            out.push_str("> ");
        }
        out.push_str(&l.text);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rx(t: f64, s: &str) -> Chunk {
        Chunk { t, d: Dir::Rx, s: s.into() }
    }
    fn tx(t: f64, s: &str) -> Chunk {
        Chunk { t, d: Dir::Tx, s: s.into() }
    }

    /// The device's lines, split where it ends them, each with the time it
    /// began; what was typed, a line per Enter, in between where it began.
    #[test]
    fn traffic_becomes_lines_both_ways() {
        let log = [
            rx(1.0, "U-Boot 2024.01\r\n"),
            rx(1.5, "\x1b[1;32mlogin\x1b[0m: "),
            tx(2.0, "r"),
            tx(2.1, "oot"),
            rx(2.05, "r"),
            tx(2.2, "\r"),
            rx(2.3, "oot\r\n# "),
            tx(3.0, "lx\x7fs\r"),
        ];
        let v = lines(&log, false);
        let shown: Vec<(f64, Dir, &str)> = v.iter().map(|l| (l.t, l.d, l.text.as_str())).collect();
        assert_eq!(
            shown,
            vec![(1.0, Dir::Rx, "U-Boot 2024.01"), (1.5, Dir::Rx, "login: root"), (2.0, Dir::Tx, "root"), (2.3, Dir::Rx, "# "), (3.0, Dir::Tx, "ls")]
        );
        let raw = lines(&log, true);
        assert_eq!(raw[1].text, "\\e[1;32mlogin\\e[0m: root\\r\\n", "raw: the escapes written out");
        assert_eq!(raw.last().unwrap().text, "lx\\x7fs\\r");
    }

    #[test]
    fn plain_text_leaves_out_escapes() {
        assert_eq!(plain("\x1b]0;title\x07ok \x1b[?25l\x1b[2Kdone\r", Dir::Rx), "ok done");
        assert_eq!(plain("a\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\b", Dir::Rx), "alinkb");
        assert_eq!(plain("ab\x08 \x08c", Dir::Rx), "ac", "an echoed backspace");
        assert_eq!(plain("x\x03\x1bOA", Dir::Tx), "x^C", "Ctrl+C sent shows; an arrow key does not");
        assert_eq!(plain("x\x03", Dir::Rx), "x");
        assert_eq!(written_out("a\\b\x1b\t\u{1}"), "a\\\\b\\e\\t\\x01");
    }

    /// Bytes are written as they come; a character cut in two by a read is
    /// kept whole.
    #[test]
    fn the_writer_keeps_characters_whole() {
        let dir = std::env::temp_dir().join(format!("weft-serial-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = Writer::new(dir.clone(), "pi:/dev/ttyUSB0");
        let han = "한".as_bytes();
        w.write(Dir::Rx, &[b'a', han[0]]);
        w.write(Dir::Rx, &han[1..]);
        w.write(Dir::Tx, b"ls\r");
        w.write(Dir::Rx, &[0xff, b'z']);
        let path = w.path();
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("serial-pi__dev_ttyUSB0-"));
        let got: Vec<(Dir, String)> = read(&path).into_iter().map(|c| (c.d, c.s)).collect();
        assert_eq!(got, vec![(Dir::Rx, "a".into()), (Dir::Rx, "한".into()), (Dir::Tx, "ls\r".into()), (Dir::Rx, "\u{10ffff}z".into())]);
        assert_eq!(files(&dir), vec![path.clone()]);
        assert!(title(&path).starts_with("pi__dev_ttyUSB0 · 20"), "{}", title(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A remote console keeps what is typed only once the port is open:
    /// the answer to "open with sudo?" and the sudo password are not kept;
    /// what came meanwhile is.
    #[test]
    fn typing_before_the_port_opens_is_not_kept() {
        let dir = std::env::temp_dir().join(format!("weft-serial-ready-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = Writer::after(dir.clone(), "h:/dev/ttyS0", "connected to ");
        w.write(Dir::Rx, b"cannot open /dev/ttyS0: Permission denied\r\nOpen it with sudo now? [y/N] ");
        w.write(Dir::Tx, b"y\r");
        w.write(Dir::Rx, b"[sudo] password for me: ");
        w.write(Dir::Tx, b"hunter2\r");
        w.write(Dir::Rx, b"\x1b[2mconnec");
        w.write(Dir::Rx, b"ted to /dev/ttyS0 at 115200\x1b[0m\r\n");
        w.write(Dir::Tx, b"ls\r");
        w.write(Dir::Rx, b"boot.img\r\n");
        // The bridge ends: what it says after is not the device's.
        w.write(Dir::Rx, format!("# {BRIDGE_END}\r\n\x1b[31m/dev/ttyS0 went away\x1b[0m\r\n").as_bytes());
        w.write(Dir::Rx, b"\r\ndisconnected from /dev/ttyS0\r\n");
        let all = std::fs::read_to_string(w.path()).unwrap();
        assert!(!all.contains("hunter2") && !all.contains("\"y\\r\""), "{all}");
        let sent: Vec<String> = read(&w.path()).into_iter().filter(|c| c.d == Dir::Tx).map(|c| c.s).collect();
        assert_eq!(sent, vec!["ls\r".to_string()], "typing is kept once connected (the word split across reads)");
        let came: String = read(&w.path()).into_iter().filter(|c| c.d == Dir::Rx).map(|c| c.s).collect();
        assert_eq!(came, "boot.img\r\n# ", "the device's, from the line after \"connected to\" to the bridge's end");
        assert!(!all.contains("Permission denied") && !all.contains("connected") && !all.contains("went away"), "the bridge's and ssh's words are not the device's: {all}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_view_as_text() {
        let a = Line { t: 0.0, d: Dir::Rx, text: "# ".into() };
        let b = Line { t: 0.0, d: Dir::Tx, text: "ls".into() };
        assert_eq!(as_text(&[&a, &b], false, true), "# \n> ls\n");
        assert_eq!(as_text(&[&b], false, false), "ls\n", "only what was sent: no mark");
        assert!(as_text(&[&a], true, true).starts_with("[19"), "with its time");
    }

    /// The bridge's end cut across reads, a byte at a time: none of it is
    /// logged, the device's text before it is, whole.
    #[test]
    fn the_bridges_end_cut_by_reads_is_not_logged() {
        let dir = std::env::temp_dir().join(format!("weft-serial-end-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = Writer::after(dir.clone(), "h:/dev/ttyS1", "connected to ");
        w.write(Dir::Rx, b"\x1b[2mconnected to /dev/ttyS1 at 9600\x1b[0m\r\n");
        let rest = format!("last line\r\n{BRIDGE_END}went away\r\n");
        for b in rest.as_bytes() {
            w.write(Dir::Rx, &[*b]);
        }
        let came: String = read(&w.path()).into_iter().map(|c| c.s).collect();
        assert_eq!(came, "last line\r\n", "the device's, none of the end");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Lines made as the pieces come, a piece at a time, are the lines made
    /// of them all at once (the viewer of a growing log takes only what is
    /// new): lines cut across pieces, CR alone, CR CR LF, both ways.
    #[test]
    fn lines_made_piece_by_piece_are_the_same() {
        let c = |t: f64, d: Dir, s: &str| Chunk { t, d, s: s.into() };
        let all = vec![c(1.0, Dir::Rx, "boo"), c(1.5, Dir::Rx, "t\r"), c(2.0, Dir::Tx, "l"), c(2.1, Dir::Tx, "s\r"), c(3.0, Dir::Rx, "\r\nok\r\r\n"), c(4.0, Dir::Rx, "50%\r"), c(5.0, Dir::Rx, "done")];
        let whole = lines(&all, false);
        let mut b = Lines::default();
        let mut ended = Vec::new();
        for piece in all.chunks(1) {
            b.push(piece, false, &mut ended);
        }
        let mut made = ended.clone();
        made.extend(b.tail(false));
        made.sort_by(|a, b| a.t.total_cmp(&b.t));
        assert_eq!(made, whole);
        assert_eq!(whole.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["boot", "ls", "ok", "50%", "done"]);
    }

    /// A device that ends its lines with CR alone, and a progress line that
    /// goes back with CR: a line each, not one line for the day; CRLF is
    /// one end.
    #[test]
    fn a_cr_alone_ends_a_line() {
        let c = |t: f64, s: &str| Chunk { t, d: Dir::Rx, s: s.into() };
        let l: Vec<String> = lines(&[c(1.0, "boot\rready\r"), c(2.0, "\nok\r\n"), c(3.0, "10%\r20%")], false).into_iter().map(|l| l.text).collect();
        assert_eq!(l, ["boot", "ready", "ok", "10%", "20%"]);
    }

    /// Bytes that are not UTF-8 are kept each, the rest as it is: a
    /// character after them, and one cut by the read (kept for the next);
    /// raw shows them \xNN, text as U+FFFD.
    #[test]
    fn bad_bytes_spoil_only_themselves() {
        let mut buf = vec![0xff, 0xED, 0x95, 0x9C, b'a', 0xED, 0x95];
        assert_eq!(take_text(&mut buf), "\u{10FFFF}\u{D55C}a");
        let c = |s: &str| Chunk { t: 1.0, d: Dir::Rx, s: s.into() };
        let mut bin = vec![0x02, 0x80, 0xfe, b'\n'];
        let s = take_text(&mut bin);
        assert_eq!(lines(&[c(&s)], true)[0].text, "\\x02\\x80\\xfe\\n", "raw: the bytes as they came");
        assert_eq!(lines(&[c(&s)], false)[0].text, "\u{FFFD}\u{FFFD}", "text: not shown as characters");
        assert_eq!(buf, vec![0xED, 0x95], "the cut character waits");
        buf.push(0x9C);
        assert_eq!(take_text(&mut buf), "\u{D55C}");
        assert!(Show::Input.shows(Dir::Tx) && !Show::Input.shows(Dir::Rx) && Show::Output.shows(Dir::Rx) && Show::Both.shows(Dir::Tx));
    }
}
