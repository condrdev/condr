//! The injected shell integration (ADR 0055), run in real shells with a home of their own.

use super::*;
use crate::terminal::shell_integration::apply;

/// A shell with Condr's integration in `home`, started the way `spawn_shell` starts one,
/// with the person's own `ZDOTDIR` when given.
fn integrated_shell(program: &str, home: &Path, zdotdir: Option<&Path>) -> Option<TerminalRuntime> {
    let program = which(program)?;
    let integration = install_shell_integration(&home.join("data")).unwrap();
    let mut command = shell_command(&program);
    command.env("HOME", home);
    match zdotdir {
        Some(zdotdir) => command.env("ZDOTDIR", zdotdir),
        None => command.env_remove("ZDOTDIR"),
    }
    command.env_remove("XDG_DATA_DIRS");
    command.cwd(home);
    let announces = apply(&mut command, &program, &integration);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(24, 120)).unwrap();
    runtime.announces = announces;
    Some(runtime)
}

fn which(program: &str) -> Option<String> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(program))
        .find(|path| path.is_file())
        .map(|path| path.display().to_string())
}

/// Whether the bash on PATH gets marks: 4.4 and later, which have PS0.
fn bash_has_marks() -> bool {
    let Ok(output) = std::process::Command::new("bash")
        .args(["-c", "echo ${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"])
        .output()
    else {
        return false;
    };
    let version = String::from_utf8_lossy(&output.stdout);
    let mut parts = version.split_whitespace().map(|part| part.parse::<u32>());
    matches!((parts.next(), parts.next()), (Some(Ok(major)), Some(Ok(minor))) if (major, minor) >= (4, 4))
}

fn wait_for(runtime: &TerminalRuntime, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if runtime.visible_text().contains(needle) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("never showed {needle:?}: {}", runtime.visible_text());
}

