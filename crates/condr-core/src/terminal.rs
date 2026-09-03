use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::{
    net::Shutdown,
    os::fd::{AsFd, AsRawFd, RawFd},
    os::unix::net::UnixStream,
};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
#[cfg(test)]
use alacritty_terminal::term::cell::Hyperlink;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Osc52, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};
#[cfg(unix)]
use filedescriptor::FileDescriptor;
pub use portable_pty::CommandBuilder;
use portable_pty::{Child, ExitStatus, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, SmolStrBuilder};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

#[cfg(unix)]
use nix::fcntl::{FcntlArg, OFlag, fcntl};
#[cfg(unix)]
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
#[cfg(unix)]
use nix::unistd::{Pid as UnixPid, getpgid, getsid};

use crate::agent::{
    AgentDetector, AgentPublish, DetectionInput, ProcessInfo, ProcessProbeResult,
    identify_agent_process,
};
use crate::{AgentKind, AgentSnapshot};

const MAX_TERMINAL_CELLS: usize = 65_536;
const MAX_TERMINAL_CELL_TEXT_BYTES: usize = 256;
const MAX_TERMINAL_HYPERLINK_URI_BYTES: usize = 8 * 1024;
const MAX_TERMINAL_HYPERLINK_BYTES: usize = 4 * 1024 * 1024;
// Agent *kind* only changes when a process starts or exits; agent *state* comes from screen
// text, so the process table can refresh well below the 500 ms agent scan cadence.
const PROCESS_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const MAX_OSC_CWD_BYTES: usize = 4 * 1024;
const MAX_TERMINAL_TITLE_CHARS: usize = 256;
const MAX_PENDING_CLIPBOARD_BYTES: usize = 1024 * 1024;
const INPUT_QUEUE_CAPACITY: usize = 64;
const TERMINAL_REPLY_QUEUE_RESERVE: usize = 1;
const TERMINAL_CONTROL_QUEUE_RESERVE: usize = INPUT_QUEUE_CAPACITY + 1;
const MAX_PENDING_INPUT_BYTES: usize = 8 * 1024 * 1024;
const IO_CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROCESS_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
#[cfg(unix)]
const MAX_CANCEL_DRAIN_READS: u8 = 4;

#[cfg(any(windows, test))]
const WINDOWS_POWERSHELL_CWD_HOOK: &str = r"if ($null -eq $global:__CondrOriginalPrompt) { $global:__CondrOriginalPrompt = $function:prompt; function global:prompt { $out = @(& $global:__CondrOriginalPrompt) -join ' '; $loc = $ExecutionContext.SessionState.Path.CurrentLocation; if ($loc.Provider.Name -eq 'FileSystem') { try { [Environment]::CurrentDirectory = $loc.ProviderPath } catch {}; $esc = [string][char]27; $out += $esc + ']9;9;' + $loc.ProviderPath + $esc + '\' }; $out } }";

#[cfg(target_os = "linux")]
const LINUX_BASH_CWD_WRAPPER: &str = r#"exec 3<<'__CONDR_BASHRC__'
if [[ -r "$HOME/.bashrc" ]]; then source "$HOME/.bashrc"; fi
__condr_user_exit=
__condr_trap=$(trap -p EXIT)
if [[ -n $__condr_trap ]]; then
  __condr_trap=${__condr_trap% EXIT}
  eval "__condr_user_exit=${__condr_trap#trap -- }"
fi
__condr_return_status(){ return "$1"; }
trap '__condr_status=$?; printf "\033]9;9;%s\033\\" "$PWD"; if [[ -n $__condr_user_exit ]]; then __condr_return_status "$__condr_status"; eval "$__condr_user_exit"; fi; __condr_return_status "$__condr_status"' EXIT
unset __condr_trap
__CONDR_BASHRC__
exec "$1" --rcfile /dev/fd/3 -i
"#;

mod input;
mod mouse;
mod process;
mod pty_io;
mod runtime;
mod view;

use input::{encode_key, encode_paste};
use mouse::{MAX_MOUSE_WHEEL_STEPS, encode_mouse};
use process::*;
use pty_io::*;
pub use runtime::{
    TerminalAgentProbe, TerminalCwdProbe, TerminalNoticeBatch, TerminalNoticeProbe,
    TerminalRuntime, default_shell_program,
};
use view::{
    SnapshotHyperlinks, TerminalDamageBaseline, publish_view, side, snapshot_terminal,
    terminal_cell, terminal_cursor, viewport_point,
};
pub use view::{
    TerminalCell, TerminalCellRun, TerminalColor, TerminalCommand, TerminalCursor,
    TerminalCursorShape, TerminalFrameError, TerminalHyperlinkBudget, TerminalKey,
    TerminalModifiers, TerminalMouseButton, TerminalMouseEvent, TerminalMousePosition,
    TerminalMouseTracking, TerminalMouseWheel, TerminalPosition, TerminalScroll, TerminalSelection,
    TerminalSide, TerminalSize, TerminalUpdate, TerminalView, TerminalViewDelta, TerminalViewFrame,
    TerminalViewSource,
};
#[cfg(test)]
use view::{blank_cell, terminal_cell_text};

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
