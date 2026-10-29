//! The injected shell integration (ADR 0055), run in real shells with a home of their own.

use super::*;

/// A shell with Condr's integration in `home`, started the way `spawn_shell` starts one.
fn integrated_shell(program: &str, home: &Path) -> Option<TerminalRuntime> {
    let program = which(program)?;
    let integration = home.join("integration");
    install_shell_integration(&integration).unwrap();
    let mut command = shell_command(&program);
    command.env("HOME", home);
    command.env_remove("ZDOTDIR");
    command.env_remove("XDG_DATA_DIRS");
    command.cwd(home);
    crate::terminal::shell_integration::apply(&mut command, &program, &integration);
    Some(TerminalRuntime::spawn(command, TerminalSize::new(24, 120)).unwrap())
}

fn which(program: &str) -> Option<String> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(program))
        .find(|path| path.is_file())
        .map(|path| path.display().to_string())
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
fn zsh_reads_the_users_startup_files_and_gets_the_marks() {
    let home = home("zsh");
    std::fs::write(home.join(".zshenv"), "export FROM_ZSHENV=env\n").unwrap();
    std::fs::write(home.join(".zshrc"), "PROMPT='user> '\n").unwrap();
    let Some(runtime) = integrated_shell("zsh", &home) else {
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
fn bash_reads_the_users_bashrc_and_gets_the_marks() {
    let home = home("bash");
    std::fs::write(home.join(".bashrc"), "PS1='user$ '\nFROM_BASHRC=rc\n").unwrap();
    let Some(runtime) = integrated_shell("bash", &home) else {
        return;
    };
    wait_for(&runtime, "user$");
    runtime
        .write(
            b"echo \"r=$FROM_BASHRC i=${CONDR_BASH_INTEGRATION-unset} m=$(declare -F __condr_prompt_mark)\"\r"
                .to_vec(),
        )
        .unwrap();
    wait_for(&runtime, "r=rc i=unset m=__condr_prompt_mark");
    let _ = std::fs::remove_dir_all(home);
}

/// Cmd+Up and Cmd+Down at the shell's prompt move the viewport by prompts.
fn jumps_between_prompts(program: &str, prompt: &str) {
    let home = home(&format!("{program}-jump"));
    std::fs::write(home.join(".zshrc"), "PROMPT='user> '\n").unwrap();
    std::fs::write(home.join(".bashrc"), "PS1='user$ '\n").unwrap();
    let Some(runtime) = integrated_shell(program, &home) else {
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
    jumps_between_prompts("zsh", "user>");
}

#[test]
fn bash_prompts_can_be_jumped_between() {
    jumps_between_prompts("bash", "user$");
}
