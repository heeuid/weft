# weft shell integration for fish: prompt and command marks (OSC 133) and the
# current directory (OSC 7). Run by `fish -C` after the user's config, from
# the WEFT_FISH environment variable (nothing is written to the remote disk).

if not functions -q __weft_orig_prompt
    functions -c fish_prompt __weft_orig_prompt
    function fish_prompt
        # $status is the last command's; the block ends before the theme runs.
        set -l s $status
        printf '\e]133;D;%s\a\e]7;file://%s%s\a\e]133;A\a' $s $hostname "$PWD"
        __weft_orig_prompt
        printf '\e]133;B\a'
    end
end

function __weft_start --on-event fish_preexec
    printf '\e]133;C\a'
end

set -gx WEFT 1
set -e WEFT_FISH
