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

pub const RC_SH: &str = include_str!("../remote/rc.sh");
pub const AGENT_PY: &str = include_str!("../remote/agent.py");

/// Pane bootstrap (one logical line, run via `eval`): pick a base64 decoder
/// from POSIX tools (or python), then start bash with the gzipped rc fed
/// through process substitution.  Falls back to a plain `sh -i` without bash.
const SHELL_BOOT: &str = r#"D="base64 -d"; printf QQ== | $D >/dev/null 2>&1 || D="base64 -D"; printf QQ== | $D >/dev/null 2>&1 || D="python3 -m base64 -d"; export W D WEFT_CWD WEFT_TOKEN; if command -v bash >/dev/null 2>&1; then exec bash -c 'z() { if command -v gzip >/dev/null 2>&1; then gzip -dc; else python3 -c "import sys,gzip;sys.stdout.buffer.write(gzip.decompress(sys.stdin.buffer.read()))"; fi; }; exec bash --rcfile <(printf %s "$W" | $D | z) -i'; fi; exec sh -i"#;

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

/// `token`: the pane's OSC token (hex), handed to the rc (see term.rs `osc_token`).
pub fn remote_shell_command(cwd: Option<&str>, token: &str) -> String {
    posix_wrap(&[("W", gz_b64(RC_SH)), ("WEFT_CWD", cwd.map(b64).unwrap_or_default()), ("WEFT_TOKEN", token.to_string())], SHELL_BOOT)
}

/// A random token for a pane's OSC guard (not cryptographic: it only has to
/// be unknowable to whoever wrote a file or a program's output).
pub fn new_osc_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(n);
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
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
    },
    /// Any command that behaves like `ssh HOST CMD` (receives the remote
    /// command string as its last argument), e.g. `["wsl", "-e", "sh", "-c"]`.
    Custom { pty: Vec<String>, agent: Vec<String> },
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
        Profile { name: host.clone(), kind: HostKind::Ssh { target: host, args: parts.collect() } }
    }

    pub fn is_local(&self) -> bool {
        matches!(self.kind, HostKind::Local)
    }

    /// argv for an interactive pane.  With `password` the ssh password comes
    /// from weft's askpass (see `askpass_env`) instead of the terminal.
    /// `token`: the pane's OSC token (local panes get it through the environment).
    pub fn pty_argv(&self, cwd: Option<&str>, password: bool, token: &str) -> Vec<String> {
        match &self.kind {
            HostKind::Local => local_shell_argv(),
            HostKind::Ssh { target, args } => {
                let mut v = vec!["ssh".to_string(), "-tt".to_string()];
                if password {
                    v.extend(["-o".to_string(), "NumberOfPasswordPrompts=1".to_string()]);
                }
                v.extend(args.iter().cloned());
                v.push(target.clone());
                v.push(remote_shell_command(cwd, token));
                v
            }
            HostKind::Custom { pty, .. } => {
                let mut v = pty.clone();
                v.push(remote_shell_command(cwd, token));
                v
            }
        }
    }

    /// argv for the agent side channel (None: served locally).  Without a
    /// password it never prompts (keys / ssh-agent only); with one, the
    /// password is answered by weft's askpass.
    pub fn agent_argv(&self, password: bool) -> Option<Vec<String>> {
        match &self.kind {
            HostKind::Local => None,
            HostKind::Ssh { target, args } => {
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
        (ASKPASS_SECRET_VAR.into(), secret.into()),
    ]
}

pub const ASKPASS_SECRET_VAR: &str = "WEFT_ASKPASS";

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

fn local_shell_argv() -> Vec<String> {
    let init = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8
$env:WEFT = '1'
# weft's OSC token: a variable only, removed from the environment so programs
# and files can't produce sequences weft would trust.
$global:__weft_tok = $env:WEFT_TOKEN
Remove-Item Env:WEFT_TOKEN -ErrorAction SilentlyContinue
$global:__weft_prompt = $function:prompt
function global:prompt {
    $ok = $global:?
    $e = [char]27; $b = [char]7; $t = "weft=$global:__weft_tok"
    $code = if ($ok) { 0 } elseif ($global:LASTEXITCODE) { $global:LASTEXITCODE } else { 1 }
    $loc = Get-Location
    $osc7 = ''
    if ($loc.Provider.Name -eq 'FileSystem') { $osc7 = "$e]7;$t;file://$env:COMPUTERNAME/" + ($loc.ProviderPath -replace '\\', '/') + $b }
    # End the block now: the user's own prompt (starship, oh-my-posh) can take
    # 50-150 ms and must not count as the command's duration.
    [Console]::Write("$e]133;$t;D;$code$b$osc7")
    "$e]133;$t;A$b" + (& $global:__weft_prompt) + "$e]133;$t;B$b"
}
if (Get-Module PSReadLine) {
    Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
        [Console]::Write("$([char]27)]133;weft=$global:__weft_tok;C$([char]7)")
    }
}
# wosc CODE [ARG...] : send an OSC sequence weft accepts (args joined by ';')
function global:wosc {
    if ($args.Count -eq 0) { Write-Error 'usage: wosc CODE [ARG...]'; return }
    $rest = if ($args.Count -gt 1) { ';' + ($args[1..($args.Count - 1)] -join ';') } else { '' }
    [Console]::Write("$([char]27)]$($args[0]);weft=$global:__weft_tok$rest$([char]7)")
}
"#;
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

    pub fn log(&self, line: &str) {
        if let Some(mut f) = self.open_log("weft.log") {
            let _ = writeln!(f, "{} {}", timestamp(), line);
        }
    }
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
    log: Option<File>,
    stderr_tail: String,
    /// Failures for requests that could not even be sent; delivered by `poll`.
    backend_failures: Vec<Value>,
}

