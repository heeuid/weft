//! Thin wrapper around portable-pty (ConPTY on Windows, openpty elsewhere).

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

pub struct Pty {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    /// The process behind the terminal (None: a serial port, read in-process).
    child: Option<Box<dyn Child + Send + Sync>>,
    exited: Arc<AtomicBool>,
    /// Set when dropped: a serial port's reader stops and lets the port go.
    closed: Arc<AtomicBool>,
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
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        wake();
                    }
                }
            }
            exited2.store(true, Ordering::SeqCst);
            wake();
        });
        Ok(Pty { master: Some(pair.master), writer, rx, child: Some(child), exited, closed: Arc::new(AtomicBool::new(false)) })
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
        Ok(Pty { master: None, writer: dev, rx, child: None, exited, closed })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
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
        // Closing a pseudo console can block until its output is drained.
        if let Some(m) = self.master.take() {
            thread::spawn(move || drop(m));
        }
    }
}
