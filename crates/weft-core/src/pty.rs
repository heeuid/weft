//! Thin wrapper around portable-pty (ConPTY on Windows, openpty elsewhere).

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Pseudo consoles on their way out, each closed on a thread of its own
/// (see Drop).
static CLOSING: Mutex<Vec<thread::JoinHandle<()>>> = Mutex::new(Vec::new());

/// Wait until the pseudo consoles being closed are closed, `limit` at most;
/// says how many were not. To be called before the process ends: a console
/// host whose pseudo console was never closed stays behind for good and
/// keeps a core busy (weft left one per closed window; after a day of
/// tests there were 270, using a third of the machine).
pub fn finish_closing(limit: Duration) -> usize {
    let end = Instant::now() + limit;
    loop {
        let left = {
            let mut closing = CLOSING.lock().unwrap_or_else(|e| e.into_inner());
            closing.retain(|h| !h.is_finished());
            closing.len()
        };
        if left == 0 || Instant::now() >= end {
            return left;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub struct Pty {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    /// The process behind the terminal (None: a serial port, read in-process).
    child: Option<Box<dyn Child + Send + Sync>>,
    exited: Arc<AtomicBool>,
    /// Set when dropped: a serial port's reader stops and lets the port go.
    closed: Arc<AtomicBool>,
    /// A cursor position report went to it (see `Drop`).
    reported: bool,
}

/// `ESC [ row ; col R` somewhere in these bytes: a cursor position report.
fn has_cursor_report(bytes: &[u8]) -> bool {
    let digits = |b: &[u8]| b.iter().take_while(|c| c.is_ascii_digit()).count();
    bytes.windows(2).enumerate().filter(|(_, w)| w == b"\x1b[").any(|(i, _)| {
        let rest = &bytes[i + 2..];
        let row = digits(rest);
        let col = if rest.get(row) == Some(&b';') { digits(&rest[row + 1..]) } else { 0 };
        row > 0 && col > 0 && rest.get(row + 1 + col) == Some(&b'R')
    })
}

impl Pty {
    pub fn spawn(
        argv: &[String],
        cwd: Option<&str>,
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Pty, String> {
        let pair = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("openpty: {e}"))?;
        let mut cmd = CommandBuilder::new(&argv[0]);
        for a in &argv[1..] {
            cmd.arg(a);
        }
        if let Some(dir) = cwd {
            if std::path::Path::new(dir).is_dir() {
                cmd.cwd(dir);
            }
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // As terminals name themselves to what runs in them
        // (`echo $TERM_PROGRAM_VERSION`; local shells: ssh passes neither).
        cmd.env("TERM_PROGRAM", "weft");
        cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| format!("spawn {}: {e}", argv[0]))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        let exited = Arc::new(AtomicBool::new(false));
        let exited2 = exited.clone();
        thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            // Once the terminal is dropped nobody takes the output, but it
            // is still read to its end: the console host writes a last frame
            // as it closes and waits until that is taken. With nobody
            // reading, closing the pseudo console never returned, and the
            // host was left behind when weft ended.
            let mut dropped = false;
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if dropped => {}
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            // Elsewhere the reader lets go at once: the
                            // master closing is what ends whatever still
                            // runs in the terminal (SIGHUP).
                            if !cfg!(windows) {
                                break;
                            }
                            dropped = true;
                        } else {
                            wake();
                        }
                    }
                }
            }
            exited2.store(true, Ordering::SeqCst);
            if !dropped {
                wake();
            }
        });
        Ok(Pty { master: Some(pair.master), writer, rx, child: Some(child), exited, closed: Arc::new(AtomicBool::new(false)), reported: false })
    }

    /// A serial port (COM3, /dev/ttyUSB0) as the terminal's other end, opened
    /// here: what is typed goes out, what comes in is shown, a lone LF as
    /// CRLF (as picocom's --imap lfcrlf). Unplugged, the pane ends.
    pub fn serial(port: &str, baud: u32, wake: impl Fn() + Send + 'static) -> Result<Pty, String> {
        let dev = serialport::new(port, baud).timeout(std::time::Duration::from_millis(100)).open().map_err(|e| match e.kind() {
            serialport::ErrorKind::NoDevice => format!("{port}: no such port"),
            serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied) => format!("{port} is in use by another program (only one can have it open)"),
            _ if e.to_string().to_lowercase().contains("denied") => format!("{port} is in use by another program (only one can have it open)"),
            _ => format!("{port}: {e}"),
        })?;
        let mut reader = dev.try_clone().map_err(|e| format!("{port}: {e}"))?;
        let (tx, rx) = mpsc::channel();
        let exited = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let (exited2, closed2) = (exited.clone(), closed.clone());
        let name = port.to_string();
        let _ = tx.send(format!("\x1b[2mconnected to {name} at {baud}\x1b[0m\r\n").into_bytes());
        thread::spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            let mut last = 0u8;
            while !closed2.load(Ordering::SeqCst) {
                match reader.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        let mut out = Vec::with_capacity(n + 8);
                        for &b in &buf[..n] {
                            if b == b'\n' && last != b'\r' {
                                out.push(b'\r');
                            }
                            out.push(b);
                            last = b;
                        }
                        if tx.send(out).is_err() {
                            break;
                        }
                        wake();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(e) => {
                        let _ = tx.send(format!("\r\n\x1b[31m{name}: {e}\x1b[0m\r\n").into_bytes());
                        break;
                    }
                }
            }
            exited2.store(true, Ordering::SeqCst);
            wake();
        });
        Ok(Pty { master: None, writer: dev, rx, child: None, exited, closed, reported: true })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if !self.reported && has_cursor_report(bytes) {
            self.reported = true;
        }
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Some(m) = self.master.as_ref() {
            let _ = m.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        }
    }

    pub fn read(&self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Ok(chunk) = self.rx.try_recv() {
            out.extend_from_slice(&chunk);
            if out.len() > 4 * 1024 * 1024 {
                break;
            }
        }
        out
    }

    pub fn exited(&mut self) -> bool {
        if self.exited.load(Ordering::SeqCst) {
            return true;
        }
        self.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(Some(_))))
    }
}

