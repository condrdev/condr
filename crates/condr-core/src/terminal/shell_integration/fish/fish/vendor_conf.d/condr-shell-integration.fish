# Condr shell integration (ADR 0055): marks each prompt and command for the terminal
# (OSC 133). Condr puts this directory on XDG_DATA_DIRS for the shell it starts, and it
# comes off again here so the programs the shell runs do not inherit it.
if set -q CONDR_FISH_DATA_DIR
    set -l kept
    for dir in (string split : -- $XDG_DATA_DIRS)
        test "$dir" = "$CONDR_FISH_DATA_DIR"; or set -a kept $dir
    end
    # Condr supplied fish's defaults behind its own when the variable was unset.
    if set -q CONDR_FISH_DATA_DIRS_UNSET
        set -e XDG_DATA_DIRS
    else
        set -gx XDG_DATA_DIRS (string join : -- $kept)
    end
    set -e CONDR_FISH_DATA_DIR CONDR_FISH_DATA_DIRS_UNSET
    if status is-interactive; and not functions -q __condr_prompt_mark
        function __condr_prompt_mark --on-event fish_prompt
            printf '\e]133;A\a'
        end
        function __condr_command_mark --on-event fish_preexec
            printf '\e]133;C\a'
        end
        function __condr_command_end_mark --on-event fish_postexec
            printf '\e]133;D;%d\a' $status
        end
    end
end
