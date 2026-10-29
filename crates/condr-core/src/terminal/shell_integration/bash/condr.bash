# Condr shell integration (ADR 0055): marks each prompt and command for the terminal
# (OSC 133). Sourced after the user's ~/.bashrc, so it runs first in PROMPT_COMMAND and
# reads the command's own exit status. bash before 4.4 has no PS0 to mark a command's
# start, and a prompt mark without one would leave the terminal thinking every program
# sits at a prompt, so it gets no marks at all.
if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4))) &&
  ! declare -F __condr_prompt_mark >/dev/null; then
  __condr_prompts=0
  __condr_prompt_mark() {
    local exit_status=$1
    (( __condr_prompts++ )) && builtin printf '\e]133;D;%d\a' "$exit_status"
    builtin printf '\e]133;A\a'
    return "$exit_status"
  }
  # Guarded, since an exported PROMPT_COMMAND reaches nested shells without the function.
  PROMPT_COMMAND='__condr_status=$?; if declare -F __condr_prompt_mark >/dev/null; then __condr_prompt_mark "$__condr_status"; else (exit "$__condr_status"); fi'"${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  PS0="${PS0-}"$'\e]133;C\a'
fi
