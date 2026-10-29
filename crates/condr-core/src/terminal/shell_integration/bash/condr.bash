# Condr shell integration (ADR 0055): marks each prompt and command for the terminal
# (OSC 133). Sourced after the user's ~/.bashrc, so it runs first in PROMPT_COMMAND and
# reads the command's own exit status. bash before 4.4 has no PS0: its prompts are
# marked, its commands are not.
if ! declare -F __condr_prompt_mark >/dev/null; then
  __condr_prompts=0
  __condr_prompt_mark() {
    local exit_status=$?
    (( __condr_prompts++ )) && builtin printf '\e]133;D;%d\a' "$exit_status"
    builtin printf '\e]133;A\a'
    return "$exit_status"
  }
  PROMPT_COMMAND="__condr_prompt_mark${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  PS0="${PS0-}"$'\e]133;C\a'
fi
