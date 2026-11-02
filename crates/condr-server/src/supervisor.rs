//! The Supervisor (ADR 0058): every local Server is started through `condr server
//! supervise`, which runs `condr server run` as its child and starts it again when it
//! crashes. A Server that stops on purpose, or returns an error, ends the Supervisor too.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const RESTART_DELAY: Duration = Duration::from_secs(1);
/// Syncthing's limits: a Server that crashes this often is crashing at startup, and
/// every restart restores its Panes and resumes their Agents again.
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const MAX_RESTARTS: usize = 4;

/// Runs the Server with `run_args` until it ends without crashing. Held for as long as
/// this runs, the Supervisor lock tells [`crate::wait_for_shutdown`] that `condr` is
/// still executing here, which on Windows keeps the file from being replaced.
pub fn supervise(socket_path: &Path, run_args: &[OsString]) -> io::Result<()> {
    let _lock = hold_lock(socket_path)?;
    // Taken once, before an update can replace the file: afterwards Linux reports this
    // process's image as `condr (deleted)`, while the path starts the new build.
    let executable = std::env::current_exe()?;
    let mut restarts = VecDeque::new();
    let mut previous_crash = None;
    loop {
        let mut command = Command::new(&executable);
        command
            .args(["server", "run"])
            .args(run_args)
            .stdin(Stdio::null());
        if let Some(crash) = &previous_crash {
            command.arg("--restarted-after").arg(crash);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            command.creation_flags(DETACHED_PROCESS);
        }
        let status = command.spawn()?.wait()?;
        let Some(crash) = crash(status) else {
            return Ok(());
        };
        if !may_restart(&mut restarts, Instant::now()) {
            eprintln!(
                "supervisor: the Server {crash}; it crashed {MAX_RESTARTS} times within {}s, so it is not started again",
                RESTART_WINDOW.as_secs()
            );
            return Ok(());
        }
        eprintln!("supervisor: the Server {crash}; starting it again");
        thread::sleep(RESTART_DELAY);
        previous_crash = Some(crash);
    }
}

/// How the Server crashed, or `None` when it ended on purpose: exit code 0, an error it
/// returned (exit code 1, such as finding another Server on its endpoint), or on Unix a
/// signal someone sends to stop a process.
fn crash(status: ExitStatus) -> Option<String> {
    #[cfg(unix)]
    {
        use nix::sys::signal::Signal;
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(number) = status.signal() {
            let signal = Signal::try_from(number).ok();
            return match signal {
                Some(Signal::SIGTERM | Signal::SIGKILL | Signal::SIGINT | Signal::SIGHUP) => None,
                Some(signal) => Some(format!("was ended by {signal}")),
                None => Some(format!("was ended by signal {number}")),
            };
        }
    }
    match status.code() {
        // A panic on the main thread, which unwinds out of `main`.
        Some(101) => Some("panicked".into()),
        // An NTSTATUS error: `process::abort` (0xC0000409), an access violation, a
        // stack overflow. Windows has no signal that says someone ended the process, and
        // `Stop-Process` ends one with 0xFFFFFFFF, outside this range.
        #[cfg(windows)]
        Some(code) if (0xC000_0000..0xD000_0000).contains(&(code as u32)) => {
            Some(format!("crashed with exception {:#010X}", code as u32))
        }
        _ => None,
    }
}

/// Records a restart at `now` unless [`MAX_RESTARTS`] already happened within
/// [`RESTART_WINDOW`].
fn may_restart(restarts: &mut VecDeque<Instant>, now: Instant) -> bool {
    while restarts
        .front()
        .is_some_and(|&at| now.duration_since(at) >= RESTART_WINDOW)
    {
        restarts.pop_front();
    }
    if restarts.len() >= MAX_RESTARTS {
        return false;
    }
    restarts.push_back(now);
    true
}

fn lock_path(socket_path: &Path) -> PathBuf {
    let mut path = socket_path.as_os_str().to_os_string();
    path.push(".supervisor.lock");
    PathBuf::from(path)
}

/// Shared: a GUI that starts its own Server while a Supervisor waits to restart one
/// runs a second Supervisor until the first one's Server loses the bind.
fn hold_lock(socket_path: &Path) -> io::Result<File> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let lock = options.open(lock_path(socket_path))?;
    lock.lock_shared()?;
    Ok(lock)
}

/// Whether a Supervisor for this endpoint is still running.
pub(crate) fn is_running(socket_path: &Path) -> io::Result<bool> {
    let lock = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path(socket_path))
    {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match lock.try_lock() {
        Ok(()) => Ok(false),
        Err(fs::TryLockError::WouldBlock) => Ok(true),
        Err(fs::TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_that_ends_on_purpose_is_not_restarted() {
        assert_eq!(crash(exit_code(0)), None);
        assert_eq!(crash(exit_code(1)), None);
        assert_eq!(crash(exit_code(101)).as_deref(), Some("panicked"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_signals_that_stop_a_process_are_not_crashes() {
        use nix::sys::signal::Signal;
        use std::os::unix::process::ExitStatusExt as _;
        // A wait status whose low bits are a signal number is a process that signal ended.
        let ended_by = |signal: Signal| crash(ExitStatus::from_raw(signal as i32));
        for signal in [
            Signal::SIGTERM,
            Signal::SIGKILL,
            Signal::SIGINT,
            Signal::SIGHUP,
        ] {
            assert_eq!(ended_by(signal), None, "{signal}");
        }
        assert_eq!(
            ended_by(Signal::SIGABRT).as_deref(),
            Some("was ended by SIGABRT")
        );
        assert_eq!(
            ended_by(Signal::SIGSEGV).as_deref(),
            Some("was ended by SIGSEGV")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_exceptions_are_crashes_and_task_manager_is_not() {
        use std::os::windows::process::ExitStatusExt as _;
        assert_eq!(
            crash(ExitStatus::from_raw(0xC000_0409)).as_deref(),
            Some("crashed with exception 0xC0000409")
        );
        assert_eq!(crash(ExitStatus::from_raw(1)), None);
        assert_eq!(
            crash(ExitStatus::from_raw(0xFFFF_FFFF)),
            None,
            "Stop-Process"
        );
    }

    #[test]
    fn four_restarts_a_minute_and_no_fifth() {
        let start = Instant::now();
        let mut restarts = VecDeque::new();
        for second in 0..4 {
            assert!(may_restart(
                &mut restarts,
                start + Duration::from_secs(second)
            ));
        }
        assert!(!may_restart(&mut restarts, start + Duration::from_secs(10)));
        // A minute after the first restart, one slot is free again.
        assert!(may_restart(&mut restarts, start + Duration::from_secs(60)));
        assert!(!may_restart(&mut restarts, start + Duration::from_secs(60)));
    }

    #[test]
    fn the_lock_is_shared_between_supervisors_and_seen_until_the_last_ends() {
        let directory = std::env::temp_dir().join(format!(
            "condr-supervisor-lock-{}",
            crate::test_support::unique_suffix()
        ));
        let socket_path = directory.join("condr.sock");
        assert!(!is_running(&socket_path).unwrap());
        let first = hold_lock(&socket_path).unwrap();
        let second = hold_lock(&socket_path).unwrap();
        assert!(is_running(&socket_path).unwrap());
        drop(first);
        assert!(is_running(&socket_path).unwrap());
        drop(second);
        assert!(!is_running(&socket_path).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    fn exit_code(code: i32) -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt as _;
            ExitStatus::from_raw(code << 8)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt as _;
            ExitStatus::from_raw(code as u32)
        }
    }
}
