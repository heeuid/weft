# weft shell integration for zsh: prompt and command marks (OSC 133) and the
# current directory (OSC 7).
#
# zsh can only take startup code from a file ($ZDOTDIR/.zshrc), and nothing
# may be written to the remote disk, so the bootstrap passes this in the
# WEFT_ZSH environment variable and weft types ` eval "$WEFT_ZSH"` at the
# first prompt (leading space + HIST_IGNORE_SPACE: not kept in history).

setopt HIST_IGNORE_SPACE

__weft_osc() { print -n "\e]$1\a" > /dev/tty }

# First in precmd: the block ends before any prompt theme spends time.
__weft_done() {
    local ec=$?
    __weft_osc "133;D;$ec"
    __weft_osc "7;file://${HOST}${PWD}"
}

# Last in precmd: themes (powerlevel10k, ...) rebuild PS1 in their own hooks.
__weft_marks() {
    [[ $PS1 == *'133;A'* ]] || PS1=$'%{\e]133;A\a%}'"$PS1"$'%{\e]133;B\a%}'
}

__weft_start() { __weft_osc "133;C" }

precmd_functions=(__weft_done ${precmd_functions:#__weft_*} __weft_marks)
preexec_functions+=(__weft_start)

# weft's key to empty the line (Ctrl+Alt+Shift+F12), in every keymap: from
# vi's command mode back into insert.
__weft_clear() { BUFFER=; CURSOR=0; zle -K main }
zle -N __weft_clear
bindkey -M emacs '^[[24;8~' __weft_clear; bindkey -M viins '^[[24;8~' __weft_clear; bindkey -M vicmd '^[[24;8~' __weft_clear
__weft_osc '7777;{"t":"clear-key"}'

export WEFT=1
unset WEFT_ZSH

# Erase the line that typed this in; the next prompt is drawn in its place.
print -n '\e[1A\r\e[2K' > /dev/tty
