# weft shell integration for bash.
#
# Delivered in memory only: the bootstrap runs `bash --rcfile <(printf %s "$R") -i`
# over the ssh PTY, so nothing is ever written to the remote disk.
# Everything here uses plain POSIX tools; python is only used by the rich
# helpers (wls/wtable) and they fall back to plain text without it.

__weft_dec=$D
__weft_cwd=$WEFT_CWD
unset W D R WEFT_CWD

# --rcfile replaces the normal startup files: read a login shell's, as bash
# does (INVOCATION in its manual), so it is as a plain ssh login: the
# system's first (servers where /etc/profile.d loads ~/.bashrc had none of
# it), then the first of the user's that is there.
if [ -r /etc/profile ]; then . /etc/profile; fi
if [ -r ~/.bash_profile ]; then . ~/.bash_profile
elif [ -r ~/.bash_login ]; then . ~/.bash_login
elif [ -r ~/.profile ]; then . ~/.profile
fi
# And a login shell's end: ~/.bash_logout, unless the user's files set
# their own EXIT trap.
if [ -z "$(trap -p EXIT)" ]; then
    trap '[ -r ~/.bash_logout ] && . ~/.bash_logout' EXIT
fi

if [ -n "$__weft_cwd" ]; then
    __weft_cwd=$(printf %s "$__weft_cwd" | $__weft_dec 2>/dev/null)
    [ -d "$__weft_cwd" ] && cd -- "$__weft_cwd"
fi
unset __weft_cwd

export WEFT=1 TERM=xterm-256color COLORTERM=truecolor
shopt -s checkwinsize

__weft_py=
for __p in python3 python; do
    if command -v "$__p" >/dev/null 2>&1 && "$__p" -c "" >/dev/null 2>&1; then
        __weft_py=$__p; break
    fi
done
unset __p

# --- low level -------------------------------------------------------------

__weft_osc() { printf '\033]%s\007' "$1" >/dev/tty; }

# Structured events travel as base64(JSON) so any byte survives the OSC.
__weft_ev() { __weft_osc "7777;$(printf %s "$1" | base64 | tr -d '\r\n')"; }