/// Kill a process and all its descendants.  On Windows, msys/cygwin `exec`
/// is emulated with child processes, so killing only the top process would
/// leave shells (and inherited pipe handles) behind.
pub fn kill_tree(pid: u32) -> bool {
    // Fire and forget: taskkill takes ~0.5 s to walk the process tree, and
    // waiting for it froze the UI on every pane close (and longer on exit).
    // It runs on its own, so it finishes even if weft exits first.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("kill").args(["-TERM", &format!("-{pid}")]).spawn().is_ok()
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // taskkill /T needs the root alive to find its descendants, so it is
        // left to end the root too; kill it directly only if that failed.
        self.closed.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.as_mut() {
            let delegated = child.process_id().map(kill_tree).unwrap_or(false);
            if !delegated {
                let _ = child.kill();
            }
        }
        // Windows' console host asks where the cursor is as it starts
        // (ESC[6n; portable-pty opens it with "inherit cursor") and does
        // nothing until it is told, not even close. The terminal answers
        // as it draws; one dropped before that (opened and closed at once,
        // or weft closed as it started) is answered here. Unanswered, the
        // close below never returned, and the host outlived weft.
        if cfg!(windows) && self.master.is_some() && !self.reported {
            let _ = self.writer.write_all(b"\x1b[1;1R");
            let _ = self.writer.flush();
        }
        // Closing a pseudo console can block until its output is drained:
        // on a thread, which `finish_closing` waits for before weft ends.
        if let Some(m) = self.master.take() {
            let closer = thread::spawn(move || drop(m));
            let mut closing = CLOSING.lock().unwrap_or_else(|e| e.into_inner());
            closing.retain(|h| !h.is_finished());
            closing.push(closer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal that is dropped has its pseudo console closed by the time
    /// `finish_closing` returns: nothing is left for the process's end to
    /// cut short (which left a console host running for good).
    #[test]
    fn a_dropped_terminal_is_closed_before_the_end() {
        let argv: Vec<String> = ["python", "-c", "import time; time.sleep(60)"].iter().map(|s| s.to_string()).collect();
        // Dropped at once, before anything it sent was answered; dropped
        // after a moment; and one that was answered as a terminal does.
        let early: Vec<Pty> = (0..2).map(|_| Pty::spawn(&argv, None, &[], 80, 24, || {}).expect("python")).collect();
        drop(early);
        let mut later: Vec<Pty> = (0..2).map(|_| Pty::spawn(&argv, None, &[], 80, 24, || {}).expect("python")).collect();
        thread::sleep(Duration::from_millis(700));
        later[0].write(b"\x1b[1;1R");
        assert!(later[0].reported && !later[1].reported);
        drop(later);
        assert_eq!(finish_closing(Duration::from_secs(20)), 0, "every pseudo console was closed");
        assert_eq!(finish_closing(Duration::ZERO), 0, "and none is waiting");
    }

    #[test]
    fn cursor_reports_are_told_from_other_input() {
        assert!(has_cursor_report(b"\x1b[1;1R"));
        assert!(has_cursor_report(b"ls\r\x1b[24;80Rmore"));
        for other in [&b"\x1b[A"[..], b"\x1b[1;5R\x1b"[..0].as_ref(), b"R", b"\x1b[;1R", b"\x1b[1;R", b"\x1b[12R", b"\x1b[1;1H", b""] {
            assert!(!has_cursor_report(other), "{other:?}");
        }
    }
}
