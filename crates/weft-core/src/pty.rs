//! Thin wrapper around portable-pty (ConPTY on Windows, openpty elsewhere).

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
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

/// Which console host this process's terminals run on, said in words
/// (Windows; elsewhere the system's ptys, and nothing to choose).
static CONSOLE_HOST: std::sync::OnceLock<ConsoleHost> = std::sync::OnceLock::new();

/// The console host in use, and why not the one that was asked for, if so.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleHost {
    /// "Windows' own (conhost.exe)", or the conpty.dll's path.
    pub what: String,
    /// The setting named a file that isn't there (Windows' own is used).
    pub problem: Option<String>,
}

/// Decide which console host terminals use, once, before the first one is
/// opened. On Windows a terminal runs on a console host: Windows has one
/// (conhost.exe), and a newer or pinned one can be given as a conpty.dll
/// (with its OpenConsole.exe beside it):
///
/// 1. the file `setting` names (a conpty.dll, or the folder it is in);
/// 2. else a conpty.dll beside weft's executable;
/// 3. else Windows' own.
///
/// Never one that merely is on PATH: portable-pty loads "conpty.dll" by
/// name, which found another terminal's (WezTerm's, when installed), so
/// weft behaved differently from one PC to the next and changed when that
/// program was updated.
pub fn choose_console_host(setting: &str) -> ConsoleHost {
    CONSOLE_HOST.get_or_init(|| pick_console_host(setting)).clone()
}

/// Where the setting, or the folder beside the executable, has a
/// conpty.dll; and what to say if the setting's isn't there.
#[cfg(any(windows, test))]
fn console_host_file(setting: &str, beside: Option<&std::path::Path>) -> (Option<std::path::PathBuf>, Option<String>) {
    let setting = setting.trim();
    let mut problem = None;
    if !setting.is_empty() {
        let p = std::path::PathBuf::from(setting);
        let file = if p.is_dir() { p.join("conpty.dll") } else { p };
        if file.is_file() {
            return (Some(file), None);
        }
        problem = Some(format!("console host: {} is not there; Windows' own is used", file.display()));
    }
    let beside = beside.map(|d| d.join("conpty.dll")).filter(|f| f.is_file());
    // (One beside the executable is used even when the setting's is missing.)
    (beside, problem)
}

#[cfg(windows)]
fn pick_console_host(setting: &str) -> ConsoleHost {
    use windows_sys::Win32::System::LibraryLoader::{LoadLibraryExW, SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LOAD_LIBRARY_SEARCH_USER_DIRS};
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf()));
    let (file, mut problem) = console_host_file(setting, exe_dir.as_deref());
    unsafe {
        // From here on a DLL asked for by name comes from weft's folder or
        // Windows' own, not from PATH or the current folder.
        SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_APPLICATION_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS);
        if let Some(file) = file {
            // Loaded by its full path now: "conpty.dll" asked for by name
            // later is this one.
            let wide: Vec<u16> = file.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
            let loaded = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
            if !loaded.is_null() {
                return ConsoleHost { what: file.display().to_string(), problem };
            }
            problem = Some(format!("console host: {} could not be loaded; Windows' own is used", file.display()));
        }
    }
    ConsoleHost { what: "Windows' own (conhost.exe)".into(), problem }
}

#[cfg(not(windows))]
fn pick_console_host(_setting: &str) -> ConsoleHost {
    ConsoleHost { what: "the system's ptys".into(), problem: None }
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
    /// Columns and rows it was opened with.
    opened: (u16, u16),
    /// The process's exit code, once it has ended.
    code: Option<u32>,
}

