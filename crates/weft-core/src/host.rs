//! Hosts and connections.
//!
//! weft is a thick local client.  For a remote host it opens
//!   * one `ssh -tt` PTY per pane running bash with weft's integration rc, and
//!   * one shared `ssh -T` side channel running a tiny python agent,
//!
//! and nothing is ever copied to the remote disk: the rc file arrives through
//! bash process substitution, the agent source through python's stdin.
//! The local host (Windows here) needs no agent; the same JSON operations are
//! served directly by Rust.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::UNIX_EPOCH;

/// Shell integrations (prompt and command marks, current directory), one per
/// shell. They travel to a remote host as one gzipped bundle in memory.
pub const BASH_SH: &str = include_str!("../../../shell/bash.sh");
pub const ZSH_ZSH: &str = include_str!("../../../shell/zsh.zsh");
pub const FISH_FISH: &str = include_str!("../../../shell/fish.fish");
pub const PWSH_PS1: &str = include_str!("../../../shell/pwsh.ps1");
pub const SH_SH: &str = include_str!("../../../shell/sh.sh");

/// All integrations in one text: sections start with `#@<name>` lines; the
/// bootstrap cuts out the one it needs with awk. Comment and blank lines are
/// left out (the command line that carries it is limited to about 8 KB).
pub fn integration_bundle() -> String {
    let mut out = String::new();
    for (name, src) in [("boot", SHELL_BOOT), ("bash", BASH_SH), ("zsh", ZSH_ZSH), ("fish", FISH_FISH), ("pwsh", PWSH_PS1), ("sh", SH_SH)] {
        out.push_str(&format!("#@{name}\n{}", strip_comments(src)));
    }
    out
}

/// Without whole-line comments and blank lines (`#!` lines stay), and an
/// indent of four spaces made one (the shells do not care, and the Python
/// in bash.sh keeps its steps: every block there is indented by fours;
/// lines indented otherwise, continuations inside brackets, stay as they
/// are). No section has a here document or here-string whose body is
/// indented.
fn strip_comments(src: &str) -> String {
    src.lines()
        .filter(|l| {
            let t = l.trim_start();
            // Comments go, the #! line stays.
            let comment = t.starts_with('#') && !t.starts_with("#!");
            !t.is_empty() && !comment
        })
        .map(|l| {
            let t = l.trim_start_matches(' ');
            let n = l.len() - t.len();
            if n % 4 == 0 { format!("{}{t}\n", " ".repeat(n / 4)) } else { format!("{l}\n") }
        })
        .collect()
}
pub const AGENT_PY: &str = include_str!("../../../remote/agent.py");

/// Pane bootstrap (run via `eval` by sh): start the shell with its
/// integration from the bundle in $W. Which shell: the one chosen for the
/// host (its name or path in $WS64, base64; WEFT_SHELL in tests), else the
/// user's login shell ($SHELL). A chosen one that is not there: said
/// (OSC 7777 "shell-missing"), and the login shell.
/// bash `--rcfile <(...)`, fish `-C`, pwsh `-Command`, nushell's own OSC 133
/// switched on, zsh through WEFT_ZSH typed in by weft (see shell/zsh.zsh),
/// the POSIX shells with no hooks (ksh, mksh, dash, ash, sh) through ENV on
/// a descriptor (shell/sh.sh; the two lines after its `exec` are the here
/// document), where /dev/fd/3 can be opened (FreeBSD without fdescfs has
/// /dev/fd/0-2 only: there, the shell as it is, no ENV left behind). Any
/// other shell starts as is, with no integration.
const SHELL_BOOT: &str = r##"WW=; [ -n "$WS64" ] && WW=$(printf %s "$WS64" | $D 2>/dev/null); [ -n "$WEFT_SHELL" ] && WW=$WEFT_SHELL; WP=${WW:-$SHELL}; if [ -n "$WW" ] && ! command -v "$WW" >/dev/null 2>&1; then WP=$SHELL; WU=${WP##*/}; WU=${WU%.exe}; printf '\033]7777;{"t":"shell-missing","wanted":"%s","using":"%s"}\007' "$WW" "${WU:-sh}"; fi; unset WS64 WEFT_SHELL; WS=${WP##*/}; WS=${WS%.exe}; export WEFT=1; case "$WS" in bash|'') WY=bash; [ -n "$WS" ] && WY=$WP; if command -v "$WY" >/dev/null 2>&1; then R=$(sect bash); export R D WEFT_CWD; exec "$WY" -c '[ ${BASH_VERSINFO[0]} -ge 4 ] && exec "$BASH" --rcfile <(printf %s "$R") -i; WEFT_RC3=1 exec "$BASH" --rcfile /dev/fd/3 -i 3<<<"$R"'; fi ;; esac; WS=${WS:-sh}; WP=${WP:-sh}; WEFT_AT=; [ -n "$WEFT_CWD" ] && C=$(printf %s "$WEFT_CWD" | $D 2>/dev/null) && [ -d "$C" ] && cd "$C" && WEFT_AT=$C; unset WEFT_CWD; printf '\033]7777;{"t":"shell","name":"%s","pid":%s}\007' "$WS" "$$"; case "$WS" in zsh) WEFT_ZSH=$(sect zsh); export WEFT_ZSH; printf '\033]7777;{"t":"inject","shell":"zsh"}\007'; exec "$WP" -l -i ;; fish) WEFT_FISH=$(sect fish); export WEFT_FISH; exec "$WP" -l -C 'eval $WEFT_FISH' ;; nu) exec "$WP" -l -e 'try { $env.config.shell_integration.osc133 = true; $env.config.shell_integration.osc7 = true } catch { $env.config.shell_integration = true }' ;; pwsh|powershell) WEFT_PS=$(sect pwsh); export WEFT_PS; exec "$WP" -NoLogo -NoExit -Command '. ([ScriptBlock]::Create($env:WEFT_PS))' ;; ksh|ksh93|mksh|dash|ash|sh) if (exec 3</dev/null; [ -r /dev/fd/3 ]) 2>/dev/null; then R=$(sect sh); WEFT_SH=$WS; ENV=/dev/fd/3; export WEFT_SH WEFT_AT ENV; exec "$WP" -i 3<<WEFT_EOF
$R
WEFT_EOF
fi ;; esac; exec "$WP" -i"##;

/// On the command line: find a base64 decoder and gunzip among POSIX tools (or
/// python), unpack the bundle in $W and run its `boot` section. Nothing
/// unpacked (none of the tools there): said, and the user's shell as it is
/// (it ended at once with code 0, and the terminal closed without a word).
const SHELL_UNPACK: &str = r##"printf '\033]7777;{"t":"sh"}\007'; t() { printf QQ== | $D >/dev/null 2>&1; }; D="base64 -d"; t || D="base64 -D"; t || D="openssl base64 -d -A"; t || D="python3 -m base64 -d"; z() { gzip -dc 2>/dev/null || python3 -c "import sys,gzip;sys.stdout.buffer.write(gzip.decompress(sys.stdin.buffer.read()))"; }; WB=$(printf %s "$W" | $D 2>/dev/null | z 2>/dev/null); sect() { printf %s "$WB" | awk -v s="#@$1" '$0==s{f=1;next} /^#@/{f=0} f'; }; BT=$(sect boot 2>/dev/null); if [ -z "$BT" ]; then echo "weft: cannot unpack its shell integration here: a plain shell"; exec "${SHELL:-sh}" -i; fi; eval "$BT""##;

/// Windows ConPTY truncates command lines around 8 KB, so the rc travels gzipped.
fn gz_b64(s: &str) -> String {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(s.as_bytes()).expect("gzip to memory");
    base64::engine::general_purpose::STANDARD.encode(e.finish().expect("gzip to memory"))
}

/// The Pythons tried on a remote host, in order, until one is 3.6 or newer
/// (settings "remote_python", "Python on remote hosts…"): names on its PATH
/// or paths, `*` allowed. Distributions put it in different places (a
/// minimal RHEL / Rocky 8 has only /usr/libexec/platform-python).
pub const DEFAULT_PYTHONS: &[&str] = &["python3", "python", "py", "/usr/libexec/platform-python", "/usr/local/bin/python3", "/opt/homebrew/bin/python3"];

static PYTHONS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

/// The list the agent and the serial bridge try (empty: DEFAULT_PYTHONS).
pub fn set_pythons(list: &[String]) {
    *PYTHONS.write().unwrap_or_else(|e| e.into_inner()) = list.to_vec();
}

fn pythons_b64() -> String {
    pythons_b64_of(&PYTHONS.read().unwrap_or_else(|e| e.into_inner()))
}

/// A list as the agent's and the bridge's commands carry it.
pub fn pythons_b64_of(list: &[String]) -> String {
    if list.is_empty() {
        b64(&DEFAULT_PYTHONS.join("\n"))
    } else {
        b64(&list.join("\n"))
    }
}

/// What the panels say for a host whose shell is none weft knows.
pub const UNSUPPORTED_SHELL: &str = "none of the shells weft supports started on this host (a POSIX sh; Windows' cmd or PowerShell): the terminal is a plain ssh, without weft's shell integration, and the Files panel, editor and search do not run there. Shells... (command palette) lists what weft supports";

/// The shell a remote host runs commands with (ssh's command, or what a
/// custom transport is given). weft's terminal integration and agent
/// start with a command in its language; which one it is, is found by
/// trying (App::shell_failed) and kept with the profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteShell {
    /// sh, bash, zsh, fish, ksh…: what weft's commands are written in.
    #[default]
    Posix,
    /// Windows' cmd or PowerShell (Windows' OpenSSH): weft starts
    /// PowerShell there, with its integration, and the agent through it.
    Windows,
    /// Neither: the terminal is a plain ssh, without weft's integration,
    /// and there is no agent (UNSUPPORTED_SHELL).
    Other,
}

impl RemoteShell {
    pub fn is_posix(&self) -> bool {
        *self == RemoteShell::Posix
    }
}

/// PowerShell's -EncodedCommand: the script as UTF-16LE, in base64.
fn ps_encoded(script: &str) -> String {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

/// A folder as weft keeps it ("/C:/Users/me", or msys's "/c/Users/me") as
/// Windows writes it ("C:/Users/me").
pub fn windows_path(p: &str) -> String {
    let b = p.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return p[1..].to_string();
    }
    if b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/') {
        return format!("{}:{}", &p[1..2], if b.len() == 2 { "/" } else { &p[2..] });
    }
    p.to_string()
}

/// A Windows host's terminal: PowerShell (from cmd or PowerShell alike),
/// saying it has started (as weft's sh one does), in `cwd`, with weft's
/// integration (shell/pwsh.ps1: prompt marks, the folder).
pub fn windows_shell_command(cwd: Option<&str>) -> String {
    let mut s = String::from("[Console]::Write([char]27 + ']7777;{\"t\":\"shell\",\"name\":\"powershell\",\"pid\":' + $PID + '}' + [char]7)\n");
    if let Some(c) = cwd {
        s.push_str(&format!("Set-Location -LiteralPath '{}' -ErrorAction SilentlyContinue\n", ps_single(&windows_path(c))));
    }
    s.push_str(PWSH_PS1);
    format!("powershell -NoLogo -NoExit -EncodedCommand {}", ps_encoded(&s))
}

/// PowerShell: the first of the list (PY64) that runs as Python 3.6+, as
/// FIND_PY does in sh; none: what was tried and what to do. Then the agent
/// as on any host: its source on stdin, run in memory.
const WIN_AGENT_PS: &str = r#"$ErrorActionPreference = 'SilentlyContinue'
$list = @([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('PY64')) -split "`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$py = $null
foreach ($e in $list) {
    $cands = if ($e -match '[*?]') { @(Resolve-Path -Path $e | ForEach-Object { $_.Path }) } else { @($e) }
    foreach ($c in $cands) {
        try {
            & $c -c 'import sys; sys.exit(sys.version_info < (3, 6))' *> $null
            if ($LASTEXITCODE -eq 0) { $py = $c; break }
        } catch { }
    }
    if ($py) { break }
}
if (-not $py) {
    [Console]::Error.WriteLine('weft: no Python 3.6 or newer found on this host: the Files panel, editor, search and serial consoles need one. Check Python is installed there (where python), or add its name or path in Python on remote hosts (command palette). Tried: ' + ($list -join ' '))
    exit 127
}
& $py -u -c 'import sys;exec(sys.stdin.buffer.read(int(sys.stdin.buffer.readline())))'
exit $LASTEXITCODE
"#;

/// A Windows host's agent: WIN_AGENT_PS with the list of Pythons.
pub fn windows_agent_command() -> String {
    windows_agent_command_of(&PYTHONS.read().unwrap_or_else(|e| e.into_inner()))
}

pub fn windows_agent_command_of(list: &[String]) -> String {
    format!("powershell -NoLogo -NoProfile -NonInteractive -EncodedCommand {}", ps_encoded(&WIN_AGENT_PS.replace("PY64", &pythons_b64_of(list))))
}

/// What a remote host says when none of the list is Python 3.6+.
pub const NO_PYTHON: &str = "weft: no Python 3.6 or newer found on this host";

/// sh: $P becomes the first of the list in $PY64 (base64, a line each; a
/// `*` matches) that runs as Python 3.6+; none: it says what was tried and
/// what to do, and ends. macOS' /usr/bin/python3 is not run where the
/// Command Line Tools are not installed (wpy): it is no Python there, only
/// asks to install them, in a window on that Mac.
const FIND_PY: &str = r#"PL=$(printf %s "$PY64" | base64 -d 2>/dev/null || printf %s "$PY64" | base64 -D 2>/dev/null || printf %s "$PY64" | openssl base64 -d -A 2>/dev/null); wpy() { [ "$(command -v "$1" 2>/dev/null)" != /usr/bin/python3 ] || [ "$(uname 2>/dev/null)" != Darwin ] || xcode-select -p >/dev/null 2>&1; }; P=$(printf '%s\n' "$PL" | while IFS= read -r e; do IFS='|'; set +f; for c in $e; do if wpy "$c" && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 6))' >/dev/null 2>&1; then printf %s "$c"; exit 0; fi; done; done); set -f; if [ -z "$P" ]; then T=; for e in $PL; do T="$T $e"; done; M=; wpy python3 || M=" On macOS python3 comes with the Command Line Tools (xcode-select --install there), or install Python (python.org, Homebrew)."; echo "weft: no Python 3.6 or newer found on this host: the Files panel, editor, search and serial consoles need one. Check python3 is installed there (command -v python3), or add its name or path in Python on remote hosts (command palette).$M Tried:$T" >&2; exit 127; fi"#;

/// Agent bootstrap: find Python (FIND_PY), read `<len>\n<source>` from stdin
/// and exec it in memory.
const AGENT_BOOT: &str = r#"exec "$P" -u -c "import sys;exec(sys.stdin.buffer.read(int(sys.stdin.buffer.readline())))""#;

pub fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

/// Wrap a POSIX script so the command string contains no `"` or `\`: it
/// survives any login shell (sh/bash/zsh/fish) and any Windows argv parser
/// (MSVCRT ssh.exe, msys/cygwin ssh and bash), which re-quote differently.
/// What a terminal does where weft's command cannot even be decoded: says
/// so and is the user's shell as it is. (No quote or backslash in it: see
/// posix_wrap.)
/// How a host with none of the decoders starts weft's command: it ran
/// there (a POSIX sh), it could not unpack (the app does not try another
/// shell for it).
pub const NO_DECODER_SAID: &str = "weft: no base64, openssl or python3 here";

/// A Windows shell said it does not know `exec` (weft's POSIX command was
/// run by cmd or PowerShell): the name in quotes, which quotes depending on
/// the language ('exec' in English and Korean, "exec" in German, „exec“ in
/// Czech). weft's own command has it after a space (`exec sh -c`).
pub fn says_exec_unknown(text: &str) -> bool {
    const QUOTES: &[char] = &['\'', '"', '„', '“', '”', '‘', '’', '‚', '«', '»', '「', '」'];
    // (One space between the quote and the name is skipped: French puts a
    // no-break one inside « ».)
    let quote = |mut cs: std::iter::Peekable<Box<dyn Iterator<Item = char> + '_>>| {
        if cs.peek().is_some_and(|c| matches!(c, ' ' | '\u{a0}' | '\u{202f}')) {
            cs.next();
        }
        cs.next().is_some_and(|c| QUOTES.contains(&c))
    };
    text.match_indices("exec").any(|(i, _)| {
        let before: Box<dyn Iterator<Item = char>> = Box::new(text[..i].chars().rev());
        let after: Box<dyn Iterator<Item = char>> = Box::new(text[i + 4..].chars());
        quote(before.peekable()) && quote(after.peekable())
    })
}
const PLAIN_SHELL: &str = "echo weft: no base64, openssl or python3 h$()ere: a plain shell, without its integration; IFS=; exec ${SHELL:-sh} -i";
/// And the agent, or the serial bridge: says so and ends.
const NO_DECODER: &str = "echo weft: no base64, openssl or python3 h$()ere: the Files panel, editor and search do not run >&2; exit 127";
const NO_DECODER_SERIAL: &str = "echo weft: no base64, openssl or python3 h$()ere: no serial console >&2; exit 127";

fn posix_wrap(vars: &[(&str, String)], script: &str, fail: &str) -> String {
    let mut s = String::from("exec sh -c '");
    for (k, v) in vars {
        s.push_str(&format!("{k}={v}; "));
    }
    let b = b64(script);
    s.push_str(&format!(
        "B={b}; set -f; d() {{ printf %s $B | $* 2>/dev/null; }}; S=$(d base64 -d) || S=$(d base64 -D) || S=$(d openssl base64 -d -A) || S=$(d python3 -m base64 -d) || {{ {fail}; }}; eval $S'"
    ));
    s
}

