//! Shell integration (ADR 0055): small scripts that make zsh, bash and fish mark each
//! prompt and command for the terminal with OSC 133, injected the way Ghostty and VS Code
//! inject theirs. The Server writes them under its data directory, and a Pane's shell
//! loads them through the variable that shell finds its startup files by.

use super::shell::shell_name;
use super::*;

const SCRIPTS: &[(&str, &str)] = &[
    ("zsh/.zshenv", include_str!("shell_integration/zsh/.zshenv")),
    (
        "bash/condr.bash",
        include_str!("shell_integration/bash/condr.bash"),
    ),
    (
        "fish/fish/vendor_conf.d/condr-shell-integration.fish",
        include_str!("shell_integration/fish/fish/vendor_conf.d/condr-shell-integration.fish"),
    ),
];

/// Writes the scripts into `directory`, replacing what another Condr build left there.
pub fn install_shell_integration(directory: &Path) -> io::Result<()> {
    for (path, script) in SCRIPTS {
        let path = directory.join(path);
        if std::fs::read(&path).is_ok_and(|current| current == script.as_bytes()) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, script)?;
    }
    Ok(())
}

/// Points `program`'s startup at the scripts in `directory`. A shell without a script
/// starts as it would have.
pub(super) fn apply(command: &mut CommandBuilder, program: &str, directory: &Path) {
    match shell_name(program).as_str() {
        // zsh reads `$ZDOTDIR/.zshenv` first; Condr's puts the user's ZDOTDIR back.
        "zsh" => {
            if let Some(zdotdir) = command.get_env("ZDOTDIR").map(ToOwned::to_owned) {
                command.env("CONDR_USER_ZDOTDIR", zdotdir);
            }
            command.env("ZDOTDIR", directory.join("zsh"));
        }
        // The rc file `shell_command` wraps bash in sources it after `~/.bashrc`.
        "bash" => command.env("CONDR_BASH_INTEGRATION", directory.join("bash/condr.bash")),
        // fish loads `vendor_conf.d` from each data directory; Condr's takes itself off.
        "fish" => {
            let data = directory.join("fish");
            let rest = command
                .get_env("XDG_DATA_DIRS")
                .filter(|dirs| !dirs.is_empty())
                .map_or_else(|| "/usr/local/share:/usr/share".into(), ToOwned::to_owned);
            let mut dirs = data.clone().into_os_string();
            dirs.push(":");
            dirs.push(rest);
            command.env("XDG_DATA_DIRS", dirs);
            command.env("CONDR_FISH_DATA_DIR", data);
        }
        _ => {}
    }
}