fn home(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!(
        "condr-shell-integration-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home
}

#[test]
fn the_scripts_install_once_per_content_and_a_missing_one_is_put_back() {
    let root = home("install");
    let directory = install_shell_integration(&root).unwrap();
    assert!(
        directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("shell-integration-")
    );
    assert_eq!(install_shell_integration(&root).unwrap(), directory);

    std::fs::remove_file(directory.join("zsh/.zshenv")).unwrap();
    let mut command = CommandBuilder::new("zsh");
    apply(&mut command, "zsh", &directory);
    assert!(
        command.get_env("CONDR_USER_ZDOTDIR").is_none() && command.get_env("ZDOTDIR").is_none(),
        "zsh pointed at a directory without Condr's .zshenv would skip the person's files"
    );
    assert_eq!(install_shell_integration(&root).unwrap(), directory);
    assert!(directory.join("zsh/.zshenv").is_file(), "put back");

    let leftovers = std::fs::read_dir(&root)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.')
        })
        .count();
    assert_eq!(leftovers, 0, "no staging directory stays behind");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn zsh_reads_the_users_startup_files_and_gets_the_marks() {
    let home = home("zsh");
    std::fs::write(home.join(".zshenv"), "export FROM_ZSHENV=env\n").unwrap();
    std::fs::write(home.join(".zshrc"), "PROMPT='user> '\n").unwrap();
    let Some(runtime) = integrated_shell("zsh", &home, None) else {
        return;
    };
    wait_for(&runtime, "user>");
    runtime
        .write(
            b"print -r \"z=${ZDOTDIR-unset} e=$FROM_ZSHENV m=${+functions[__condr_prompt_mark]}\"\r"
                .to_vec(),
        )
        .unwrap();
    wait_for(&runtime, "z=unset e=env m=1");
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn zsh_gets_the_users_own_zdotdir_back() {
    let home = home("zsh-zdotdir");
    let zdotdir = home.join("dot");
    std::fs::create_dir_all(&zdotdir).unwrap();
    std::fs::write(zdotdir.join(".zshenv"), "export FROM_ZDOTDIR=yes\n").unwrap();
    std::fs::write(zdotdir.join(".zshrc"), "PROMPT='dot> '\n").unwrap();
    let Some(runtime) = integrated_shell("zsh", &home, Some(&zdotdir)) else {
        return;
    };
    wait_for(&runtime, "dot>");
    runtime
        .write(
            b"print -r \"z=$ZDOTDIR c=${CONDR_USER_ZDOTDIR-unset} e=$FROM_ZDOTDIR m=${+functions[__condr_prompt_mark]}\"\r"
                .to_vec(),
        )
        .unwrap();
    wait_for(
        &runtime,
        &format!("z={} c=unset e=yes m=1", zdotdir.display()),
    );
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn bash_reads_the_users_bashrc_without_condrs_variable_and_gets_marks_from_4_4() {
    let home = home("bash");
    std::fs::write(
        home.join(".bashrc"),
        "PS1='user$ '\nFROM_BASHRC=rc\nSEEN=${CONDR_BASH_INTEGRATION-unset}\n",
    )
    .unwrap();
    let Some(runtime) = integrated_shell("bash", &home, None) else {
        return;
    };
    wait_for(&runtime, "user$");
    runtime
        .write(
            b"echo \"r=$FROM_BASHRC s=$SEEN i=${CONDR_BASH_INTEGRATION-unset} m=$(declare -F __condr_prompt_mark)\"\r"
                .to_vec(),
        )
        .unwrap();
    let marks = if bash_has_marks() {
        "__condr_prompt_mark"
    } else {
        ""
    };
    wait_for(&runtime, &format!("r=rc s=unset i=unset m={marks}"));
    let _ = std::fs::remove_dir_all(home);
}

/// Cmd+Up and Cmd+Down at the shell's prompt move the viewport by prompts; while a
/// command runs they belong to it.
fn jumps_between_prompts(program: &str, prompt: &str, marked: bool) {
    let home = home(&format!("{program}-jump"));
    std::fs::write(home.join(".zshrc"), "PROMPT='user> '\n").unwrap();
    std::fs::write(home.join(".bashrc"), "PS1='user$ '\n").unwrap();
    let Some(runtime) = integrated_shell(program, &home, None) else {
        return;
    };
    wait_for(&runtime, prompt);
    for round in ["first", "second"] {
        runtime
            .write(format!("seq 1 40; echo {round}-done\r").into_bytes())
            .unwrap();
        wait_for(&runtime, &format!("{round}-done"));
    }
    wait_for_prompt_after(&runtime, "second-done", prompt);
    let key = |key| TerminalCommand::Key {
        key,
        modifiers: TerminalModifiers {
            platform: true,
            ..TerminalModifiers::default()
        },
        kind: TerminalKeyEventKind::Press,
    };
    runtime.execute(key(TerminalKey::Up)).unwrap();
    let up = runtime.view().display_offset;
    if !marked {
        assert_eq!(up, 0, "{program}: no marks, no jump");
        let _ = std::fs::remove_dir_all(home);
        return;
    }
    assert!(up > 0, "{program}: Cmd+Up left the bottom");
    runtime.execute(key(TerminalKey::Up)).unwrap();
    assert!(
        runtime.view().display_offset > up,
        "{program}: one more prompt up"
    );
    runtime.execute(key(TerminalKey::Down)).unwrap();
    assert_eq!(
        runtime.view().display_offset,
        up,
        "{program}: and back down"
    );

    // The command's start mark (133;C) arrived, so the program gets the key.
    runtime
        .write(b"sleep 2; echo slept-$((40 + 2))\r".to_vec())
        .unwrap();
    thread::sleep(Duration::from_millis(300));
    runtime.execute(key(TerminalKey::Up)).unwrap();
    assert_eq!(
        runtime.view().display_offset,
        0,
        "{program}: a running command keeps Cmd+Up"
    );
    wait_for(&runtime, "slept-42");
    let _ = std::fs::remove_dir_all(home);
}

fn wait_for_prompt_after(runtime: &TerminalRuntime, marker: &str, prompt: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let text = runtime.visible_text();
        if text
            .split_once(marker)
            .is_some_and(|(_, after)| after.contains(prompt))
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("no prompt after {marker:?}: {}", runtime.visible_text());
}

#[test]
fn zsh_prompts_can_be_jumped_between() {
    jumps_between_prompts("zsh", "user>", true);
}

#[test]
fn bash_prompts_can_be_jumped_between_from_4_4() {
    jumps_between_prompts("bash", "user$", bash_has_marks());
}

fn wait_for_cursor(runtime: &TerminalRuntime, shape: TerminalCursorShape, why: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if runtime
            .view()
            .cursor
            .is_some_and(|cursor| cursor.shape == shape && cursor.blinking)
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("{why}: the cursor stayed {:?}", runtime.view().cursor);
}

/// A blinking bar at the prompt whatever the last program left, a blinking block in vi
/// command mode around the person's own widget, and the default for a command.
#[test]
fn zsh_sets_the_cursor_for_its_prompt_vi_mode_and_commands() {
    let home = home("zsh-cursor");
    std::fs::write(
        home.join(".zshrc"),
        "PROMPT='user> '\nbindkey -v\nKEYTIMEOUT=1\n\
         typeset -gi KEYMAP_CHANGES=0\n\
         zle-keymap-select() { (( ++KEYMAP_CHANGES )) }\nzle -N zle-keymap-select\n",
    )
    .unwrap();
    let Some(runtime) = integrated_shell("zsh", &home, None) else {
        return;
    };
    wait_for(&runtime, "user>");
    wait_for_cursor(&runtime, TerminalCursorShape::Beam, "first prompt");

    runtime
        .write(b"printf '\\e[2 q'; echo shaped\r".to_vec())
        .unwrap();
    wait_for_prompt_after(&runtime, "shaped", "user>");
    wait_for_cursor(
        &runtime,
        TerminalCursorShape::Beam,
        "the prompt after a block",
    );

    runtime
        .write(b"sleep 3; echo slept-$((40 + 2))".to_vec())
        .unwrap();
    runtime.write(b"\x1b".to_vec()).unwrap();
    wait_for_cursor(&runtime, TerminalCursorShape::Block, "vi command mode");
    runtime.write(b"\r".to_vec()).unwrap();
    wait_for_cursor(&runtime, TerminalCursorShape::Beam, "the command's default");
    assert!(
        !runtime.visible_text().contains("slept-42"),
        "reset before the command ended"
    );
    wait_for_prompt_after(&runtime, "slept-42", "user>");

    runtime
        .write(b"echo changes=$(( KEYMAP_CHANGES > 0 ))\r".to_vec())
        .unwrap();
    wait_for(&runtime, "changes=1");
    let _ = std::fs::remove_dir_all(home);
}

/// The person's own vi mode hooks keep running beside Condr's, in Ghostty's order: a
/// widget they defined runs after Condr's, so its cursor wins; add-zle-hook-widget runs
/// Condr's after theirs, so Condr's does.
#[test]
fn zsh_runs_the_persons_vi_mode_hooks_in_ghosttys_order() {
    // Succeeds either way: add-zle-hook-widget stops at a hook that fails.
    let set_underline = "underline() { [[ $KEYMAP != vicmd ]] || printf '\\e[3 q' }\n";
    for (name, hook, vicmd) in [
        (
            "widget",
            "zle-keymap-select() { underline }\nzle -N zle-keymap-select\n",
            TerminalCursorShape::Underline,
        ),
        (
            "azhw",
            "autoload -Uz add-zle-hook-widget\nadd-zle-hook-widget keymap-select underline\n",
            TerminalCursorShape::Block,
        ),
    ] {
        let home = home(&format!("zsh-cursor-{name}"));
        std::fs::write(
            home.join(".zshrc"),
            format!("PROMPT='user> '\nbindkey -v\nKEYTIMEOUT=1\n{set_underline}{hook}"),
        )
        .unwrap();
        let Some(runtime) = integrated_shell("zsh", &home, None) else {
            return;
        };
        wait_for(&runtime, "user>");
        wait_for_cursor(&runtime, TerminalCursorShape::Beam, name);
        runtime.write(b"\x1b".to_vec()).unwrap();
        wait_for_cursor(&runtime, vicmd, name);
        runtime.write(b"i".to_vec()).unwrap();
        wait_for_cursor(&runtime, TerminalCursorShape::Beam, name);
        let _ = std::fs::remove_dir_all(home);
    }
}

/// The prompt's bar is part of PS1, so it comes after whatever PROMPT_COMMAND printed.
#[test]
fn bash_sets_the_cursor_at_its_prompt_from_4_4() {
    if !bash_has_marks() {
        return;
    }
    let home = home("bash-cursor");
    std::fs::write(home.join(".bashrc"), "PS1='user$ '\n").unwrap();
    let Some(runtime) = integrated_shell("bash", &home, None) else {
        return;
    };
    wait_for(&runtime, "user$");
    runtime
        .write(b"printf '\\e[2 q'; echo shaped\r".to_vec())
        .unwrap();
    wait_for_prompt_after(&runtime, "shaped", "user$");
    wait_for_cursor(
        &runtime,
        TerminalCursorShape::Beam,
        "the prompt after a block",
    );

    runtime
        .write(
            b"PROMPT_COMMAND+=$'\\n'\"printf '\\\\e[1 q'; echo hooked-\\$((40 + 2))\"\r".to_vec(),
        )
        .unwrap();
    wait_for_prompt_after(&runtime, "hooked-42", "user$");
    wait_for_cursor(&runtime, TerminalCursorShape::Beam, "after PROMPT_COMMAND");
    let _ = std::fs::remove_dir_all(home);
}

/// The directory the shell reported (OSC 7), as the PTY reader recorded it.
fn wait_for_reported_cwd(runtime: &TerminalRuntime, expected: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let reported = || runtime.cwd_probe().reported_cwd.lock().unwrap().cwd.clone();
    while Instant::now() < deadline {
        if reported().as_deref() == Some(expected) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("reported {:?}, not {expected:?}", reported());
}

fn reports_its_directory(program: &str, prompt: &str) {
    let home = home(&format!("{program}-pwd"));
    std::fs::write(home.join(".zshrc"), "PROMPT='user> '\n").unwrap();
    std::fs::write(home.join(".bashrc"), "PS1='user$ '\n").unwrap();
    let Some(runtime) = integrated_shell(program, &home, None) else {
        return;
    };
    wait_for(&runtime, prompt);
    // The shell's own $PWD, which may resolve a symlinked temporary directory.
    let home_dir = std::fs::canonicalize(&home).unwrap();
    wait_for_reported_cwd(&runtime, &home_dir);
    std::fs::create_dir_all(home.join("with space")).unwrap();
    runtime.write(b"cd 'with space'\r".to_vec()).unwrap();
    wait_for_reported_cwd(&runtime, &home_dir.join("with space"));
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn zsh_reports_its_directory() {
    reports_its_directory("zsh", "user>");
}

#[test]
fn bash_reports_its_directory_from_4_4() {
    if bash_has_marks() {
        reports_its_directory("bash", "user$");
    }
}

/// The marks live in PS1 and PS2 only while the prompt shows: a command sees the prompt
/// as the person set it.
#[test]
fn zsh_commands_see_the_prompt_as_the_person_set_it() {
    let home = home("zsh-unmarked");
    std::fs::write(home.join(".zshrc"), "PROMPT=$'first\\nuser> '\n").unwrap();
    let Some(runtime) = integrated_shell("zsh", &home, None) else {
        return;
    };
    wait_for(&runtime, "user>");
    runtime
        .write(b"[[ $PS1 == $'first\\nuser> ' ]] && echo clean-$((40 + 2))\r".to_vec())
        .unwrap();
    wait_for(&runtime, "clean-42");
    let _ = std::fs::remove_dir_all(home);
}

/// Polls until the shell is ready to be typed into, or panics after ten seconds.
fn ready_shell(runtime: &TerminalRuntime) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match runtime.launch_probe().readiness() {
            ShellReadiness::Ready(shell) => return shell,
            _ if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            other => panic!("never ready ({other:?}): {}", runtime.visible_text()),
        }
    }
}

#[test]
fn an_announced_zsh_is_ready_only_at_its_prompt() {
    let home = home("zsh-ready");
    // Builtins only: the process probe took this for a shell at its prompt.
    std::fs::write(
        home.join(".zshrc"),
        "zmodload zsh/zselect; zselect -t 150; PROMPT='user> '\n",
    )
    .unwrap();
    let Some(runtime) = integrated_shell("zsh", &home, None) else {
        return;
    };
    assert!(runtime.announces);
    assert_eq!(ready_shell(&runtime), "zsh");
    assert!(
        runtime.visible_text().contains("user>"),
        "ready before the prompt: {}",
        runtime.visible_text()
    );
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn an_announced_zsh_stuck_in_a_startup_read_is_never_ready() {
    let home = home("zsh-read");
    std::fs::write(home.join(".zshrc"), "read -k 1 answer; PROMPT='user> '\n").unwrap();
    let Some(runtime) = integrated_shell("zsh", &home, None) else {
        return;
    };
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        assert_eq!(
            runtime.launch_probe().readiness(),
            ShellReadiness::Starting,
            "the probe would have typed into the read"
        );
        thread::sleep(Duration::from_millis(50));
    }
    // Whatever the person types reaches the read, and the prompt follows.
    assert!(!runtime.typed());
    runtime.execute(TerminalCommand::Text("y".into())).unwrap();
    assert!(runtime.typed());
    assert_eq!(ready_shell(&runtime), "zsh");
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn bash_is_ready_by_its_marks_from_4_4_and_by_the_probe_before() {
    let home = home("bash-ready");
    std::fs::write(home.join(".bashrc"), "PS1='user> '\n").unwrap();
    let Some(runtime) = integrated_shell("bash", &home, None) else {
        return;
    };
    assert_eq!(ready_shell(&runtime), "bash");
    wait_for(&runtime, "user>");
    let announced = runtime.notices.lock().unwrap().shell_announced;
    assert_eq!(announced, bash_has_marks());
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn a_shell_without_integration_is_left_to_the_probe() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.env("PS1", "plain> ");
    let runtime = TerminalRuntime::spawn(command, TerminalSize::new(24, 120)).unwrap();
    assert!(!runtime.announces);
    // macOS's /bin/sh is bash.
    let shell = ready_shell(&runtime);
    assert!(["sh", "bash", "dash"].contains(&shell.as_str()), "{shell}");
}