__weft_js() {
    local s=$1
    s=${s//\\/\\\\}; s=${s//\"/\\\"}; s=${s//$'\t'/\\t}
    s=${s//$'\n'/\\n}; s=${s//$'\r'/}
    printf '"%s"' "$s"
}

__weft_abs() {
    case "$1" in
        /*) printf %s "$1" ;;
        *) printf %s "$PWD/$1" ;;
    esac
}

# --- prompt marks (OSC 133 / OSC 7) ----------------------------------------

__weft_precmd() {
    local ec=$?
    __weft_osc "133;D;$ec"
    __weft_osc "7;file://${HOSTNAME}${PWD}"
    __weft_in_cmd=
    return $ec
}

__weft_ps1() {
    case "$PS1" in
        *'133;A'*) ;;
        *) PS1='\[\033]133;A\007\]'"$PS1"'\[\033]133;B\007\]' ;;
    esac
}

PROMPT_COMMAND="__weft_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND};__weft_ps1"

if [ "${BASH_VERSINFO[0]}" -gt 4 ] || { [ "${BASH_VERSINFO[0]}" -eq 4 ] && [ "${BASH_VERSINFO[1]}" -ge 4 ]; }; then
    PS0='\033]133;C\007'
else
    # bash < 4.4 (macOS /bin/bash) has no PS0: use the DEBUG trap once per command.
    __weft_preexec() {
        [ -n "$COMP_LINE" ] && return
        [ -n "$__weft_in_cmd" ] && return
        case "$BASH_COMMAND" in __weft_precmd*) return ;; esac
        __weft_in_cmd=1
        __weft_osc "133;C"
    }
    trap '__weft_preexec' DEBUG
fi

# --- helpers: GUI widgets woven into the text grid ---------------------------
# Each helper prints a plain-text fallback that occupies exactly the rows the
# GUI widget covers, so copy/paste and non-weft terminals still work.

# wopen FILE [LINE] : open a file in weft's local editor (read via the agent)
wopen() {
    local f
    [ $# -eq 0 ] && { echo "usage: wopen FILE [LINE]" >&2; return 2; }
    f=$(__weft_abs "$1")
    __weft_ev "{\"t\":\"open\",\"path\":$(__weft_js "$f"),\"line\":${2:-1}}"
}
alias wedit=wopen

# wnotify MESSAGE : toast in the GUI
wnotify() { __weft_ev "{\"t\":\"notify\",\"msg\":$(__weft_js "$*")}"; }

# wclip [TEXT] : copy TEXT or stdin to the local clipboard (OSC 52)
wclip() {
    local b
    b=$(if [ $# -gt 0 ]; then printf %s "$*"; else cat; fi | base64 | tr -d '\r\n')
    __weft_osc "52;c;$b"
}

# wchoose OPTION... : GUI picker, the choice is printed on stdout
#   e.g.  branch=$(wchoose $(git branch --format='%(refname:short)'))
wchoose() {
    [ $# -eq 0 ] && { echo "usage: wchoose OPTION..." >&2; return 2; }
    local opts="" o i=1 ans
    for o in "$@"; do opts="$opts${opts:+,}$(__weft_js "$o")"; done
    __weft_ev "{\"t\":\"widget\",\"kind\":\"choose\",\"id\":\"c$$-$RANDOM\",\"rows\":$(($# + 1)),\"options\":[$opts]}"
    printf 'choose one (number):\n' >/dev/tty
    for o in "$@"; do printf '  %d) %s\n' "$i" "$o" >/dev/tty; i=$((i + 1)); done
    stty -echo </dev/tty 2>/dev/null
    IFS= read -r ans </dev/tty
    stty echo </dev/tty 2>/dev/null
    case "$ans" in
        ''|*[!0-9]*) ;;
        *) [ "$ans" -ge 1 ] && [ "$ans" -le $# ] && ans=${!ans} ;;
    esac
    printf '%s\n' "$ans"
}

# wprogress PERCENT [LABEL] [ID] : progress bar drawn over the current line
wprogress() {
    local pct=${1:-0} label=${2:-} id=${3:-p$PPID} n bar=""
    __weft_ev "{\"t\":\"widget\",\"kind\":\"progress\",\"id\":$(__weft_js "$id"),\"rows\":1,\"pct\":$pct,\"label\":$(__weft_js "$label")}"
    n=$((pct / 5))
    while [ "$n" -gt 0 ]; do bar="$bar#"; n=$((n - 1)); done
    printf '\r[%-20s] %3d%% %s\033[K' "$bar" "$pct" "$label" >/dev/tty
    [ "$pct" -ge 100 ] && printf '\n' >/dev/tty
    return 0
}

__weft_py_common='
import sys, os, json, base64, time, stat
def ev(o):
    try:
        t = open("/dev/tty", "w")
    except OSError:
        t = sys.stdout
    t.write("\033]7777;" + base64.b64encode(json.dumps(o).encode()).decode() + "\007")
    t.flush()
    return t
def fit(t, line):
    # Clip the text fallback to the terminal width so it never soft-wraps
    # and stays exactly under the GUI widget rows.
    w = int(os.environ.get("COLUMNS") or 0)  # live value from bash (checkwinsize)
    if w <= 0:
        try:
            w = os.get_terminal_size(t.fileno()).columns
        except (OSError, ValueError, AttributeError):
            w = 200
    return line if len(line) < w else line[:max(w - 2, 1)] + "~"
def human(n):
    for u in "BKMGT":
        if n < 1024: return ("%d%s" % (n, u)) if u == "B" else ("%.1f%s" % (n, u))
        n /= 1024.0
    return "%.1fP" % n
'

__weft_py_wls='
d = os.path.abspath(os.path.expanduser(sys.argv[1] if len(sys.argv) > 1 else "."))
hidden = os.environ.get("WEFT_LS_ALL") == "1"
rows = []
for e in os.scandir(d):
    if e.name.startswith(".") and not hidden: continue
    try:
        st = e.stat(follow_symlinks=False)
        isdir = e.is_dir()
    except OSError:
        continue
    rows.append({"name": e.name + ("/" if isdir else ""), "path": e.path, "dir": isdir,
                 "size": 0 if isdir else st.st_size, "mtime": int(st.st_mtime),
                 "mode": stat.filemode(st.st_mode)})
rows.sort(key=lambda r: (not r["dir"], r["name"].lower()))
t = ev({"t": "widget", "kind": "files", "id": "ls%d-%d" % (os.getpid(), time.time()),
        "rows": len(rows) + 1, "dir": d, "entries": rows})
t.write(fit(t, "%-10s %8s  %-16s %s" % ("MODE", "SIZE", "MODIFIED", "NAME")) + "\n")
for r in rows:
    t.write(fit(t, "%-10s %8s  %-16s %s" % (r["mode"], "-" if r["dir"] else human(r["size"]),
            time.strftime("%Y-%m-%d %H:%M", time.localtime(r["mtime"])), r["name"])) + "\n")
t.flush()
'

__weft_py_wtable='
lines = [l.rstrip("\n") for l in sys.stdin if l.strip()]
if not lines: sys.exit(0)
tab = "\t" in lines[0]
head = lines[0].split("\t") if tab else lines[0].split()
rows = []
for l in lines[1:]:
    cells = l.split("\t") if tab else l.split(None, len(head) - 1)
    rows.append(cells + [""] * (len(head) - len(cells)))
t = ev({"t": "widget", "kind": "table", "id": "tb%d-%d" % (os.getpid(), time.time()),
        "rows": len(lines), "columns": head, "data": rows})
t.write("".join(fit(t, l) + "\n" for l in lines))
t.flush()
'

# wls [DIR] : directory listing as an interactive table (WEFT_LS_ALL=1 for dotfiles)
wls() {
    if [ -n "$__weft_py" ]; then
        COLUMNS=$COLUMNS "$__weft_py" -c "$__weft_py_common$__weft_py_wls" "$@"
    else
        ls -la "$@"
    fi
}

# CMD | wtable : turn whitespace/TSV columns (ps aux, df -h, ...) into a sortable table
wtable() {
    if [ -n "$__weft_py" ]; then
        COLUMNS=$COLUMNS "$__weft_py" -c "$__weft_py_common$__weft_py_wtable"
    else
        cat
    fi
}

__weft_osc "7;file://${HOSTNAME}${PWD}"
