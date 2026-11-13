//! Hosts and connections.
//!
//! weft is a thick local client.  For a remote host it opens
//!   * one `ssh -tt` PTY per pane running bash with weft's integration rc, and
//!   * one shared `ssh -T` side channel running a tiny python agent,
//! and nothing is ever copied to the remote disk: the rc file arrives through
//! bash process substitution, the agent source through python's stdin.
//! The local host (Windows here) needs no agent; the same JSON operations are
//! served directly by Rust.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::UNIX_EPOCH;

/// Shell integrations (prompt and command marks, current directory), one per
/// shell. They travel to a remote host as one gzipped bundle in memory.
pub const BASH_SH: &str = include_str!("../shell/bash.sh");
pub const ZSH_ZSH: &str = include_str!("../shell/zsh.zsh");
pub const FISH_FISH: &str = include_str!("../shell/fish.fish");
pub const PWSH_PS1: &str = include_str!("../shell/pwsh.ps1");

/// All integrations in one text: sections start with `#@<name>` lines; the
/// bootstrap cuts out the one it needs with awk. Comment and blank lines are
/// left out (the command line that carries it is limited to about 8 KB).
pub fn integration_bundle() -> String {
    let mut out = String::new();
    for (name, src) in [("boot", SHELL_BOOT), ("bash", BASH_SH), ("zsh", ZSH_ZSH), ("fish", FISH_FISH), ("pwsh", PWSH_PS1)] {
        out.push_str(&format!("#@{name}\n{}", strip_comments(src)));
    }
    out
}

/// Without whole-line comments and blank lines (`#!` lines stay).
fn strip_comments(src: &str) -> String {
    src.lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.is_empty() && !(t.starts_with('#') && !t.starts_with("#!"))
        })
        .map(|l| format!("{l}\n"))
        .collect()
}
pub const AGENT_PY: &str = include_str!("../remote/agent.py");

/// Pane bootstrap (one logical line, run via `eval` by sh): pick a base64
/// decoder from POSIX tools (or python), then start the user's login shell
/// ($SHELL, or WEFT_SHELL) with its integration from the bundle in $W:
/// bash `--rcfile <(...)`, fish `-C`, pwsh `-Command`, nushell's own OSC 133
/// switched on, zsh through WEFT_ZSH typed in by weft (see shell/zsh.zsh).
/// Any other shell starts as is; weft guesses its command blocks.
const SHELL_BOOT: &str = r##"WP=${WEFT_SHELL:-$SHELL}; WS=${WP##*/}; WS=${WS%.exe}; export WEFT=1; case "$WS" in bash|'') if command -v bash >/dev/null 2>&1; then R=$(sect bash); export R D WEFT_CWD; exec bash -c 'exec bash --rcfile <(printf %s "$R") -i'; fi ;; esac; if [ -n "$WEFT_CWD" ]; then C=$(printf %s "$WEFT_CWD" | $D 2>/dev/null) && [ -d "$C" ] && cd "$C"; fi; unset WEFT_CWD; printf '\033]7777;{"t":"shell","name":"%s","pid":%s}\007' "$WS" "$$"; case "$WS" in zsh) WEFT_ZSH=$(sect zsh); export WEFT_ZSH; printf ']7777;{"t":"inject","shell":"zsh"}'; exec "$WP" -l -i ;; fish) WEFT_FISH=$(sect fish); export WEFT_FISH; exec "$WP" -l -C 'eval $WEFT_FISH' ;; nu) exec "$WP" -l -e 'try { $env.config.shell_integration.osc133 = true; $env.config.shell_integration.osc7 = true } catch { $env.config.shell_integration = true }' ;; pwsh|powershell) WEFT_PS=$(sect pwsh); export WEFT_PS; exec "$WP" -NoLogo -NoExit -Command '. ([ScriptBlock]::Create($env:WEFT_PS))' ;; esac; if [ -n "$WP" ]; then exec "$WP" -i; fi; exec sh -i"##;

/// On the command line: find a base64 decoder and gunzip among POSIX tools (or
/// python), unpack the bundle in $W and run its `boot` section.
const SHELL_UNPACK: &str = r##"D="base64 -d"; printf QQ== | $D >/dev/null 2>&1 || D="base64 -D"; printf QQ== | $D >/dev/null 2>&1 || D="python3 -m base64 -d"; z() { if command -v gzip >/dev/null 2>&1; then gzip -dc; else python3 -c "import sys,gzip;sys.stdout.buffer.write(gzip.decompress(sys.stdin.buffer.read()))"; fi; }; WB=$(printf %s "$W" | $D | z); sect() { printf %s "$WB" | awk -v s="#@$1" '$0==s{f=1;next} /^#@/{f=0} f'; }; eval "$(sect boot)""##;

/// Windows ConPTY truncates command lines around 8 KB, so the rc travels gzipped.
fn gz_b64(s: &str) -> String {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(s.as_bytes()).expect("gzip to memory");
    base64::engine::general_purpose::STANDARD.encode(e.finish().expect("gzip to memory"))
}

/// Agent bootstrap: read `<len>\n<source>` from stdin and exec it in memory.
const AGENT_BOOT: &str = r#"P=python3; $P -c "" >/dev/null 2>&1 || P=python; exec $P -u -c "import sys;exec(sys.stdin.buffer.read(int(sys.stdin.buffer.readline())))""#;

pub fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

/// Wrap a POSIX script so the command string contains no `"` or `\`: it
/// survives any login shell (sh/bash/zsh/fish) and any Windows argv parser
/// (MSVCRT ssh.exe, msys/cygwin ssh and bash), which re-quote differently.
fn posix_wrap(vars: &[(&str, String)], script: &str) -> String {
    let mut s = String::from("exec sh -c '");
    for (k, v) in vars {
        s.push_str(&format!("{k}={v}; "));
    }
    let b = b64(script);
    s.push_str(&format!(
        "B={b}; set -f; S=$(printf %s $B | base64 -d 2>/dev/null) || S=$(printf %s $B | base64 -D 2>/dev/null) || S=$(printf %s $B | python3 -m base64 -d); eval $S'"
    ));
    s
}

pub fn remote_shell_command(cwd: Option<&str>) -> String {
    posix_wrap(&[("W", gz_b64(&integration_bundle())), ("WEFT_CWD", cwd.map(b64).unwrap_or_default())], SHELL_UNPACK)
}

