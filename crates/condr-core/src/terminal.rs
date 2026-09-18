mod cursor_settle;
mod input;
mod input_queue;
mod launch;
mod mouse;
mod notices;
mod osc;
mod probes;
mod process;
mod pty_io;
#[cfg(unix)]
mod pty_unix;
mod resize;
mod runtime;
mod shell;
mod view;
mod view_source;

use crate::agent::{
    AGENT_EVENT_OSC_PREFIX, AgentDetector, AgentEvent, AgentPublish, ProcessInfo,
    ProcessProbeResult, identify_agent_process,
};
use crate::{AgentKind, AgentSnapshot, PaneId};
use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
#[cfg(test)]
use alacritty_terminal::term::cell::Hyperlink;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::search::{RegexIter, RegexSearch};
use alacritty_terminal::term::{Config, Osc52, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};
use cursor_settle::{CURSOR_POSITION_SETTLE_ENABLED, CursorSettle};
#[cfg(unix)]
use filedescriptor::FileDescriptor;
use input::{encode_key, encode_key_in_mode, encode_paste, encode_text_in_mode};
use input_queue::*;
use mouse::{MAX_MOUSE_WHEEL_STEPS, encode_mouse};
#[cfg(unix)]
use nix::fcntl::{FcntlArg, OFlag, fcntl};
#[cfg(unix)]
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
#[cfg(unix)]
use nix::unistd::{Pid as UnixPid, getpgid, getsid};
use notices::*;
use osc::*;
use portable_pty::{Child, ExitStatus, MasterPty, PtySize, native_pty_system};
use probes::recent_text;
use process::*;
use pty_io::*;
#[cfg(unix)]
use pty_unix::*;
use resize::*;
#[cfg(test)]
use runtime::terminal_config;
use serde::{Deserialize, Serialize};
use shell::shell_command;
use smol_str::{SmolStr, SmolStrBuilder};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
#[cfg(unix)]
use std::{
    net::Shutdown,
    os::fd::{AsFd, AsRawFd, RawFd},
    os::unix::net::UnixStream,
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};
use view::{
    SnapshotHyperlinks, detect_links, publish_view, side, snapshot_terminal,
    snapshot_terminal_with_links, terminal_cell, terminal_cursor, viewport_point,
    viewport_selection,
};
#[cfg(test)]
use view::{blank_cell, terminal_cell_text};
use view_source::TerminalDamageBaseline;

pub use cursor_settle::CURSOR_POSITION_SETTLE;
pub use input::{
    TerminalCommand, TerminalKey, TerminalModifiers, TerminalMouseButton, TerminalMouseEvent,
    TerminalMousePosition, TerminalMouseWheel, TerminalScroll,
};
pub use launch::TerminalLaunchProbe;
pub use portable_pty::CommandBuilder;
pub use probes::{TerminalAgentProbe, TerminalCwdProbe, TerminalNoticeBatch, TerminalNoticeProbe};
pub use runtime::TerminalRuntime;
pub(crate) use shell::KNOWN_SHELLS;
pub use shell::{PaneEnvironment, default_shell_program};
pub use view::{
    DEFAULT_ANSI_COLORS, DEFAULT_BACKGROUND_COLOR, DEFAULT_CURSOR_COLOR, DEFAULT_FOREGROUND_COLOR,
    DETECTED_LINK_FLAG, TerminalCell, TerminalCellRun, TerminalColor, TerminalCursor,
    TerminalCursorShape, TerminalFrameError, TerminalHyperlinkBudget, TerminalMouseTracking,
    TerminalPosition, TerminalSelection, TerminalSelectionUnit, TerminalSide, TerminalSize,
    TerminalUpdate, TerminalView, TerminalViewDelta, TerminalViewFrame, default_indexed_color,
};
pub use view_source::TerminalViewSource;

const MAX_TERMINAL_CELLS: usize = 65_536;
const MAX_TERMINAL_CELL_TEXT_BYTES: usize = 256;
const MAX_TERMINAL_HYPERLINK_URI_BYTES: usize = 8 * 1024;
const MAX_TERMINAL_HYPERLINK_BYTES: usize = 4 * 1024 * 1024;
// Process identity is refreshed separately from the state reported by agent hooks.
const PROCESS_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const MAX_OSC_CWD_BYTES: usize = 4 * 1024;
const MAX_TERMINAL_TITLE_CHARS: usize = 256;
const MAX_PENDING_CLIPBOARD_BYTES: usize = 1024 * 1024;
const INPUT_QUEUE_CAPACITY: usize = 64;
const TERMINAL_REPLY_QUEUE_RESERVE: usize = 1;
const TERMINAL_CONTROL_QUEUE_RESERVE: usize = INPUT_QUEUE_CAPACITY + 1;
const MAX_PENDING_INPUT_BYTES: usize = 8 * 1024 * 1024;
const IO_CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// How long a process tree gets to leave after a signal it may handle (hangup, term).
const PROCESS_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
/// How long it gets after a kill it cannot handle. Exit is then certain, only the
/// scheduler's timing is not: each poll enumerates every process, which on a loaded
/// machine can cost more than the whole handled-signal grace, and declaring a
/// terminating process leaked turns a slow machine into a shutdown error.
const PROCESS_KILL_GRACE: Duration = Duration::from_secs(2);
#[cfg(unix)]
const MAX_CANCEL_DRAIN_READS: u8 = 4;

fn join(thread: &mut Option<JoinHandle<io::Result<()>>>, name: &str) -> io::Result<()> {
    match thread.take() {
        Some(thread) => thread
            .join()
            .map_err(|_| io::Error::other(format!("{name} panicked")))?,
        None => Ok(()),
    }
}

fn combine_cleanup_results<T>(
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

fn other_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests;
