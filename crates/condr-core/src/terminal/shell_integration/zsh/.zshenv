# Condr shell integration (ADR 0055). Condr starts zsh with ZDOTDIR pointing here, so this
# file runs first: it puts the user's ZDOTDIR back, reads the user's .zshenv in its place,
# and in an interactive shell marks each prompt and command for the terminal (OSC 133).
if [[ -n ${CONDR_USER_ZDOTDIR+set} ]]; then
  ZDOTDIR=$CONDR_USER_ZDOTDIR
  unset CONDR_USER_ZDOTDIR
else
  unset ZDOTDIR
fi
if [[ -f ${ZDOTDIR:-$HOME}/.zshenv ]]; then
  builtin source -- "${ZDOTDIR:-$HOME}/.zshenv"
fi
if [[ -o interactive ]] && (( ! ${+functions[__condr_prompt_mark]} )); then
  typeset -gi __condr_prompts=0
  # Added before the user's .zshrc adds any, so it runs first and reads the command's
  # own exit status.
  __condr_prompt_mark() {
    local exit_status=$?
    (( __condr_prompts++ )) && builtin printf '\e]133;D;%d\a' $exit_status
    builtin printf '\e]133;A\a'
  }
  __condr_command_mark() {
    builtin printf '\e]133;C\a'
  }
  autoload -Uz add-zsh-hook
  add-zsh-hook precmd __condr_prompt_mark
  add-zsh-hook preexec __condr_command_mark
fi