pub fn remote_shell_command(cwd: Option<&str>) -> String {
    remote_shell_command_with(cwd, None)
}

/// `remote_shell_command`, starting this shell (a name on the host's PATH,
/// or a path) instead of the user's login shell.
pub fn remote_shell_command_with(cwd: Option<&str>, shell: Option<&str>) -> String {
    let shell = shell.map(str::trim).filter(|s| !s.is_empty());
    posix_wrap(
        &[("W", gz_b64(&integration_bundle())), ("WEFT_CWD", cwd.map(b64).unwrap_or_default()), ("WS64", shell.map(b64).unwrap_or_default())],
        SHELL_UNPACK,
        PLAIN_SHELL,
    )
}

/// How far weft's integration goes in a shell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShellSupport {
    /// Prompts, commands, their output and exit codes, the folder: exact.
    Full,
    /// Some of it; `ShellInfo::missing` says what is not there.
    Partial,
    /// None: a terminal as any other.
    None,
}

/// A shell weft knows, and what it has there.
pub struct ShellInfo {
    /// The names it goes by (the file name of the shell's program).
    pub names: &'static [&'static str],
    pub title: &'static str,
    pub support: ShellSupport,
    /// What is not there (empty with full support).
    pub missing: &'static [&'static str],
    /// It can be the local shell on Windows / on macOS and Linux.
    pub local_windows: bool,
    pub local_unix: bool,
    /// It can be a remote host's shell.
    pub remote: bool,
}

const NO_CODES: &str = "exit codes (the shell does not tell them)";
const NO_TEXT: &str = "the text of a command you type (none kept in history, no rerun of it; weft's own runs keep theirs)";
/// FreeBSD's sh, and NetBSD's without promptcmds, run no command in a
/// prompt: the integration leaves the prompt as it was there.
const NO_BSD_SH: &str = "all of it on FreeBSD's sh and NetBSD's without promptcmds (no command runs in their prompt)";
const NO_HELPERS: &str = "the w* helper commands (wopen, wls, ...; bash only)";
/// What no integration at all leaves out.
pub const NOT_INTEGRATED: &[&str] = &["command blocks (where a command and its output start and end)", "exit codes", "command history", "the folder, unless weft's command started the shell and the host has Python for weft's agent (not a plain ssh)", NO_HELPERS];

/// The shells weft has an integration for. Any other: `ShellSupport::None`.
pub const SHELLS: &[ShellInfo] = &[
    ShellInfo { names: &["bash"], title: "bash", support: ShellSupport::Full, missing: &[], local_windows: false, local_unix: true, remote: true },
    ShellInfo { names: &["zsh"], title: "zsh", support: ShellSupport::Full, missing: &[NO_HELPERS], local_windows: false, local_unix: true, remote: true },
    ShellInfo { names: &["fish"], title: "fish", support: ShellSupport::Full, missing: &[NO_HELPERS], local_windows: false, local_unix: true, remote: true },
    ShellInfo { names: &["pwsh", "powershell"], title: "PowerShell (pwsh 7, Windows PowerShell 5.1)", support: ShellSupport::Full, missing: &[NO_HELPERS], local_windows: true, local_unix: true, remote: true },
    ShellInfo { names: &["nu"], title: "nushell", support: ShellSupport::Full, missing: &[NO_HELPERS], local_windows: true, local_unix: true, remote: true },
    ShellInfo { names: &["cmd"], title: "cmd", support: ShellSupport::Partial, missing: &[NO_CODES, NO_HELPERS], local_windows: true, local_unix: false, remote: false },
    ShellInfo { names: &["ksh", "ksh93"], title: "ksh", support: ShellSupport::Partial, missing: &[NO_TEXT, NO_HELPERS], local_windows: false, local_unix: true, remote: true },
    ShellInfo { names: &["mksh"], title: "mksh", support: ShellSupport::Partial, missing: &[NO_TEXT, NO_HELPERS], local_windows: false, local_unix: true, remote: true },
    ShellInfo { names: &["dash"], title: "dash", support: ShellSupport::Partial, missing: &[NO_TEXT, NO_HELPERS], local_windows: false, local_unix: true, remote: true },
    ShellInfo { names: &["ash", "sh"], title: "ash, sh (busybox, a plain POSIX sh)", support: ShellSupport::Partial, missing: &[NO_TEXT, NO_HELPERS, NO_BSD_SH], local_windows: false, local_unix: true, remote: true },
];

/// What weft has in the shell of this name (its program's file name, or a
/// path to it); none it knows: no support.
pub fn shell_info(name: &str) -> Option<&'static ShellInfo> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let name = name.strip_suffix(".exe").unwrap_or(name).to_ascii_lowercase();
    SHELLS.iter().find(|s| s.names.contains(&name.as_str()))
}

pub fn agent_command() -> String {
    posix_wrap(&[("PY64", pythons_b64())], &format!("{FIND_PY}; {AGENT_BOOT}"), NO_DECODER)
}

/// `agent_command` with these Pythons.
pub fn agent_command_of(list: &[String]) -> String {
    posix_wrap(&[("PY64", pythons_b64_of(list))], &format!("{FIND_PY}; {AGENT_BOOT}"), NO_DECODER)
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
        /// The shell it runs commands with: weft's terminal and agent start
        /// in its language. Found by trying, kept here; can be set by hand.
        #[serde(default, skip_serializing_if = "RemoteShell::is_posix")]
        shell: RemoteShell,
        /// weft 0.10.5's word for a host whose shell was not POSIX: read as
        /// `shell: windows` (Profile::remote_shell), not written again.
        #[serde(default, skip_serializing)]
        plain: bool,
    },
    /// Any command that behaves like `ssh HOST CMD` (receives the remote
    /// command string as its last argument), e.g. `["wsl", "-e", "sh", "-c"]`.
    Custom {
        pty: Vec<String>,
        agent: Vec<String>,
        /// As Ssh's.
        #[serde(default, skip_serializing_if = "RemoteShell::is_posix")]
        shell: RemoteShell,
        #[serde(default, skip_serializing)]
        plain: bool,
    },
    /// A serial console (UART): `port` on the host named `via` ("local": a
    /// COM port opened by weft; a remote host: remote/serial.py run there).
    Serial { via: String, port: String, baud: u32 },
}

/// The profile for what was typed in Connect: one of that name, else one
/// for the same host with the same options (under any name), else a new
/// one (added to `profiles`). Its name is returned. (The same host on two
/// ports got one profile, the first port's.)
pub fn connect_profile(profiles: &mut Vec<Profile>, typed: &str) -> String {
    if typed == "local" || profiles.iter().any(|p| p.name == typed) {
        return typed.to_string();
    }
    let new = Profile::ssh(typed);
    let same = |p: &Profile| matches!((&p.kind, &new.kind), (HostKind::Ssh { target: a, args: x, .. }, HostKind::Ssh { target: b, args: y, .. }) if a == b && x == y);
    if let Some(p) = profiles.iter().find(|p| same(p)) {
        return p.name.clone();
    }
    let name = new.name.clone();
    profiles.push(new);
    name
}

/// The name of an ssh profile: the host, ":port" for `-p N` (or `-pN`),
/// then any other options as typed.
pub fn ssh_name(host: &str, args: &[String]) -> String {
    let mut name = host.to_string();
    let mut rest: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-p" && i + 1 < args.len() {
            name = format!("{name}:{}", args[i + 1]);
            i += 2;
            continue;
        }
        match a.strip_prefix("-p").filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
            Some(port) => name = format!("{name}:{port}"),
            None => rest.push(a),
        }
        i += 1;
    }
    if !rest.is_empty() {
        name = format!("{name} {}", rest.join(" "));
    }
    name
}

pub const SERIAL_PY: &str = include_str!("../../../remote/serial.py");

/// Speeds offered for serial consoles.
/// (Any other can be typed: 74880 for an ESP8266's boot, say.)
pub const BAUDS: &[u32] = &[9600, 19200, 38400, 57600, 74880, 115200, 230400, 250000, 460800, 500000, 921600, 1000000, 1500000, 2000000, 3000000];

/// What the bridge prints once the port is open (serial logs keep what
/// was typed only from then on: not an answer or a sudo password before).
pub const SERIAL_READY: &str = "connected to ";

/// The remote command for a serial console: the bridge, sent in the command
/// itself (base64) so nothing is written on the host. When the port may not
/// be opened and sudo is asked for, the bridge ends with 77 and runs again
/// under sudo (its password prompt in the terminal).
pub fn serial_command(port: &str, baud: u32) -> String {
    serial_command_of(port, baud, &PYTHONS.read().unwrap_or_else(|e| e.into_inner()).clone())
}

/// `serial_command` with these Pythons.
pub fn serial_command_of(port: &str, baud: u32, list: &[String]) -> String {
    // Gzipped: the bridge with sudo and its holders grew past what a
    // Windows console takes on a command line (about 8 KB).
    // sudo runs the Python by its path: a name found on the user's PATH
    // (pyenv, conda, ~/.local/bin) is not on sudo's secure_path.
    // In sh whatever the login shell (fish's and tcsh's own words broke
    // it): the script base64 as the agent's, the bridge beside it.
    let src = gz_b64(SERIAL_PY);
    let port = sh_quote(port);
    let script = format!(
        "{FIND_PY}; S=\"import base64,zlib;exec(zlib.decompress(base64.b64decode('$SRC'),47))\"; \"$P\" -c \"$S\" {port} {baud}; s=$?; \
         [ $s = 77 ] && exec sudo -p '[sudo] password for %u: ' \"$(command -v \"$P\")\" -c \"$S\" {port} {baud}; exit $s"
    );
    posix_wrap(&[("PY64", pythons_b64_of(list)), ("SRC", src)], &script, NO_DECODER_SERIAL)
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

    /// An ssh profile from what is typed in Connect ("user@host -p 2222 …").
    /// Its name tells it apart from the same host on another port or with
    /// other options: "user@host:2222" (it was the host alone, so a second
    /// port got the first one's profile).
    pub fn ssh(target: &str) -> Profile {
        let mut parts = target.split_whitespace().map(String::from);
        let host = parts.next().unwrap_or_default();
        let args: Vec<String> = parts.collect();
        Profile { name: ssh_name(&host, &args), kind: HostKind::Ssh { target: host, args, x11: false, shell: RemoteShell::Posix, plain: false } }
    }

    /// The shell a remote host runs commands with (POSIX for this PC and
    /// a serial port, which have none of their own). 0.10.5's `plain` is
    /// Windows.
    pub fn remote_shell(&self) -> RemoteShell {
        match &self.kind {
            HostKind::Ssh { shell, plain, .. } | HostKind::Custom { shell, plain, .. } => {
                if *plain && shell.is_posix() {
                    RemoteShell::Windows
                } else {
                    *shell
                }
            }
            HostKind::Local | HostKind::Serial { .. } => RemoteShell::Posix,
        }
    }

    /// Keep what its shell was found to be.
    pub fn set_remote_shell(&mut self, to: RemoteShell) {
        if let HostKind::Ssh { shell, plain, .. } | HostKind::Custom { shell, plain, .. } = &mut self.kind {
            *shell = to;
            *plain = false;
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self.kind, HostKind::Local)
    }

    /// argv for an interactive pane.  With `password` the ssh password comes
    /// from weft's askpass (see `askpass_env`) instead of the terminal.
    pub fn pty_argv(&self, cwd: Option<&str>, password: bool) -> Vec<String> {
        self.pty_argv_with(cwd, password, None)
    }

    /// `pty_argv`, starting this shell there (a name on the host's PATH or a
    /// path; POSIX hosts) instead of the user's login shell.
    pub fn pty_argv_with(&self, cwd: Option<&str>, password: bool, shell: Option<&str>) -> Vec<String> {
        match &self.kind {
            HostKind::Local => local_shell(LocalShell::Auto).0,
            HostKind::Ssh { target, args, x11, .. } => {
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
                v.extend(self.shell_command(cwd, shell));
                v
            }
            HostKind::Custom { pty, .. } => {
                let mut v = pty.clone();
                v.extend(self.shell_command(cwd, shell));
                v
            }
            // Built from the host it is reached through (App::start_pty).
            HostKind::Serial { .. } => Vec::new(),
        }
    }

    /// The command a terminal of it starts with, in its shell's language
    /// (none: a plain ssh, its own shell as it is).
    fn shell_command(&self, cwd: Option<&str>, shell: Option<&str>) -> Option<String> {
        match self.remote_shell() {
            RemoteShell::Posix => Some(remote_shell_command_with(cwd, shell)),
            RemoteShell::Windows => Some(windows_shell_command(cwd)),
            RemoteShell::Other => None,
        }
    }

    /// argv for a serial console on a remote host: this host's terminal
    /// command with the bridge instead of a shell (a POSIX host's only:
    /// the bridge is termios; none elsewhere, see App::start_serial).
    pub fn serial_argv(&self, port: &str, baud: u32, password: bool) -> Vec<String> {
        self.serial_argv_with(port, baud, password, None)
    }

    /// `serial_argv`, the bridge run by one of these Pythons.
    pub fn serial_argv_with(&self, port: &str, baud: u32, password: bool, pythons: Option<&[String]>) -> Vec<String> {
        if !self.remote_shell().is_posix() {
            return Vec::new();
        }
        let cmd = match pythons {
            Some(list) => serial_command_of(port, baud, list),
            None => serial_command(port, baud),
        };
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
        self.agent_argv_with(password, None)
    }

    /// `agent_argv`, its agent trying these Pythons (none: the process's).
    pub fn agent_argv_with(&self, password: bool, pythons: Option<&[String]>) -> Option<Vec<String>> {
        match &self.kind {
            // A serial console has no file system of its own: served locally.
            HostKind::Local | HostKind::Serial { .. } => None,
            HostKind::Ssh { target, args, .. } => {
                let auth = if password { "NumberOfPasswordPrompts=1" } else { "BatchMode=yes" };
                let mut v: Vec<String> = ["ssh", "-T", "-o", auth, "-o", "ConnectTimeout=10"].iter().map(|s| s.to_string()).collect();
                v.extend(args.iter().cloned());
                v.push(target.clone());
                v.push(self.agent_command(pythons));
                Some(v)
            }
            HostKind::Custom { agent, .. } => {
                let mut v = agent.clone();
                v.push(self.agent_command(pythons));
                Some(v)
            }
        }
    }

    /// The agent's command, in its shell's language.
    fn agent_command(&self, pythons: Option<&[String]>) -> String {
        let process = PYTHONS.read().unwrap_or_else(|e| e.into_inner()).clone();
        let list = pythons.unwrap_or(&process);
        match self.remote_shell() {
            RemoteShell::Windows => windows_agent_command_of(list),
            _ => agent_command_of(list),
        }
    }

    /// The language its terminals' shell most likely speaks, where the
    /// terminal itself does not say (see `ShellLang::of_local`): a Windows
    /// host's PowerShell, this PC's own (PowerShell on Windows, a POSIX sh
    /// on macOS and Linux), else a POSIX sh.
    pub fn shell_lang(&self) -> ShellLang {
        if self.remote_shell() == RemoteShell::Windows && !self.is_local() {
            ShellLang::PowerShell { remote_windows: true }
        } else if self.is_local() && cfg!(windows) {
            ShellLang::PowerShell { remote_windows: false }
        } else {
            ShellLang::Posix
        }
    }

    /// Shell command that changes directory, quoted for this host's shell.
    pub fn cd_command(&self, path: &str) -> String {
        self.shell_lang().cd(path)
    }

    pub fn quote_path(&self, path: &str) -> String {
        self.shell_lang().quote(path)
    }
}

/// What a terminal's shell takes a cd and a path in. (This PC's terminals
/// were all taken for PowerShell: on macOS and Linux "cd here" typed
/// Set-Location into bash, and a path went in with PowerShell's quotes.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShellLang {
    #[default]
    Posix,
    /// A Windows host's paths come as /C:/…: made C:/… for it.
    PowerShell { remote_windows: bool },
    Cmd,
    Nu,
}

impl ShellLang {
    /// A local terminal's, by the program it starts (on Windows: Windows
    /// PowerShell, pwsh, cmd, nu; macOS and Linux start sh, whatever shell
    /// it then runs).
    pub fn of_local(program: &str) -> ShellLang {
        let name = Path::new(program).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        match name.as_str() {
            "powershell" | "pwsh" => ShellLang::PowerShell { remote_windows: false },
            "cmd" => ShellLang::Cmd,
            "nu" => ShellLang::Nu,
            _ => ShellLang::Posix,
        }
    }

    /// The shell weft's command started on a POSIX host, or on macOS and
    /// Linux here, by the name it said: nu and pwsh have their own; the
    /// rest (bash, zsh, fish, the sh family) take a POSIX sh's.
    pub fn of_started(name: &str) -> ShellLang {
        match ShellLang::of_local(name) {
            ShellLang::Cmd => ShellLang::Posix,
            lang => lang,
        }
    }

    pub fn cd(self, path: &str) -> String {
        match self {
            ShellLang::PowerShell { remote_windows } => {
                let p = if remote_windows { windows_path(path) } else { path.to_string() };
                format!("Set-Location -LiteralPath '{}'", ps_single(&p))
            }
            // (No " can be in a Windows path.)
            ShellLang::Cmd => format!("cd /d \"{path}\""),
            ShellLang::Nu => format!("cd {}", nu_quote(path)),
            ShellLang::Posix => format!("cd -- {}", sh_quote(path)),
        }
    }

