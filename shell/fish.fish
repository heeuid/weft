# weft shell integration for fish: prompt and command marks (OSC 133) and the
# current directory (OSC 7). Run by `fish -C` after the user's config, from
# the WEFT_FISH environment variable (nothing is written to the remote disk).

# The block ends and the prompt starts before the theme runs: before the
# mode prompt (vi's [I], drawn first: it was taken for the command's
# output), else before the prompt. $status is the last command's.
function __weft_head -a s
    printf '\e]133;D;%s\a\e]7;file://%s%s\a\e]133;A\a' $s $hostname "$PWD"
end

if not functions -q __weft_orig_prompt
    functions -c fish_prompt __weft_orig_prompt
    functions -q fish_mode_prompt; and functions -c fish_mode_prompt __weft_orig_mode_prompt
    function fish_mode_prompt
        set -l s $status
        __weft_head $s
        set -g __weft_headed 1
        if functions -q __weft_orig_mode_prompt
            __weft_status $s
            __weft_orig_mode_prompt
        end
    end
    function fish_prompt
        set -l s $status
        if set -q __weft_headed
            set -e __weft_headed
        else
            __weft_head $s
        end
        __weft_keys
        # The theme sees the command's status, not printf's.
        __weft_status $s
        __weft_orig_prompt
        printf '\e]133;B\a'
    end
end

function __weft_status
    return $argv[1]
end

function __weft_start --on-event fish_preexec
    printf '\e]133;C\a'
end

# weft's key to empty the line (Ctrl+Alt+Shift+F12), in every mode. With vi
# bindings "default" is the normal mode, left for insert; with emacs ones it
# is the only mode, and stays. fish 4 names the key, fish 3 takes its bytes
# (it would bind the name as typed letters). Bound again at each prompt:
# fish_vi_key_bindings and fish_default_key_bindings erase all bindings.
function __weft_keys
    set -l k \e\[24\;8~
    test (string split . -- $version)[1] -ge 4 2>/dev/null; and set k ctrl-alt-shift-f12
    string match -q '*vi*' -- "$fish_key_bindings"; or begin; bind $k 'commandline ""'; return; end
    for i in default visual insert
        bind -M $i -m insert $k 'commandline ""'
    end
end
__weft_keys
printf '\e]7777;{"t":"clear-key"}\a'

set -gx WEFT 1
set -e WEFT_FISH
# The first prompt's status is 0 (set -e of a name not set is 4).
true