pub fn agent_command() -> String {
    posix_wrap(&[], AGENT_BOOT)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum HostKind {
    Local,
    Ssh {
        target: String,
        #[serde(default)]
        args: Vec<String>,
        /// X11 forwarding: GUI programs run there show here (see x11.rs).
        #[serde(default)]
        x11: bool,
    },
    /// Any command that behaves like `ssh HOST CMD` (receives the remote
    /// command string as its last argument), e.g. `["wsl", "-e", "sh", "-c"]`.
    Custom { pty: Vec<String>, agent: Vec<String> },
    /// A serial console (UART): `port` on the host named `via` ("local": a
    /// COM port opened by weft; a remote host: remote/serial.py run there).
    Serial { via: String, port: String, baud: u32 },
}

pub const SERIAL_PY: &str = include_str!("../remote/serial.py");

/// Speeds offered for serial consoles.
pub const BAUDS: &[u32] = &[9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1000000, 1500000, 3000000];

/// The remote command for a serial console: the bridge, sent in the command
/// itself (base64) so nothing is written on the host.
pub fn serial_command(port: &str, baud: u32) -> String {
    let src = base64::engine::general_purpose::STANDARD.encode(SERIAL_PY);
    format!(
        "P=$(command -v python3 || command -v python); exec \"$P\" -c 'import base64;exec(base64.b64decode(\"{src}\"))' {} {baud}",
        sh_quote(port)
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    #[serde(flatten)]
    pub kind: HostKind,
}

impl Profile {
    pub fn local() -> Profile {
        Profile { name: "local".into(), kind: HostKind::Local }
    }

    pub fn ssh(target: &str) -> Profile {
        let mut parts = target.split_whitespace().map(String::from);
        let host = parts.next().unwrap_or_default();
        Profile { name: host.clone(), kind: HostKind::Ssh { target: host, args: parts.collect(), x11: false } }
    }

    pub fn is_local(&self) -> bool {
        matches!(self.kind, HostKind::Local)
    }

    /// argv for an interactive pane.  With `password` the ssh password comes
    /// from weft's askpass (see `askpass_env`) instead of the terminal.
    pub fn pty_argv(&self, cwd: Option<&str>, password: bool) -> Vec<String> {
        match &self.kind {
            HostKind::Local => local_shell(LocalShell::Auto).0,
            HostKind::Ssh { target, args, x11 } => {
                let mut v = vec!["ssh".to_string(), "-tt".to_string()];
                if password {
                    v.extend(["-o".to_string(), "NumberOfPasswordPrompts=1".to_string()]);
                }
                // GUI programs show here (trusted forwarding: with -X many fail).
                if *x11 {
                    v.push("-Y".to_string());
                }
                v.extend(args.iter().cloned());
                v.push(target.clone());
                v.push(remote_shell_command(cwd));
                v
            }
            HostKind::Custom { pty, .. } => {
                let mut v = pty.clone();
                v.push(remote_shell_command(cwd));
                v
            }
            // Built from the host it is reached through (App::start_pty).
            HostKind::Serial { .. } => Vec::new(),
        }
    }

    /// argv for a serial console on a remote host: this host's terminal
    /// command with the bridge instead of a shell.
    pub fn serial_argv(&self, port: &str, baud: u32, password: bool) -> Vec<String> {
        let cmd = serial_command(port, baud);
        match &self.kind {
            HostKind::Ssh { target, args, .. } => {
                let mut v = vec!["ssh".to_string(), "-tt".to_string()];
                if password {
                    v.extend(["-o".to_string(), "NumberOfPasswordPrompts=1".to_string()]);
                }
                v.extend(args.iter().cloned());
                v.push(target.clone());
                v.push(cmd);
                v
            }
            HostKind::Custom { pty, .. } => {
                let mut v = pty.clone();
                v.push(cmd);
                v
            }
            HostKind::Local | HostKind::Serial { .. } => Vec::new(),
        }
    }

    /// argv for the agent side channel (None: served locally).  Without a
    /// password it never prompts (keys / ssh-agent only); with one, the
    /// password is answered by weft's askpass.
    pub fn agent_argv(&self, password: bool) -> Option<Vec<String>> {
        match &self.kind {
            // A serial console has no file system of its own: served locally.
            HostKind::Local | HostKind::Serial { .. } => None,
            HostKind::Ssh { target, args, .. } => {
                let auth = if password { "NumberOfPasswordPrompts=1" } else { "BatchMode=yes" };
                let mut v: Vec<String> = ["ssh", "-T", "-o", auth, "-o", "ConnectTimeout=10"].iter().map(|s| s.to_string()).collect();
                v.extend(args.iter().cloned());
                v.push(target.clone());
                v.push(agent_command());
                Some(v)
            }
            HostKind::Custom { agent, .. } => {
                let mut v = agent.clone();
                v.push(agent_command());
                Some(v)
            }
        }
    }

    /// Shell command that changes directory, quoted for this host's shell.
    pub fn cd_command(&self, path: &str) -> String {
        if self.is_local() {
            format!("Set-Location -LiteralPath '{}'", path.replace('\'', "''"))
        } else {
            format!("cd -- {}", sh_quote(path))
        }
    }

    pub fn quote_path(&self, path: &str) -> String {
        if self.is_local() {
            if path.contains([' ', '\'', '(', ')', '&', ';']) {
                format!("'{}'", path.replace('\'', "''"))
            } else {
                path.to_string()
            }
        } else if path.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+~:@%".contains(c)) {
            path.to_string()
        } else {
            sh_quote(path)
        }
    }
}

/// Environment that makes ssh ask weft (`weft --askpass`) instead of a
/// terminal.  The secret only lives in this process environment: it is never
/// written to settings or logs.  `DISPLAY` lets OpenSSH < 8.4 use askpass too.
pub fn askpass_env(secret: &str) -> Vec<(String, String)> {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "weft".into());
    vec![
        ("SSH_ASKPASS".into(), exe),
        ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
        ("DISPLAY".into(), std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into())),
    ]
    .into_iter()
    // Not the password itself: a one-time ticket for it (see secret.rs).
    .chain(crate::secret::ticket_env(secret))
    .collect()
}

/// What `weft --askpass PROMPT` answers: host key questions go to the user
/// (never auto-accepted), password/passphrase prompts get the secret.
pub enum AskpassAnswer {
    ConfirmHostKey,
    Secret(String),
    Refuse,
}

pub fn askpass_answer(prompt: &str, secret: Option<String>) -> AskpassAnswer {
    if prompt.contains("(yes/no") {
        return AskpassAnswer::ConfirmHostKey;
    }
    match secret {
        Some(s) if !s.is_empty() => AskpassAnswer::Secret(s),
        _ => AskpassAnswer::Refuse,
    }
}

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The local shell for new panes (settings "local_shell").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalShell {
    /// pwsh if installed, else Windows PowerShell.
    #[default]
    Auto,
    Pwsh,
    Powershell,
    Cmd,
    Nu,
}

impl LocalShell {
    pub fn label(self) -> &'static str {
        match self {
            LocalShell::Auto => "auto (pwsh, else Windows PowerShell)",
            LocalShell::Pwsh => "PowerShell 7 (pwsh)",
            LocalShell::Powershell => "Windows PowerShell 5.1",
            LocalShell::Cmd => "cmd",
            LocalShell::Nu => "nushell",
        }
    }
}

/// cmd has no hooks, but its PROMPT can carry escapes ($E): mark the prompt
/// (133 A/B), end the previous command (a bare 133;D: cmd can't report exit
/// codes) and report the directory ($P) as OSC 9;9. Enter starts a command.
fn cmd_prompt() -> String {
    let user = std::env::var("PROMPT").ok().filter(|p| !p.is_empty() && !p.contains("133;")).unwrap_or_else(|| "$P$G".into());
    format!("$E]133;D$E\\$E]9;9;$P$E\\$E]133;A$E\\{user}$E]133;B$E\\")
}

/// argv and extra environment for a local pane's shell.
pub fn local_shell(choice: LocalShell) -> (Vec<String>, Vec<(String, String)>) {
    if !cfg!(windows) {
        return (vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()), "-i".into()], Vec::new());
    }
    match choice {
        LocalShell::Cmd => (vec!["cmd.exe".into(), "/k".into(), "chcp 65001 >nul".into()], vec![("PROMPT".into(), cmd_prompt())]),
        LocalShell::Nu if which("nu.exe").is_some() => (
            vec!["nu.exe".into(), "-e".into(), "try { $env.config.shell_integration.osc133 = true; $env.config.shell_integration.osc7 = true } catch { $env.config.shell_integration = true }".into()],
            Vec::new(),
        ),
        LocalShell::Powershell => (powershell_argv("powershell.exe"), Vec::new()),
        _ => (local_shell_argv(), Vec::new()),
    }
}

fn powershell_argv(exe: &str) -> Vec<String> {
    let utf16: Vec<u8> = PWSH_PS1.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    vec![exe.into(), "-NoLogo".into(), "-NoExit".into(), "-EncodedCommand".into(), encoded]
}

fn local_shell_argv() -> Vec<String> {
    let init = PWSH_PS1;
    let utf16: Vec<u8> = init.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let shell = if cfg!(windows) {
        if which("pwsh.exe").is_some() {
            "pwsh.exe"
        } else {
            "powershell.exe"
        }
    } else {
        return vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()), "-i".into()];
    };
    vec![shell.into(), "-NoLogo".into(), "-NoExit".into(), "-EncodedCommand".into(), encoded]
}

pub fn which(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(exe)).find(|p| p.is_file()))
}

