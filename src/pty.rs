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
    child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
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
        Ok(Pty { master: Some(pair.master), writer, rx, child, exited })
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
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

/// Kill a process and all its descendants.  On Windows, msys/cygwin `exec`
/// is emulated with child processes, so killing only the top process would
/// leave shells (and inherited pipe handles) behind.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x0800_0000)
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill").args(["-TERM", &format!("-{pid}")]).status();
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Some(pid) = self.child.process_id() {
            kill_tree(pid);
        }
        let _ = self.child.kill();
        // Closing a pseudo console can block until its output is drained.
        if let Some(m) = self.master.take() {
            thread::spawn(move || drop(m));
        }
    }
}