/// These bytes with their first cursor position report kept inside a screen
/// of `rows` by `cols`.
fn fit_cursor_report(bytes: &[u8], rows: u16, cols: u16) -> Vec<u8> {
    let digits = |b: &[u8]| b.iter().take_while(|c| c.is_ascii_digit()).count();
    for i in 0..bytes.len().saturating_sub(1) {
        if &bytes[i..i + 2] != b"\x1b[" {
            continue;
        }
        let rest = &bytes[i + 2..];
        let r = digits(rest);
        let c = if rest.get(r) == Some(&b';') { digits(&rest[r + 1..]) } else { 0 };
        if r == 0 || c == 0 || rest.get(r + 1 + c) != Some(&b'R') {
            continue;
        }
        let number = |b: &[u8]| std::str::from_utf8(b).ok().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
        let row = number(&rest[..r]).clamp(1, rows.max(1) as u32);
        let col = number(&rest[r + 1..r + 1 + c]).clamp(1, cols.max(1) as u32);
        let mut out = bytes[..i].to_vec();
        out.extend_from_slice(format!("\x1b[{row};{col}R").as_bytes());
        out.extend_from_slice(&rest[r + 2 + c..]);
        return out;
    }
    bytes.to_vec()
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
        // (Decided by now when weft runs; for whoever opens a terminal
        // without having said: no setting.)
        choose_console_host("");
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
        // A program started from the Finder or the Dock (or a desktop menu)
        // has no locale: `ls` on a Mac then shows Korean names as ????.
        // As Terminal.app does, the terminal gets a UTF-8 one when none is
        // set (one that is, here or in `env`, is left as it is).
        let lang = if cfg!(windows) { None } else { utf8_locale(|k| std::env::var(k).ok().or_else(|| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())), cfg!(target_os = "macos"), mac_locale) };
        if let Some(lang) = lang {
            cmd.env("LANG", lang);
        }
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
        Ok(Pty { master: Some(pair.master), writer, rx, child: Some(child), exited, closed: Arc::new(AtomicBool::new(false)), reported: false, opened: (cols, rows), code: None })
    }

    /// A serial port (COM3, /dev/ttyUSB0) as the terminal's other end, opened
    /// here: what is typed goes out, what comes in is shown, a lone LF as
    /// CRLF (as picocom's --imap lfcrlf). Unplugged, the pane ends.
    pub fn serial(port: &str, baud: u32, wake: impl Fn() + Send + 'static) -> Result<Pty, String> {
        let dev = serialport::new(port, baud).timeout(std::time::Duration::from_millis(100)).open().map_err(|e| serial_open_error(port, &e))?;
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
        Ok(Pty { master: None, writer: dev, rx, child: None, exited, closed, reported: true, opened: (0, 0), code: None })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if cfg!(windows) && !self.reported && has_cursor_report(bytes) {
            self.reported = true;
            // The first one answers the console host's question as it
            // starts, when its screen is still the size it was opened
            // with. A place outside that it does not take, and goes on
            // waiting for good (the terminal's own screen can be larger by
            // then: a resize is on its way).
            let fitted = fit_cursor_report(bytes, self.opened.1, self.opened.0);
            let _ = self.writer.write_all(&fitted);
            let _ = self.writer.flush();
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

    /// The process's exit code, once it has ended (None while it runs, and
    /// for a serial port). The terminal is seen to close a moment before
    /// the process's code can be read: asked for `wait`, it waits that
    /// long for it (the code was taken for none, and a Windows host's
    /// failed command went unseen).
    pub fn exit_code(&mut self, wait: std::time::Duration) -> Option<u32> {
        let until = std::time::Instant::now() + wait;
        while self.code.is_none() {
            match self.child.as_mut().map(|c| c.try_wait()) {
                Some(Ok(Some(status))) => self.code = Some(status.exit_code()),
                Some(Ok(None)) if std::time::Instant::now() < until => std::thread::sleep(std::time::Duration::from_millis(10)),
                _ => break,
            }
        }
        self.code
    }

    pub fn exited(&mut self) -> bool {
        if self.exited.load(Ordering::SeqCst) {
            return true;
        }
        self.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(Some(_))))
    }
}