    pub fn quote(self, path: &str) -> String {
        match self {
            ShellLang::PowerShell { remote_windows } => {
                let p = if remote_windows { windows_path(path) } else { path.to_string() };
                if p.contains([' ', '(', ')', '&', ';', '$', '`', '{', '}', '@', ',', '#', '|', '<', '>']) || p.contains(PS_QUOTES) || p.contains(['"', '“', '”', '„']) {
                    format!("'{}'", ps_single(&p))
                } else {
                    p
                }
            }
            // A / unquoted is a switch to cmd's own commands (dir C:/Users).
            ShellLang::Cmd if !path.chars().all(|c| c.is_ascii_alphanumeric() || "\\._-:".contains(c)) => format!("\"{path}\""),
            ShellLang::Nu if !path.chars().all(|c| c.is_ascii_alphanumeric() || "/\\._-+~:".contains(c)) => nu_quote(path),
            ShellLang::Posix if !path.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+~:@%".contains(c)) => sh_quote(path),
            _ => path.to_string(),
        }
    }
}

/// The characters PowerShell takes for a single quote: each is doubled
/// inside one (not ' alone: C:/Bob’s ended the string at ’).
const PS_QUOTES: [char; 5] = ['\'', '‘', '’', '‚', '‛'];

/// `s` for inside PowerShell's single quotes.
fn ps_single(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        out.push(c);
        if PS_QUOTES.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// A string in nu: double quotes, \ and " escaped.
fn nu_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
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
        return (posix_local_argv(&posix_sh()), Vec::new());
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

/// A local terminal on macOS and Linux: the user's shell ($SHELL) with
/// weft's integration, started as on a remote host, by the same command
/// without ssh (it was the bare shell: no prompt marks, exit codes,
/// folder or w* commands).
pub fn posix_local_argv(sh: &str) -> Vec<String> {
    posix_local_argv_with(sh, None)
}

/// `posix_local_argv`, with this shell (settings "unix_shell") instead of
/// the user's login shell.
pub fn posix_local_argv_with(sh: &str, shell: Option<&str>) -> Vec<String> {
    vec![sh.into(), "-c".into(), remote_shell_command_with(None, shell)]
}

/// The local shell: Windows' by `choice`; on macOS and Linux `unix_shell`
/// (a name or path; empty: the login shell).
pub fn local_shell_with(choice: LocalShell, unix_shell: &str) -> (Vec<String>, Vec<(String, String)>) {
    if !cfg!(windows) {
        return (posix_local_argv_with(&posix_sh(), Some(unix_shell)), Vec::new());
    }
    local_shell(choice)
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
        return vec![std::env::var("SHELL").unwrap_or_else(|_| posix_sh()), "-i".into()];
    };
    vec![shell.into(), "-NoLogo".into(), "-NoExit".into(), "-EncodedCommand".into(), encoded]
}

/// The POSIX sh: the one on PATH (POSIX names no path for it: Termux has
/// none at /bin), /bin/sh where PATH has none.
pub fn posix_sh() -> String {
    which("sh").map(|p| p.display().to_string()).unwrap_or_else(|| "/bin/sh".into())
}

/// Git for Windows' bash, wherever Git is (its bin/ is beside the cmd/ on
/// PATH), or at its usual place.
pub fn git_bash() -> Option<PathBuf> {
    let by_git = which("git.exe").and_then(|g| Some(g.parent()?.parent()?.join("bin").join("bash.exe")));
    by_git.into_iter().chain([PathBuf::from("C:/Program Files/Git/bin/bash.exe")]).find(|p| p.is_file())
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
    ssh_config_hosts_in(&text)
}

/// The `Host` names of an ssh config's text, as ssh reads them: a name in
/// "double quotes" is the name without them (it was handed to ssh quotes
/// and all, which then matched no Host and resolved no address).
pub fn ssh_config_hosts_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let mut it = line.splitn(2, |c: char| c.is_whitespace() || c == '=');
        if it.next().map(|k| k.eq_ignore_ascii_case("host")).unwrap_or(false) {
            let rest = it.next().unwrap_or("").trim_start_matches(|c: char| c.is_whitespace() || c == '=');
            let mut words: Vec<String> = Vec::new();
            let mut cur = String::new();
            let mut quoted = false;
            for c in rest.chars() {
                match c {
                    '"' => quoted = !quoted,
                    c if c.is_whitespace() && !quoted => {
                        if !cur.is_empty() {
                            words.push(std::mem::take(&mut cur));
                        }
                    }
                    c => cur.push(c),
                }
            }
            if !cur.is_empty() {
                words.push(cur);
            }
            for h in words {
                if !h.contains(['*', '?', '!']) && !out.contains(&h) {
                    out.push(h);
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
        DataDir::at(root)
    }

    /// This folder as the data folder (tests each have their own: the
    /// environment is one for the whole process).
    pub fn at(root: impl AsRef<Path>) -> DataDir {
        let root = root.as_ref().to_path_buf();
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
    /// Served by a thread of ours: listing a big folder, a search or a slow
    /// disk never holds up a frame.
    Local { tx: Sender<Value>, rx: Receiver<Value> },
    Remote { stdin: Option<ChildStdin>, rx: Receiver<AgentMsg>, child: Child, tree: crate::pty::Tree },
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
    /// The Pythons its agent tries (settings "remote_python"; none: the
    /// process's list, set_pythons).
    pub pythons: Option<Vec<String>>,
}

impl Conn {
    pub fn open(profile: Profile, data: &DataDir, wake: impl Fn() + Send + Clone + 'static, password: Option<String>) -> Conn {
        Self::open_with_pythons(profile, data, wake, password, None)
    }

    /// `open`, its agent trying these Pythons.
    pub fn open_with_pythons(profile: Profile, data: &DataDir, wake: impl Fn() + Send + Clone + 'static, password: Option<String>, pythons: Option<Vec<String>>) -> Conn {
        let mut conn = Conn {
            pythons,
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
        // A shell weft does not know: the agent cannot start there.
        if self.profile.remote_shell() == RemoteShell::Other {
            self.write_log(&format!("no agent: {UNSUPPORTED_SHELL}"));
            self.status = ConnStatus::Failed(UNSUPPORTED_SHELL.into());
            self.backend = Backend::Dead;
            return;
        }
        let env = password.as_deref().map(askpass_env).unwrap_or_default();
        match self.profile.agent_argv_with(password.is_some(), self.pythons.as_deref()) {
            None => {
                self.status = ConnStatus::Ready;
                self.info = local_hello();
                let (tx, requests) = mpsc::channel::<Value>();
                let (answers, rx) = mpsc::channel::<Value>();
                thread::spawn(move || {
                    // In order, as they were asked; ends when the Conn goes.
                    for req in requests {
                        // git on a thread of its own: a commit waiting on a
                        // hook or a key, a big work tree's status, held up
                        // every other answer here (Files, opening a file).
                        if req["op"] == "git" {
                            let (answers, wake) = (answers.clone(), wake.clone());
                            thread::spawn(move || {
                                let mut res = local_op(&req);
                                res["id"] = req["id"].clone();
                                let _ = answers.send(res);
                                wake();
                            });
                            continue;
                        }
                        let mut res = local_op(&req);
                        res["id"] = req["id"].clone();
                        if answers.send(res).is_err() {
                            break;
                        }
                        wake();
                    }
                });
                self.backend = Backend::Local { tx, rx };
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
            Backend::Local { tx, .. } => {
                if tx.send(req).is_err() {
                    self.backend_failures.push(json!({"id": id, "ok": false, "error": "local backend stopped"}));
                }
            }
            Backend::Remote { stdin: Some(stdin), .. } => {
                if stdin.write_all(line.as_bytes()).and_then(|_| stdin.write_all(b"\n")).and_then(|_| stdin.flush()).is_err()
                {
                    self.status = ConnStatus::Failed("agent pipe closed".into());
                    // Whoever asked gets an answer (a save waits for one).
                    self.backend_failures.push(json!({"id": id, "ok": false, "error": "agent offline: its pipe closed"}));
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
            Backend::Local { rx, .. } => out.extend(rx.try_iter()),
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
                            // weft's own word on why (no Python…) stays the
                            // reason over what ssh says after it.
                            if !self.stderr_tail.starts_with("weft: ") || s.starts_with("weft: ") {
                                self.stderr_tail = s;
                            }
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
        if let Backend::Remote { child, stdin, tree, .. } = &mut self.backend {
            stdin.take();
            if !tree.kill() {
                let _ = child.kill();
            }
        }
    }
}

/// Ship the agent source through stdin: it is exec'd in memory remotely.
/// ssh may have ended before taking it (keys refused: "Permission denied",
/// no python…): that is no error here, what ssh said is read from its
/// stderr and becomes the reason, so a password is asked for. (A Mac's
/// pipe holds 16 KB, less than the agent: there it failed with only "send
/// agent: Broken pipe".)
fn ship_agent(stdin: &mut impl Write) -> Result<(), String> {
    let header = format!("{}\n", AGENT_PY.len());
    match stdin.write_all(header.as_bytes()).and_then(|_| stdin.write_all(AGENT_PY.as_bytes())).and_then(|_| stdin.flush()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(format!("send agent: {e}")),
        Ok(()) => Ok(()),
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
    // It and what it starts (ssh, a ProxyCommand), ended together.
    let tree = crate::pty::Tree::of(child.id());
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    ship_agent(&mut stdin)?;
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
    Ok(Backend::Remote { stdin: Some(stdin), rx, child, tree })
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
        // The OS's own names (not COMPUTERNAME/USERNAME, Windows' only:
        // Linux showed "localhost").
        "host": sysinfo::System::host_name().unwrap_or_else(|_| "localhost".into()),
        "user": user_name(None).unwrap_or_default(),
        "platform": std::env::consts::OS,
    })
}

/// A user's name as the system has it (the password database, NIS, LDAP,
/// whatever it uses): the one with this id, or this process's.
#[cfg(unix)]
pub fn user_name(uid: Option<u32>) -> Option<String> {
    // SAFETY: getpwuid_r fills our buffer; the name is copied out of it.
    unsafe {
        let uid = uid.unwrap_or_else(|| libc::geteuid());
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0u8; 16384];
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr().cast(), buf.len(), &mut out);
        if rc != 0 || out.is_null() || pw.pw_name.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr(pw.pw_name).to_string_lossy().into_owned())
    }
}

#[cfg(not(unix))]
pub fn user_name(_uid: Option<u32>) -> Option<String> {
    std::env::var("USERNAME").ok()
}

/// The user's downloads folder as the system has it: Windows' known folder
/// (wherever it was moved), the XDG user dir on Linux and the BSDs (named
/// in the desktop's language: ~/다운로드), ~/Downloads otherwise (macOS).
pub fn downloads_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Com::CoTaskMemFree;
        use windows_sys::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath};
        // SAFETY: the path is a NUL-ended wide string we free once read.
        unsafe {
            let mut p: windows_sys::core::PWSTR = std::ptr::null_mut();
            let ok = SHGetKnownFolderPath(&FOLDERID_Downloads, 0, std::ptr::null_mut(), &mut p) == 0 && !p.is_null();
            let path = ok.then(|| {
                let len = (0..).take_while(|&i| *p.add(i) != 0).count();
                std::path::PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)))
            });
            CoTaskMemFree(p as _);
            path
        }
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").ok()?;
        let xdg = std::env::var("XDG_DOWNLOAD_DIR").ok().or_else(|| {
            let config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
            user_dir(&std::fs::read_to_string(format!("{config}/user-dirs.dirs")).ok()?, "XDG_DOWNLOAD_DIR", &home)
        });
        Some(xdg.unwrap_or_else(|| format!("{home}/Downloads")).into())
    }
}

/// One entry of user-dirs.dirs (`XDG_DOWNLOAD_DIR="$HOME/다운로드"`), its
/// $HOME put in.
#[cfg_attr(windows, allow(dead_code))]
fn user_dir(file: &str, key: &str, home: &str) -> Option<String> {
    let line = file.lines().map(str::trim).find(|l| l.strip_prefix(key).is_some_and(|r| r.trim_start().starts_with('=')))?;
    let v = line.split_once('=')?.1.trim().trim_matches('"');
    let v = v.replace("$HOME", home);
    (!v.is_empty() && v.trim_end_matches('/') != home.trim_end_matches('/')).then_some(v)
}

/// Whether two paths are one file (not a link to it): asked of the file
/// system, whatever its rules for names are.
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ma.dev() == mb.dev() && ma.ino() == mb.ino()
    }
    #[cfg(not(unix))]
    {
        !ma.file_type().is_symlink() && !mb.file_type().is_symlink() && matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
    }
}

/// A file's type and permissions as ls shows them (drwxr-xr-x); on
/// Windows only its type and whether it may be written.
pub fn file_mode(md: &std::fs::Metadata) -> String {
    let t = md.file_type();
    let kind = if t.is_symlink() { 'l' } else if t.is_dir() { 'd' } else { '-' };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = md.permissions().mode();
        let bit = |b: u32, c: char| if m & b != 0 { c } else { '-' };
        let mut s = String::from(kind);
        for (r, w, x) in [(0o400, 0o200, 0o100), (0o040, 0o020, 0o010), (0o004, 0o002, 0o001)] {
            s.push(bit(r, 'r'));
            s.push(bit(w, 'w'));
            s.push(bit(x, 'x'));
        }
        s
    }
    #[cfg(not(unix))]
    {
        format!("{kind}r{}", if md.permissions().readonly() { '-' } else { 'w' })
    }
}

/// What a git command may print for weft (a diff of a big file): more is
/// cut, and said so.
const GIT_MAX: usize = 4 * 1024 * 1024;

/// git in a folder, its arguments as weft gives them (no shell), with what
/// it prints and its exit code (the agent's `git` on this PC).
fn local_git(req: &Value) -> Result<Value, String> {
    let dir = req.get("dir").and_then(|v| v.as_str()).unwrap_or(".");
    let args: Vec<String> = req.get("args").and_then(|v| v.as_array()).map(|a| a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect()).unwrap_or_default();
    let input = req.get("input").and_then(|v| v.as_str());
    let mut cmd = Command::new("git");
    // Paths as they are ([1] is no pattern, :x no magic), never a prompt,
    // its words in English (weft reads none of them but "not a git
    // repository").
    cmd.arg("-C").arg(dir).args(&args).env("GIT_TERMINAL_PROMPT", "0").env("GIT_EDITOR", "true").env("GIT_LITERAL_PATHSPECS", "1").env("GIT_OPTIONAL_LOCKS", "0").env("LC_ALL", "C");
    cmd.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    // A group of its own: its time limit ends it and what it started (a
    // hook, gpg) with it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "git is not installed here".to_string() } else { e.to_string() })?;
    if let Some(text) = input {
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let text = text.to_string();
        // On a thread: a big input and a big output would wait on each other.
        thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    // Two minutes at most (a hook or a signing key waiting for what never
    // comes), as the agent's.
    let pid = child.id();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let killed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let (done, killed) = (done.clone(), killed.clone());
        thread::spawn(move || {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                if std::time::Instant::now() > end {
                    killed.store(true, std::sync::atomic::Ordering::SeqCst);
                    crate::pty::kill_tree(pid);
                    return;
                }
                thread::sleep(std::time::Duration::from_millis(200));
            }
        });
    }
    // What it prints read up to the most shown, then it is ended (a diff
    // of a 1 GB file never held in memory whole).
    use std::io::Read as _;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let errs = thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stderr.read_to_end(&mut v);
        v
    });
    let mut text = Vec::new();
    let _ = (&mut stdout).take(GIT_MAX as u64 + 1).read_to_end(&mut text);
    let cut = text.len() > GIT_MAX;
    if cut {
        crate::pty::kill_tree(pid);
    }
    drop(stdout);
    let status = child.wait().map_err(|e| e.to_string());
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    let status = status?;
    let stderr = errs.join().unwrap_or_default();
    if killed.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(json!({"code": -1, "out": "", "err": "stopped after 2 minutes: it waited for something (a hook, a signing key's passphrase): run it in a terminal", "cut": false}));
    }
    text.truncate(GIT_MAX);
    let text = String::from_utf8_lossy(&text).into_owned();
    let err = String::from_utf8_lossy(&stderr);
    let err: String = err.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect();
    // (Ended for being long: what it printed is the answer.)
    let code = if cut { 0 } else { status.code().unwrap_or(-1) };
    Ok(json!({"code": code, "out": text, "err": err, "cut": cut}))
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
                let mut item = json!({"n": name, "d": d, "l": link, "s": size, "m": m});
                if req.get("mode").and_then(|v| v.as_bool()).unwrap_or(false) {
                    item["p"] = json!(std::fs::symlink_metadata(e.path()).map(|m| file_mode(&m)).unwrap_or_default());
                }
                entries.push(item);
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
            let num = |k: &str, d: u64| req.get(k).and_then(|v| v.as_u64()).unwrap_or(d) as usize;
            // Here the matcher itself decides (weft's own; a remote host's grep
            // only narrows the lines down).
            // (Glob and fuzzy only: text and regex are what grep finds, an
            // empty match too: ^$ finds the blank lines.)
            let exact = req.get("match").and_then(|m| {
                let mode: crate::find::MatchMode = serde_json::from_value(m["mode"].clone()).ok()?;
                crate::find::Matcher::build(mode, m["query"].as_str()?, crate::find::Target::Line, m["case"].as_bool()).ok()
            }).filter(|m| m.exact_on_grep());
            local_grep(&s("path"), &s("pattern"), flag("icase"), flag("regex"), flag("hidden"), exact.as_ref(), num("limit", 500), num("line_max", 400))
        }
        "git" => local_git(req),
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
        "procs" => local_procs(),
        "kill" => {
            let pid = req.get("pid").and_then(|v| v.as_u64()).ok_or("no pid")?;
            local_kill(pid as u32, &s("sig"))?;
            Ok(json!({"pid": pid}))
        }
        "mkdir" => {
            let p = s("path");
            std::fs::create_dir(&p).map_err(|e| e.to_string())?;
            Ok(json!({"path": p}))
        }
        "create" => {
            let p = s("path");
            OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| e.to_string())?;
            Ok(json!({"path": p}))
        }
        "rename" => {
            let (from, to) = (s("from"), s("to"));
            // Never onto something else (a rename replaces silently). The
            // same file by another name (its letters' case on a file system
            // that ignores case: Windows', macOS') is not something else.
            let same = same_file(Path::new(&from), Path::new(&to));
            if !same && std::fs::symlink_metadata(&to).is_ok() {
                return Err(format!("{} already exists", Path::new(&to).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(to)));
            }
            std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
            Ok(json!({"path": to}))
        }
        "delete" => {
            let p = s("path");
            if p.trim().is_empty() || Path::new(&p).parent().is_none() {
                return Err("refusing to delete a root".into());
            }
            let md = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
            let trash = req.get("trash").and_then(|v| v.as_bool()).unwrap_or(false);
            if trash {
                to_trash(&p)?;
            } else if md.is_dir() {
                // Links inside are removed, not followed.
                std::fs::remove_dir_all(&p).map_err(|e| e.to_string())?;
            } else {
                // A link to a folder is a folder to Windows, a file elsewhere.
                std::fs::remove_file(&p).or_else(|_| std::fs::remove_dir(&p)).map_err(|e| e.to_string())?;
            }
            Ok(json!({"path": p, "trashed": trash}))
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

/// The processes of this PC, in the rows agent.py makes of `ps`: pid,
/// parent, user, CPU % (of the whole machine, as Task Manager counts it),
/// memory %, time running, command line.
fn local_procs() -> Result<Value, String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};
    // Kept between calls: CPU use is what was used since the call before
    // (so the first list shows none).
    static STATE: std::sync::Mutex<Option<(System, Users)>> = std::sync::Mutex::new(None);
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.is_none() {
        let sys = System::new().map_err(|e| format!("{e:?}"))?;
        let users = Users::new_with_refreshed_list().map_err(|e| format!("{e:?}"))?;
        *state = Some((sys, users));
    }
    let Some((sys, users)) = state.as_mut() else { return Err("no process list".into()) };
    sys.refresh_memory();
    let kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_cmd(UpdateKind::OnlyIfNotSet).with_user(UpdateKind::OnlyIfNotSet).without_tasks();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    let cores = thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
    let total = sys.total_memory().max(1) as f64;
    let rows: Vec<Value> = sys
        .processes()
        .iter()
        .map(|(pid, p)| {
            let user = p.user_id().and_then(|u| users.get_user_by_id(u)).map(|u| u.name().to_string()).unwrap_or_default();
            let cmd = p.cmd().iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" ");
            let cmd = if cmd.is_empty() { p.name().to_string_lossy().into_owned() } else { cmd };
            json!([pid.as_u32(), p.parent().map(|p| p.as_u32()).unwrap_or(0), user, p.cpu_usage() as f64 / cores, p.memory() as f64 / total * 100.0, elapsed(p.run_time()), cmd])
        })
        .collect();
    Ok(json!({"procs": rows}))
}

