//! X11 forwarding: GUI programs started in a remote terminal (xclock,
//! gedit, a simulator's window…) show on this PC. ssh carries them
//! (`ssh -Y`, see host.rs) to an X server here, a separate program: VcXsrv,
//! X410 or Xming on Windows (weft starts one when it is installed but not
//! running), the desktop's own elsewhere.
//!
//! The X server is started without `-ac`: its access list (X0.hosts,
//! localhost by default) lets in what ssh forwards and nobody else on the
//! network, who could otherwise watch the screen and the keys.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// The DISPLAY ssh is given: the X server on this PC, display 0 over TCP.
pub const DISPLAY: &str = "localhost:0.0";

/// Is an X server listening for display 0 here?
pub fn server_running() -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], 6000).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// An installed X server and how to start it for display 0.
fn installed() -> Option<(std::path::PathBuf, Vec<&'static str>, &'static str)> {
    let env = |k: &str| std::env::var(k).ok();
    let candidates: [(Option<String>, &str, Vec<&'static str>, &'static str); 4] = [
        (env("ProgramFiles"), "VcXsrv/vcxsrv.exe", vec![":0", "-multiwindow", "-clipboard", "-wgl"], "VcXsrv"),
        (env("ProgramFiles(x86)"), "VcXsrv/vcxsrv.exe", vec![":0", "-multiwindow", "-clipboard", "-wgl"], "VcXsrv"),
        (env("ProgramFiles(x86)"), "Xming/Xming.exe", vec![":0", "-multiwindow", "-clipboard"], "Xming"),
        (env("LOCALAPPDATA"), "Microsoft/WindowsApps/X410.exe", vec![], "X410"),
    ];
    candidates.into_iter().find_map(|(base, rel, args, name)| {
        let p = std::path::PathBuf::from(base?).join(rel);
        p.is_file().then_some((p, args, name))
    })
}

/// Make sure an X server is there for a tab with X11 forwarding: running
/// already (None), started now (its name; not waited for: the terminal
/// opens meanwhile, GUI programs need it only once they start), or what
/// to install.
pub fn ensure_server() -> Result<Option<&'static str>, String> {
    if !cfg!(windows) || server_running() {
        return Ok(None);
    }
    let Some((exe, args, name)) = installed() else {
        return Err("X11 forwarding needs an X server on this PC: install VcXsrv (winget install marha.VcXsrv) or X410, then open the tab again".into());
    };
    std::process::Command::new(&exe).args(&args).spawn().map_err(|e| format!("starting {name}: {e}"))?;
    Ok(Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_gets_y_and_display() {
        let mut p = crate::host::Profile::ssh("pi");
        assert!(!p.pty_argv(None, false).contains(&"-Y".to_string()));
        if let crate::host::HostKind::Ssh { x11, .. } = &mut p.kind {
            *x11 = true;
        }
        let argv = p.pty_argv(None, false);
        let y = argv.iter().position(|a| a == "-Y").expect("-Y");
        assert!(y < argv.iter().position(|a| a == "pi").unwrap(), "before the host: {argv:?}");
        // Older settings without the field load with it off.
        let old: crate::host::Profile = serde_json::from_str(r#"{"name":"pi","kind":"ssh","target":"pi"}"#).unwrap();
        assert!(matches!(old.kind, crate::host::HostKind::Ssh { x11: false, .. }));
    }

    #[test]
    fn a_listening_port_counts_as_a_server() {
        // Only checks the probe: something on 6000 (a real server, or ours).
        if let Ok(l) = std::net::TcpListener::bind("127.0.0.1:6000") {
            assert!(server_running());
            drop(l);
        } else {
            assert!(server_running(), "6000 is taken: by an X server, presumably");
        }
    }
}