/// Why a serial port did not open, in words. A port that is not there is
/// NoDevice on Windows and a missing file (NotFound) on Linux and macOS.
/// Refused: on Windows another program has it (one at a time there); on
/// Linux and macOS the user may not open it (its group, dialout or uucp),
/// which "in use" sent looking in the wrong place.
fn serial_open_error(port: &str, e: &serialport::Error) -> String {
    let denied = matches!(e.kind(), serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied)) || e.to_string().to_lowercase().contains("denied");
    match e.kind() {
        serialport::ErrorKind::NoDevice | serialport::ErrorKind::Io(std::io::ErrorKind::NotFound) => format!("{port}: no such port"),
        _ if denied && cfg!(windows) => format!("{port} is in use by another program (only one can have it open)"),
        _ if denied => format!("{port}: you may not open it: its group (ls -l {port}) is one your user is not in (Linux: sudo usermod -aG dialout $USER, then log in again)"),
        _ => format!("{port}: {e}"),
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
    // The process group the terminal's shell leads (a session of its own),
    // by the system call. It was `kill -TERM -<pid>`, and procps-ng 4.0.4
    // (Ubuntu 24.04) reads that as kill(-<first digit>): -500 went to group
    // 5, and -1036 to -1, every process of the user.
    #[cfg(not(windows))]
    {
        let Ok(group) = i32::try_from(pid) else { return false };
        // 0 is our own group, 1 init's: never.
        if group <= 1 {
            return false;
        }
        // SAFETY: kill(2) takes plain integers and touches no memory.
        unsafe { libc::kill(-group, libc::SIGTERM) == 0 }
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
            // This crate's own tests open terminals without an app around
            // them, and the process ends with the last test: each waits
            // here for its close (weft itself does once, as it ends).
            #[cfg(test)]
            {
                let until = Instant::now() + Duration::from_secs(10);
                while !closer.is_finished() && Instant::now() < until {
                    thread::sleep(Duration::from_millis(2));
                }
            }
            let mut closing = CLOSING.lock().unwrap_or_else(|e| e.into_inner());
            closing.retain(|h| !h.is_finished());
            closing.push(closer);
        }
    }
}

/// The LANG a terminal is given when nothing sets a locale (`get` reads the
/// environment): none when LC_ALL, LC_CTYPE or LANG is there. On a Mac,
/// the language and region set in System Settings (as Terminal.app does:
/// ko_KR.UTF-8), else en_US.UTF-8; elsewhere C.UTF-8. (Not asked on
/// Windows, where programs take no locale from the environment.)
pub fn utf8_locale(get: impl Fn(&str) -> Option<String>, macos: bool, apple_locale: fn() -> Option<String>) -> Option<String> {
    if ["LC_ALL", "LC_CTYPE", "LANG"].iter().any(|k| get(k).is_some_and(|v| !v.is_empty())) {
        return None;
    }
    if !macos {
        return Some("C.UTF-8".into());
    }
    // "ko_KR", "en_US@rg=krzzzz": the part before any @, letters and _ only.
    let region = apple_locale().map(|l| l.trim().split('@').next().unwrap_or("").to_string()).filter(|l| l.len() >= 2 && l.chars().all(|c| c.is_ascii_alphabetic() || c == '_'));
    Some(format!("{}.UTF-8", region.as_deref().unwrap_or("en_US")))
}

