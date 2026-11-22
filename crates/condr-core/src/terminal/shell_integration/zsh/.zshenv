# Condr shell integration (ADR 0055). Condr starts zsh with ZDOTDIR pointing here, so this
# file runs first: it puts the user's ZDOTDIR back and reads the user's .zshenv in its
# place. An interactive shell then does what Ghostty's integration does, written afresh
# here: it marks each prompt and command for the terminal (OSC 133), reports its
# directory (OSC 7) and sets the cursor's shape. Before any of the person's files, it
# tells the Server marks will come (ADR 0059).
[[ -o interactive ]] && builtin printf '\e]777;notify;condr://shell;integration\a'
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
  # 0 before the first prompt, 1 while a command runs, 2 at a prompt.
  builtin typeset -gi __condr_state=0
  # The marks reach the terminal even while the shell's output is redirected, through a
  # descriptor the programs it starts do not inherit. Opened now, before the person's
  # .zshrc runs anything that might close it.
  builtin typeset -gi __condr_fd=1
  {
    builtin zmodload zsh/system && builtin sysopen -o cloexec -wu __condr_fd -- ${TTY:-/dev/tty}
  } 2>/dev/null || __condr_fd=1
  # The prompts as the person set them, and as Condr last marked them.
  builtin typeset -g __condr_ps1 __condr_ps2 __condr_marked_ps1

  # Hooked last in precmd_functions, so that the prompt it marks is the one other hooks
  # set; zsh hands each hook the command's own exit status.
  __condr_prompt_mark() {
    builtin local -i exit_status=$?
    builtin emulate -L zsh -o no_aliases
    # A plugin that runs the hooks from inside the line editor gets no marks printed.
    if ! builtin zle; then
      if (( __condr_state == 1 )); then
        builtin print -nu $__condr_fd "\e]133;D;$exit_status\a"
      elif (( __condr_state == 2 )); then
        # Closes a command another program may have started.
        builtin print -nu $__condr_fd '\e]133;D\a'
      fi
    fi
    __condr_report_pwd
    if [[ -o prompt_percent && ${precmd_functions[-1]} == __condr_prompt_mark ]]; then
      __condr_mark_prompts
      return
    fi
    # A hook after this one may set the prompt again: take the last place for the next
    # prompt, and mark this one by hand.
    [[ -o prompt_percent ]] && precmd_functions=(${precmd_functions:#__condr_prompt_mark} __condr_prompt_mark)
    if ! builtin zle; then
      builtin print -nu $__condr_fd '\e]133;A;cl=line\a'
      __condr_state=2
    fi
  }

  # The marks go into PS1 and PS2 themselves, so the terminal gets them again whenever
  # zsh redraws the prompt: on a resize, a reset-prompt or a job's notice.
  __condr_mark_prompts() {
    builtin local changed=0
    if [[ -n $__condr_marked_ps1 ]]; then
      if [[ $PS1 == "$__condr_marked_ps1" ]]; then
        # Unchanged since: start again from the person's own.
        PS1=$__condr_ps1 PS2=$__condr_ps2
      elif [[ $PS1 != "$__condr_ps1" ]]; then
        changed=1
      fi
    fi
    __condr_ps1=$PS1 __condr_ps2=$PS2
    # A prompt ending in a lone % would make it one escape with the mark's %{.
    [[ $PS1 == *[^%]% || $PS1 == % ]] && PS1+=%
    PS1=$'%{\e]133;A;cl=line\a%}'$PS1$'%{\e]133;B\a%}'
    # Each further line of the prompt is a secondary one, left alone in a prompt a theme
    # rebuilt since, as it may take the prompt apart by pattern.
    (( changed )) || PS1=${PS1//$'\n'/$'\n%{\e]133;P;k=s\a%}'}
    [[ $PS2 == *[^%]% || $PS2 == % ]] && PS2+=%
    PS2=$'%{\e]133;P;k=s\a%}'$PS2$'%{\e]133;B\a%}'
    __condr_marked_ps1=$PS1
    __condr_state=2
  }

  __condr_command_mark() {
    builtin emulate -L zsh -o no_aliases
    # Other hooks see the prompts as the person set them, unless one changed them since.
    if [[ -n $__condr_marked_ps1 && $PS1 == "$__condr_marked_ps1" ]]; then
      PS1=$__condr_ps1 PS2=$__condr_ps2
    fi
    # The command starts with the terminal's default cursor.
    builtin print -nu $__condr_fd '\e]133;C\a\e[0 q'
    __condr_state=1
  }

  # A directory's name is untrusted: print -r keeps its backslashes as they are, and its
  # control bytes go, so it can neither end the sequence nor start one of its own.
  __condr_report_pwd() {
    builtin print -rnu $__condr_fd -- \
      $'\e]7;kitty-shell-cwd://'"${HOST//[[:cntrl:]]}${PWD//[[:cntrl:]]}"$'\a'
  }

  # A blinking bar while typing, a blinking block in vi command mode.
  __condr_cursor() {
    case $KEYMAP in
      vicmd|visual) builtin print -nu $__condr_fd '\e[1 q' ;;
      *) builtin print -nu $__condr_fd '\e[5 q' ;;
    esac
  }
  __condr_zle_line_init() {
    # Something set the prompt after Condr marked it: mark where the input starts.
    [[ $PS1 == *$'%{\e]133;A'* ]] || builtin print -nu $__condr_fd '\e]133;P;k=i\a\e]133;B\a'
    __condr_cursor
  }
  __condr_zle_line_finish() { __condr_cursor }
  __condr_zle_keymap_select() { __condr_cursor }

  # At the first prompt, once the person's .zshrc has run: the hooks then go where they
  # see the prompts and widgets it set.
  __condr_init() {
    builtin emulate -L zsh -o no_aliases
    precmd_functions=(${precmd_functions:#__condr_init})
    builtin local hook widget fn flag
    for hook in line-init line-finish keymap-select; do
      widget=zle-$hook fn=__condr_zle_${hook//-/_}
      if [[ $widgets[$widget] == user:azhw:* ]] && (( $+functions[add-zle-hook-widget] )); then
        # Hooked through add-zle-hook-widget: joining its list is the one safe way in.
        add-zle-hook-widget $hook $fn
        continue
      fi
      if (( $+widgets[$widget] )); then
        # Keep the widget the person defined, and run it after Condr's.
        builtin zle -A $widget .__condr_orig_$widget
        flag=w
        [[ $widgets[$widget] == user:* ]] && flag=
        functions[$fn]+=$'\n'"builtin zle .__condr_orig_$widget -N$flag -- \"\$@\""
      fi
      builtin zle -N $widget $fn
    done
    builtin typeset -ga preexec_functions chpwd_functions
    preexec_functions+=(__condr_command_mark)
    chpwd_functions+=(__condr_report_pwd)
    precmd_functions+=(__condr_prompt_mark)
    __condr_prompt_mark
  }
  builtin typeset -ga precmd_functions
  precmd_functions+=(__condr_init)
fi
