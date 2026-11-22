# weft shell integration for bash.
#
# Delivered in memory only: the bootstrap runs `bash --rcfile <(printf %s "$R") -i`
# over the ssh PTY, so nothing is ever written to the remote disk.
# Everything here uses plain POSIX tools; python is only used by the rich
# helpers (wls/wtable) and they fall back to plain text without it.

__weft_dec=$D
__weft_cwd=$WEFT_CWD
unset W D R WEFT_CWD
# bash 3 had this file on descriptor 3 (it cannot read an rcfile from a
# pipe): read now, closed.
if [ -n "$WEFT_RC3" ]; then exec 3<&-; fi
unset WEFT_RC3

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


# --- low level -------------------------------------------------------------

__weft_osc() { printf '\033]%s\007' "$1" >/dev/tty; }

# Structured events travel as plain JSON (no base64 tool needed): its
# strings come from __weft_js, which leaves no control byte to end the OSC.
__weft_ev() { __weft_osc "7777;$1"; }

__weft_js() {
    local s=$1
    s=${s//\\/\\\\}; s=${s//\"/\\\"}; s=${s//$'\t'/\\t}
    s=${s//$'\n'/\\n}; s=${s//[[:cntrl:]]/}
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
    __weft_armed=
    __weft_osc "133;D;$ec"
    __weft_osc "7;file://${HOSTNAME}${PWD}"
    return $ec
}

__weft_ps1() {
    case "$PS1" in
        *'133;A'*) ;;
        *) PS1='\[\033]133;A\007\]'"$PS1"'\[\033]133;B\007\]' ;;
    esac
    # The prompt is next: the first command after it is the user's.
    __weft_armed=1
}

# An array of more than one, which bash 5.1+ runs element by element: ours
# first and last, as for a string (a later element could set PS1 anew after
# __weft_ps1). Before 5.1 only its first element runs: ours go into that
# one, as ever. Joined by newlines: the user's often ends in `;` (`history
# -a; $PROMPT_COMMAND` with it empty), and `; ;` is an error at every
# prompt that ran none of it.
if [[ ${PROMPT_COMMAND[1]+x} ]] && (( BASH_VERSINFO[0] * 100 + BASH_VERSINFO[1] > 500 )); then
    PROMPT_COMMAND=(__weft_precmd "${PROMPT_COMMAND[@]}" __weft_ps1)
else
    PROMPT_COMMAND="__weft_precmd${PROMPT_COMMAND:+$'\n'$PROMPT_COMMAND}"$'\n'__weft_ps1
fi

if (( BASH_VERSINFO[0] * 100 + BASH_VERSINFO[1] >= 404 )); then
    PS0='\033]133;C\007'
else
    # bash < 4.4 (macOS /bin/bash) has no PS0: the DEBUG trap, which runs
    # before every simple command, the prompt's own (PROMPT_COMMAND) too.
    # Only the first one after the prompt is shown is the user's (its
    # function sets __weft_armed last); it said 133;C at the prompt's own
    # before, and not at the user's command.
    __weft_preexec() {
        [ -n "${COMP_LINE:-}" ] && return
        [ -z "${__weft_armed:-}" ] && return
        case "$BASH_COMMAND" in __weft_precmd*) return ;; esac
        __weft_armed=
        __weft_osc "133;C"
    }
    trap '__weft_preexec' DEBUG
fi

# --- weft's key to empty the line --------------------------------------------
# Ctrl+Alt+Shift+F12, in every editing mode (emacs, vi's insert and command:
# from the command one into insert first): weft's runs (Rerun, cd here)
# empty what was typed with it, whatever mode the line is in.
bind -m emacs '"\e[24;8~": kill-whole-line' && bind -m vi-insert '"\e[24;8~": kill-whole-line' && bind -m vi-command '"\e[24;8~": "i\e[24;8~"' && __weft_ev '{"t":"clear-key"}'


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

# Clip a line to the terminal's width: it never wraps, and stays under its
# widget's rows. COLUMNS: bash's own (checkwinsize).
__weft_fit() {
    local w=${COLUMNS:-0} l
    while IFS= read -r l; do
        if [ "$w" -gt 2 ] && [ ${#l} -ge "$w" ]; then l="${l:0:w-2}~"; fi
        printf '%s\n' "$l"
    done
}

# wls [DIR] : directory listing as an interactive table (WEFT_LS_ALL=1 for
# dotfiles). Its rows are ls' (a header, then one a name); weft lists the
# folder itself, through its agent.
wls() {
    local d t n
    d=$(cd -- "${1:-.}" 2>/dev/null && pwd -P) || { ls -l "$@"; return; }
    if [ "${WEFT_LS_ALL:-}" = 1 ]; then t=$(ls -lA -- "$d" | __weft_fit); else t=$(ls -l -- "$d" | __weft_fit); fi
    n=$(printf '%s\n' "$t" | wc -l)
    __weft_ev "{\"t\":\"widget\",\"kind\":\"files\",\"id\":\"ls$$-$RANDOM\",\"rows\":${n// /},\"dir\":$(__weft_js "$d"),\"hidden\":$([ "${WEFT_LS_ALL:-}" = 1 ] && echo true || echo false)}"
    printf '%s\n' "$t" >/dev/tty
}

# CMD | wtable : turn whitespace/TSV columns (ps aux, df -h, ...) into a
# sortable table; weft splits the text into its columns.
wtable() {
    local l t= n=0
    while IFS= read -r l || [ -n "$l" ]; do
        [ -n "${l//[[:space:]]/}" ] || continue
        t+=$l$'\n'; n=$((n + 1))
    done
    [ "$n" -gt 0 ] || return 0
    t=$(printf '%s' "$t" | __weft_fit)
    __weft_ev "{\"t\":\"widget\",\"kind\":\"table\",\"id\":\"tb$$-$RANDOM\",\"rows\":$n,\"text\":$(__weft_js "$t")}"
    printf '%s\n' "$t" >/dev/tty
}

__weft_osc "7;file://${HOSTNAME}${PWD}"
