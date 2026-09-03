//! Starting the GUI from a console-subsystem binary, the way herdr starts its server
//! daemon: `condr` is a console program so `condr <command>` prints, waits and exits
//! like any CLI, while a bare `condr` hands the GUI to a detached copy of itself and
//! returns. The detached copy owns no console, so `GetConsoleWindow()` is null there,
//! which is the same test herdr's `current_process_is_detached_server_daemon` uses.

/// Runs the GUI. On Windows, a process that still holds a console (started from a
/// shell, or from Explorer, which allocates one) relaunches itself detached first so
/// the shell prompt returns and the Explorer console closes; the detached child runs
/// the GUI in place. `--foreground` skips the relaunch for development.
pub(crate) fn run_gui(foreground: bool) {
    #[cfg(windows)]
    if !foreground && started_from_a_terminal() {
        match relaunch_detached() {
            Ok(()) => return,
            Err(error) => eprintln!("condr: running in this console; failed to detach: {error}"),
        }
    }
    let _ = foreground;
    crate::app::run();
}

#[cfg(windows)]
fn relaunch_detached() -> std::io::Result<()> {
    use windows_spawn::{Command, CreationFlags, SpawnOptions, Stdio};

    // The flag, not a console check, tells the child it is the detached copy: its
    // stdio is NUL, which is a valid handle and would otherwise read as "redirected".
    Command::new(std::env::current_exe()?)
        .arg("--detached")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // The explicit handle allowlist keeps a caller's pipes out of the GUI process,
        // so `condr | Out-String` returns as soon as this process exits.
        .spawn_with(SpawnOptions::new().creation_flags(CreationFlags::DETACHED_PROCESS))
        .map(drop)
}

/// A console window (PowerShell, cmd, Windows Terminal, or the one Explorer allocates)
/// or a redirected stdout (mintty / Git Bash hand out pipes, not a console) both mean
/// a caller is waiting on us. The detached child we spawn has neither.
#[cfg(windows)]
fn started_from_a_terminal() -> bool {
    use std::os::windows::io::AsRawHandle as _;

    let stdout = std::io::stdout().as_raw_handle();
    has_console() || (!stdout.is_null() && stdout as isize != -1)
}

/// Whether this process is attached to a console window.
#[cfg(windows)]
#[allow(unsafe_code)]
fn has_console() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleWindow() -> *mut core::ffi::c_void;
    }
    // SAFETY: a documented Win32 call without arguments; a null return means no console.
    !unsafe { GetConsoleWindow() }.is_null()
}