/// `Host` entries (no wildcards) from ~/.ssh/config.
pub fn ssh_config_hosts() -> Vec<String> {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let Ok(text) = std::fs::read_to_string(Path::new(&home).join(".ssh").join("config")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let mut it = line.splitn(2, char::is_whitespace);
        if it.next().map(|k| k.eq_ignore_ascii_case("host")).unwrap_or(false) {
            for h in it.next().unwrap_or("").split_whitespace() {
                if !h.contains(['*', '?', '!']) && !out.iter().any(|o| o == h) {
                    out.push(h.to_string());
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Data directory (next to the executable) and logging
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DataDir {
    pub root: PathBuf,
}

impl DataDir {
    pub fn locate() -> DataDir {
        let root = std::env::var_os("WEFT_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("data")))
                .unwrap_or_else(|| PathBuf::from("data"))
        });
        for sub in ["", "logs", "history"] {
            let _ = std::fs::create_dir_all(root.join(sub));
        }
        DataDir { root }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn open_log(&self, name: &str) -> Option<File> {
        OpenOptions::new().create(true).append(true).open(self.root.join("logs").join(name)).ok()
    }

    /// weft's own log, one file a day (logs/weft-<date>.log); nothing when
    /// switched off in the records settings.
    pub fn log(&self, line: &str) {
        if !crate::records::app_log_on() {
            return;
        }
        if let Some(mut f) = self.open_log(&format!("weft-{}.log", crate::records::today())) {
            let _ = writeln!(f, "{} {}", timestamp(), line);
        }
    }
}

/// An agent log's file name for a host and day.
pub fn agent_log_name(host: &str, day: &str) -> String {
    format!("agent-{}-{day}.log", safe_name(host))
}

pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    format!("{secs:.3}")
}

pub fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect()
}

fn truncate_for_log(s: &str) -> String {
    if s.len() <= 1500 {
        return s.to_string();
    }
    let mut end = 1500;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…(+{} bytes)", &s[..end], s.len() - end)
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum ConnStatus {
    Connecting,
    Ready,
    Failed(String),
}

enum AgentMsg {
    Line(Value),
    Stderr(String),
    Exit,
}

enum Backend {
    Local { queue: VecDeque<Value> },
    Remote { stdin: Option<ChildStdin>, rx: Receiver<AgentMsg>, child: Child },
    Dead,
}

pub struct Conn {
    pub profile: Profile,
    pub status: ConnStatus,
    pub info: Value,
    next_id: u64,
    backend: Backend,
    /// logs/agent-<host>-<date>.log, opened for the day it is.
    log_dir: PathBuf,
    log: Option<(String, File)>,
    stderr_tail: String,
    /// Failures for requests that could not even be sent; delivered by `poll`.
    backend_failures: Vec<Value>,
}

impl Conn {
    pub fn open(profile: Profile, data: &DataDir, wake: impl Fn() + Send + Clone + 'static, password: Option<String>) -> Conn {
        let mut conn = Conn {
            log_dir: data.path("logs"),
            log: None,
            profile,
            status: ConnStatus::Connecting,
            info: Value::Null,
            next_id: 1,
            backend: Backend::Dead,
            stderr_tail: String::new(),
            backend_failures: Vec::new(),
        };
        conn.connect(wake, password);
        conn
    }

    /// (Re)start the agent.  `password`: answer ssh's password prompt with
    /// it; `None`: keys / ssh-agent only, never prompt.
    pub fn connect(&mut self, wake: impl Fn() + Send + Clone + 'static, password: Option<String>) {
        self.stderr_tail.clear();
        let env = password.as_deref().map(askpass_env).unwrap_or_default();
        match self.profile.agent_argv(password.is_some()) {
            None => {
                self.status = ConnStatus::Ready;
                self.info = local_hello();
                self.backend = Backend::Local { queue: VecDeque::new() };
                self.write_log("local backend ready");
            }
            Some(argv) => {
                self.status = ConnStatus::Connecting;
                let auth = if env.is_empty() { "keys/ssh-agent" } else { "password via askpass" };
                self.write_log(&format!("spawn agent ({auth}): {}", truncate_for_log(&argv.join(" "))));
                match spawn_agent(&argv, &env, wake) {
                    Ok(b) => self.backend = b,
                    Err(e) => {
                        self.write_log(&format!("spawn failed: {e}"));
                        self.status = ConnStatus::Failed(e);
                        self.backend = Backend::Dead;
                    }
                }
            }
        }
    }

    fn write_log(&mut self, line: &str) {
        if !crate::records::agent_log_on() {
            return;
        }
        let day = crate::records::today();
        if self.log.as_ref().map(|l| l.0 != day).unwrap_or(true) {
            let path = self.log_dir.join(agent_log_name(&self.profile.name, &day));
            self.log = OpenOptions::new().create(true).append(true).open(path).ok().map(|f| (day, f));
        }
        if let Some((_, f)) = self.log.as_mut() {
            let _ = writeln!(f, "{} {}", timestamp(), line);
        }
    }

    /// Send a request; the response arrives later from `poll` with the same id.
    pub fn request(&mut self, mut req: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        req["id"] = json!(id);
        let line = req.to_string();
        self.write_log(&format!("-> {}", truncate_for_log(&line)));
        match &mut self.backend {
            Backend::Local { queue } => {
                let mut res = local_op(&req);
                res["id"] = json!(id);
                queue.push_back(res);
            }
            Backend::Remote { stdin: Some(stdin), .. } => {
                if stdin.write_all(line.as_bytes()).and_then(|_| stdin.write_all(b"\n")).and_then(|_| stdin.flush()).is_err()
                {
                    self.status = ConnStatus::Failed("agent pipe closed".into());
                }
            }
            _ => {
                let err = match &self.status {
                    ConnStatus::Failed(e) => format!("agent offline: {e}"),
                    _ => "agent not connected yet".into(),
                };
                self.backend_failures.push(json!({"id": id, "ok": false, "error": err}));
            }
        }
        id
    }

    /// Responses that arrived since the last call.
    pub fn poll(&mut self) -> Vec<Value> {
        let mut out: Vec<Value> = std::mem::take(&mut self.backend_failures);
        let mut logs = Vec::new();
        let mut exited = false;
        match &mut self.backend {
            Backend::Local { queue } => out.extend(queue.drain(..)),
            Backend::Remote { rx, .. } => {
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        AgentMsg::Line(v) => {
                            logs.push(format!("<- {}", truncate_for_log(&v.to_string())));
                            if v.get("event").and_then(|e| e.as_str()) == Some("ready") {
                                self.status = ConnStatus::Ready;
                                self.info = v;
                            } else {
                                out.push(v);
                            }
                        }
                        AgentMsg::Stderr(s) => {
                            logs.push(format!("stderr: {s}"));
                            self.stderr_tail = s;
                        }
                        AgentMsg::Exit => exited = true,
                    }
                }
            }
            Backend::Dead => {}
        }
        for l in logs {
            self.write_log(&l);
        }
        if exited {
            let reason = if self.stderr_tail.is_empty() { "agent exited".to_string() } else { self.stderr_tail.clone() };
            self.write_log(&format!("agent exited: {reason}"));
            self.status = ConnStatus::Failed(reason);
            self.backend = Backend::Dead;
        }
        out
    }

    pub fn home(&self) -> Option<String> {
        self.info.get("home").and_then(|v| v.as_str()).map(|s| s.replace('\\', "/"))
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        if let Backend::Remote { child, stdin, .. } = &mut self.backend {
            stdin.take();
            if !crate::pty::kill_tree(child.id()) {
                let _ = child.kill();
            }
        }
    }
}

fn spawn_agent(argv: &[String], env: &[(String, String)], wake: impl Fn() + Send + Clone + 'static) -> Result<Backend, String> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).envs(env.iter().cloned()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", argv[0]))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    // Ship the agent source through stdin: it is exec'd in memory remotely.
    let header = format!("{}\n", AGENT_PY.len());
    stdin
        .write_all(header.as_bytes())
        .and_then(|_| stdin.write_all(AGENT_PY.as_bytes()))
        .and_then(|_| stdin.flush())
        .map_err(|e| format!("send agent: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    let wake2 = wake.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(AgentMsg::Line(v));
                wake();
            }
        }
        let _ = tx.send(AgentMsg::Exit);
        wake();
    });
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if !line.trim().is_empty() {
                let _ = tx2.send(AgentMsg::Stderr(line));
                wake2();
            }
        }
    });
    Ok(Backend::Remote { stdin: Some(stdin), rx, child })
}

// ---------------------------------------------------------------------------
// Local backend: the agent protocol served with std::fs
// ---------------------------------------------------------------------------

fn local_hello() -> Value {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    json!({
        "ok": true,
        "home": home.replace('\\', "/"),
        "cwd": std::env::current_dir().map(|p| p.display().to_string().replace('\\', "/")).unwrap_or_default(),
        "host": std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into()),
        "user": std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default(),
        "platform": std::env::consts::OS,
    })
}

pub fn mtime_of(md: &std::fs::Metadata) -> f64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Whether a directory entry stays out of listings. Dot names and, on Windows,
/// the hidden attribute show only with `show_hidden`; hidden+system entries
/// never show, as in Explorer. Those are OS files and the compatibility links
/// like "My Documents" that refuse to be listed.
fn is_hidden(e: &std::fs::DirEntry, name: &str, show_hidden: bool) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        let attrs = e.metadata().map(|m| m.file_attributes()).unwrap_or(0);
        if attrs & (HIDDEN | SYSTEM) == HIDDEN | SYSTEM {
            return true;
        }
        if !show_hidden && attrs & HIDDEN != 0 {
            return true;
        }
    }
    #[cfg(not(windows))]
    let _ = e;
    !show_hidden && name.starts_with('.')
}

