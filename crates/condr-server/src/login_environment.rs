//! The environment the user's login shell builds (ADR 0057). A Server started by launchd,
//! systemd or a GUI opened from the Dock or a desktop menu inherits a bare environment,
//! without what the user's shell profile adds, PATH above all, and every Pane and agent
//! lookup there would miss the user's tools. So `spawn_server` hands every Server it
//! starts the login shell's environment, whoever asked, and the GUI adopts it for itself,
//! as Zed and VS Code do. Unix only: on Windows a Pane's environment comes from the
//! registry.

use std::io::Read as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

/// Separates the environment from whatever the shell's profile prints first.
const MARKER: &[u8] = b"__CONDR_LOGIN_ENVIRONMENT__";
/// The probe shell's own bookkeeping, not the user's environment.
const SKIPPED: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];
/// A profile that waits on something gets this long before Condr goes on without it.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The login shell's environment, read once per process.
pub fn captured() -> Result<&'static [(String, String)], String> {
    static CAPTURED: OnceLock<Result<Vec<(String, String)>, String>> = OnceLock::new();
    CAPTURED.get_or_init(read).as_deref().map_err(Clone::clone)
}

fn read() -> Result<Vec<(String, String)>, String> {
    let shell = condr_core::default_shell_program();
    let marker = std::str::from_utf8(MARKER).expect("ASCII marker");
    let mut child = without_terminal(
        Command::new(&shell)
            .args([
                "-l",
                "-i",
                "-c",
                &format!("printf {marker}; /usr/bin/env -0"),
            ])
            .current_dir(std::env::home_dir().unwrap_or_else(|| "/".into()))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .spawn()
    .map_err(|error| format!("{shell}: {error}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let (sender, receiver) = std::sync::mpsc::channel();
    // A thread, not polling: an environment larger than the pipe buffer would block the
    // shell until someone reads it.
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = sender.send(stdout.read_to_end(&mut output).map(|_| output));
    });
    let output = receiver.recv_timeout(TIMEOUT);
    let _ = child.kill();
    let _ = child.wait();
    match output {
        Ok(Ok(output)) => parse(&output).ok_or_else(|| format!("{shell} printed no environment")),
        Ok(Err(error)) => Err(format!("{shell}: {error}")),
        Err(_) => Err(format!("{shell} took longer than {TIMEOUT:?}")),
    }
}

/// Runs the probe in a session of its own, with no terminal to take over: started from one,
/// `zsh -i` makes itself the terminal's foreground group, and a prompt theme draws there.
#[allow(unsafe_code)] // `pre_exec` has no safe form.
fn without_terminal(command: &mut Command) -> &mut Command {
    // SAFETY: between fork and exec the closure only calls `setsid`, which is
    // async-signal-safe and changes nothing but the child.
    unsafe { command.pre_exec(|| nix::unistd::setsid().map(drop).map_err(Into::into)) }
}

/// The `KEY=value` entries `env -0` printed after the marker.
fn parse(output: &[u8]) -> Option<Vec<(String, String)>> {
    let start = output
        .windows(MARKER.len())
        .position(|window| window == MARKER)?
        + MARKER.len();
    Some(
        output[start..]
            .split(|byte| *byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok()?.split_once('='))
            .filter(|(key, _)| !key.is_empty() && !SKIPPED.contains(key))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_what_env_printed_after_the_marker() {
        let mut output = b"profile noise PATH=/wrong\n".to_vec();
        output.extend_from_slice(MARKER);
        output.extend_from_slice(b"PATH=/a:/b\0PWD=/home\0NOTE=x=y\nz\0\0");
        assert_eq!(
            parse(&output),
            Some(vec![
                ("PATH".into(), "/a:/b".into()),
                ("NOTE".into(), "x=y\nz".into()),
            ])
        );
        assert_eq!(parse(b"PATH=/a\0"), None);
    }
}
