//! Restart into a new weft (a new build, or a newly installed one) keeping
//! the settings and the session: the tabs, splits, hosts and folders come
//! back; programs running in the terminals do not (their shells start again
//! in the same folders), and ssh passwords held in memory are asked again.
//!
//! Which weft: weft.bat passes its own path (WEFT_LAUNCHER) and where it
//! builds to (WEFT_BUILT); running the launcher again rebuilds and starts the
//! new one. Installed (install.bat), the new weft.exe is next to this one.

use crate::app::{App, ToastKind};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// Where a newer weft would appear.
pub fn update_source() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WEFT_BUILT").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(format!("weft{}", std::env::consts::EXE_SUFFIX))).filter(|p| p.is_file())
}

fn modified(p: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl App {
    /// Every 15 s: has a newer weft appeared since this one started?
    pub fn check_update(&mut self) {
        if self.update_ready {
            return;
        }
        // An idle weft draws no frames: come back for the next check.
        let every = Duration::from_secs(15);
        self.ctx.request_repaint_after(every.saturating_sub(self.update_checked.elapsed()).max(Duration::from_millis(100)));
        if self.update_checked.elapsed() < every {
            return;
        }
        self.update_checked = Instant::now();
        let newer = self.newer_weft_at.clone().or_else(update_source).and_then(|p| modified(&p)).is_some_and(|m| m > self.started_at + Duration::from_secs(2));
        if newer {
            self.update_ready = true;
            self.data.log("a newer weft is there: offering a restart");
        }
    }

    /// Save everything, start the new weft, close this one.
    pub fn restart(&mut self) {
        // Unsaved files, running commands: asked first (confirm.rs).
        if !self.may_quit(true) {
            return;
        }
        self.flush_memos();
        self.save_settings();
        let started = match std::env::var_os("WEFT_LAUNCHER").map(PathBuf::from).filter(|p| p.is_file()) {
            // weft.bat: it rebuilds, and shows the build in its own console.
            Some(bat) => {
                let mut c = std::process::Command::new("cmd");
                c.arg("/c").arg(&bat);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NEW_CONSOLE: u32 = 0x10;
                    c.creation_flags(CREATE_NEW_CONSOLE);
                }
                c.spawn().map(|_| bat.display().to_string())
            }
            None => {
                let exe = update_source().or_else(|| std::env::current_exe().ok());
                match exe {
                    Some(exe) => std::process::Command::new(&exe).spawn().map(|_| exe.display().to_string()),
                    None => Err(std::io::Error::other("no weft to start")),
                }
            }
        };
        match started {
            Ok(what) => {
                self.data.log(&format!("restart: started {what}; closing"));
                self.quit_confirmed = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => {
                self.quit_confirmed = false;
                self.toast(format!("restart failed: {e}"), ToastKind::Error)
            }
        }
    }
}