/// Seconds as `ps` writes them: 05:09, 01:02:03, 2-03:04:05.
fn elapsed(t: u64) -> String {
    let (d, h, m, s) = (t / 86400, t % 86400 / 3600, t % 3600 / 60, t % 60);
    match (d, h) {
        (0, 0) => format!("{m:02}:{s:02}"),
        (0, _) => format!("{h:02}:{m:02}:{s:02}"),
        _ => format!("{d}-{h:02}:{m:02}:{s:02}"),
    }
}

/// End a process of this PC. TERM asks it to, where the system has such a
/// thing; Windows has none, and ends it either way.
fn local_kill(pid: u32, sig: &str) -> Result<(), String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System};
    let mut sys = System::new().map_err(|e| format!("{e:?}"))?;
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
    let p = sys.process(pid).ok_or("no such process")?;
    let done = if sig.eq_ignore_ascii_case("KILL") { p.kill() } else { p.kill_with(Signal::Term).unwrap_or_else(|| p.kill()) };
    if done {
        Ok(())
    } else {
        Err("the process could not be ended (it may belong to another user)".into())
    }
}

/// Move a file or folder to the Recycle Bin.
#[cfg(windows)]
fn to_trash(path: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW};
    // The shell wants backslashes and a list ending in two NULs.
    let from: Vec<u16> = path.replace('/', "\\").encode_utf16().chain([0, 0]).collect();
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE as _;
    op.pFrom = from.as_ptr();
    // What can't go to the bin (a network share, a drive without one,
    // something too big for it) would be deleted for good, silently:
    // WANTNUKEWARNING has Windows say so and ask, whatever NOCONFIRMATION.
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING) as _;
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 {
        Err(format!("could not move it to the Recycle Bin (error {rc})"))
    } else if op.fAnyOperationsAborted != 0 {
        Err("not deleted".into())
    } else {
        Ok(())
    }
}

/// Move a file or folder to the trash: on macOS its ~/.Trash (or the
/// disk's .Trashes/<uid>); elsewhere the freedesktop.org trash, the one
/// file managers show and put back from: $XDG_DATA_HOME/Trash on the home's
/// disk, else the disk's own .Trash/<uid> (a sticky folder made for it)
/// or .Trash-<uid>.
#[cfg(not(windows))]
fn to_trash(path: &str) -> Result<(), String> {
    let home = PathBuf::from(std::env::var("HOME").map_err(|_| "no home folder to have a trash in".to_string())?);
    let data = std::env::var("XDG_DATA_HOME").ok().filter(|d| d.starts_with('/')).map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    trash_into(path, &home, &data, cfg!(target_os = "macos"))
}

