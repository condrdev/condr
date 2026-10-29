//! What every runtime submodule of `terminal` shares through `use super::*`: its imports,
//! limits and helpers. Only the `runtime` feature builds it (ADR 0039).

pub(super) use super::cursor_settle::{CURSOR_POSITION_SETTLE_ENABLED, CursorSettle};
pub(super) use super::encode::{encode_key, encode_legacy_key, encode_paste};
pub(super) use super::input_queue::*;
#[cfg(windows)]
pub(super) use super::job::Job;
pub(super) use super::mouse::{MAX_MOUSE_WHEEL_STEPS, encode_mouse};
pub(super) use super::notices::*;
pub(super) use super::osc::*;
pub(super) use super::process::*;
pub(super) use super::pty_io::*;
#[cfg(unix)]
pub(super) use super::pty_unix::*;
pub(super) use super::resize::*;
pub(super) use super::shell::shell_command;
#[cfg(test)]
pub(super) use super::view::blank_cell;
pub(super) use super::view::publish_view;
pub(super) use super::view_source::TerminalDamageBaseline;
pub(super) use super::vt::{TermMode, Vt};
pub(super) use crate::agent::{
    AGENT_EVENT_OSC_PREFIX, AGENT_POLL_INTERVAL, AgentDetector, AgentEvent, AgentPublish,
    ProcessInfo, ProcessProbeResult, ProgramStatus, identify_agent_process,
};
pub(super) use crate::{AgentKind, AgentSnapshot, PaneId};
#[cfg(unix)]
pub(super) use filedescriptor::FileDescriptor;
#[cfg(unix)]
pub(super) use nix::fcntl::{FcntlArg, OFlag, fcntl};
#[cfg(unix)]
pub(super) use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
#[cfg(unix)]
pub(super) use nix::unistd::{Pid as UnixPid, getpgid, getsid};
pub(super) use portable_pty::{Child, ExitStatus, MasterPty, PtySize, native_pty_system};
pub(super) use std::collections::VecDeque;
pub(super) use std::io::{self, Read, Write};
pub(super) use std::path::{Path, PathBuf};
pub(super) use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
pub(super) use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak, mpsc};
pub(super) use std::thread::{self, JoinHandle};
pub(super) use std::time::{Duration, Instant};
#[cfg(unix)]
pub(super) use std::{
    net::Shutdown,
    os::fd::{AsFd, AsRawFd, RawFd},
    os::unix::net::UnixStream,
};
#[cfg(unix)]
pub(super) use sysinfo::Signal;
pub(super) use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

// Process identity is refreshed separately from the state reported by agent hooks.
pub(super) const PROCESS_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
pub(super) const MAX_TERMINAL_TITLE_CHARS: usize = 256;
pub(super) const MAX_PENDING_CLIPBOARD_BYTES: usize = 1024 * 1024;
pub(super) const INPUT_QUEUE_CAPACITY: usize = 64;
pub(super) const TERMINAL_REPLY_QUEUE_RESERVE: usize = 1;
pub(super) const TERMINAL_CONTROL_QUEUE_RESERVE: usize = INPUT_QUEUE_CAPACITY + 1;
pub(super) const MAX_PENDING_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub(super) const IO_CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// How long a process tree gets to leave after a signal it may handle (hangup, term).
#[cfg(unix)]
pub(super) const PROCESS_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
/// How long it gets after a kill it cannot handle. Exit is then certain, only the
/// scheduler's timing is not: each poll enumerates every process, which on a loaded
/// machine can cost more than the whole handled-signal grace, and declaring a
/// terminating process leaked turns a slow machine into a shutdown error.
pub(super) const PROCESS_KILL_GRACE: Duration = Duration::from_secs(2);
#[cfg(unix)]
pub(super) const MAX_CANCEL_DRAIN_READS: u8 = 4;

/// A Pane's lock as any thread but the Pane's own takes it. Poisoned means a panic ended
/// the Pane, and what the lock guards may be half-changed, so it is not read (ADR 0053).
pub(super) fn lock_pane<T>(mutex: &Mutex<T>) -> io::Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| io::Error::other("the terminal ended after an internal error"))
}

/// The size lock only ever holds a whole `TerminalSize` copied in or out, so even a
/// poisoned one holds a size the Pane had (ADR 0053).
pub(super) fn pane_size(size: &Mutex<crate::TerminalSize>) -> crate::TerminalSize {
    *size
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn join(thread: &mut Option<JoinHandle<io::Result<()>>>, name: &str) -> io::Result<()> {
    match thread.take() {
        Some(thread) => thread
            .join()
            .map_err(|_| io::Error::other(format!("{name} panicked")))?,
        None => Ok(()),
    }
}

pub(super) fn combine_cleanup_results<T>(
    primary: io::Result<T>,
    cleanup: io::Result<()>,
    cleanup_context: &str,
) -> io::Result<T> {
    match (primary, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(primary), Err(cleanup)) => Err(io::Error::new(
            primary.kind(),
            format!("{primary}; additionally failed to {cleanup_context}: {cleanup}"),
        )),
    }
}

pub(super) fn other_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
