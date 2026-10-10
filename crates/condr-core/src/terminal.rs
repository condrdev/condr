mod input;
mod view;

#[cfg(feature = "runtime")]
mod cursor_settle;
#[cfg(feature = "runtime")]
mod encode;
#[cfg(feature = "runtime")]
mod input_queue;
#[cfg(all(feature = "runtime", windows))]
mod job;
#[cfg(feature = "runtime")]
mod launch;
#[cfg(feature = "runtime")]
mod mouse;
#[cfg(feature = "runtime")]
mod notices;
#[cfg(feature = "runtime")]
mod osc;
#[cfg(feature = "runtime")]
mod prelude;
#[cfg(feature = "runtime")]
mod probes;
#[cfg(feature = "runtime")]
mod process;
#[cfg(feature = "runtime")]
mod pty_io;
#[cfg(all(feature = "runtime", unix))]
mod pty_unix;
#[cfg(feature = "runtime")]
mod resize;
#[cfg(feature = "runtime")]
mod runtime;
#[cfg(feature = "runtime")]
mod shell;
#[cfg(feature = "runtime")]
mod view_source;

#[cfg(feature = "runtime")]
use prelude::*;

pub use input::{
    TerminalCommand, TerminalKey, TerminalKeyEventKind, TerminalModifiers, TerminalMouseButton,
    TerminalMouseEvent, TerminalMousePosition, TerminalMouseWheel, TerminalScroll,
};
pub use view::{
    DEFAULT_ANSI_COLORS, DEFAULT_BACKGROUND_COLOR, DEFAULT_CURSOR_COLOR, DEFAULT_FOREGROUND_COLOR,
    DETECTED_LINK_FLAG, TerminalCell, TerminalCellRun, TerminalColor, TerminalCursor,
    TerminalCursorShape, TerminalFrameError, TerminalHyperlinkBudget, TerminalMouseTracking,
    TerminalPosition, TerminalSelection, TerminalSelectionUnit, TerminalSide, TerminalSize,
    TerminalUpdate, TerminalView, TerminalViewDelta, TerminalViewFrame, default_indexed_color,
};
pub(crate) use view::{
    decode_position, decode_selection, decode_size, encode_position, encode_selection,
};

#[cfg(feature = "runtime")]
pub use cursor_settle::CURSOR_POSITION_SETTLE;
#[cfg(feature = "runtime")]
pub use launch::TerminalLaunchProbe;
#[cfg(feature = "runtime")]
pub use portable_pty::CommandBuilder;
#[cfg(feature = "runtime")]
pub use probes::{TerminalAgentProbe, TerminalCwdProbe, TerminalNoticeBatch, TerminalNoticeProbe};
#[cfg(feature = "runtime")]
pub use runtime::TerminalRuntime;
#[cfg(feature = "runtime")]
pub(crate) use shell::KNOWN_SHELLS;
#[cfg(feature = "runtime")]
pub use shell::{PaneEnvironment, default_shell_program};
#[cfg(feature = "runtime")]
pub use view_source::TerminalViewSource;

const MAX_TERMINAL_CELLS: usize = 65_536;
const MAX_TERMINAL_CELL_TEXT_BYTES: usize = 256;
const MAX_TERMINAL_HYPERLINK_URI_BYTES: usize = 8 * 1024;
const MAX_TERMINAL_HYPERLINK_BYTES: usize = 4 * 1024 * 1024;

#[cfg(all(test, feature = "runtime"))]
mod tests;