/// `to_trash` with the home and XDG_DATA_HOME given, macOS' way or the
/// freedesktop.org one.
#[cfg(not(windows))]
fn trash_into(path: &str, home: &Path, data: &Path, mac: bool) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let p = std::path::absolute(path).map_err(|e| e.to_string())?;
    let dev = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?.dev();
    let home = home.to_path_buf();
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    let same_disk = |d: &Path| std::fs::metadata(d).is_ok_and(|m| m.dev() == dev);
    // The top of the disk the file is on: its last parent on that disk.
    let top = || {
        let mut t = p.clone();
        while let Some(up) = t.parent().filter(|u| same_disk(u)) {
            t = up.to_path_buf();
        }
        t
    };
    let name = p.file_name().ok_or("refusing to trash a root")?.to_string_lossy().into_owned();
    if mac {
        let trash = if same_disk(&home) { home.join(".Trash") } else { top().join(".Trashes").join(uid.to_string()) };
        std::fs::create_dir_all(&trash).map_err(|e| format!("{}: {e}", trash.display()))?;
        let to = (1..).map(|k| trash.join(trash_name(&name, k))).find(|t| std::fs::symlink_metadata(t).is_err()).expect("a free name");
        return std::fs::rename(&p, &to).map_err(|e| e.to_string());
    }
    let (trash, relative) = if same_disk(data) || (!data.exists() && same_disk(&home)) {
        (data.join("Trash"), None)
    } else {
        let top = top();
        let shared = top.join(".Trash");
        let sticky = std::fs::symlink_metadata(&shared).is_ok_and(|m| m.is_dir() && m.mode() & 0o1000 != 0);
        (if sticky { shared.join(uid.to_string()) } else { top.join(format!(".Trash-{uid}")) }, Some(top))
    };
    for d in [trash.join("files"), trash.join("info")] {
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    // The name taken by making its .trashinfo (made new: two trashing at
    // once never take one name), then the file moved in.
    let shown = match &relative {
        Some(top) => p.strip_prefix(top).map(Path::to_path_buf).unwrap_or_else(|_| p.clone()),
        None => p.clone(),
    };
    let info = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", trash_path_url(&shown.to_string_lossy()), chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"));
    for k in 1.. {
        let n = trash_name(&name, k);
        let info_file = trash.join("info").join(format!("{n}.trashinfo"));
        match OpenOptions::new().write(true).create_new(true).open(&info_file) {
            Ok(mut f) => {
                f.write_all(info.as_bytes()).map_err(|e| e.to_string())?;
                return std::fs::rename(&p, trash.join("files").join(&n)).map_err(|e| {
                    let _ = std::fs::remove_file(&info_file);
                    e.to_string()
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", info_file.display())),
        }
    }
    unreachable!()
}

/// A name in the trash: as it was, then "name.2", "name.3"…
#[cfg_attr(windows, allow(dead_code))]
fn trash_name(name: &str, k: u32) -> String {
    if k == 1 { name.to_string() } else { format!("{name}.{k}") }
}

/// A path as .trashinfo writes it: percent-encoded but for its slashes and
/// the characters URLs leave as they are.
#[cfg_attr(windows, allow(dead_code))]
fn trash_path_url(p: &str) -> String {
    p.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
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
    // Written on a thread of its own while the answer is read: git answers
    // as it reads, and with more names than fit the pipes (a folder of
    // thousands of ignored files) each side waited for the other for good.
    let input = names.join("\0").into_bytes();
    let writer = child.stdin.take().map(|mut stdin| {
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let out = child.wait_with_output();
    if let Some(w) = writer {
        let _ = w.join();
    }
    let Ok(out) = out else { return Default::default() };
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
/// Every name under `root`, the nearest first (a folder at a time, by
/// depth: at the limit what is cut is the deepest, not a folder next to
/// the root that a big one listed before it crowded out); a link to a
/// folder is gone into (~/work → /mnt/…), each real folder once (a link
/// back up goes nowhere).
fn local_tree(root: &str, hidden: bool, limit: usize) -> Value {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([String::new()]);
    let mut seen = std::collections::HashSet::new();
    if let Ok(real) = std::fs::canonicalize(root) {
        seen.insert(real);
    }
    while let Some(rel) = queue.pop_front() {
        let dir = if rel.is_empty() { PathBuf::from(root) } else { Path::new(root).join(&rel) };
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_hidden(&e, &name, hidden) {
                continue;
            }
            let link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
            let is_dir = if link { std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false) } else { e.file_type().map(|t| t.is_dir()).unwrap_or(false) };
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            out.push(json!([r, is_dir]));
            if out.len() >= limit {
                return json!({"root": root, "entries": out, "truncated": true});
            }
            if is_dir && !TREE_SKIP.contains(&name.as_str()) && std::fs::canonicalize(e.path()).map(|p| seen.insert(p)).unwrap_or(!link) {
                queue.push_back(r);
            }
        }
    }
    json!({"root": root, "entries": out, "truncated": false})
}

/// Search file contents under `root`: the text as typed, or with `regex` a
/// regular expression (the extended syntax the agent's `grep -E` takes).
/// Lines under `root` that `pattern` finds (as grep -E), each decided by
/// `exact` when given; `limit` of them, each at most `line_max` characters.
#[allow(clippy::too_many_arguments)]
fn local_grep(root: &str, pattern: &str, icase: bool, regex: bool, hidden: bool, exact: Option<&crate::find::Matcher>, limit: usize, line_max: usize) -> Result<Value, String> {
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
                if re.is_match(line) && exact.filter(|m| m.exact_on_grep()).is_none_or(|m| !m.find_in(line).is_empty()) {
                    let t: String = line.chars().take(line_max).collect();
                    hits.push(json!([p.display().to_string().replace('\\', "/"), i + 1, t]));
                    if hits.len() >= limit {
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

    /// One file by two names is told by the file system: its case on one
    /// that ignores case; two files are two; a missing one is none.
    #[test]
    fn the_same_file_is_asked_of_the_file_system() {
        let d = std::env::temp_dir().join(format!("weft-same-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::write(d.join("b.txt"), "b").unwrap();
        assert!(same_file(&d.join("a.txt"), &d.join("a.txt")));
        assert!(!same_file(&d.join("a.txt"), &d.join("b.txt")));
        assert!(!same_file(&d.join("a.txt"), &d.join("none.txt")));
        let ignores_case = d.join("A.TXT").exists();
        assert_eq!(same_file(&d.join("a.txt"), &d.join("A.TXT")), ignores_case);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The trash, freedesktop.org's: the file in files/, its .trashinfo
    /// saying where it was (percent-encoded) and when; a second of the same
    /// name as name.2. macOS': into ~/.Trash.
    #[test]
    #[cfg(unix)]
    fn deleting_to_the_trash_keeps_where_it_was() {
        let d = std::env::temp_dir().join(format!("weft-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (home, data) = (d.join("home"), d.join("home/.local/share"));
        std::fs::create_dir_all(home.join("work")).unwrap();
        for _ in 0..2 {
            std::fs::write(home.join("work/a b%.txt"), "x").unwrap();
            trash_into(&home.join("work/a b%.txt").display().to_string(), &home, &data, false).unwrap();
        }
        assert!(!home.join("work/a b%.txt").exists(), "moved out");
        assert!(data.join("Trash/files/a b%.txt").is_file() && data.join("Trash/files/a b%.txt.2").is_file(), "both in, two names");
        let info = std::fs::read_to_string(data.join("Trash/info/a b%.txt.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=") && info.contains("/work/a%20b%25.txt\n") && info.contains("DeletionDate=20"), "{info}");
        // A folder, macOS' way.
        std::fs::create_dir_all(home.join("work/dir/in")).unwrap();
        trash_into(&home.join("work/dir").display().to_string(), &home, &data, true).unwrap();
        assert!(home.join(".Trash/dir/in").is_dir(), "a folder whole in ~/.Trash");
        // The agent's, for a remote Linux or Mac: the same.
        let agent_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../remote");
        std::fs::write(home.join("work/b.txt"), "x").unwrap();
        let code = format!("import sys; sys.dont_write_bytecode = True; sys.path.insert(0, {agent_dir:?}); import agent; agent._trash({:?}, {:?}, False)", home.join("work/b.txt").display().to_string(), home.display().to_string());
        let out = Command::new(crate::pty::tests::python()).args(["-c", &code]).env("XDG_DATA_HOME", &data).stdin(Stdio::null()).output().unwrap();
        assert!(data.join("Trash/files/b.txt").is_file() && std::fs::read_to_string(data.join("Trash/info/b.txt.trashinfo")).unwrap().contains("/work/b.txt"), "the agent's trash: {}", String::from_utf8_lossy(&out.stderr));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// weft's git takes a path as it is: discarding b[1].txt leaves b1.txt
    /// alone (as a pattern, [1] matched b1.txt too and threw its change
    /// away), and a name starting ":" is a name.
    #[test]
    fn git_paths_are_taken_as_they_are() {
        let dir = std::env::temp_dir().join(format!("weft-gitpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.display().to_string();
        let git = |args: &[&str]| local_op(&json!({"op": "git", "dir": d, "args": args}));
        if git(&["init", "-q"])["code"] != 0 {
            return; // no git here
        }
        for (k, v) in [("user.name", "t"), ("user.email", "t@example.com"), ("commit.gpgsign", "false")] {
            git(&["config", k, v]);
        }
        for n in ["b[1].txt", "b1.txt", ":x.txt"] {
            std::fs::write(dir.join(n), "one\n").unwrap_or_default();
        }
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "first"]);
        for n in ["b[1].txt", "b1.txt"] {
            std::fs::write(dir.join(n), "two\n").unwrap();
        }
        assert_eq!(git(&["checkout", "-q", "--", "b[1].txt"])["code"], 0);
        assert_eq!(std::fs::read_to_string(dir.join("b[1].txt")).unwrap().trim(), "one", "that one back");
        assert_eq!(std::fs::read_to_string(dir.join("b1.txt")).unwrap().trim(), "two", "the other left alone");
        if dir.join(":x.txt").exists() {
            std::fs::write(dir.join(":x.txt"), "two\n").unwrap();
            assert_eq!(git(&["add", "--", ":x.txt"])["code"], 0, "a name, not magic");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The downloads folder is the system's: an XDG user dir in the
    /// desktop's language, $HOME put in; Windows' known folder is there.
    #[test]
    fn the_downloads_folder_is_the_systems() {
        let dirs = "# written by xdg-user-dirs-update
XDG_DESKTOP_DIR=\"$HOME/바탕화면\"
XDG_DOWNLOAD_DIR=\"$HOME/다운로드\"
";
        assert_eq!(user_dir(dirs, "XDG_DOWNLOAD_DIR", "/home/u").as_deref(), Some("/home/u/다운로드"));
        assert_eq!(user_dir(dirs, "XDG_MUSIC_DIR", "/home/u"), None);
        // Set to the home itself: none of its own (the default then).
        assert_eq!(user_dir("XDG_DOWNLOAD_DIR=\"$HOME/\"
", "XDG_DOWNLOAD_DIR", "/home/u/"), None);
        let d = downloads_dir().expect("a downloads folder");
        if cfg!(windows) {
            assert!(d.is_absolute() && d.is_dir(), "{d:?}");
        }
    }

    #[test]
    fn local_grep_modes_and_hidden() {
        use crate::find::{MatchMode, Matcher, Target};
        let d = std::env::temp_dir().join(format!("weft-grep-modes-{}", std::process::id()));
        std::fs::create_dir_all(d.join(".hid")).unwrap();
        std::fs::write(d.join("a.txt"), "git status\nvoid Start(int x);\n\nstash it\n").unwrap();
        std::fs::write(d.join(".env"), "git status hidden\n").unwrap();
        std::fs::write(d.join(".hid").join("b.txt"), "git status in hidden dir\n").unwrap();
        let root = d.display().to_string();
        // As the Search tab does it: grep asked for what may match, the
        // matcher deciding (glob and fuzzy).
        let search = |mode: MatchMode, q: &str, icase: bool, hidden| -> Vec<String> {
            let m = Matcher::build(mode, q, Target::Line, Some(!icase)).unwrap();
            let (p, regex, icase) = m.grep();
            let v = local_grep(&root, &p, icase, regex, hidden, Some(&m), 500, 400).unwrap();
            let mut l: Vec<String> = v["hits"].as_array().unwrap().iter().map(|h| h[2].as_str().unwrap().to_string()).collect();
            l.sort();
            l
        };
        assert_eq!(search(MatchMode::Fuzzy, "gst", true, false), ["git status"]);
        assert_eq!(search(MatchMode::Glob, "*Start*(int*", false, false), ["void Start(int x);"], "a glob: the whole line");
        assert_eq!(search(MatchMode::Glob, "st?sh*", false, false), ["stash it"]);
        assert!(search(MatchMode::Glob, "st?sh", false, false).is_empty(), "not a part of a line");
        assert_eq!(search(MatchMode::Glob, "stash", false, false), ["stash it"], "no wildcard: anywhere");
        assert_eq!(search(MatchMode::Regex, r"^(git|stash) ", false, false), ["git status", "stash it"]);
        assert_eq!(search(MatchMode::Regex, r"^$", false, false), [""], "an empty match: the blank line");
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
            let v = local_grep(&root, p, icase, rx, false, None, 500, 400)?;
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
        // What the app knows the no-decoder host by: in what sh prints, not
        // in the command's own text (Windows PowerShell 5.1 prints that
        // back when it refuses it, and it was taken for weft's command run).
        assert!(PLAIN_SHELL.replace("$()", "").contains(NO_DECODER_SAID) && NO_DECODER.replace("$()", "").contains(NO_DECODER_SAID));
        for cmd in [remote_shell_command(None), remote_shell_command_with(None, Some("fish"))] {
            assert!(!cmd.contains(NO_DECODER_SAID), "the words are not in the command itself");
        }
        assert!(SHELL_UNPACK.starts_with(r#"printf '\033]7777;{"t":"sh"}\007'; "#), "it says it runs before anything else");
        // The boot section runs in the unpacker's shell: it must not set the
        // names the unpacker keeps (the bundle in WB once had the bash
        // integration read back empty).
        for name in ["W", "WB", "D", "B", "S", "BT", "t", "z", "d", "sect"] {
            let set = [format!("; {name}="), format!(" {name}=$"), format!("{name}() {{")];
            assert!(!set.iter().any(|s| SHELL_BOOT.contains(s.as_str())), "the boot section sets {name}, the unpacker's");
        }
        let n = remote_shell_command(Some("/home/someone/projects/a long directory name")).len();
        println!("remote shell command: {n} bytes");
        assert!(n < 7000, "{n} bytes");
        // With a shell chosen by its path too.
        let n = remote_shell_command_with(Some("/home/someone/projects/a long directory name"), Some("/opt/homebrew/Cellar/fish/3.7.1/bin/fish")).len();
        println!("with a chosen shell: {n} bytes");
        assert!(n < 7000, "with a chosen shell: {n} bytes");
    }

    /// cmd's and PowerShell's refusal of `exec`, in the languages' own
    /// quotes; not weft's own command or a word that has exec in it.
    #[test]
    fn a_refusal_of_exec_is_told_in_any_language() {
        for said in [
            "'exec' is not recognized as an internal or external command,",
            "'exec'은(는) 내부 또는 외부 명령, 실행할 수 있는 프로그램, 또는",
            "Der Befehl \"exec\" ist entweder falsch geschrieben oder",
            "Die Benennung \"exec\" wurde nicht als Name eines Cmdlet",
            "Příkaz „exec“ nebyl nalezen",
            "exec : The term 'exec' is not recognized as the name of a cmdlet",
            // French: a no-break space inside « ».
            "Le terme «\u{a0}exec\u{a0}» n'est pas reconnu comme nom d'applet de commande",
            "« exec » n'est pas reconnu",
        ] {
            assert!(says_exec_unknown(said), "{said}");
        }
        for said in ["exec sh -c 'printf hi'", "executable not found", "sh: exec: line 1: no such file", "\"execute\""] {
            assert!(!says_exec_unknown(said), "{said}");
        }
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
        for (name, src) in [("boot", SHELL_BOOT), ("bash", BASH_SH), ("zsh", ZSH_ZSH), ("fish", FISH_FISH), ("pwsh", PWSH_PS1), ("sh", SH_SH)] {
            assert_eq!(sect(name), strip_comments(src), "{name}");
            assert!(!src.lines().any(|l| l.starts_with("#@")), "{name} must not contain section markers");
        }
        assert!(ZSH_ZSH.contains("precmd_functions") && ZSH_ZSH.contains("preexec_functions"));
        assert!(FISH_FISH.contains("fish_preexec") && FISH_FISH.contains("fish_prompt"));
        // bash's helpers need no Python on the host (wls is listed by the
        // agent, wtable split by weft), nor base64 for their events.
        let bash = strip_comments(BASH_SH);
        assert!(!bash.contains("python") && !bash.contains("__weft_py"), "no shell-side Python");
        assert!(!bash.lines().any(|l| l.contains("__weft_ev") && l.contains("base64")), "events as plain JSON");
    }

    /// A Windows host's terminal and agent commands: PowerShell, from cmd
    /// or PowerShell alike, saying it started, in the folder asked, with
    /// weft's integration; the agent finding Python by the same list and
    /// saying the same when there is none (but `where python`). 0.10.5's
    /// "plain" is a Windows host. Its folders are Windows' paths.
    #[test]
    fn windows_hosts_get_powershell() {
        let decode = |cmd: &str| -> String {
            let b64 = cmd.rsplit(' ').next().unwrap();
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
            String::from_utf16(&bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap()
        };
        let term = windows_shell_command(Some("/C:/Users/me/it's"));
        assert!(term.starts_with("powershell -NoLogo -NoExit -EncodedCommand "));
        let script = decode(&term);
        assert!(script.contains("]7777;{\"t\":\"shell\""), "it says it started");
        assert!(script.contains("Set-Location -LiteralPath 'C:/Users/me/it''s'"), "in the folder: {script}");
        assert!(script.contains("133;A") && script.contains("]7;file://"), "weft's integration");
        let agent = decode(&windows_agent_command_of(&["py".into(), "C:/Python3*/python.exe".into()]));
        assert!(agent.contains(&format!("FromBase64String('{}')", b64("py\nC:/Python3*/python.exe"))));
        assert!(agent.contains(NO_PYTHON) && agent.contains("where python") && agent.contains("Python on remote hosts"));
        assert_eq!(windows_path("/C:/Users/me"), "C:/Users/me");
        assert_eq!(windows_path("/c/Users/me"), "c:/Users/me");
        assert_eq!(windows_path("/d"), "d:/");
        assert_eq!(windows_path("C:/x"), "C:/x");
        // The kinds of shell, kept; 0.10.5's plain read as Windows.
        let old: Profile = serde_json::from_str(r#"{"name": "w", "kind": "ssh", "target": "u@w", "plain": true}"#).unwrap();
        assert_eq!(old.remote_shell(), RemoteShell::Windows);
        assert!(old.pty_argv(None, false).last().unwrap().starts_with("powershell "), "its terminal is PowerShell");
        assert!(old.agent_argv(false).unwrap().last().unwrap().starts_with("powershell "), "and its agent");
        assert!(old.serial_argv("COM3", 9600, false).is_empty(), "no serial bridge (termios)");
        assert_eq!(old.cd_command("/C:/x y"), "Set-Location -LiteralPath 'C:/x y'");
        // Each shell's cd and quotes; this PC's by its program.
        assert_eq!(ShellLang::Posix.cd("/a b/it's"), r"cd -- '/a b/it'\''s'");
        assert_eq!(ShellLang::Posix.quote("/tmp/$x*"), "'/tmp/$x*'");
        assert_eq!(ShellLang::Posix.quote("/usr/lib"), "/usr/lib");
        assert_eq!(ShellLang::PowerShell { remote_windows: false }.quote("C:/$x"), "'C:/$x'");
        assert_eq!(ShellLang::Cmd.cd("C:/a b"), "cd /d \"C:/a b\"");
        assert_eq!(ShellLang::Cmd.quote("C:/a&b"), "\"C:/a&b\"");
        assert_eq!(ShellLang::Nu.cd(r#"C:\a "b""#), r#"cd "C:\\a \"b\"""#);
        assert_eq!(ShellLang::of_local("C:/Windows/System32/cmd.exe"), ShellLang::Cmd);
        assert_eq!(ShellLang::of_local("nu"), ShellLang::Nu);
        assert_eq!(ShellLang::of_local("pwsh.exe"), ShellLang::PowerShell { remote_windows: false });
        assert_eq!(ShellLang::of_local("/bin/sh"), ShellLang::Posix);
        // PowerShell's other single quotes are doubled too (C:/Bob’s ended
        // the string at ’); a Windows host's /C:/… is C:/… in a path too.
        let ps = ShellLang::PowerShell { remote_windows: false };
        assert_eq!(ps.quote("C:/Bob’s"), "'C:/Bob’’s'");
        assert_eq!(ps.cd("C:/it's"), "Set-Location -LiteralPath 'C:/it''s'");
        assert_eq!(ShellLang::PowerShell { remote_windows: true }.quote("/C:/Users/x"), "C:/Users/x");
        // cmd: a / unquoted is a switch (dir C:/Users).
        assert_eq!(ShellLang::Cmd.quote("C:/Users"), "\"C:/Users\"");
        assert_eq!(ShellLang::Cmd.quote("notes.txt"), "notes.txt");
        // By the name the boot says: nu and pwsh have their own.
        assert_eq!(ShellLang::of_started("nu"), ShellLang::Nu);
        assert_eq!(ShellLang::of_started("/usr/bin/pwsh"), ShellLang::PowerShell { remote_windows: false });
        assert_eq!(ShellLang::of_started("fish"), ShellLang::Posix);
        assert_eq!(ShellLang::Nu.cd("/home/u/it's"), "cd \"/home/u/it's\"", "no --: nu has no such flag");
        let saved = serde_json::to_string(&{
            let mut p = old.clone();
            p.set_remote_shell(RemoteShell::Windows);
            p
        })
        .unwrap();
        assert!(saved.contains("\"shell\":\"windows\"") && !saved.contains("plain"), "{saved}");
        let mut other = Profile::ssh("u@h");
        assert_eq!(other.remote_shell(), RemoteShell::Posix);
        assert!(!serde_json::to_string(&other).unwrap().contains("shell"), "POSIX is not written");
        other.set_remote_shell(RemoteShell::Other);
        assert_eq!(other.pty_argv(None, false).last().unwrap(), "u@h", "a plain ssh");
    }

    /// What weft puts on a command line stays well under the limits there:
    /// Windows' console (ConPTY) takes about 8 KB, cmd 8191 characters.
    /// (The agent itself goes through ssh's stdin, any size: a pipe's
    /// buffer only says how much at once.)
    #[test]
    fn the_commands_fit_a_command_line() {
        let long_cwd = "/home/someone/a/fairly/long/path/to/a/project/folder/with/more/in/it";
        for (what, cmd) in [
            ("POSIX terminal", remote_shell_command(Some(long_cwd))),
            ("POSIX agent", agent_command()),
            ("Windows terminal", windows_shell_command(Some(long_cwd))),
            ("Windows agent", windows_agent_command()),
            ("serial bridge", serial_command("/dev/ttyUSB0", 115200)),
        ] {
            assert!(cmd.len() < 8000, "{what}: {} bytes", cmd.len());
        }
    }

    /// weft's bash reads a login shell's startup files as bash does: the
    /// system's (/etc/profile) first, then the first of ~/.bash_profile,
    /// ~/.bash_login and ~/.profile; ~/.bash_logout as it ends (issue #5:
    /// a server whose /etc/profile.d loads ~/.bashrc got none of it).
    #[test]
    fn bash_reads_a_login_shells_files() {
        use std::io::Write;
        let bash = if cfg!(windows) { PathBuf::from("C:/Program Files/Git/bin/bash.exe") } else { which("bash").unwrap_or_default() };
        if !bash.is_file() {
            return;
        }
        let slash = |p: &std::path::Path| p.display().to_string().replace('\\', "/");
        let dir = std::env::temp_dir().join(format!("weft-login-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        // In place of /etc/profile: a system's that loads ~/.bashrc.
        let etc = dir.join("profile");
        std::fs::write(&etc, "SYS=1\n[ -r ~/.bashrc ] && . ~/.bashrc\n").unwrap();
        std::fs::write(home.join(".bashrc"), "RC=1\n").unwrap();
        std::fs::write(home.join(".bash_logout"), "echo bye > \"$HOME/logged-out\"\n").unwrap();
        let rc = dir.join("rc");
        std::fs::write(&rc, BASH_SH.replace("/etc/profile", &slash(&etc))).unwrap();
        let got = || {
            let mut child = std::process::Command::new(&bash)
                .args(["--rcfile", &slash(&rc), "-i"])
                .env("HOME", slash(&home))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(b"echo \"got SYS=$SYS RC=$RC P=$P L=$L Q=$Q\"; exit\n").unwrap();
            let out = String::from_utf8_lossy(&child.wait_with_output().unwrap().stdout).to_string();
            out.lines().find(|l| l.starts_with("got ")).unwrap_or_default().to_string()
        };
        // No startup file of the user's: the system's, and the ~/.bashrc it loads.
        assert_eq!(got(), "got SYS=1 RC=1 P= L= Q=");
        assert!(home.join("logged-out").is_file(), "~/.bash_logout as it ended");
        // The user's: the first that is there, only.
        std::fs::write(home.join(".bash_login"), "L=1\n").unwrap();
        std::fs::write(home.join(".profile"), "Q=1\n").unwrap();
        assert_eq!(got(), "got SYS=1 RC=1 P= L=1 Q=");
        std::fs::write(home.join(".bash_profile"), "P=1\n").unwrap();
        assert_eq!(got(), "got SYS=1 RC=1 P=1 L= Q=");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The agent's parts for a Windows host, run here on Windows' Python:
    /// search with no grep, processes, memory and disk.
    #[test]
    #[cfg(windows)]
    fn the_agent_runs_on_windows() {
        let Some(py) = which("python.exe").filter(|p| !p.display().to_string().contains("WindowsApps")) else { return };
        let dir = std::env::temp_dir().join(format!("weft-win-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("src").join("a.txt"), "one\nTwo needle\nthree\n").unwrap();
        std::fs::write(dir.join(".hidden").join("b.txt"), "needle\n").unwrap();
        std::fs::write(dir.join("bin.dat"), b"needle\0\x01").unwrap();
        let probe = format!(
            r#"
import json, os, sys
src = open(r'{agent}', encoding='utf-8').read().replace('\nmain()\n', '\n')
g = {{'__name__': 'agent'}}
exec(compile(src, 'agent.py', 'exec'), g)
hits = g['_grep_python'](r'{dir}', 'NEEDLE', True, False, 50)
procs = g['op_procs']({{}})['procs']
mine = [p for p in procs if p[0] == os.getpid()]
s = g['op_sys']({{'path': r'{dir}'}})
print(json.dumps({{'hits': hits, 'mine': mine, 'sys': s, 'path': g['_path']('/C:/Windows')}}))
"#,
            agent = concat!(env!("CARGO_MANIFEST_DIR"), "/../../remote/agent.py"),
            dir = dir.display()
        );
        let out = Command::new(py).args(["-c", &probe]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        let hits = v["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "one hit: not in .hidden, not in a binary file: {hits:?}");
        assert!(hits[0][0].as_str().unwrap().ends_with("a.txt") && hits[0][1] == 2 && hits[0][2] == "Two needle");
        let mine = v["mine"].as_array().unwrap();
        assert_eq!(mine.len(), 1, "its own process among them");
        assert!(mine[0][6].as_str().unwrap().to_lowercase().contains("python"), "{mine:?}");
        assert!(v["sys"]["mem_total"].as_u64().unwrap_or(0) > 0 && v["sys"]["disk_total"].as_u64().unwrap_or(0) > 0, "{}", v["sys"]);
        assert_eq!(v["path"].as_str().unwrap().to_lowercase(), "c:\\windows");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Host names in ~/.ssh/config, quoted ones without their quotes.
    #[test]
    fn ssh_config_host_names() {
        let text = "Host pi  \"54:22008__FUSE_TEST_r8\" *.lan\n  HostName 10.0.0.2\nhost=other\nHost \"my board\"\nHost !bad\nMatch all\n";
        assert_eq!(ssh_config_hosts_in(text), vec!["pi", "54:22008__FUSE_TEST_r8", "other", "my board"]);
    }

    /// One host on two ports is two profiles, each running ssh with its own
    /// port. (Both got the first one's: the profile was named by the host
    /// alone, and the second found the first's name.)
    #[test]
    fn one_host_on_two_ports_is_two_profiles() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(ssh_name("u@h", &[]), "u@h");
        assert_eq!(ssh_name("u@h", &args(&["-p", "22024"])), "u@h:22024");
        assert_eq!(ssh_name("u@h", &args(&["-p22008"])), "u@h:22008");
        assert_eq!(ssh_name("u@h", &args(&["-i", "key", "-p", "2"])), "u@h:2 -i key");
        let mut profiles = vec![Profile { name: "local".into(), kind: HostKind::Local }];
        let a = connect_profile(&mut profiles, "u@10.20.20.54 -p 22008");
        let b = connect_profile(&mut profiles, "u@10.20.20.54 -p 22024");
        assert_eq!((a.as_str(), b.as_str()), ("u@10.20.20.54:22008", "u@10.20.20.54:22024"));
        let port = |name: &str| {
            let p = profiles.iter().find(|p| p.name == name).unwrap();
            let argv = p.pty_argv(None, false);
            argv.iter().position(|a| a == "-p").map(|i| argv[i + 1].clone())
        };
        assert_eq!((port(&a), port(&b)), (Some("22008".into()), Some("22024".into())), "each ssh with its own port");
        // Again: the same profiles, none added; by name too.
        assert_eq!(connect_profile(&mut profiles, "u@10.20.20.54   -p 22024"), b);
        assert_eq!(connect_profile(&mut profiles, "u@10.20.20.54:22008"), a);
        assert_eq!(profiles.len(), 3);
        // A profile made before, named by the host alone, is found by its
        // host and options, under that name.
        let mut old = vec![Profile { name: "u@h".into(), kind: HostKind::Ssh { target: "u@h".into(), args: args(&["-p", "22008"]), x11: false, shell: RemoteShell::Posix, plain: false } }];
        assert_eq!(connect_profile(&mut old, "u@h -p 22008"), "u@h");
        assert_eq!(connect_profile(&mut old, "u@h -p 22024"), "u@h:22024");
        assert_eq!(old.len(), 2);
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

    /// Procs on this PC: the list has this very process, in `ps`-like rows,
    /// and kill ends one.
    #[test]
    fn local_processes_list_and_kill() {
        let v = local_op(&json!({"op": "procs"}));
        assert_eq!(v["ok"], json!(true), "{v}");
        let rows = v["procs"].as_array().unwrap();
        let me = std::process::id() as u64;
        let mine = rows.iter().find(|r| r[0].as_u64() == Some(me)).expect("this process is listed");
        assert_eq!(mine.as_array().unwrap().len(), 7);
        assert!(mine[6].as_str().unwrap().to_lowercase().contains("weft"), "its command line: {mine}");
        assert!(mine[3].is_number() && mine[4].as_f64().unwrap() > 0.0, "cpu and memory: {mine}");
        assert!(mine[5].as_str().unwrap().contains(':'), "time running: {mine}");
        assert_eq!((elapsed(309), elapsed(3723), elapsed(183845)), ("05:09".to_string(), "01:02:03".to_string(), "2-03:04:05".to_string()));

        let mut child = Command::new(crate::pty::tests::python()).args(["-c", "import time; time.sleep(120)"]).spawn().expect("python");
        let k = local_op(&json!({"op": "kill", "pid": child.id(), "sig": "KILL"}));
        assert_eq!(k["ok"], json!(true), "{k}");
        let t = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(t.elapsed().as_secs() < 10, "the process ended");
            thread::sleep(std::time::Duration::from_millis(50));
        }
        let gone = local_op(&json!({"op": "kill", "pid": 0xFFFF_FFF0u32, "sig": "TERM"}));
        assert_eq!(gone["ok"], json!(false));
    }

    /// A folder with thousands of ignored files is still listed (asking git
    /// about them all at once used to wait for ever).
    #[test]
    fn a_folder_of_thousands_of_ignored_files_is_listed() {
        let dir = std::env::temp_dir().join(format!("weft-ignored-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| Command::new("git").arg("-C").arg(&dir).args(args).output().map(|o| o.status.success()).unwrap_or(false);
        if !git(&["init", "-q"]) {
            eprintln!("no git: skipped");
            return;
        }
        std::fs::write(dir.join(".gitignore"), "*.tmp\n").unwrap();
        for i in 0..4000 {
            std::fs::write(dir.join(format!("a-rather-long-name-for-a-build-product-number-{i:05}.tmp")), "").unwrap();
        }
        std::fs::write(dir.join("kept.txt"), "").unwrap();
        let path = dir.display().to_string().replace('\\', "/");
        let (done, wait) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(local_op(&json!({"op": "list", "path": path, "hidden": true, "ignored": false})));
        });
        let v = wait.recv_timeout(std::time::Duration::from_secs(60)).expect("the listing came back");
        // In the folder's own order (the panel sorts): ext4's is not by name.
        let mut names: Vec<&str> = v["entries"].as_array().unwrap().iter().filter_map(|e| e["n"].as_str()).filter(|n| *n != ".git").collect();
        names.sort();
        assert_eq!(names, [".gitignore", "kept.txt"], "the 4000 ignored ones are left out");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same file operations as the remote agent does them (agent.py,
    /// run here by python): what happens, and what is refused, matches
    /// this PC's.
    #[test]
    fn agent_file_operations_match_the_local_ones() {
        let dir = std::env::temp_dir().join(format!("weft-agentops-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = r#"
import json, os, sys
src = open(sys.argv[1], encoding='utf-8').read().split('\ndef main')[0]
exec(src)
root = sys.argv[2]
at = lambda n: os.path.join(root, n)
def run(op, **req):
    try:
        OPS[op](req)
        return True
    except Exception as e:
        return str(e)
out = {}
out['mkdir'] = run('mkdir', path=at('sub'))
out['mkdir_again'] = run('mkdir', path=at('sub'))
out['create'] = run('create', path=at('sub/a.txt'))
open(at('sub/a.txt'), 'w').write('kept')
out['create_again'] = run('create', path=at('sub/a.txt'))
open(at('sub/b.txt'), 'w').write('other')
out['rename_clash'] = run('rename', **{'from': at('sub/a.txt'), 'to': at('sub/b.txt')})
out['rename'] = run('rename', **{'from': at('sub/a.txt'), 'to': at('sub/c.txt')})
out['rename_dir'] = run('rename', **{'from': at('sub'), 'to': at('moved')})
out['delete_file'] = run('delete', path=at('moved/b.txt'))
out['left'] = sorted(os.listdir(at('moved')))
out['kept'] = open(at('moved/c.txt')).read()
out['delete_dir'] = run('delete', path=at('moved'))
out['gone'] = not os.path.exists(at('moved'))
out['delete_missing'] = run('delete', path=at('nothing'))
out['delete_root'] = run('delete', path=os.path.abspath(os.sep))
out['delete_empty'] = run('delete', path='')
print(json.dumps(out))
"#;
        let agent = concat!(env!("CARGO_MANIFEST_DIR"), "/../../remote/agent.py");
        let out = Command::new(crate::pty::tests::python()).args(["-c", script, agent, &dir.display().to_string()]).output().expect("python");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        for done in ["mkdir", "create", "rename", "rename_dir", "delete_file", "delete_dir", "gone"] {
            assert_eq!(v[done], json!(true), "{done}: {v}");
        }
        for refused in ["mkdir_again", "create_again", "delete_missing", "delete_root", "delete_empty"] {
            assert!(v[refused].is_string(), "{refused} is refused: {v}");
        }
        assert!(v["rename_clash"].as_str().unwrap().contains("b.txt already exists"), "{v}");
        assert_eq!(v["left"], json!(["c.txt"]));
        assert_eq!(v["kept"], json!("kept"), "the file that was never to be replaced");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// New folder, new file, rename, delete: what Files asks of this PC.
    #[test]
    fn file_operations() {
        let dir = std::env::temp_dir().join(format!("weft-fileops-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let at = |n: &str| format!("{}/{n}", dir.display().to_string().replace('\\', "/"));
        let ok = |v: &Value| v["ok"] == json!(true);
        assert!(ok(&local_op(&json!({"op": "mkdir", "path": at("sub")}))));
        assert!(dir.join("sub").is_dir());
        assert!(!ok(&local_op(&json!({"op": "mkdir", "path": at("sub")}))), "it is already there");
        assert!(ok(&local_op(&json!({"op": "create", "path": at("sub/a.txt")}))));
        assert_eq!(std::fs::read(dir.join("sub/a.txt")).unwrap(), b"");
        std::fs::write(dir.join("sub/a.txt"), "kept").unwrap();
        assert!(!ok(&local_op(&json!({"op": "create", "path": at("sub/a.txt")}))), "never over a file that exists");
        assert_eq!(std::fs::read(dir.join("sub/a.txt")).unwrap(), b"kept");
        // Rename; never onto another file.
        std::fs::write(dir.join("sub/b.txt"), "other").unwrap();
        let clash = local_op(&json!({"op": "rename", "from": at("sub/a.txt"), "to": at("sub/b.txt")}));
        assert!(!ok(&clash) && clash["error"].as_str().unwrap().contains("b.txt already exists"), "{clash}");
        assert_eq!(std::fs::read(dir.join("sub/b.txt")).unwrap(), b"other");
        assert!(ok(&local_op(&json!({"op": "rename", "from": at("sub/a.txt"), "to": at("sub/c.txt")}))));
        assert!(!dir.join("sub/a.txt").exists() && std::fs::read(dir.join("sub/c.txt")).unwrap() == b"kept");
        assert!(ok(&local_op(&json!({"op": "rename", "from": at("sub"), "to": at("moved")}))), "a folder too");
        // Delete: a file, then a folder with what is in it; never a root.
        assert!(ok(&local_op(&json!({"op": "delete", "path": at("moved/b.txt")}))));
        assert!(!dir.join("moved/b.txt").exists() && dir.join("moved/c.txt").exists());
        assert!(ok(&local_op(&json!({"op": "delete", "path": at("moved")}))));
        assert!(!dir.join("moved").exists());
        assert!(!ok(&local_op(&json!({"op": "delete", "path": at("gone")}))), "nothing there");
        for root in ["/", "C:/", ""] {
            let r = local_op(&json!({"op": "delete", "path": root}));
            assert!(!ok(&r), "{root:?}: {r}");
        }
        let _ = std::fs::remove_dir_all(&dir);
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
        let script = format!("import json,sys\nsrc=open(r'{}/remote/agent.py',encoding='utf-8').read().split('\\ndef main')[0]\nexec(src)\nprint(json.dumps(op_tree({{'path': sys.argv[1]}})))", concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        if let Ok(out) = std::process::Command::new(crate::pty::tests::python()).arg("-c").arg(&script).arg(&root).output() {
            if out.status.success() {
                let v: Value = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(names(&v), local, "agent.py op_tree");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The nearest names first: at the limit a folder next to the root is
    /// there however big a deep one is (all of one folder was listed
    /// before the next: works/abc was not found); a link to a folder is
    /// gone into, a link back up is not gone round. Local and agent alike.
    #[test]
    fn tree_lists_the_nearest_first_and_through_links() {
        let dir = std::env::temp_dir().join(format!("weft-tree2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("aaa/b/c/d")).unwrap();
        std::fs::create_dir_all(dir.join("works/abc")).unwrap();
        for n in 0..40 {
            std::fs::write(dir.join(format!("aaa/b/c/d/f{n}.txt")), "x").unwrap();
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("works"), dir.join("linked")).unwrap();
            std::os::unix::fs::symlink(&dir, dir.join("works/up")).unwrap();
        }
        let root = dir.display().to_string().replace('\\', "/");
        let names = |v: &Value| v["entries"].as_array().unwrap().iter().map(|e| e[0].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let script = format!("import json,sys\nsrc=open(r'{}/remote/agent.py',encoding='utf-8').read().split('\\ndef main')[0]\nexec(src)\nprint(json.dumps(op_tree({{'path': sys.argv[1], 'max': int(sys.argv[2])}})))", concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let agent = |max: usize| -> Option<Value> {
            let out = std::process::Command::new(crate::pty::tests::python()).arg("-c").arg(&script).arg(&root).arg(max.to_string()).output().ok()?;
            out.status.success().then(|| serde_json::from_slice(&out.stdout).unwrap())
        };
        for (who, v) in [("local", Some(local_op(&json!({"op": "tree", "path": root, "max": 12})))), ("agent", agent(12))] {
            let Some(v) = v else { continue };
            assert!(v["truncated"].as_bool().unwrap(), "{who}: cut");
            // (Through the link to it, on Unix, if that came first: one real
            // folder is listed once.)
            let n = names(&v);
            assert!(n.contains(&"works/abc".to_string()) || n.contains(&"linked/abc".to_string()), "{who}: the near folder kept: {n:?}");
            // Whatever order the folders are read in: every name nearer
            // than the deepest one kept is kept.
            let all = if who == "local" { names(&local_op(&json!({"op": "tree", "path": root}))) } else { agent(20000).map(|v| names(&v)).unwrap_or_default() };
            let depth = |p: &str| p.matches('/').count();
            let deepest = n.iter().map(|p| depth(p)).max().unwrap_or(0);
            for p in all.iter().filter(|p| depth(p) < deepest) {
                assert!(n.contains(p), "{who}: {p} is nearer than what was kept: {n:?}");
            }
        }
        #[cfg(unix)]
        for (who, v) in [("local", Some(local_op(&json!({"op": "tree", "path": root})))), ("agent", agent(20000))] {
            let Some(v) = v else { continue };
            let n = names(&v);
            // (One real folder listed once: through the link or not.)
            assert!(n.contains(&"linked/abc".to_string()) || n.contains(&"works/abc".to_string()), "{who}: {n:?}");
            assert!(n.contains(&"linked".to_string()) && (n.contains(&"works/up".to_string()) || n.contains(&"linked/up".to_string())), "{who}: the links themselves listed: {n:?}");
            assert!(!v["truncated"].as_bool().unwrap() && n.len() < 100, "{who}: the link back up not gone round: {}", n.len());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_can_leave_out_what_git_ignores() {
        // This repository ignores target/ (where cargo builds).
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..").replace('\\', "/");
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
        if let Ok(out) = std::process::Command::new(crate::pty::tests::python()).arg("-c").arg(&script).arg(&root).output() {
            if out.status.success() {
                let mut want = kept.clone();
                want.retain(|n| !n.starts_with('.'));
                want.sort();
                let got: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(got, want, "agent.py list without ignored");
            }
        }
    }

    /// The agent and the bridge run on Python 3.6 (RHEL / Rocky 8's
    /// platform-python): nothing newer in their grammar. (Their library
    /// calls are kept to 3.6 by hand: no capture_output, no text=.)
    #[test]
    fn remote_python_stays_3_6() {
        for (name, src) in [("agent.py", AGENT_PY), ("serial.py", SERIAL_PY)] {
            assert!(!src.contains("capture_output") && !src.contains("text=True"), "{name}: 3.7+ subprocess arguments");
            // ast's feature_version is 3.8+; an older Python (Rocky 8's 3.6)
            // compiles it with its own grammar, which is the very check.
            let check = "import ast,sys\nsrc = sys.stdin.buffer.read()\nif sys.version_info >= (3, 8):\n    ast.parse(src, feature_version=(3, 6))\nelse:\n    compile(src, 'agent', 'exec')";
            let Ok(mut child) = Command::new(crate::pty::tests::python()).args(["-c", check]).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn() else { return };
            child.stdin.take().unwrap().write_all(src.as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{name} is not 3.6 grammar: {}", String::from_utf8_lossy(&out.stderr));
        }
    }

    /// The remote bridge run for real on a pseudo-terminal standing for the
    /// device: a long paste reaches the device whole (it was written
    /// without waiting, and the rest dropped), what the device sends comes
    /// as it sent it (shown with CRLF by weft, logged as it came), what is
    /// typed before Ctrl+] goes, and an unplugged device ends it (it read
    /// nothing for good, a core busy).
    #[test]
    #[cfg(unix)]
    fn the_serial_bridge_on_a_device() {
        use std::time::{Duration, Instant};
        if Command::new(crate::pty::tests::python()).arg("-V").output().is_err() {
            return;
        }
        let open = || {
            let (mut master, mut slave) = (0, 0);
            let mut name = [0 as libc::c_char; 128];
            // SAFETY: openpty fills the two descriptors and the name.
            let rc = unsafe { libc::openpty(&mut master, &mut slave, name.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) };
            assert_eq!(rc, 0, "a pseudo-terminal");
            // SAFETY: the name is NUL-ended by openpty.
            let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_string_lossy().into_owned();
            (master, slave, name)
        };
        let read_dev = move |fd: i32, want: usize, secs: u64| {
            let mut got = Vec::new();
            let end = Instant::now() + Duration::from_secs(secs);
            while got.len() < want && Instant::now() < end {
                let mut buf = [0u8; 65536];
                let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                // SAFETY: plain descriptors and a buffer of ours.
                if unsafe { libc::poll(&mut p, 1, 100) } > 0 {
                    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                    if n > 0 {
                        got.extend_from_slice(&buf[..n as usize]);
                    }
                }
            }
            got
        };
        let (master, slave, dev) = open();
        // Only the device's side held here (the bridge would see this
        // process holding its end, and ask).
        // SAFETY: our descriptor, closed once.
        unsafe { libc::close(slave) };
        // The device's side as raw as a UART: no echo, no LF made CRLF.
        // SAFETY: termios of our own descriptor.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            libc::tcgetattr(master, &mut t);
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(master, libc::TCSANOW, &t);
        }
        let argv = vec!["sh".to_string(), "-c".to_string(), serial_command(&dev, 115200)];
        let mut pty = crate::pty::Pty::spawn(&argv, None, &[], 100, 30, || {}).unwrap();
        let mut seen = Vec::new();
        let wait_for = |pty: &mut crate::pty::Pty, seen: &mut Vec<u8>, what: &[u8], secs: u64| {
            let end = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < end {
                seen.extend(pty.read());
                if seen.windows(what.len()).any(|w| w == what) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        assert!(wait_for(&mut pty, &mut seen, SERIAL_READY.as_bytes(), 30), "connected: {}", String::from_utf8_lossy(&seen));
        // A long paste, whole.
        let paste: Vec<u8> = (0..65536u32).map(|i| b'a' + (i % 26) as u8).collect();
        // (The device reads as it comes, as a UART does.)
        let n = paste.len();
        let reading = std::thread::spawn(move || read_dev(master, n, 30));
        pty.write(&paste);
        let got = reading.join().unwrap();
        assert_eq!(got.len(), paste.len(), "all of it reached the device");
        assert!(got == paste, "as it was sent");
        // What the device sends: as it sent it.
        // SAFETY: a buffer of ours.
        unsafe { libc::write(master, b"one\ntwo\r\n".as_ptr().cast(), 9) };
        assert!(wait_for(&mut pty, &mut seen, b"one\ntwo\r\n", 10), "a lone LF as it came: {:?}", String::from_utf8_lossy(&seen));
        // Typed before Ctrl+]: sent, then it ends.
        // (The device reads as it comes: from before it is sent.)
        let reading = std::thread::spawn(move || read_dev(master, 3, 10));
        pty.write(b"bye\x1d");
        assert_eq!(reading.join().unwrap(), b"bye", "what came before Ctrl+]");
        assert!(wait_for(&mut pty, &mut seen, b"disconnected", 10), "ended by Ctrl+]");
        // SAFETY: our descriptor, closed once.
        unsafe { libc::close(master) };
        // Unplugged: the bridge ends, not spinning.
        let (master, slave, dev) = open();
        // SAFETY: as above.
        unsafe { libc::close(slave) };
        let argv = vec!["sh".to_string(), "-c".to_string(), serial_command(&dev, 115200)];
        let mut pty = crate::pty::Pty::spawn(&argv, None, &[], 100, 30, || {}).unwrap();
        seen.clear();
        assert!(wait_for(&mut pty, &mut seen, SERIAL_READY.as_bytes(), 30), "connected again");
        // SAFETY: as above.
        unsafe { libc::close(master) };
        assert!(wait_for(&mut pty, &mut seen, b"went away", 10), "unplugged: said so: {}", String::from_utf8_lossy(&seen));
        let end = Instant::now() + Duration::from_secs(10);
        while !pty.exited() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(pty.exited(), "and ended");
    }

    #[test]
    fn serial_bridge_and_command() {
        // The bridge compiles (it runs on the remote host only: no termios here).
        let check = format!("compile(open(r'{}/remote/serial.py', encoding='utf-8').read(), 'serial.py', 'exec')", concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        if let Ok(out) = std::process::Command::new(crate::pty::tests::python()).arg("-c").arg(&check).output() {
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        }
        // The command carries it whole, the port quoted; all of it in sh
        // (`exec sh -c '…'` with no quote in it: any login shell runs it).
        let wrapped = serial_command("/dev/tty USB0", 115200);
        assert!(wrapped.starts_with("exec sh -c '") && wrapped.matches('\'').count() == 2, "{wrapped}");
        let b = wrapped.split("B=").nth(1).and_then(|s| s.split(';').next()).unwrap();
        let cmd = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b).unwrap()).unwrap();
        assert!(cmd.contains("'/dev/tty USB0' 115200"), "{cmd}");
        let b64 = wrapped.split("SRC=").nth(1).and_then(|s| s.split(';').next()).unwrap();
        let gz = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let mut whole = String::new();
        std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&gz[..]), &mut whole).unwrap();
        assert_eq!(whole, SERIAL_PY, "the bridge, whole (gzipped)");
        assert_eq!(wrapped.matches("SRC=").count(), 1, "the bridge is sent once, also for sudo");
        assert!(SERIAL_PY.contains("SUDO_EXIT = 77") && cmd.contains("[ $s = 77 ] && exec sudo"), "sudo when the bridge asks for it");
        assert!(SERIAL_PY.contains(&format!("\"\\x1b[2m{SERIAL_READY}")), "the bridge says it is ready as the logs expect");
        // Through ssh: the target, then the bridge instead of a shell.
        let argv = Profile::ssh("pi").serial_argv("/dev/ttyUSB0", 9600, false);
        assert_eq!(&argv[..3], ["ssh", "-tt", "pi"]);
        assert!(argv[3].starts_with("exec sh -c '") && argv[3].contains("SRC="));
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
            concat!(env!("CARGO_MANIFEST_DIR"), "/../..")
        );
        if let Ok(out) = std::process::Command::new(crate::pty::tests::python()).env("PYTHONIOENCODING", "utf-8").arg("-c").arg(&script).arg(&root).output() {
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
        Some(Profile { name: "loop".into(), kind: HostKind::Custom { pty: vec![b.clone(), "-c".into()], agent: vec![b, "-c".into()], shell: RemoteShell::Posix, plain: false } })
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
        t.user_typed();
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
        t.user_typed();
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
    /// where the shell has them. (The choices are Windows' shells.)
    #[cfg(windows)]
    fn local_shell_check(choice: LocalShell, cmd: &str, exit: Option<i32>) {
        let (argv, env) = local_shell(choice);
        argv_check(&format!("{choice:?}"), &argv, &env, cmd, exit, true);
    }

    /// A terminal started so has weft's integration: a prompt, a command
    /// with its exit code and output; `here`: started in the repository's
    /// folder, which it reports.
    fn argv_check(choice: &str, argv: &[String], env: &[(String, String)], cmd: &str, exit: Option<i32>, here: bool) {
        if which(&argv[0]).is_none() && !Path::new(&argv[0]).is_file() {
            println!("skip: {} not installed", argv[0]);
            return;
        }
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).canonicalize().unwrap();
        let _turn = crate::pty::tests::TURN.lock().unwrap_or_else(|e| e.into_inner());
        let mut pty = Pty::spawn(argv, here.then_some(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")), env, 100, 30, || {}).unwrap();
        let mut t = Term::new(100, 30);
        let mut p = vte::Parser::new();
        // zsh takes its integration typed in (as the app does on this event).
        let asked = |t: &Term| t.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["t"] == "inject"));
        pump(&mut pty, &mut t, &mut p, |t| t.integrated || asked(t), 30);
        if asked(&t) && !t.integrated {
            pty.write(b" eval \"$WEFT_ZSH\"\r");
        }
        // nushell reports the directory only from its first command on.
        assert!(pump(&mut pty, &mut t, &mut p, |t| t.integrated && !t.blocks.is_empty(), 60), "{choice:?}: prompt: {:?}", screen(&t));
        assert!(t.integrated, "{choice:?}");
        let missing = t.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["t"] == "shell-missing"));
        assert!(!missing, "{choice:?}: the shell asked for is the one running");
        pump(&mut pty, &mut t, &mut p, |_| false, 1);
        pty.write(cmd.as_bytes());
        pump(&mut pty, &mut t, &mut p, |t| t.screen[t.cy].text().contains(cmd), 5);
        t.user_typed();
        t.user_enter();
        pty.write(b"\r");
        // A shell with part of the integration (the login shell is busybox
        // ash on Alpine, dash or ksh elsewhere) tells no command text: its
        // block is the one that printed the output, with its exit code.
        let shell = t.events.iter().find_map(|e| match e {
            TermEvent::Weft { value, .. } if value["t"] == "shell" => value["name"].as_str().map(String::from),
            _ => None,
        });
        let partial = shell.as_deref().and_then(shell_info).is_some_and(|i| i.support == ShellSupport::Partial);
        let ours = |t: &Term, b: &crate::term::Block| if partial { b.command.is_empty() && t.block_output(b).trim() == "weft-ok" } else { b.command.contains(cmd) };
        let done = pump(&mut pty, &mut t, &mut p, |t| t.blocks.iter().any(|b| ours(t, b) && b.state() == BlockState::Done), 15);
        assert!(done, "{choice:?} ({shell:?}): {:?} {:?}", t.blocks.iter().map(|b| (&b.command, b.exit)).collect::<Vec<_>>(), screen(&t));
        let b = t.blocks.iter().find(|b| ours(&t, b) && b.state() == BlockState::Done).unwrap();
        assert_eq!(b.exit, exit, "{choice:?}: {:?} {:?}", t.blocks.iter().map(|b| (&b.command, b.exit, b.state())).collect::<Vec<_>>(), screen(&t));
        assert_eq!(t.block_output(b).trim(), "weft-ok", "{choice:?}");
        if here {
            let name = root.file_name().unwrap().to_string_lossy().to_string();
            assert!(t.cwd.as_deref().map(|c| c.trim_end_matches('/').ends_with(&name)).unwrap_or(false), "{choice:?}: cwd {:?}", t.cwd);
        }
    }

    /// No $SHELL and no bash (a container run by docker or kubectl exec,
    /// Alpine's): sh, with its integration. It went past the sh family's
    /// branch (an empty name) to a plain sh. Here busybox alone on PATH.
    #[test]
    #[cfg(unix)]
    fn no_shell_variable_and_no_bash_gets_sh_with_its_integration() {
        let Some(busybox) = which("busybox").or_else(|| ["/usr/local/bin/ash", "/bin/busybox"].iter().map(PathBuf::from).find(|p| p.is_file())) else { return };
        let list = Command::new(&busybox).arg("--list").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        if !list.lines().any(|l| l == "base64") || !list.lines().any(|l| l == "gzip") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("weft-bb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for applet in list.lines().filter(|l| !l.is_empty()) {
            let _ = std::os::unix::fs::symlink(&busybox, dir.join(applet));
        }
        let path = dir.display().to_string();
        let argv = vec![format!("{path}/sh"), "-c".to_string(), remote_shell_command(None)];
        let env = [("PATH".to_string(), path.clone()), ("SHELL".to_string(), String::new()), ("HOME".to_string(), path.clone())];
        let _turn = crate::pty::tests::TURN.lock().unwrap_or_else(|e| e.into_inner());
        let mut pty = Pty::spawn(&argv, None, &env, 100, 30, || {}).unwrap();
        let mut t = Term::new(100, 30);
        let mut p = vte::Parser::new();
        let named = |t: &Term| t.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["t"] == "shell" && value["name"] == "sh"));
        assert!(pump(&mut pty, &mut t, &mut p, |t| t.integrated && named(t), 30), "sh, integrated: {:?}", screen(&t));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A PS1 the user's profile exports goes, with weft's call in it, to
    /// the shells started from this one (`sh`, vi's :sh), where the
    /// function is not: their prompts say nothing of it (they said
    /// "__weft_p: not found" at each).
    #[test]
    #[cfg(unix)]
    fn an_exported_ps1_leaves_the_shells_started_from_it_quiet() {
        let Some(dash) = which("dash") else { return };
        let dir = std::env::temp_dir().join(format!("weft-ps1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sh.sh"), SH_SH).unwrap();
        std::fs::write(dir.join(".profile"), "PS1='mine$ '; export PS1\n").unwrap();
        let mut c = Command::new(&dash)
            .arg("-i")
            .env("HOME", &dir)
            .env("WEFT_SH", "dash")
            .env("ENV", dir.join("sh.sh"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        c.stdin.take().unwrap().write_all(format!("env -u ENV {} -i\necho child-ran\nexit\nexit\n", dash.display()).as_bytes()).unwrap();
        let o = c.wait_with_output().unwrap();
        let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        assert!(all.contains("child-ran") && all.contains("mine$"), "{all}");
        assert!(!all.contains("not found"), "the child's prompt: {all}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FreeBSD's sh runs no command in its prompt, NetBSD's only with
    /// promptcmds: there the prompt is left as it was (no `__weft_p` shown),
    /// and the rest of the integration still runs (a failed `set` once ended
    /// its reading in dash). Here dash, as `sh`, with uname saying so.
    #[test]
    #[cfg(unix)]
    fn a_bsd_sh_keeps_its_prompt() {
        let Some(dash) = which("dash") else { return };
        let dir = std::env::temp_dir().join(format!("weft-bsd-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("sh.sh"), SH_SH).unwrap();
        for (os, prompt) in [("Linux", "`__weft_p`"), ("FreeBSD", ""), ("NetBSD", "")] {
            let uname = dir.join("bin/uname");
            std::fs::write(&uname, format!("#!/bin/sh\necho {os}\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&uname, std::fs::Permissions::from_mode(0o755)).unwrap();
            let path = format!("{}:{}", dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
            // The integration reads /etc/profile first: Debian's and
            // Alpine's set PATH anew, and this uname is not the one it runs.
            let seen = Command::new(&dash).args(["-c", ". /etc/profile >/dev/null 2>&1; uname -s"]).env("PATH", &path).stderr(Stdio::null()).output().unwrap();
            if String::from_utf8_lossy(&seen.stdout).trim() != os {
                eprintln!("skipped: /etc/profile sets PATH here");
                break;
            }
            let mut c = Command::new(&dash)
                .arg("-i")
                .env("PATH", path)
                .env("HOME", &dir)
                .env("WEFT_SH", "sh")
                .env("ENV", dir.join("sh.sh"))
                .env("PS1", "$ ")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            use std::io::Write;
            c.stdin.take().unwrap().write_all(b"echo \"PS1=[$PS1] WEFT=$WEFT\"\nexit\n").unwrap();
            let out = String::from_utf8_lossy(&c.wait_with_output().unwrap().stdout).to_string();
            // (root's /etc/profile may have made it `# `.)
            let line = out.lines().find(|l| l.starts_with("PS1=[")).unwrap_or_else(|| panic!("{os}: {out}"));
            assert!(line.ends_with("] WEFT=1") && line.contains("__weft_p") != prompt.is_empty(), "{os}: {line}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every shell weft has an integration for, of those installed here
    /// (Git's bash and dash on Windows; CI's Linuxes have the others),
    /// started by weft's command as the chosen shell: the full ones through
    /// `argv_check`; the partial ones (POSIX shells with no hooks) tell each
    /// prompt, the exit code and the folder, not the command's text; one
    /// weft has nothing for (tcsh) starts as it is and says its name.
    #[test]
    fn each_shell_here_gets_what_weft_has_for_it() {
        let sh = std::env::var("WEFT_TEST_SH").unwrap_or_else(|_| if cfg!(windows) { "C:/Program Files/Git/bin/sh.exe" } else { "/bin/sh" }.to_string());
        let sh = sh.as_str();
        if !Path::new(sh).is_file() {
            return;
        }
        let there = |name: &str| Command::new(sh).args(["-c", &format!("command -v {name}")]).output().map(|o| o.status.success()).unwrap_or(false);
        let argv = |name: &str| vec![sh.to_string(), "-c".to_string(), remote_shell_command_with(None, Some(name))];
        // With another sh given, its PATH too: a terminal on Windows takes
        // PATH from the registry, not from this process.
        let mut env: Vec<(String, String)> = Vec::new();
        if std::env::var("WEFT_TEST_SH").is_ok() {
            env.push(("PATH".to_string(), std::env::var("PATH").unwrap_or_default()));
        }
        // zsh with none of the user's startup files starts its new-user menu
        // (zsh-newuser-install; not for root), as in any terminal: an empty
        // ~/.zshrc of its own, as nearly every zsh user has.
        let zdot = std::env::temp_dir().join(format!("weft-zdot-{}", std::process::id()));
        std::fs::create_dir_all(&zdot).unwrap();
        std::fs::write(zdot.join(".zshrc"), "").unwrap();
        env.push(("ZDOTDIR".to_string(), zdot.display().to_string().replace('\\', "/")));
        let with = |more: &[(&str, &str)]| env.iter().cloned().chain(more.iter().map(|(k, v)| (k.to_string(), v.to_string()))).collect::<Vec<_>>();
        let mut tried = Vec::new();
        for name in ["bash", "zsh", "fish", "ksh", "mksh", "dash", "ash", "tcsh"] {
            if !there(name) {
                continue;
            }
            tried.push(name);
            let info = shell_info(name);
            match info.map(|i| i.support) {
                Some(ShellSupport::Full) => {
                    argv_check(name, &argv(name), &env, "echo weft-ok", Some(0), true);
                    // And as the login shell, nothing chosen: what a host
                    // whose user has this shell gets.
                    let path = Command::new(sh).args(["-c", &format!("command -v {name}")]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
                    let login = vec![sh.to_string(), "-c".to_string(), remote_shell_command(None)];
                    argv_check(&format!("login {name}"), &login, &with(&[("SHELL", &path)]), "echo weft-ok", Some(0), true);
                    // fish with vi bindings: its mode prompt ([I]) is drawn
                    // before the prompt, and was taken for the command's
                    // output.
                    if name == "fish" {
                        let cfg = std::env::temp_dir().join(format!("weft-fishvi-{}", std::process::id()));
                        std::fs::create_dir_all(cfg.join("fish")).unwrap();
                        std::fs::write(cfg.join("fish").join("config.fish"), "set -g fish_key_bindings fish_vi_key_bindings\n").unwrap();
                        let xdg = cfg.display().to_string().replace('\\', "/");
                        argv_check("fish in vi mode", &argv(name), &with(&[("XDG_CONFIG_HOME", &xdg)]), "echo weft-ok", Some(0), true);
                        let _ = std::fs::remove_dir_all(&cfg);
                    }
                }
                Some(ShellSupport::Partial) => {
                    let _turn = crate::pty::tests::TURN.lock().unwrap_or_else(|e| e.into_inner());
                    let mut pty = Pty::spawn(&argv(name), Some(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")), &env, 100, 30, || {}).unwrap();
                    let mut t = Term::new(100, 30);
                    let mut p = vte::Parser::new();
                    assert!(pump(&mut pty, &mut t, &mut p, |t| t.integrated && t.blocks.last().is_some_and(|b| b.state() == BlockState::Prompt), 40), "{name}: prompt: {:?}", screen(&t));
                    let named = t.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["t"] == "shell" && value["name"] == name));
                    assert!(named, "{name}: it says which shell it is");
                    let run = |t: &mut Term, pty: &mut Pty, p: &mut vte::Parser, cmd: &str| -> crate::term::Block {
                        let before = t.blocks.iter().filter(|b| b.state() == BlockState::Done).count();
                        pty.write(cmd.as_bytes());
                        pump(pty, t, p, |t| t.screen[t.cy].text().contains(cmd), 5);
                        t.user_typed();
                        t.user_enter();
                        pty.write(b"\r");
                        let done = pump(pty, t, p, |t| t.blocks.iter().filter(|b| b.state() == BlockState::Done).count() > before && t.blocks.last().is_some_and(|b| b.state() == BlockState::Prompt), 15);
                        assert!(done, "{name}: {cmd}: {:?}", screen(t));
                        t.blocks.iter().rfind(|b| b.state() == BlockState::Done).unwrap().clone()
                    };
                    let b = run(&mut t, &mut pty, &mut p, "echo weft-ok");
                    assert_eq!((b.exit, t.block_output(&b).trim()), (Some(0), "weft-ok"), "{name}: exit code and output");
                    assert!(b.command.is_empty() && !b.guessed, "{name}: no text of the command, and nothing guessed");
                    let b = run(&mut t, &mut pty, &mut p, "(exit 3)");
                    assert_eq!(b.exit, Some(3), "{name}: a failing command's code");
                    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).canonicalize().unwrap();
                    let dir = root.file_name().unwrap().to_string_lossy().to_string();
                    assert!(t.cwd.as_deref().is_some_and(|c| c.trim_end_matches('/').ends_with(&dir)), "{name}: cwd {:?}", t.cwd);
                }
                _ => {
                    let _turn = crate::pty::tests::TURN.lock().unwrap_or_else(|e| e.into_inner());
                    let mut pty = Pty::spawn(&argv(name), None, &env, 100, 30, || {}).unwrap();
                    let mut t = Term::new(100, 30);
                    let mut p = vte::Parser::new();
                    let named = |t: &Term| t.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["t"] == "shell" && value["name"] == name));
                    assert!(pump(&mut pty, &mut t, &mut p, named, 30), "{name}: it says which shell it is: {:?}", screen(&t));
                    pump(&mut pty, &mut t, &mut p, |_| false, 2);
                    // Backquotes and expr: what tcsh reads as sh does (its $(( )) is an error that drops the line).
                    pty.write(b"echo weft-`expr 40 + 2`\r");
                    assert!(pump(&mut pty, &mut t, &mut p, |t| screen(t).iter().any(|l| l.contains("weft-42")), 15), "{name}: it runs: {:?}", screen(&t));
                    assert!(!t.integrated, "{name}: no integration");
                }
            }
        }
        println!("shells tried: {tried:?}");
        assert!(tried.contains(&"bash") || !cfg!(windows), "Git's bash at least");
        // A chosen shell that is not there: said, and the login shell.
        let _turn = crate::pty::tests::TURN.lock().unwrap_or_else(|e| e.into_inner());
        let mut pty = Pty::spawn(&argv("no-such-shell"), None, &with(&[("SHELL", "/bin/sh")]), 100, 30, || {}).unwrap();
        let mut t = Term::new(100, 30);
        let mut p = vte::Parser::new();
        let missing = |t: &Term| t.events.iter().any(|e| matches!(e, TermEvent::Weft { value, .. } if value["t"] == "shell-missing" && value["wanted"] == "no-such-shell" && value["using"] == "sh"));
        assert!(pump(&mut pty, &mut t, &mut p, missing, 30), "the missing one is said: {:?} {:?}", screen(&t), t.events.iter().filter_map(|e| if let TermEvent::Weft { value, .. } = e { Some(value.to_string()) } else { None }).collect::<Vec<_>>());
        // That login shell is a sh (bash as sh, here and on many systems):
        // it has the sh family's part, and its prompt comes without an error.
        assert!(pump(&mut pty, &mut t, &mut p, |t| t.integrated && t.blocks.last().is_some_and(|b| b.state() == BlockState::Prompt), 20), "the login sh's prompt: {:?}", screen(&t));
        pump(&mut pty, &mut t, &mut p, |_| false, 1);
        assert!(!screen(&t).iter().any(|l| l.contains("syntax error")), "its prompt has no error: {:?}", screen(&t));
        pty.write(b"echo weft-$((40 + 2))\r");
        assert!(pump(&mut pty, &mut t, &mut p, |t| screen(t).iter().any(|l| l.contains("weft-42")), 15), "and the login shell runs: {:?}", screen(&t));
        // The table: names, paths, what is not known.
        assert_eq!(shell_info("/usr/bin/zsh").map(|i| i.support), Some(ShellSupport::Full));
        assert_eq!(shell_info("C:\\Windows\\System32\\cmd.exe").map(|i| i.support), Some(ShellSupport::Partial));
        assert!(shell_info("dash").is_some_and(|i| !i.missing.is_empty()) && shell_info("tcsh").is_none());
    }

    /// Over a real ssh, where one is given (CI: WEFT_TEST_SSH=localhost, each
    /// system to itself): the terminal has weft's integration, a command
    /// runs with its exit code, and the agent's command starts the agent.
    #[test]
    fn over_ssh_where_one_is_given() {
        let Ok(target) = std::env::var("WEFT_TEST_SSH") else { return };
        let mut prof = Profile::ssh(&target);
        if std::env::var("WEFT_TEST_SSH_SHELL").as_deref() == Ok("windows") {
            prof.set_remote_shell(RemoteShell::Windows);
        }
        argv_check(&format!("ssh {target}"), &prof.pty_argv(None, false), &[], "echo weft-ok", Some(0), false);
        // The agent: its hello over the same ssh.
        let argv = prof.agent_argv(false).unwrap();
        let mut child = Command::new(&argv[0]).args(&argv[1..]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        ship_agent(&mut stdin).unwrap();
        use std::io::BufRead;
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut first = String::new();
        let _ = out.read_line(&mut first);
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        assert!(first.contains("\"ready\""), "the agent said it is ready over ssh: {first:?}");
    }

    /// bash with a user's ~/.bash_profile that has `set -u` and a
    /// PROMPT_COMMAND array (bash 5.1+ runs each element) whose second
    /// element sets PS1 anew: weft's marks are still on every prompt (only
    /// the array's first element was weft's: the later one wiped them), and
    /// nothing is unbound. (bash < 4.4's `set -u` case: CI's old bash.)
    /// A PROMPT_COMMAND string ending in `;` too.
    #[test]
    fn bash_keeps_its_marks_with_a_users_prompt_command_array_and_set_u() {
        let sh = if cfg!(windows) { "C:/Program Files/Git/bin/sh.exe" } else { "/bin/sh" };
        if !Path::new(sh).is_file() || which("bash").is_none() && !cfg!(windows) {
            return;
        }
        // An array; a string that ends in `;` (`history -a; $PROMPT_COMMAND`
        // with it empty: joined with `;`, an error at every prompt).
        for (what, profile) in [("a PROMPT_COMMAND array", "PROMPT_COMMAND=(true 'PS1=\"arr> \"')"), ("a PROMPT_COMMAND ending in ;", "PROMPT_COMMAND='history -a; '")] {
            let home = std::env::temp_dir().join(format!("weft-bashprof-{}", std::process::id()));
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(home.join(".bash_profile"), format!("set -u\n{profile}\n")).unwrap();
            let home = home.display().to_string().replace('\\', "/");
            let argv = vec![sh.to_string(), "-c".to_string(), remote_shell_command_with(None, Some("bash"))];
            argv_check(&format!("bash with {what}"), &argv, &[("HOME".to_string(), home.clone())], "echo weft-ok", Some(0), true);
            let _ = std::fs::remove_dir_all(&home);
        }
    }

    /// A local terminal on macOS and Linux has weft's integration (the
    /// remote bootstrap run by the local sh): tried here with Git's sh, its
    /// bash.
    #[test]
    fn a_posix_local_terminal_is_integrated() {
        let sh = if cfg!(windows) { "C:/Program Files/Git/bin/sh.exe" } else { "/bin/sh" };
        if !Path::new(sh).is_file() {
            return;
        }
        #[cfg(not(windows))]
        assert_eq!(local_shell(LocalShell::Auto).0, posix_local_argv(&posix_sh()), "macOS and Linux start it so");
        argv_check("posix local", &posix_local_argv(sh), &[], "echo weft-ok", Some(0), true);
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

    /// ssh refused before the agent was sent (keys not accepted, here with
    /// the agent's stdin closed): the reason is what ssh said, so weft asks
    /// for a password; it was "send agent: Broken pipe" on a Mac.
    #[test]
    fn an_agent_refused_before_it_is_sent_says_why() {
        let Some(b) = bash() else { return };
        let script = "exec 0<&-; echo 'user@host: Permission denied (publickey,password).' >&2; sleep 1".to_string();
        let prof = Profile { name: "refused".into(), kind: HostKind::Custom { pty: vec![b.clone(), "-c".into()], agent: vec![b, "-c".into(), script], shell: RemoteShell::Posix, plain: false } };
        let data = DataDir { root: std::env::temp_dir().join(format!("weft-test-refused-{}", std::process::id())) };
        let _ = std::fs::create_dir_all(data.root.join("logs"));
        let mut c = Conn::open(prof, &data, || {}, None);
        let end = Instant::now() + Duration::from_secs(20);
        while !matches!(c.status, ConnStatus::Failed(_)) && Instant::now() < end {
            c.poll();
            std::thread::sleep(Duration::from_millis(50));
        }
        match &c.status {
            ConnStatus::Failed(why) => assert!(why.contains("Permission denied") && !why.contains("Broken pipe"), "{why}"),
            s => panic!("{s:?}"),
        }
        drop(c);
        let _ = std::fs::remove_dir_all(&data.root);
    }

    /// Sending the agent to an ssh that has already ended (its end of the
    /// pipe closed) is not the failure: ssh's own words are.
    #[test]
    fn shipping_to_an_ended_ssh_is_no_error() {
        let Some(b) = bash() else { return };
        let mut child = Command::new(b).args(["-c", "exit 255"]).stdin(Stdio::piped()).spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let _ = child.wait();
        // Nobody at the other end. (A process another test forks in that
        // moment holds the pipe's far end until it execs: a write can still
        // go in then. Again until it is gone.)
        let mut raw = Ok(());
        for _ in 0..100 {
            raw = stdin.write_all(AGENT_PY.as_bytes()).and_then(|_| stdin.flush());
            if raw.is_err() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(raw.map_err(|e| e.kind()), Err(std::io::ErrorKind::BrokenPipe), "nobody at the other end");
        assert_eq!(ship_agent(&mut stdin), Ok(()));
        struct Failing;
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk on fire"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(ship_agent(&mut Failing), Err("send agent: disk on fire".into()), "other errors still are");
    }

    /// The agent's command finds Python by the list (settings), runs the
    /// first that is 3.6 or newer, and says what to do when none is. Here
    /// through Git Bash with a PATH of its own: a Python under an odd name
    /// from the list, a Python 2 before it (passed over), none at all.
    /// A stand-in program: a script, runnable where that takes the mode
    /// (MSYS goes by its #! line).
    fn script(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn the_agent_finds_python_by_the_list() {
        assert!(!agent_command().contains('"') && !agent_command().contains('\\'), "still nothing a shell or argv parser re-quotes");
        assert!(serial_command("/dev/ttyS0", 9600).contains("PY64="), "the serial bridge too");
        let (Some(b), Some(py)) = (bash(), which(crate::pty::tests::python())) else { return };
        let dir = std::env::temp_dir().join(format!("weft-find-py-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("opt/py-3.x/bin")).unwrap();
        let unix = |p: &std::path::Path| p.display().to_string().replace('\\', "/");
        // A Python 2 (any python that is not 3.6+ fails the check) and the
        // real one under a name no default has.
        script(&dir.join("bin/python3"), "#!/bin/sh\nexit 1\n");
        script(&dir.join("opt/py-3.x/bin/py3odd"), &format!("#!/bin/sh\nexec '{}' \"$@\"\n", unix(&py)));
        let run = |list: &[&str]| -> String {
            let list: Vec<String> = list.iter().map(|s| s.replace("$D", &unix(&dir))).collect();
            let cmd = posix_wrap(&[("PY64", b64(&list.join("\n")))], &format!("{FIND_PY}; echo found=$P"), NO_DECODER);
            let out = Command::new(&b)
                .args(["-c", &format!("export PATH='{}/bin:/usr/bin:/bin'; eval \"$1\"", unix(&dir)), "x", &cmd])
                .output()
                .unwrap();
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
        };
        // ("nothere": a name no system has; a Linux has a real "python".)
        let got = run(&["python3", "nothere", "$D/opt/*/bin/py3odd"]);
        assert!(got.contains(&format!("found={}/opt/py-3.x/bin/py3odd", unix(&dir))), "the odd one, by a pattern; python3 (2.x here) passed over: {got}");
        let got = run(&["python3", "nothere"]);
        let said: Vec<&str> = got.lines().filter(|l| l.starts_with("weft: ")).collect();
        assert_eq!(said.len(), 1, "the reason on one line (the status bar keeps one): {got}");
        assert!(said[0].starts_with(NO_PYTHON) && said[0].ends_with("Tried: python3 nothere") && said[0].contains("command -v python3") && said[0].contains("Python on remote hosts") && !got.contains("found="), "{got}");
        assert!(!FIND_PY.contains('\n'), "no newline in the script: eval of it unquoted makes it a space");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// macOS' /usr/bin/python3 where the Command Line Tools are not
    /// installed only asks to install them (a window on that Mac): it is
    /// not run, and what to do is said. With the tools it is Python.
    #[test]
    fn a_macs_python_that_only_asks_to_install_is_not_run() {
        let (Some(b), Some(py)) = (bash(), which(crate::pty::tests::python())) else { return };
        let dir = std::env::temp_dir().join(format!("weft-mac-py-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        // As the shell there names it (Git Bash: /c/Users/…).
        let unix = |p: &std::path::Path| {
            let s = p.display().to_string().replace('\\', "/");
            match s.as_bytes() {
                [d, b':', ..] if cfg!(windows) => format!("/{}{}", (*d as char).to_ascii_lowercase(), &s[2..]),
                _ => s,
            }
        };
        let stub = format!("{}/bin/python3", unix(&dir));
        script(&dir.join("bin/uname"), "#!/bin/sh\necho Darwin\n");
        script(&dir.join("bin/python3"), &format!("#!/bin/sh\necho ran > '{}/ran'\nexec '{}' \"$@\"\n", unix(&dir), py.display().to_string().replace('\\', "/")));
        let tools = |there: bool| script(&dir.join("bin/xcode-select"), &format!("#!/bin/sh\nexit {}\n", if there { 0 } else { 2 }));
        let run = || -> String {
            let find = FIND_PY.replace("/usr/bin/python3", &stub);
            let cmd = posix_wrap(&[("PY64", b64("python3\nnothere"))], &format!("{find}; echo found=$P"), NO_DECODER);
            let out = Command::new(&b).args(["-c", &format!("export PATH='{}/bin':/usr/bin:/bin; eval \"$1\"", unix(&dir)), "x", &cmd]).output().unwrap();
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
        };
        tools(false);
        let got = run();
        assert!(!dir.join("ran").exists(), "not run: {got}");
        assert!(got.contains(NO_PYTHON) && got.contains("xcode-select --install") && got.contains("Tried: python3 nothere") && !got.contains("found="), "{got}");
        tools(true);
        let got = run();
        assert!(got.contains("found=python3") && dir.join("ran").exists(), "with the tools it is Python: {got}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A host with none of the tools weft's command needs to unpack itself
    /// (base64 or openssl, gzip, awk; or python3): the terminal says so and
    /// is the user's shell as it is. (It ended at once with code 0: the
    /// terminal closed without a word.) The agent says so and ends.
    #[test]
    fn a_host_that_cannot_unpack_gets_a_plain_shell() {
        use std::io::Write;
        let sh = if cfg!(windows) { PathBuf::from("C:/Program Files/Git/bin/sh.exe") } else { PathBuf::from("/bin/sh") };
        if !sh.is_file() {
            return;
        }
        let run = |script: &str| -> (String, Option<i32>) {
            let mut c = Command::new(&sh);
            c.args(["-c", script]).env("SHELL", "/bin/sh").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            let mut child = c.spawn().unwrap();
            child.stdin.take().unwrap().write_all(b"echo plain-$((40 + 2))\nexit\n").unwrap();
            let out = child.wait_with_output().unwrap();
            (format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)), out.status.code())
        };
        // What ssh hands the login shell is `exec sh -c '<this>'`.
        let inner = |cmd: &str| cmd.strip_prefix("exec sh -c '").and_then(|c| c.strip_suffix('\'')).expect("exec sh -c '…'").to_string();
        // No decoder at all (nothing on the PATH): said, and a shell.
        let nothing = "PATH=/weft-nothing-here; ";
        let (got, _) = run(&format!("{nothing}{}", inner(&remote_shell_command(None))));
        assert!(got.contains("weft: no base64, openssl or python3 here") && got.contains("plain-42"), "{got}");
        // The agent: said, and ended as failed.
        let (got, code) = run(&format!("{nothing}{}", inner(&agent_command())));
        assert!(got.contains("weft: no base64, openssl or python3 here") && code == Some(127), "{code:?} {got}");
        // Decoded, but the bundle does not unpack (no gzip or awk there: here
        // a bundle that is none): said, and a shell.
        let (got, _) = run(&format!("W=AAAA; {SHELL_UNPACK}"));
        assert!(got.contains("weft: cannot unpack its shell integration") && got.contains("plain-42"), "{got}");
        // And as ever where they are there.
        assert!(!PLAIN_SHELL.contains(['"', '\\', '\'']) && !NO_DECODER.contains(['"', '\\', '\'']), "nothing a shell or argv parser re-quotes");
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
        let id = c.request(json!({"op": "list", "path": concat!(env!("CARGO_MANIFEST_DIR"), "/../..")}));
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
