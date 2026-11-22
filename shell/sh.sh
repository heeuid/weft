# weft shell integration for POSIX shells that have no prompt hooks: ksh,
# mksh, dash, busybox ash, a plain sh.
#
# Delivered in memory only: the bootstrap starts the shell with ENV=/dev/fd/3
# and this text on that descriptor, so nothing is written to the remote disk.
#
# Partial: these shells run nothing before a command, only expand PS1 before
# a prompt. So weft is told where each prompt starts, the exit code of what
# ran before it and the folder (OSC 133 D and A, OSC 7), all exact; not
# where the typed command ends, nor its text. No w* helpers either.

__weft_at=$WEFT_AT
__weft_sh=$WEFT_SH
unset W D R ENV WEFT_AT WEFT_SH WEFT_CWD

# ENV took the place of the shell's own startup files: read a login shell's
# (/etc/profile, ~/.profile), then the interactive one the shell would have
# read: $ENV if those set it, else ksh's and mksh's own.
# (`command .`: an error in one of them, a bash-only line say, ends that
# file and not this one; with a bare `.` the shell stopped reading here and
# had no integration.)
if [ -r /etc/profile ]; then command . /etc/profile; fi
if [ -r "$HOME/.profile" ]; then command . "$HOME/.profile"; fi
if [ -n "${ENV:-}" ]; then
    __weft_env=$ENV
    [ -r "$__weft_env" ] || eval "__weft_env=\"$ENV\"" 2>/dev/null
    if [ -r "$__weft_env" ]; then command . "$__weft_env"; fi
    unset __weft_env
else
    case $__weft_sh in
    ksh|ksh93)
        if [ -r /etc/ksh.kshrc ]; then command . /etc/ksh.kshrc; fi
        if [ -r "$HOME/.kshrc" ]; then command . "$HOME/.kshrc"; fi ;;
    mksh)
        if [ -r "$HOME/.mkshrc" ]; then command . "$HOME/.mkshrc"; fi ;;
    esac
fi

if [ -n "$__weft_at" ] && [ -d "$__weft_at" ]; then cd "$__weft_at"; fi
# FreeBSD's sh (DragonFly's too) runs no command in a prompt, NetBSD's
# only with promptcmds (8.0 on): where none runs, the prompt is left as it
# was (no marks, and no `__weft_p` shown in it). (Tried in a subshell
# first: a failed `set` ends the reading of this file in dash and others.)
__weft_cmd=1
case $__weft_sh:`uname -s` in
sh:FreeBSD|sh:DragonFly) __weft_cmd= ;;
sh:NetBSD) (set -o promptcmds) 2>/dev/null && set -o promptcmds || __weft_cmd= ;;
esac
unset __weft_at __weft_sh

export WEFT=1
__weft_host=${HOSTNAME:-$(uname -n 2>/dev/null)}

# Run by the shell as it expands PS1, in a subshell: its output goes to the
# terminal itself, and none into the prompt (a prompt's width stays right).
__weft_p() {
    __weft_e=$?
    printf '\033]133;D;%s\007\033]133;A\007\033]7;file://%s%s\007' "$__weft_e" "$__weft_host" "$PWD" >/dev/tty 2>/dev/null
    return "$__weft_e"
}
# (Backquotes, not $(...): where sh is Cygwin's or MSYS' bash, $(...) in a
# PS1 that has a newline is a syntax error at every prompt.)
# (2>&-: a PS1 the user's profile exported goes to the shells started from
# this one, where __weft_p is not: "not found" at each of their prompts.)
[ -z "$__weft_cmd" ] || PS1='`__weft_p 2>&-`'"${PS1:-\$ }"
unset __weft_cmd

exec 3<&-
