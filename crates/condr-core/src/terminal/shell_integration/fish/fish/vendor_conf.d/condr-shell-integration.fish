# Condr shell integration (ADR 0055). It does what Ghostty's integration does: it marks
# each prompt and command for the terminal (OSC 133), reports its directory (OSC 7) and
# sets the cursor's shape. Condr puts this directory on XDG_DATA_DIRS for the shell it
# starts, and it comes off again here so the programs the shell runs do not inherit it.
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
    if status is-interactive; and not functions -q __condr_setup
        # vendor_conf.d runs before config.fish: tell the Server marks will come (ADR 0059).
        printf '\e]777;notify;condr://shell;integration\a'
        # At the first prompt, so the handlers come after whatever the person's config
        # set up.
        function __condr_setup --on-event fish_prompt
            functions -e __condr_setup
            # fish 4.1 and later take a click at the prompt to move the cursor.
            set -g __condr_prompt_start '\e]133;A\a'
            set -l release (string match -r '^(\d+)\.(\d+)' -- $version)
            if test "$release[2]" -gt 4 2>/dev/null
                or test "$release[2]" -eq 4 -a "$release[3]" -ge 1 2>/dev/null
                set -g __condr_prompt_start '\e]133;A;click_events=1\a'
            end
            # A blinking bar while typing, the terminal's default for a command; fish's own
            # vi mode cursor wins when it has one.
            function __condr_prompt_cursor --on-event fish_prompt
                functions -q fish_vi_cursor_handle; or printf '\e[5 q'
            end
            function __condr_command_cursor --on-event fish_preexec
                functions -q fish_vi_cursor_handle; or printf '\e[0 q'
            end
            function __condr_prompt_mark --on-event fish_prompt --on-event fish_posterror
                # A command whose end went unmarked still gets one.
                test "$__condr_state" = command; and printf '\e]133;D\a'
                set -g __condr_state prompt
                printf $__condr_prompt_start
            end
            function __condr_command_mark --on-event fish_preexec
                set -g __condr_state command
                printf '\e]133;C\a'
            end
            function __condr_command_end_mark --on-event fish_postexec
                printf '\e]133;D;%d\a' $status
                set -g __condr_state ended
            end
            # fish reports its directory itself only to terminals it knows.
            function __condr_report_pwd --on-variable PWD
                status is-command-substitution; and return
                printf '\e]7;file://%s%s\a' $hostname (string escape --style=url -- $PWD)
            end
            # The terminal clears the prompt on a resize, so fish redraws it whole.
            set -g fish_handle_reflow 1
            # This first prompt drew before the handlers existed.
            __condr_prompt_cursor
            __condr_prompt_mark
            __condr_report_pwd
        end
    end
end
