//! Shell integration (ADR 0055): small scripts that make zsh, bash and fish mark each
//! prompt and command for the terminal with OSC 133, injected the way Ghostty and VS Code
//! inject theirs. The Server writes them under its data directory, and a Pane's shell
//! loads them through the variable that shell finds its startup files by.

use super::shell::shell_name;
use super::*;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Writes the scripts under `root`, in a directory named for their content, and returns
/// it. The directory is written whole elsewhere and renamed into place, so a shell never
/// reads half a script, and Servers of different builds each keep their own.
pub fn install_shell_integration(root: &Path) -> io::Result<PathBuf> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    SCRIPTS.hash(&mut hasher);
    let directory = root.join(format!("shell-integration-{:016x}", hasher.finish()));
    if installed(&directory) {
        return Ok(directory);
    }
    let staging = root.join(format!(
        ".shell-integration-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let written = SCRIPTS.iter().try_for_each(|(path, script)| {
        let path = staging.join(path);
        std::fs::create_dir_all(path.parent().unwrap_or(&staging))?;
        std::fs::write(path, script)
    });
    let placed = written.and_then(|()| {
        // A directory missing a script (someone deleted one) is replaced.
        if directory.exists() && !installed(&directory) {
            std::fs::remove_dir_all(&directory)?;
        }
        std::fs::rename(&staging, &directory)
    });
    let _ = std::fs::remove_dir_all(&staging);
    match placed {
        Ok(()) => Ok(directory),
        // Another Server placed the same content first.
        Err(_) if installed(&directory) => Ok(directory),
        Err(error) => Err(error),
    }
}

fn installed(directory: &Path) -> bool {
    SCRIPTS
        .iter()
        .all(|(path, _)| directory.join(path).is_file())
}

/// Points `program`'s startup at the scripts in `directory`. A shell without a script
/// starts as it would have.
pub(super) fn apply(command: &mut CommandBuilder, program: &str, directory: &Path) {
    // zsh pointed at a directory without Condr's .zshenv would read none of the person's
    // startup files, so a script removed since the Server started means no integration.
    if !installed(directory) {
        return;
    }
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
            let current = command
                .get_env("XDG_DATA_DIRS")
                .filter(|dirs| !dirs.is_empty())
                .map(ToOwned::to_owned);
            if current.is_none() {
                command.env("CONDR_FISH_DATA_DIRS_UNSET", "1");
            }
            let rest = current.unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            let mut dirs = data.clone().into_os_string();
            dirs.push(":");
            dirs.push(rest);
            command.env("XDG_DATA_DIRS", dirs);
            command.env("CONDR_FISH_DATA_DIR", data);
        }
        _ => {}
    }
}