impl Conn {
    pub fn open(profile: Profile, data: &DataDir, wake: impl Fn() + Send + Clone + 'static, password: Option<String>) -> Conn {
        let log = data.open_log(&format!("agent-{}.log", safe_name(&profile.name)));
        let mut conn = Conn {
            profile,
            status: ConnStatus::Connecting,
            info: Value::Null,
            next_id: 1,
            backend: Backend::Dead,
            log,
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
        if let Some(f) = self.log.as_mut() {
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

fn mtime_of(md: &std::fs::Metadata) -> f64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()).unwrap_or(0.0)
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
                if !hidden && name.starts_with('.') {
                    continue;
                }
                let link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
                let md = std::fs::metadata(e.path()).ok();
                let d = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = md.as_ref().map(|m| if m.is_dir() { 0 } else { m.len() }).unwrap_or(0);
                let m = md.as_ref().map(mtime_of).unwrap_or(0.0) as i64;
                entries.push(json!({"n": name, "d": d, "l": link, "s": size, "m": m}));
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
        "grep" => local_grep(&s("path"), &s("pattern"), req.get("icase").and_then(|v| v.as_bool()).unwrap_or(false)),
        "tree" => Ok(local_tree(&s("path"), req.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false), req.get("max").and_then(|v| v.as_u64()).unwrap_or(20000) as usize)),
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
            if !hidden && name.starts_with('.') {
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

fn local_grep(root: &str, pattern: &str, icase: bool) -> Result<Value, String> {
    let needle = if icase { pattern.to_lowercase() } else { pattern.to_string() };
    let mut hits = Vec::new();
    let mut stack = vec![PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
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
                let hay = if icase { line.to_lowercase() } else { line.to_string() };
                if hay.contains(&needle) {
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
    fn bootstrap_commands_survive_any_argv_parser() {
        for cmd in [remote_shell_command(Some("/tmp/a b"), "0123abcd"), agent_command()] {
            assert!(!cmd.contains('"') && !cmd.contains('\\'), "{cmd}");
            let inner = cmd.strip_prefix("exec sh -c '").unwrap().strip_suffix('\'').unwrap();
            assert!(!inner.contains('\''));
        }
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
        assert!(askpass_env("s").iter().any(|(k, v)| k == ASKPASS_SECRET_VAR && v == "s"));
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

    /// The OSC guard end to end: the shell integration (prompt marks, cwd)
    /// and `wosc` work with the pane's token; the same OSC printed by a
    /// program is ignored; the token isn't in the shell's environment.
    fn guard_check(argv: Vec<String>, env: &[(String, String)], tok: &str, untagged: &str, wosc: &str, show_env: &str) {
        let mut pty = Pty::spawn(&argv, None, env, 120, 30, || {}).expect("spawn");
        let mut t = Term::new(120, 30);
        t.osc_token = Some(tok.to_string());
        let mut p = vte::Parser::new();
        assert!(pump(&mut pty, &mut t, &mut p, |t| !t.blocks.is_empty() && t.cwd.is_some(), 30), "prompt marks and cwd with the token: {:?}", screen(&t));
        let clip = |t: &mut Term| std::mem::take(&mut t.events).into_iter().filter(|e| matches!(e, TermEvent::Clipboard(_))).count();
        clip(&mut t);
        pty.write(untagged.as_bytes());
        pump(&mut pty, &mut t, &mut p, |_| false, 3);
        assert_eq!(clip(&mut t), 0, "OSC 52 printed by a program must be ignored");
        pty.write(wosc.as_bytes());
        assert!(pump(&mut pty, &mut t, &mut p, |t| t.events.iter().any(|e| matches!(e, TermEvent::Clipboard(s) if s == "hi")), 10), "wosc accepted");
        pty.write(show_env.as_bytes());
        pump(&mut pty, &mut t, &mut p, |_| false, 3);
        let all: String = screen(&t).join("\n");
        assert!(!all.contains(tok), "token visible in the environment: {all}");
    }

    #[test]
    #[ignore]
    fn osc_guard_bash() {
        let Some(prof) = profile() else { return };
        let tok = new_osc_token();
        guard_check(
            prof.pty_argv(Some("/"), false, &tok),
            &[],
            &tok,
            "printf '\\033]52;c;aGk=\\007'\r",
            "wosc 52 c aGk=\r",
            "env | grep -c WEFT_TOKEN; env | grep -i weft\r",
        );
    }

    #[test]
    #[ignore]
    #[cfg(windows)]
    fn osc_guard_powershell() {
        let tok = new_osc_token();
        guard_check(
            local_shell_argv(),
            &[("WEFT_TOKEN".to_string(), tok.clone())],
            &tok,
            "Write-Host -NoNewline \"`e]52;c;aGk=`a\"\r",
            "wosc 52 c aGk=\r",
            "Get-ChildItem Env: | Out-String -Width 300\r",
        );
    }

    fn screen(t: &Term) -> Vec<String> {
        (0..t.rows).map(|y| t.screen[y].text()).filter(|s| !s.is_empty()).collect()
    }

    #[test]
    #[ignore]
    fn shell_integration_over_in_memory_rc() {
        let Some(prof) = profile() else { return };
        let argv = prof.pty_argv(Some("/"), false, "");
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
        let mut pty = Pty::spawn(&prof.pty_argv(Some("/"), false, ""), None, &[], 80, 24, || {}).unwrap();
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
