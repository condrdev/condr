# Condr shell integration (ADR 0055), sourced after the user's ~/.bashrc. It does what
# Ghostty's integration does, written afresh here: it marks each prompt and command for
# the terminal (OSC 133), reports its directory (OSC 7) and sets the cursor's shape. bash
# before 4.4 has no PS0 to mark a command's start, and a prompt mark without one would
# leave the terminal thinking every program sits at a prompt, so it gets nothing.
if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4))) &&
  ! declare -F __condr_prompt_mark >/dev/null; then
  __condr_prompts=0
  __condr_prompt_mark() {
    local exit_status=$1
    (( __condr_prompts++ )) && builtin printf '\e]133;D;%d;aid=%d\a' "$exit_status" "$BASHPID"
    # The marks go into PS1 and PS2, so readline sends them again whenever it redraws the
    # prompt. Start again from the person's own prompts, unless something set new ones.
    [[ $PS1 == "${__condr_marked_ps1-}" ]] && PS1=$__condr_ps1 PS2=$__condr_ps2
    __condr_ps1=$PS1 __condr_ps2=$PS2
    # P rather than A inside the prompt, so a redraw (a vi mode switch, Ctrl-L) does not
    # move to a fresh line. Each further line of the prompt is a secondary one; only the
    # \n escape counts, since a literal newline may sit inside a $(...). The cursor is a
    # blinking bar while typing.
    PS1='\[\e]133;P;k=i\a\]'${PS1//\\n/\\n'\[\e]133;P;k=s\a\]'}'\[\e]133;B\a\]\[\e[5 q\]'
    PS2='\[\e]133;P;k=s\a\]'$PS2'\[\e]133;B\a\]'
    __condr_marked_ps1=$PS1
    # bash redraws only a prompt's last line after a resize. ble.sh keeps track of the
    # cursor itself, and A's move to a fresh line would throw it off.
    if [[ -n ${BLE_VERSION-} ]]; then
      builtin printf '\e]133;P;k=i\a'
    else
      builtin printf '\e]133;A;redraw=last;cl=line;aid=%d\a' "$BASHPID"
    fi
    # bash has no hook for a change of directory, so the prompt reports it. Its name is
    # untrusted: printf leaves escapes in an argument alone, and control bytes go.
    if [[ $PWD != "${__condr_pwd-}" ]]; then
      __condr_pwd=$PWD
      builtin printf '\e]7;kitty-shell-cwd://%s%s\a' "${HOSTNAME//[[:cntrl:]]/}" "${PWD//[[:cntrl:]]/}"
    fi
  }
  # Last in PROMPT_COMMAND, so the prompt it marks is the one the person's commands set,
  # with the command's exit status taken first. An exported PROMPT_COMMAND reaches nested
  # shells without the function, hence the guard.
  __condr_hook='declare -F __condr_prompt_mark >/dev/null && __condr_prompt_mark "$__condr_status"'
  if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == "declare -a"* ]]; then
    # bash hands each entry the command's status.
    PROMPT_COMMAND+=('__condr_status=$?; '"$__condr_hook")
  elif [[ -n ${PROMPT_COMMAND-} ]]; then
    PROMPT_COMMAND='__condr_status=$?; (exit "$__condr_status")'$'\n'"$PROMPT_COMMAND"$'\n'"$__condr_hook"
  else
    PROMPT_COMMAND='__condr_status=$?; '"$__condr_hook"
  fi
  unset __condr_hook
  # The command starts with the terminal's default cursor.
  PS0="${PS0-}"$'\e]133;C\a\e[0 q'
  # From 5.3, PS0 can run a function in the shell itself: the command then sees the
  # prompts as the person set them, unless something changed them since.
  if ((BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 3))); then
    __condr_unmark_prompts() {
      [[ $PS1 == "${__condr_marked_ps1-}" ]] && PS1=$__condr_ps1 PS2=$__condr_ps2
      __condr_marked_ps1=
    }
    PS0+='${ __condr_unmark_prompts; }'
  fi
fi
