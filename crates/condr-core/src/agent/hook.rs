//! The `condr agent-hook` side of ADR 0014: turn one hook invocation into an OSC 777
//! event on the controlling terminal, where the Server's PTY reader picks it up.
//!
//! Everything here fails quietly. A hook that blocks or errors would stall the agent it
//! observes, so the worst outcome is a missed status update.

use std::io::{IsTerminal as _, Read as _, Write as _};
use std::time::Duration;

use super::{AgentEvent, AgentEventKind, AgentKind};

const MAX_STDIN: u64 = 64 * 1024;
/// A hook that cannot deliver in this long has nothing left to gain by trying.
const DEADLINE: Duration = Duration::from_secs(2);

/// Runs the hook. `agent` and `event` are the slugs the installer wrote into the agent's
/// configuration; stdin carries the agent's JSON, of which only `source` is kept. Returns
/// whether the event reached a terminal; callers exit 0 regardless.
pub fn run(agent: &str, event: &str) -> bool {
    if std::env::var_os(crate::PaneEnvironment::ENV).is_none_or(|value| value != "1") {
        return false;
    }
    let (Some(agent), Some(event)) = (AgentKind::parse_label(agent), AgentEventKind::parse(event))
    else {
        return false;
    };
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let source = (event == AgentEventKind::SessionStart)
            .then(read_stdin)
            .flatten()
            .and_then(|input| source_of(&input));
        let delivered =
            write_to_controlling_terminal(&AgentEvent::new(agent, event, source).encode());
        let _ = done.send(delivered);
    });
    wait.recv_timeout(DEADLINE).unwrap_or(false)
}

fn read_stdin() -> Option<String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut input = String::new();
    stdin.take(MAX_STDIN).read_to_string(&mut input).ok()?;
    Some(input)
}

fn source_of(input: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(input).ok()?;
    value.get("source")?.as_str().map(str::to_owned)
}

#[cfg(unix)]
fn write_to_controlling_terminal(bytes: &[u8]) -> bool {
    if write_device("/dev/tty", bytes) {
        return true;
    }
    // Without a controlling terminal of our own, the nearest ancestor that has one is
    // the agent, or the shell it runs in.
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..8 {
        if pid <= 1 {
            break;
        }
        let Ok(output) = std::process::Command::new("ps")
            .args(["-o", "tty=", "-o", "ppid=", "-p", &pid.to_string()])
            .output()
        else {
            break;
        };
        let line = String::from_utf8_lossy(&output.stdout);
        let mut fields = line.split_whitespace();
        let tty = fields.next().unwrap_or("");
        let parent = fields
            .next()
            .and_then(|ppid| ppid.parse().ok())
            .unwrap_or(1);
        if !tty.is_empty() && tty != "?" && tty != "??" {
            return write_device(&format!("/dev/{tty}"), bytes);
        }
        pid = parent;
    }
    false
}

#[cfg(unix)]
fn write_device(path: &str, bytes: &[u8]) -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|mut tty| tty.write_all(bytes).and_then(|()| tty.flush()))
        .is_ok()
}

#[cfg(windows)]
fn write_to_controlling_terminal(bytes: &[u8]) -> bool {
    use windows_sys::Win32::System::Console::{AttachConsole, FreeConsole};

    // The console we inherited is the ConPTY the agent runs in, when there is one.
    if write_console_out(bytes) {
        return true;
    }
    // Otherwise borrow an ancestor's: the agent, or the shell it runs in.
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        sysinfo::ProcessRefreshKind::new(),
    );
    let mut pid = sysinfo::Pid::from_u32(std::process::id());
    for _ in 0..16 {
        let Some(parent) = system.process(pid).and_then(sysinfo::Process::parent) else {
            break;
        };
        // SAFETY: plain Win32 console calls with no pointers involved.
        let attached = unsafe {
            FreeConsole();
            AttachConsole(parent.as_u32()) != 0
        };
        if attached && write_console_out(bytes) {
            return true;
        }
        pid = parent;
    }
    false
}

#[cfg(windows)]
fn write_console_out(bytes: &[u8]) -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
        .and_then(|mut out| out.write_all(bytes).and_then(|()| out.flush()))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_session_source_is_kept_from_the_hook_input() {
        assert_eq!(
            source_of(r#"{"session_id":"abc","source":"compact","cwd":"/x"}"#),
            Some("compact".to_owned())
        );
        assert_eq!(source_of(r#"{"session_id":"abc"}"#), None);
        assert_eq!(source_of("not json"), None);
    }
}
