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
    apply(&mut command, &program, &integration);
    Some(TerminalRuntime::spawn(command, TerminalSize::new(24, 120)).unwrap())
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