/// The Mac's language and region (`defaults read -g AppleLocale`), asked
/// once.
fn mac_locale() -> Option<String> {
    static LOCALE: OnceLock<Option<String>> = OnceLock::new();
    LOCALE
        .get_or_init(|| {
            let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLocale"]).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
        .clone()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The tests that open terminals take turns (those of other modules
    /// too): some here count this process's console hosts.
    pub(crate) static TURN: Mutex<()> = Mutex::new(());

    /// The Python the tests run: "python" on Windows ("python3" there is
    /// the Store's stand-in, which only prints "Python"); elsewhere
    /// "python3", which macOS and most Linuxes have (and often no "python").
    pub(crate) fn python() -> &'static str {
        if cfg!(windows) { "python" } else { "python3" }
    }

    /// Ending a terminal ends its process group and nothing else: two
    /// processes, each leading a group of its own; the one asked for ends,
    /// the other is untouched (procps-ng 4.0.4's `kill -TERM -<pid>` sent
    /// the signal to the group named by the pid's first digit, and to every
    /// process of the user for 1…). Groups 0 and 1 are never signalled.
    #[test]
    #[cfg(unix)]
    fn ending_a_terminal_ends_its_group_only() {
        use std::os::unix::process::CommandExt;
        let spawn = || {
            let mut c = std::process::Command::new("sleep");
            c.arg("60").process_group(0);
            // TERM as by default, whatever this process inherited (an
            // ignored signal stays ignored across exec).
            // SAFETY: signal(2) is async-signal-safe, fit for after fork.
            unsafe {
                c.pre_exec(|| {
                    libc::signal(libc::SIGTERM, libc::SIG_DFL);
                    Ok(())
                });
            }
            c.spawn().unwrap()
        };
        let (mut a, mut b) = (spawn(), spawn());
        assert!(kill_tree(a.id()), "the group was signalled");
        let ended = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            a.try_wait().unwrap().is_some()
        });
        assert!(ended, "the group asked for ended");
        assert!(b.try_wait().unwrap().is_none(), "another group is untouched");
        assert!(!kill_tree(0) && !kill_tree(1), "our own group and init's are never signalled");
        assert!(b.try_wait().unwrap().is_none(), "still untouched");
        let _ = b.kill();
        let _ = b.wait();
    }

    /// Why a serial port did not open, as each system says it: not there
    /// (Windows' NoDevice, Linux's and macOS' missing file), refused (another
    /// program on Windows; the user's groups on Linux and macOS).
    #[test]
    fn a_serial_port_that_does_not_open_says_why() {
        use serialport::{Error, ErrorKind};
        let io = |k| Error::new(ErrorKind::Io(k), "os error");
        assert_eq!(serial_open_error("COM99", &Error::new(ErrorKind::NoDevice, "x")), "COM99: no such port");
        assert_eq!(serial_open_error("/dev/ttyUSB9", &io(std::io::ErrorKind::NotFound)), "/dev/ttyUSB9: no such port");
        let refused = serial_open_error("/dev/ttyUSB0", &io(std::io::ErrorKind::PermissionDenied));
        if cfg!(windows) {
            assert!(refused.contains("in use by another program"), "{refused}");
        } else {
            assert!(refused.contains("you may not open it") && refused.contains("dialout") && !refused.contains("in use"), "{refused}");
        }
        assert_eq!(serial_open_error("COM3", &Error::new(ErrorKind::InvalidInput, "bad baud")), "COM3: bad baud");
    }

    /// The console hosts (Windows: one per pseudo console) that are this
    /// process's children.
    fn console_hosts() -> std::collections::HashSet<u32> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
        let mut sys = System::new().expect("the process list");
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
        let me = sysinfo::Pid::from_u32(std::process::id());
        sys.processes()
            .iter()
            .filter(|(_, p)| p.parent() == Some(me) && matches!(p.name().to_string_lossy().to_lowercase().as_str(), "openconsole.exe" | "conhost.exe"))
            .map(|(pid, _)| pid.as_u32())
            .collect()
    }

    /// No console host is left of terminals that were dropped: each stayed
    /// for good and kept a core busy. (Found by counting processes by hand
    /// after test runs; this counts them in the test.)
    fn no_console_host_is_left(before: &std::collections::HashSet<u32>) {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            let left: Vec<u32> = console_hosts().difference(before).copied().collect();
            if left.is_empty() {
                return;
            }
            assert!(Instant::now() < until, "console hosts left behind by dropped terminals: {left:?}");
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// A terminal that is dropped has its pseudo console closed by the time
    /// `finish_closing` returns: nothing is left for the process's end to
    /// cut short (which left a console host running for good).
    #[test]
    fn a_dropped_terminal_is_closed_before_the_end() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let hosts = console_hosts();
        let argv: Vec<String> = [python(), "-c", "import time; time.sleep(60)"].iter().map(|s| s.to_string()).collect();
        // Dropped at once, before anything it sent was answered; dropped
        // after a moment; and one that was answered as a terminal does.
        let early: Vec<Pty> = (0..2).map(|_| Pty::spawn(&argv, None, &[], 80, 24, || {}).expect("python")).collect();
        drop(early);
        let mut later: Vec<Pty> = (0..2).map(|_| Pty::spawn(&argv, None, &[], 80, 24, || {}).expect("python")).collect();
        thread::sleep(Duration::from_millis(700));
        later[0].write(b"\x1b[1;1R");
        // (Only Windows' console host asks, so only there is it noted.)
        assert_eq!((later[0].reported, later[1].reported), (cfg!(windows), false));
        drop(later);
        assert_eq!(finish_closing(Duration::from_secs(20)), 0, "every pseudo console was closed");
        assert_eq!(finish_closing(Duration::ZERO), 0, "and none is waiting");
        no_console_host_is_left(&hosts);
    }

    /// A restart starts the new weft as a child of the old one, with its
    /// terminals still open, then closes them: the new process must not
    /// hold on to them (an inherited handle of a terminal's pipes kept its
    /// console host, and the old shell, running for good). The terminals
    /// close, and their console hosts go, while the new process runs on.
    #[test]
    fn a_process_started_meanwhile_keeps_no_terminal_open() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let hosts = console_hosts();
        let argv: Vec<String> = [python(), "-c", "import time; time.sleep(60)"].iter().map(|s| s.to_string()).collect();
        let mut panes: Vec<Pty> = (0..2).map(|_| Pty::spawn(&argv, None, &[], 80, 24, || {}).expect("python")).collect();
        thread::sleep(Duration::from_millis(700));
        for p in &mut panes {
            p.write(b"\x1b[1;1R");
        }
        // As restart does: a plain spawn, the new program left running.
        let mut new_one = std::process::Command::new(python()).args(["-c", "import time; time.sleep(30)"]).spawn().expect("python");
        drop(panes);
        assert_eq!(finish_closing(Duration::from_secs(20)), 0, "every pseudo console was closed");
        no_console_host_is_left(&hosts);
        assert!(new_one.try_wait().unwrap().is_none(), "the new program still runs");
        let _ = new_one.kill();
        let _ = new_one.wait();
    }

    /// A terminal started with no locale (from the Finder or the Dock) gets
    /// a UTF-8 one, so `ls` shows Korean names rather than ????: on a Mac
    /// the one its settings name, else en_US.UTF-8; elsewhere C.UTF-8. One
    /// that is set is left alone.
    #[test]
    fn a_terminal_gets_a_utf8_locale_when_none_is_set() {
        let none = |_: &str| None;
        assert_eq!(utf8_locale(none, true, || Some("ko_KR".into())).as_deref(), Some("ko_KR.UTF-8"));
        assert_eq!(utf8_locale(none, true, || Some("en_US@rg=krzzzz".into())).as_deref(), Some("en_US.UTF-8"), "the region part is left off");
        assert_eq!(utf8_locale(none, true, || None).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(utf8_locale(none, true, || Some("not a locale!".into())).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(utf8_locale(none, false, || Some("ko_KR".into())).as_deref(), Some("C.UTF-8"), "Linux and the rest");
        for k in ["LANG", "LC_ALL", "LC_CTYPE"] {
            let set = move |n: &str| (n == k).then(|| "de_DE.UTF-8".to_string());
            assert_eq!(utf8_locale(set, true, || Some("ko_KR".into())), None, "{k} set: left alone");
        }
        let empty = |n: &str| (n == "LANG").then(String::new);
        assert!(utf8_locale(empty, true, || None).is_some(), "an empty LANG is none");
    }

    /// The cursor's place told to a console host that is just starting is
    /// kept inside the screen it was opened with: outside it, the host
    /// never took the answer, and never closed (a terminal whose screen
    /// had grown and filled before the host asked).
    #[test]
    fn the_first_cursor_report_fits_the_screen_it_was_opened_with() {
        assert_eq!(fit_cursor_report(b"\x1b[43;1R", 24, 80), b"\x1b[24;1R");
        assert_eq!(fit_cursor_report(b"\x1b[5;200R", 24, 80), b"\x1b[5;80R");
        assert_eq!(fit_cursor_report(b"\x1b[5;7R", 24, 80), b"\x1b[5;7R");
        assert_eq!(fit_cursor_report(b"ab\x1b[99;99Rcd", 24, 80), b"ab\x1b[24;80Rcd");
        assert_eq!(fit_cursor_report(b"plain \x1b[A", 24, 80), b"plain \x1b[A");
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let hosts = console_hosts();
        let argv: Vec<String> = [python(), "-c", "import time; time.sleep(60)"].iter().map(|s| s.to_string()).collect();
        let mut pty = Pty::spawn(&argv, None, &[], 80, 24, || {}).expect("python");
        thread::sleep(Duration::from_millis(700));
        // As a terminal whose screen is 43 rows by now, its cursor at the bottom.
        pty.write(b"\x1b[43;1R");
        drop(pty);
        assert_eq!(finish_closing(Duration::from_secs(20)), 0, "the console host took the answer and closed");
        no_console_host_is_left(&hosts);
    }

    /// Which conpty.dll is used: the setting's (a file or its folder), else
    /// one beside the executable, else none (Windows' own). A setting that
    /// names nothing is said, and does not stop the one beside from being
    /// used.
    #[test]
    fn the_console_host_is_the_settings_or_the_one_beside_or_windows_own() {
        let dir = std::env::temp_dir().join(format!("weft-conhost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (given, beside, empty) = (dir.join("given"), dir.join("beside"), dir.join("empty"));
        for d in [&given, &beside, &empty] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(given.join("conpty.dll"), "").unwrap();
        std::fs::write(beside.join("conpty.dll"), "").unwrap();
        let s = |p: &std::path::Path| p.display().to_string();
        // Nothing set, nothing beside: Windows' own.
        assert_eq!(console_host_file("", Some(&empty)), (None, None));
        assert_eq!(console_host_file("  ", None), (None, None));
        // Beside the executable.
        assert_eq!(console_host_file("", Some(&beside)), (Some(beside.join("conpty.dll")), None));
        // The setting wins: as a file, or as its folder.
        assert_eq!(console_host_file(&s(&given.join("conpty.dll")), Some(&beside)), (Some(given.join("conpty.dll")), None));
        assert_eq!(console_host_file(&s(&given), Some(&beside)), (Some(given.join("conpty.dll")), None));
        // A setting that names nothing: said, and the one beside is used.
        let (file, problem) = console_host_file(&s(&dir.join("nowhere")), Some(&beside));
        assert_eq!(file, Some(beside.join("conpty.dll")));
        assert!(problem.is_some_and(|p| p.contains("nowhere") && p.contains("not there")));
        let (file, problem) = console_host_file(&s(&empty), Some(&empty));
        assert!(file.is_none() && problem.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With nothing set and no conpty.dll beside the test binary, a
    /// terminal runs on Windows' own console host, whatever is on PATH
    /// (it used WezTerm's, where that is installed).
    #[test]
    #[cfg(windows)]
    fn terminals_run_on_windows_own_console_host_by_default() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let argv: Vec<String> = [python(), "-c", "import time; time.sleep(60)"].iter().map(|s| s.to_string()).collect();
        let before = console_hosts();
        let pty = Pty::spawn(&argv, None, &[], 80, 24, || {}).expect("python");
        assert_eq!(choose_console_host("ignored: it was decided already").what, "Windows' own (conhost.exe)");
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut sys = System::new().expect("the process list");
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_exe(UpdateKind::Always));
        let mine: Vec<String> = console_hosts().difference(&before).filter_map(|pid| sys.process(sysinfo::Pid::from_u32(*pid))).map(|p| p.name().to_string_lossy().to_lowercase()).collect();
        assert_eq!(mine, ["conhost.exe"], "the terminal's console host");
        drop(pty);
        assert_eq!(finish_closing(Duration::from_secs(20)), 0);
        no_console_host_is_left(&before);
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