fn local_op(req: &Value) -> Value {
    let s = |k: &str| req.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let result: Result<Value, String> = (|| match req.get("op").and_then(|v| v.as_str()).unwrap_or("") {
        "hello" => Ok(local_hello()),
        "list" => {
            let dir = s("path");
            let hidden = req.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut entries = Vec::new();
            for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if is_hidden(&e, &name, hidden) {
                    continue;
                }
                let link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
                let md = std::fs::metadata(e.path()).ok();
                let d = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = md.as_ref().map(|m| if m.is_dir() { 0 } else { m.len() }).unwrap_or(0);
                let m = md.as_ref().map(mtime_of).unwrap_or(0.0) as i64;
                entries.push(json!({"n": name, "d": d, "l": link, "s": size, "m": m}));
            }
            if req.get("ignored").and_then(|v| v.as_bool()) == Some(false) {
                let names: Vec<String> = entries.iter().filter_map(|e| e["n"].as_str().map(String::from)).collect();
                let drop = git_ignored(&dir, &names);
                entries.retain(|e| !e["n"].as_str().is_some_and(|n| drop.contains(n)));
            }
            Ok(json!({"path": dir, "entries": entries}))
        }
        "stat" => {
            let p = s("path");
            Ok(match std::fs::metadata(&p) {
                Ok(md) => json!({"path": p, "exists": true, "dir": md.is_dir(), "size": md.len(), "mtime": mtime_of(&md)}),
                Err(_) => json!({"path": p, "exists": false}),
            })
        }
        "read" => {
            let p = s("path");
            let md = std::fs::metadata(&p).map_err(|e| e.to_string())?;
            if md.is_dir() {
                return Err("IsADirectoryError: is a directory".into());
            }
            let max = req.get("max").and_then(|v| v.as_u64()).unwrap_or(8 * 1024 * 1024);
            if md.len() > max {
                return Err(format!("file too large ({} bytes)", md.len()));
            }
            let raw = std::fs::read(&p).map_err(|e| e.to_string())?;
            let mut v = json!({"path": p, "size": raw.len(), "mtime": mtime_of(&md)});
            match String::from_utf8(raw) {
                Ok(t) => v["text"] = json!(t),
                Err(e) => v["b64"] = json!(base64::engine::general_purpose::STANDARD.encode(e.into_bytes())),
            }
            Ok(v)
        }
        "write" => {
            let p = s("path");
            if let (Some(expect), false) =
                (req.get("expect_mtime").and_then(|v| v.as_f64()), req.get("force").and_then(|v| v.as_bool()).unwrap_or(false))
            {
                if let Ok(md) = std::fs::metadata(&p) {
                    let cur = mtime_of(&md);
                    if (cur - expect).abs() > 1e-3 {
                        return Ok(json!({"path": p, "conflict": true, "mtime": cur}));
                    }
                }
            }
            let data = match req.get("b64").and_then(|v| v.as_str()) {
                Some(b) => base64::engine::general_purpose::STANDARD.decode(b).map_err(|e| e.to_string())?,
                None => s("text").into_bytes(),
            };
            std::fs::write(&p, &data).map_err(|e| e.to_string())?;
            let md = std::fs::metadata(&p).map_err(|e| e.to_string())?;
            Ok(json!({"path": p, "size": data.len(), "mtime": mtime_of(&md)}))
        }
        "complete" => {
            let dir = s("dir");
            let prefix = s("prefix");
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .map_err(|e| e.to_string())?
                .flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if !n.starts_with(&prefix) {
                        return None;
                    }
                    let isdir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    Some(if isdir { format!("{n}/") } else { n })
                })
                .collect();
            names.sort();
            Ok(json!({"dir": dir, "prefix": prefix, "names": names}))
        }
        "grep" => {
            let flag = |k: &str| req.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
            local_grep(&s("path"), &s("pattern"), flag("icase"), flag("regex"), flag("hidden"))
        }
        "tree" => Ok(local_tree(&s("path"), req.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false), req.get("max").and_then(|v| v.as_u64()).unwrap_or(20000) as usize)),
        // File transfer: the same ops as agent.py (see there).
        "walk" => local_walk(&s("path")),
        "rchunk" => {
            use std::io::{Read, Seek};
            let off = req.get("off").and_then(|v| v.as_u64()).unwrap_or(0);
            let n = req.get("len").and_then(|v| v.as_u64()).unwrap_or(MAX_CHUNK).min(MAX_CHUNK);
            let mut f = File::open(s("path")).map_err(|e| e.to_string())?;
            let size = f.metadata().map_err(|e| e.to_string())?.len();
            f.seek(std::io::SeekFrom::Start(off)).map_err(|e| e.to_string())?;
            let mut raw = Vec::with_capacity(n as usize);
            f.take(n).read_to_end(&mut raw).map_err(|e| e.to_string())?;
            let got = raw.len() as u64;
            Ok(json!({"b64": base64::engine::general_purpose::STANDARD.encode(raw), "n": got, "eof": off + got >= size, "size": size}))
        }
        "wchunk" => {
            use std::io::{Seek, Write};
            let off = req.get("off").and_then(|v| v.as_u64()).unwrap_or(0);
            let data = base64::engine::general_purpose::STANDARD.decode(s("b64")).map_err(|e| e.to_string())?;
            let mut f = if off > 0 { OpenOptions::new().write(true).open(s("path")) } else { File::create(s("path")) }.map_err(|e| e.to_string())?;
            f.seek(std::io::SeekFrom::Start(off)).map_err(|e| e.to_string())?;
            f.write_all(&data).map_err(|e| e.to_string())?;
            Ok(json!({"n": data.len()}))
        }
        "finish" => {
            if req.get("abort").and_then(|v| v.as_bool()).unwrap_or(false) {
                let _ = std::fs::remove_file(s("part"));
            } else {
                std::fs::rename(s("part"), s("path")).map_err(|e| e.to_string())?;
            }
            Ok(json!({}))
        }
        "mkdirs" => {
            let root = s("root");
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            for d in req.get("dirs").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|d| d.as_str()) {
                std::fs::create_dir_all(format!("{root}/{d}")).map_err(|e| e.to_string())?;
            }
            Ok(json!({}))
        }
        op => Err(format!("'{op}' is only available on remote hosts")),
    })();
    match result {
        Ok(mut v) => {
            v["ok"] = json!(true);
            v
        }
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// The names in folder `dir` that git ignores (.gitignore and friends), as
/// agent.py does; none outside a repository or without git.
fn git_ignored(dir: &str, names: &[String]) -> std::collections::HashSet<String> {
    use std::io::Write as _;
    if names.is_empty() {
        return Default::default();
    }
    let mut cmd = std::process::Command::new("git");
    cmd.args(["-C", dir, "check-ignore", "-z", "--stdin"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    let Ok(mut child) = cmd.spawn() else { return Default::default() };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(names.join("\0").as_bytes());
    }
    let Ok(out) = child.wait_with_output() else { return Default::default() };
    // 0: some ignored, 1: none, 128: not a repository.
    if out.status.code() != Some(0) {
        return Default::default();
    }
    String::from_utf8_lossy(&out.stdout).split('\0').filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Largest chunk a transfer reads at once (agent.py has the same).
pub const MAX_CHUNK: u64 = 4 * 1024 * 1024;

/// What a transfer source holds: one file, or a folder's subfolders and
/// files (paths relative to it, '/'-separated, sorted).
fn local_walk(path: &str) -> Result<Value, String> {
    let md = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !md.is_dir() {
        return Ok(json!({"dir": false, "items": [{"p": "", "s": md.len(), "d": false}]}));
    }
    let mut items = Vec::new();
    fn walk(dir: &std::path::Path, rel: &str, items: &mut Vec<Value>) -> Result<(), String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        let (dirs, files): (Vec<_>, Vec<_>) = entries.into_iter().partition(|e| e.metadata().map(|m| m.is_dir()).unwrap_or(false));
        for f in files {
            let Ok(md) = f.metadata() else { continue };
            let name = f.file_name().to_string_lossy().into_owned();
            items.push(json!({"p": if rel.is_empty() { name } else { format!("{rel}/{name}") }, "s": md.len(), "d": false}));
        }
        for d in dirs {
            let name = d.file_name().to_string_lossy().into_owned();
            let sub = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            items.push(json!({"p": sub, "s": 0, "d": true}));
            walk(&d.path(), &sub, items)?;
        }
        Ok(())
    }
    walk(std::path::Path::new(path), "", &mut items)?;
    Ok(json!({"dir": true, "items": items}))
}

/// Folders listed but not descended into by "tree" (same set as agent.py).
const TREE_SKIP: &[&str] = &[".git", "node_modules", "target", "__pycache__", ".venv", "venv", ".cache", ".tox", "dist", "build"];

/// Every name under `root` as [relative path, is dir], for the file filter.
fn local_tree(root: &str, hidden: bool, limit: usize) -> Value {
    let mut out = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        let dir = if rel.is_empty() { PathBuf::from(root) } else { Path::new(root).join(&rel) };
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_hidden(&e, &name, hidden) {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            out.push(json!([r, is_dir]));
            if out.len() >= limit {
                return json!({"root": root, "entries": out, "truncated": true});
            }
            if is_dir && !TREE_SKIP.contains(&name.as_str()) {
                stack.push(r);
            }
        }
    }
    json!({"root": root, "entries": out, "truncated": false})
}

/// Search file contents under `root`: the text as typed, or with `regex` a
/// regular expression (the extended syntax the agent's `grep -E` takes).
fn local_grep(root: &str, pattern: &str, icase: bool, regex: bool, hidden: bool) -> Result<Value, String> {
    let pat = if regex { pattern.to_string() } else { regex::escape(pattern) };
    let re = regex::RegexBuilder::new(&pat).case_insensitive(icase).build().map_err(|e| format!("bad regex: {e}"))?;
    let mut hits = Vec::new();
    let mut stack = vec![PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if !hidden && (name.starts_with('.') || is_hidden(&e, &name, false)) {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if ![".git", "node_modules", "target", "__pycache__"].contains(&name.as_str()) {
                    stack.push(p);
                }
                continue;
            }
            if e.metadata().map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(true) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else { continue };
            if bytes.iter().take(4096).any(|&b| b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    let t: String = line.chars().take(400).collect();
                    hits.push(json!([p.display().to_string().replace('\\', "/"), i + 1, t]));
                    if hits.len() >= 500 {
                        return Ok(json!({"root": root, "pattern": pattern, "hits": hits, "truncated": true}));
                    }
                }
            }
        }
    }
    Ok(json!({"root": root, "pattern": pattern, "hits": hits, "truncated": false}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_grep_modes_and_hidden() {
        use crate::editfind::{to_regex, MatchMode};
        let d = std::env::temp_dir().join(format!("weft-grep-modes-{}", std::process::id()));
        std::fs::create_dir_all(d.join(".hid")).unwrap();
        std::fs::write(d.join("a.txt"), "git status\nvoid Start(int x);\nstash it\n").unwrap();
        std::fs::write(d.join(".env"), "git status hidden\n").unwrap();
        std::fs::write(d.join(".hid").join("b.txt"), "git status in hidden dir\n").unwrap();
        let root = d.display().to_string();
        // As the Search tab sends it: fuzzy and glob as extended regex.
        let search = |mode: MatchMode, q: &str, icase, hidden| -> Vec<String> {
            let regex = mode != MatchMode::Text;
            let p = if regex { to_regex(mode, q, false) } else { q.to_string() };
            let v = local_grep(&root, &p, icase, regex, hidden).unwrap();
            let mut l: Vec<String> = v["hits"].as_array().unwrap().iter().map(|h| h[2].as_str().unwrap().to_string()).collect();
            l.sort();
            l
        };
        assert_eq!(search(MatchMode::Fuzzy, "gst", true, false), ["git status"]);
        assert_eq!(search(MatchMode::Glob, "Start*(int", false, false), ["void Start(int x);"]);
        assert_eq!(search(MatchMode::Glob, "st?sh", false, false), ["stash it"]);
        assert_eq!(search(MatchMode::Regex, r"^(git|stash) ", false, false), ["git status", "stash it"]);
        assert_eq!(search(MatchMode::Text, "GIT", true, false), ["git status"], "ignore case on");
        assert!(search(MatchMode::Text, "GIT", false, false).is_empty(), "ignore case off");
        assert_eq!(search(MatchMode::Text, "git status", false, true), ["git status", "git status hidden", "git status in hidden dir"], "hidden on");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn local_grep_text_or_regex() {
        let d = std::env::temp_dir().join(format!("weft-grep-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.c"), "void Start(int x);\nStart here\nrestart(\n").unwrap();
        let root = d.display().to_string();
        let lines = |p: &str, icase, rx| -> Result<Vec<String>, String> {
            let v = local_grep(&root, p, icase, rx, false)?;
            Ok(v["hits"].as_array().unwrap().iter().map(|h| h[2].as_str().unwrap().to_string()).collect())
        };
        assert_eq!(lines("Start(", false, false).unwrap(), ["void Start(int x);"], "text: ( is a character");
        assert_eq!(lines(r"Start\(", false, true).unwrap(), ["void Start(int x);"], r"regex: \( is a bracket");
        assert_eq!(lines(r"(St|st)art\(", false, true).unwrap(), ["void Start(int x);", "restart("]);
        assert_eq!(lines("start here", true, false).unwrap(), ["Start here"], "ignore case");
        assert!(lines("Start(", false, true).unwrap_err().contains("bad regex"), "an unclosed group is an error, not zero hits");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn listing_leaves_out_protected_windows_links() {
        let home = std::env::var("USERPROFILE").unwrap();
        for hidden in [false, true] {
            let v = local_op(&json!({"op": "list", "path": home, "hidden": hidden}));
            let names: Vec<&str> = v["entries"].as_array().unwrap().iter().map(|e| e["n"].as_str().unwrap()).collect();
            for link in ["My Documents", "Application Data", "Local Settings", "NetHood"] {
                assert!(!names.contains(&link), "{link} listed (hidden={hidden})");
            }
            assert!(names.contains(&"Documents"), "{names:?}");
        }
        let v = local_op(&json!({"op": "list", "path": home, "hidden": false}));
        assert!(!v["entries"].as_array().unwrap().iter().any(|e| e["n"] == "NTUSER.DAT"));
    }

    #[test]
    fn bootstrap_commands_survive_any_argv_parser() {
        for cmd in [remote_shell_command(Some("/tmp/a b")), agent_command()] {
            assert!(!cmd.contains('"') && !cmd.contains('\\'), "{cmd}");
            let inner = cmd.strip_prefix("exec sh -c '").unwrap().strip_suffix('\'').unwrap();
            assert!(!inner.contains('\''));
        }
    }

    /// Windows ConPTY cuts command lines around 8 KB, and the ssh argv also
    /// carries the target and options.
    #[test]
    fn bootstrap_fits_the_command_line() {
        let n = remote_shell_command(Some("/home/someone/projects/a long directory name")).len();
        println!("remote shell command: {n} bytes");
        assert!(n < 7000, "{n} bytes");
    }

    /// Each shell's integration is cut out of the bundle intact.
    #[test]
    fn bundle_sections_are_intact() {
        let b = integration_bundle();
        let sect = |name: &str| -> String {
            let mut out = String::new();
            let mut on = false;
            for l in b.lines() {
                if l == format!("#@{name}") {
                    on = true;
                    continue;
                }
                if l.starts_with("#@") {
                    on = false;
                }
                if on {
                    out.push_str(l);
                    out.push('\n');
                }
            }
            out
        };
        for (name, src) in [("boot", SHELL_BOOT), ("bash", BASH_SH), ("zsh", ZSH_ZSH), ("fish", FISH_FISH), ("pwsh", PWSH_PS1)] {
            assert_eq!(sect(name), strip_comments(src), "{name}");
            assert!(!src.lines().any(|l| l.starts_with("#@")), "{name} must not contain section markers");
        }
        assert!(ZSH_ZSH.contains("precmd_functions") && ZSH_ZSH.contains("preexec_functions"));
        assert!(FISH_FISH.contains("fish_preexec") && FISH_FISH.contains("fish_prompt"));
    }

    #[test]
    fn askpass_answers() {
        let s = || Some("pw".to_string());
        assert!(matches!(askpass_answer("sunghee@192.168.30.85's password: ", s()), AskpassAnswer::Secret(p) if p == "pw"));
        assert!(matches!(askpass_answer("(sunghee@host) Password:", s()), AskpassAnswer::Secret(_)));
        assert!(matches!(
            askpass_answer("Are you sure you want to continue connecting (yes/no/[fingerprint])? ", s()),
            AskpassAnswer::ConfirmHostKey
        ));
        assert!(matches!(askpass_answer("x's password: ", None), AskpassAnswer::Refuse));
    }

    #[test]
    fn password_switches_agent_off_batch_mode() {
        let p = Profile::ssh("u@h -p 2222");
        let keys = p.agent_argv(false).unwrap();
        let pw = p.agent_argv(true).unwrap();
        assert!(keys.contains(&"BatchMode=yes".to_string()));
        assert!(!pw.contains(&"BatchMode=yes".to_string()));
        assert!(pw.contains(&"NumberOfPasswordPrompts=1".to_string()));
        let env = askpass_env("s3cret");
        assert!(env.iter().all(|(_, v)| !v.contains("s3cret")), "the password is not in ssh's environment");
        assert!(env.iter().any(|(k, _)| k == crate::secret::TICKET_VAR));
    }

    #[test]
    fn local_backend_roundtrip() {
        let dir = std::env::temp_dir().join(format!("weft-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt").display().to_string().replace('\\', "/");
        let w = local_op(&json!({"op": "write", "path": p, "text": "héllo"}));
        assert_eq!(w["ok"], true);
        let r = local_op(&json!({"op": "read", "path": p}));
        assert_eq!(r["text"], "héllo");
        let c = local_op(&json!({"op": "write", "path": p, "text": "x", "expect_mtime": 1.0}));
        assert_eq!(c["conflict"], true);
        let l = local_op(&json!({"op": "list", "path": dir.display().to_string()}));
        assert_eq!(l["entries"][0]["n"], "a.txt");
        let g = local_op(&json!({"op": "grep", "path": dir.display().to_string(), "pattern": "LL", "icase": true}));
        assert_eq!(g["hits"][0][1], 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The file filter's name list: local (Rust) and remote (agent.py) agree,
    /// heavy folders are listed but not descended into, dot files skipped.
    #[test]
    fn tree_lists_names_like_the_agent() {
        let dir = std::env::temp_dir().join(format!("weft-tree-{}", std::process::id()));
        for d in ["src/ui", "node_modules/pkg", ".git"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in ["src/main.rs", "src/ui/view.rs", "node_modules/pkg/index.js", ".env", "README.md"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let root = dir.display().to_string().replace('\\', "/");
        let names = |v: &Value| {
            let mut n: Vec<String> = v["entries"].as_array().unwrap().iter().map(|e| format!("{}{}", e[0].as_str().unwrap(), if e[1].as_bool().unwrap() { "/" } else { "" })).collect();
            n.sort();
            n
        };
        let local = names(&local_op(&json!({"op": "tree", "path": root})));
        assert_eq!(local, ["README.md", "node_modules/", "src/", "src/main.rs", "src/ui/", "src/ui/view.rs"]);
        // The same op in the agent (its functions, without starting the loop).
        let script = format!("import json,sys\nsrc=open(r'{}/remote/agent.py',encoding='utf-8').read().split('\\ndef main')[0]\nexec(src)\nprint(json.dumps(op_tree({{'path': sys.argv[1]}})))", env!("CARGO_MANIFEST_DIR"));
        if let Ok(out) = std::process::Command::new("python").arg("-c").arg(&script).arg(&root).output() {
            if out.status.success() {
                let v: Value = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(names(&v), local, "agent.py op_tree");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_can_leave_out_what_git_ignores() {
        // This repository ignores target/ (where cargo builds).
        let root = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
        let names = |v: &Value| v["entries"].as_array().unwrap().iter().map(|e| e["n"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let all = names(&local_op(&json!({"op": "list", "path": root, "ignored": true})));
        let kept = names(&local_op(&json!({"op": "list", "path": root, "ignored": false})));
        assert!(all.contains(&"target".to_string()) && all.contains(&"src".to_string()), "{all:?}");
        if all.len() == kept.len() {
            return; // no git here
        }
        assert!(!kept.contains(&"target".to_string()) && kept.contains(&"src".to_string()), "{kept:?}");
        // The agent the same.
        let script = format!("import json,sys\nsrc=open(r'{root}/remote/agent.py',encoding='utf-8').read().split('\\ndef main')[0]\nexec(src)\nprint(json.dumps(sorted(e['n'] for e in op_list({{'path': sys.argv[1], 'ignored': False}})['entries'])))");
        if let Ok(out) = std::process::Command::new("python").arg("-c").arg(&script).arg(&root).output() {
            if out.status.success() {
                let mut want = kept.clone();
                want.retain(|n| !n.starts_with('.'));
                want.sort();
                let got: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(got, want, "agent.py list without ignored");
            }
        }
    }

    #[test]
    fn serial_bridge_and_command() {
        // The bridge compiles (it runs on the remote host only: no termios here).
        let check = format!("compile(open(r'{}/remote/serial.py', encoding='utf-8').read(), 'serial.py', 'exec')", env!("CARGO_MANIFEST_DIR"));
        if let Ok(out) = std::process::Command::new("python").arg("-c").arg(&check).output() {
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        }
        // The command carries it whole, the port quoted.
        let cmd = serial_command("/dev/tty USB0", 115200);
        assert!(cmd.contains("'/dev/tty USB0' 115200"), "{cmd}");
        let b64 = cmd.split("b64decode(\"").nth(1).and_then(|s| s.split('"').next()).unwrap();
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(b64).unwrap(), SERIAL_PY.as_bytes());
        // Through ssh: the target, then the bridge instead of a shell.
        let argv = Profile::ssh("pi").serial_argv("/dev/ttyUSB0", 9600, false);
        assert_eq!(&argv[..3], ["ssh", "-tt", "pi"]);
        assert!(argv[3].contains("exec \"$P\" -c"));
        // A serial host has no agent of its own.
        assert_eq!(Profile { name: "COM3".into(), kind: HostKind::Serial { via: "local".into(), port: "COM3".into(), baud: 115200 } }.agent_argv(false), None);
    }

    #[test]
    fn transfer_ops_copy_a_folder_in_chunks() {
        let dir = std::env::temp_dir().join(format!("weft-xfer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/sub")).unwrap();
        std::fs::create_dir_all(dir.join("src/empty")).unwrap();
        let big: Vec<u8> = (0..10_000u32).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(dir.join("src/big.bin"), &big).unwrap();
        std::fs::write(dir.join("src/sub/한글.txt"), "안녕").unwrap();
        std::fs::write(dir.join("src/zero"), "").unwrap();
        let root = dir.display().to_string().replace('\\', "/");
        let op = |v: Value| {
            let r = local_op(&v);
            assert_eq!(r["ok"], true, "{v} -> {r}");
            r
        };
        let walk = op(json!({"op": "walk", "path": format!("{root}/src")}));
        assert_eq!(walk["dir"], true);
        let items: Vec<(String, u64, bool)> = walk["items"].as_array().unwrap().iter().map(|i| (i["p"].as_str().unwrap().to_string(), i["s"].as_u64().unwrap(), i["d"].as_bool().unwrap())).collect();
        let files: Vec<&str> = items.iter().filter(|i| !i.2).map(|i| i.0.as_str()).collect();
        assert_eq!(files, ["big.bin", "zero", "sub/한글.txt"]);
        let dirs: Vec<String> = items.iter().filter(|i| i.2).map(|i| i.0.clone()).collect();
        op(json!({"op": "mkdirs", "root": format!("{root}/dst"), "dirs": dirs}));
        // Every file in 4 KB chunks, into a .weft-part renamed at the end.
        for (rel, size, _) in items.iter().filter(|i| !i.2) {
            let part = format!("{root}/dst/{rel}.weft-part");
            let mut off = 0;
            loop {
                let c = op(json!({"op": "rchunk", "path": format!("{root}/src/{rel}"), "off": off, "len": 4096}));
                op(json!({"op": "wchunk", "path": part, "off": off, "b64": c["b64"]}));
                off += c["n"].as_u64().unwrap();
                if c["eof"] == true {
                    break;
                }
            }
            assert_eq!(off, *size);
            op(json!({"op": "finish", "part": part, "path": format!("{root}/dst/{rel}")}));
        }
        assert_eq!(std::fs::read(dir.join("dst/big.bin")).unwrap(), big);
        assert_eq!(std::fs::read_to_string(dir.join("dst/sub/한글.txt")).unwrap(), "안녕");
        assert_eq!(std::fs::read(dir.join("dst/zero")).unwrap(), b"");
        assert!(dir.join("dst/empty").is_dir());
        assert!(!dir.join("dst/big.bin.weft-part").exists());
        // An aborted part goes away.
        std::fs::write(dir.join("x.weft-part"), "half").unwrap();
        op(json!({"op": "finish", "part": format!("{root}/x.weft-part"), "abort": true}));
        assert!(!dir.join("x.weft-part").exists());
        // The agent walks the same and copies the same, through its functions.
        let script = format!(
            "import json,sys\nsrc=open(r'{}/remote/agent.py',encoding='utf-8').read().split('\\ndef main')[0]\nexec(src)\n\
             r=sys.argv[1]\nw=op_walk({{'path': r+'/src'}})\nop_mkdirs({{'root': r+'/dst2', 'dirs': [i['p'] for i in w['items'] if i['d']]}})\n\
             for i in w['items']:\n    if i['d']: continue\n    off=0\n    while True:\n        c=op_rchunk({{'path': r+'/src/'+i['p'], 'off': off, 'len': 4096}})\n        op_wchunk({{'path': r+'/dst2/'+i['p']+'.weft-part', 'off': off, 'b64': c['b64']}})\n        off+=c['n']\n        if c['eof']: break\n    op_finish({{'part': r+'/dst2/'+i['p']+'.weft-part', 'path': r+'/dst2/'+i['p']}})\n\
             print(json.dumps(w))",
            env!("CARGO_MANIFEST_DIR")
        );
        if let Ok(out) = std::process::Command::new("python").env("PYTHONIOENCODING", "utf-8").arg("-c").arg(&script).arg(&root).output() {
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            let w: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(w["items"], walk["items"], "agent.py op_walk");
            assert_eq!(std::fs::read(dir.join("dst2/big.bin")).unwrap(), big);
            assert_eq!(std::fs::read_to_string(dir.join("dst2/sub/한글.txt")).unwrap(), "안녕");
            assert!(dir.join("dst2/empty").is_dir() && dir.join("dst2/zero").is_file());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// End-to-end checks against a POSIX shell standing in for a remote host
/// (Git Bash on Windows).  Run with `cargo test -- --ignored`.
#[cfg(test)]
mod loopback {
    use super::*;
    use crate::pty::Pty;
    use crate::term::{BlockState, Term, TermEvent};
    use std::time::{Duration, Instant};

    fn bash() -> Option<String> {
        let p = "C:/Program Files/Git/bin/bash.exe";
        if std::path::Path::new(p).is_file() {
            Some(p.into())
        } else {
            which("bash").map(|p| p.display().to_string())
        }
    }

    fn profile() -> Option<Profile> {
        let b = bash()?;
        Some(Profile { name: "loop".into(), kind: HostKind::Custom { pty: vec![b.clone(), "-c".into()], agent: vec![b, "-c".into()] } })
    }

    fn pump(pty: &mut Pty, t: &mut Term, p: &mut vte::Parser, until: impl Fn(&Term) -> bool, secs: u64) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            let b = pty.read();
            if !b.is_empty() {
                t.feed(p, &b);
                let r = std::mem::take(&mut t.replies);
                pty.write(&r);
            }
            if until(t) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        false
    }

    /// A command's block time is the command, not the user's prompt: starship
    /// (or oh-my-posh) takes 50-150 ms per prompt and used to be counted in.
    /// Uses the real local PowerShell with the user's profile.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn powershell_block_time_is_the_command_only() {
        let home = std::env::var("USERPROFILE").unwrap();
        let mut pty = Pty::spawn(&local_shell_argv(), Some(&home), &[], 120, 30, || {}).expect("spawn");
        let mut t = Term::new(120, 30);
        let mut p = vte::Parser::new();
        // Read often: the block times are taken when the bytes are parsed.
        let mut pump = |pty: &mut Pty, t: &mut Term, until: &dyn Fn(&Term) -> bool, secs: u64| {
            let end = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < end {
                let b = pty.read();
                if !b.is_empty() {
                    t.feed(&mut p, &b);
                    let r = std::mem::take(&mut t.replies);
                    pty.write(&r);
                }
                if until(t) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            false
        };
        assert!(pump(&mut pty, &mut t, &|t| !t.blocks.is_empty(), 30), "first prompt");
        pump(&mut pty, &mut t, &|_| false, 1);
        let mut times = Vec::new();
        for _ in 0..3 {
            let done = t.blocks.iter().filter(|b| b.duration.is_some()).count();
            pty.write(b"cd .\r");
            assert!(pump(&mut pty, &mut t, &|t| t.blocks.iter().filter(|b| b.duration.is_some()).count() > done, 10), "command finished");
            times.push(t.blocks.iter().rev().find_map(|b| b.duration).unwrap());
            pump(&mut pty, &mut t, &|_| false, 1);
        }
        println!("cd block times: {times:?}");
        assert!(times.iter().all(|d| *d < Duration::from_millis(50)), "{times:?}");
    }

    /// While typing at the PowerShell prompt the cursor sits on the cell
    /// after the last character (the user saw it covering a character).
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn powershell_cursor_follows_typing() {
        let home = std::env::var("USERPROFILE").unwrap();
        let mut pty = Pty::spawn(&local_shell_argv(), Some(&home), &[], 120, 30, || {}).expect("spawn");
        let mut t = Term::new(120, 30);
        let mut p = vte::Parser::new();
        assert!(pump(&mut pty, &mut t, &mut p, |t| !t.blocks.is_empty(), 30), "first prompt");
        pump(&mut pty, &mut t, &mut p, |_| false, 1);
        let typed = "cd ..\\claude-workspace\\";
        let mut report = Vec::new();
        for (n, ch) in typed.chars().enumerate() {
            pty.write(ch.to_string().as_bytes());
            let want: String = typed.chars().take(n + 1).collect();
            pump(&mut pty, &mut t, &mut p, |t| t.screen[t.cy].text().trim_end().ends_with(want.as_str()), 3);
            // Let PSReadLine finish redrawing (prediction text, cursor move).
            pump(&mut pty, &mut t, &mut p, |_| false, 1);
            let row = t.screen[t.cy].text();
            let line = row.trim_end();
            let under = t.screen[t.cy].cells.get(t.cx).map(|c| c.ch).unwrap_or(' ');
            report.push(format!("{:>2} typed={:?} cx={} line_end={} under={:?} line={:?}", n + 1, ch, t.cx, line.chars().count(), under, line));
        }
        for r in &report {
            println!("{r}");
        }
        let last = t.screen[t.cy].text();
        assert!(last.trim_end().ends_with(typed), "{last:?}");
        assert_eq!(t.cx, last.trim_end().chars().count(), "cursor right after the text");
        pty.write(b"\x03");
    }

    /// A shell weft knows nothing about (dash: no OSC 133): prompts and
    /// commands are still found, by guessing.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn unknown_shell_blocks_are_guessed() {
        let dash = "C:/Program Files/Git/usr/bin/dash.exe";
        if !Path::new(dash).is_file() {
            return;
        }
        let mut pty = Pty::spawn(&[dash.to_string(), "-i".to_string()], None, &[], 80, 24, || {}).expect("spawn");
        let mut t = Term::new(80, 24);
        let mut p = vte::Parser::new();
        let mut wait = |pty: &mut Pty, t: &mut Term, until: &dyn Fn(&Term) -> bool, secs: u64| {
            let end = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < end {
                let b = pty.read();
                if !b.is_empty() {
                    t.feed(&mut p, &b);
                    let r = std::mem::take(&mut t.replies);
                    pty.write(&r);
                }
                t.tick();
                if until(t) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        assert!(wait(&mut pty, &mut t, &|t| !t.blocks.is_empty(), 20), "prompt guessed: {:?}", screen(&t));
        assert!(!t.integrated);
        pty.write(b"echo hi");
        wait(&mut pty, &mut t, &|t| t.screen[t.cy].text().contains("echo hi"), 5);
        t.user_enter();
        pty.write(b"\r");
        assert!(wait(&mut pty, &mut t, &|t| t.blocks.first().map(|b| b.state() == BlockState::Done).unwrap_or(false), 10), "end guessed: {:?}", screen(&t));
        let b = &t.blocks[0];
        assert_eq!(b.command, "echo hi");
        assert!(b.guessed && b.exit.is_none());
        assert_eq!(t.line(b.out_line.unwrap()).unwrap().text().trim_end(), "hi");
        assert_eq!(b.end_line.unwrap() - b.out_line.unwrap(), 1, "one line of output");
        assert_eq!(t.blocks.len(), 2, "and the next prompt");
    }

    fn screen(t: &Term) -> Vec<String> {
        (0..t.rows).map(|y| t.screen[y].text()).filter(|s| !s.is_empty()).collect()
    }

    /// The bootstrap starts the shell named by WEFT_SHELL (normally the login
    /// shell, $SHELL) with its integration; `cmd` runs and its block must end
    /// with `exit` (None: guessed, no code).
    fn login_shell_check(shell: &str, cmd: &str, exit: Option<i32>, integrated: bool) {
        let Some(prof) = profile() else { return };
        if !Path::new(shell).is_file() {
            println!("skip: {shell} not installed");
            return;
        }
        let env = [("WEFT_SHELL".to_string(), shell.to_string())];
        let mut pty = Pty::spawn(&prof.pty_argv(Some("/"), false), None, &env, 100, 30, || {}).unwrap();
        let mut t = Term::new(100, 30);
        let mut p = vte::Parser::new();
        let mut wait = |pty: &mut Pty, t: &mut Term, until: &dyn Fn(&Term) -> bool, secs: u64| {
            let end = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < end {
                let b = pty.read();
                if !b.is_empty() {
                    t.feed(&mut p, &b);
                    let r = std::mem::take(&mut t.replies);
                    pty.write(&r);
                }
                t.tick();
                if until(t) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        assert!(wait(&mut pty, &mut t, &|t| !t.blocks.is_empty(), 40), "{shell}: no prompt: {:?}", screen(&t));
        assert_eq!(t.integrated, integrated, "{shell}: integration");
        // The bootstrap names the shell and its process (for its directory
        // from outside when it has no integration).
        let named = t.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["t"] == "shell" && value["pid"].as_u64().is_some()));
        assert!(named, "{shell}: shell/pid event");
        wait(&mut pty, &mut t, &|_| false, 1);
        pty.write(cmd.as_bytes());
        wait(&mut pty, &mut t, &|t| t.screen[t.cy].text().contains(cmd), 5);
        // As the app does: Enter goes through user_enter.
        t.user_enter();
        pty.write(b"\r");
        let done = wait(&mut pty, &mut t, &|t| t.blocks.iter().any(|b| b.command.contains(cmd) && b.state() == BlockState::Done), 20);
        assert!(done, "{shell}: blocks {:?} screen {:?}", t.blocks.iter().map(|b| (&b.command, b.exit)).collect::<Vec<_>>(), screen(&t));
        let b = t.blocks.iter().find(|b| b.command.contains(cmd)).unwrap();
        assert_eq!(b.exit, exit, "{shell}: exit code");
        if integrated {
            assert!(t.cwd.is_some(), "{shell}: OSC 7 cwd");
        }
    }

    /// A local pane with each local shell choice: marks, cwd, exit codes
    /// where the shell has them.
    fn local_shell_check(choice: LocalShell, cmd: &str, exit: Option<i32>) {
        let (argv, env) = local_shell(choice);
        if which(&argv[0]).is_none() && !Path::new(&argv[0]).is_file() {
            println!("skip: {} not installed", argv[0]);
            return;
        }
        let mut pty = Pty::spawn(&argv, Some(env!("CARGO_MANIFEST_DIR")), &env, 100, 30, || {}).unwrap();
        let mut t = Term::new(100, 30);
        let mut p = vte::Parser::new();
        // nushell reports the directory only from its first command on.
        assert!(pump(&mut pty, &mut t, &mut p, |t| !t.blocks.is_empty(), 30), "{choice:?}: prompt: {:?}", screen(&t));
        assert!(t.integrated, "{choice:?}");
        pump(&mut pty, &mut t, &mut p, |_| false, 1);
        pty.write(cmd.as_bytes());
        pump(&mut pty, &mut t, &mut p, |t| t.screen[t.cy].text().contains(cmd), 5);
        t.user_enter();
        pty.write(b"\r");
        let done = pump(&mut pty, &mut t, &mut p, |t| t.blocks.iter().any(|b| b.command.contains(cmd) && b.state() == BlockState::Done), 15);
        assert!(done, "{choice:?}: {:?} {:?}", t.blocks.iter().map(|b| (&b.command, b.exit)).collect::<Vec<_>>(), screen(&t));
        let b = t.blocks.iter().find(|b| b.command.contains(cmd)).unwrap();
        assert_eq!(b.exit, exit, "{choice:?}");
        assert_eq!(t.block_output(b).trim(), "weft-ok", "{choice:?}");
        assert!(t.cwd.as_deref().map(|c| c.ends_with("weft-initial")).unwrap_or(false), "{choice:?}: cwd {:?}", t.cwd);
    }

    #[test]
    #[ignore]
    #[cfg(windows)]
    fn local_cmd() {
        local_shell_check(LocalShell::Cmd, "echo weft-ok", None);
    }

    #[test]
    #[ignore]
    #[cfg(windows)]
    fn local_nushell() {
        local_shell_check(LocalShell::Nu, "echo weft-ok", Some(0));
    }

    #[test]
    #[ignore]
    #[cfg(windows)]
    fn local_windows_powershell() {
        local_shell_check(LocalShell::Powershell, "Write-Output weft-ok", Some(0));
    }

    #[test]
    #[ignore]
    fn login_shell_nushell() {
        login_shell_check("C:/Program Files/nu/bin/nu.exe", "echo weft-ok", Some(0), true);
    }

    #[test]
    #[ignore]
    fn login_shell_pwsh_remote_style() {
        let pwsh = which("pwsh.exe").map(|p| p.display().to_string().replace('\\', "/")).unwrap_or_default();
        login_shell_check(&pwsh, "Write-Output weft-ok", Some(0), true);
    }

    #[test]
    #[ignore]
    fn login_shell_windows_powershell() {
        login_shell_check("C:/WINDOWS/System32/WindowsPowerShell/v1.0/powershell.exe", "Write-Output weft-ok", Some(0), true);
    }

    #[test]
    #[ignore]
    fn login_shell_unknown_is_guessed() {
        login_shell_check("C:/Program Files/Git/usr/bin/dash.exe", "echo weft-ok", None, false);
    }

    #[test]
    #[ignore]
    fn shell_integration_over_in_memory_rc() {
        let Some(prof) = profile() else { return };
        let argv = prof.pty_argv(Some("/"), false);
        let mut pty = Pty::spawn(&argv, None, &[], 100, 30, || {}).unwrap();
        let mut t = Term::new(100, 30);
        let mut p = vte::Parser::new();
        assert!(pump(&mut pty, &mut t, &mut p, |t| t.at_prompt(), 20), "no prompt: {:?}", screen(&t));
        assert_eq!(t.cwd.as_deref(), Some("/"), "OSC 7 cwd");
        pty.write(b"echo weft-ok\r");
        let ok = pump(&mut pty, &mut t, &mut p, |t| t.blocks.iter().any(|b| b.command == "echo weft-ok" && b.state() == BlockState::Done), 10);
        assert!(ok, "blocks {:?} screen {:?}", t.blocks.iter().map(|b| (&b.command, b.exit)).collect::<Vec<_>>(), screen(&t));
        let b = t.blocks.iter().find(|b| b.command == "echo weft-ok").unwrap().clone();
        assert_eq!(b.exit, Some(0));
        assert_eq!(t.block_output(&b), "weft-ok");
        pty.write(b"false\r");
        assert!(pump(&mut pty, &mut t, &mut p, |t| t.blocks.iter().any(|b| b.command == "false" && b.exit == Some(1)), 10));
        pty.write(b"wnotify hello there\r");
        let ok = pump(
            &mut pty,
            &mut t,
            &mut p,
            |t| t.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["msg"] == "hello there")),
            10,
        );
        assert!(ok, "screen {:?}", screen(&t));
        pty.write(b"wls\r");
        let ok = pump(
            &mut pty,
            &mut t,
            &mut p,
            |t| {
                t.events.iter().any(|e| {
                    matches!(e, TermEvent::Weft { value, .. }
                    if value["kind"] == "files" && value["rows"].as_u64() == Some(value["entries"].as_array().map(|a| a.len() as u64 + 1).unwrap_or(0)))
                })
            },
            10,
        );
        assert!(ok, "wls widget missing; screen {:?}", screen(&t));
    }

    #[test]
    #[ignore]
    fn closing_a_pane_is_quick() {
        let Some(prof) = profile() else { return };
        let mut pty = Pty::spawn(&prof.pty_argv(Some("/"), false), None, &[], 80, 24, || {}).unwrap();
        let mut t = Term::new(80, 24);
        let mut p = vte::Parser::new();
        assert!(pump(&mut pty, &mut t, &mut p, |t| t.at_prompt(), 20));
        let started = Instant::now();
        drop(pty);
        let took = started.elapsed();
        eprintln!("dropping a pane took {took:?}");
        assert!(took < Duration::from_millis(100), "closing a pane blocked the UI for {took:?}");
    }

    #[test]
    #[ignore]
    fn agent_shipped_over_stdin() {
        let Some(prof) = profile() else { return };
        let data = DataDir { root: std::env::temp_dir().join("weft-test-data") };
        let _ = std::fs::create_dir_all(data.root.join("logs"));
        let mut c = Conn::open(prof, &data, || {}, None);
        let end = Instant::now() + Duration::from_secs(20);
        while c.status != ConnStatus::Ready && Instant::now() < end {
            c.poll();
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(c.status, ConnStatus::Ready);
        let id = c.request(json!({"op": "list", "path": env!("CARGO_MANIFEST_DIR")}));
        let mut got = None;
        while got.is_none() && Instant::now() < end {
            got = c.poll().into_iter().find(|r| r["id"] == id);
            std::thread::sleep(Duration::from_millis(50));
        }
        let r = got.expect("list response");
        assert_eq!(r["ok"], true);
        assert!(r["entries"].as_array().unwrap().iter().any(|e| e["n"] == "Cargo.toml"));
    }
}
