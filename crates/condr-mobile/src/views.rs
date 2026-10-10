//! What Swift and Kotlin read: plain records built from the model when asked, never kept.

use condr_core::{
    AgentDisplayState, DEFAULT_ANSI_COLORS, DEFAULT_BACKGROUND_COLOR, DEFAULT_CURSOR_COLOR,
    DEFAULT_FOREGROUND_COLOR, TerminalColor, TerminalView, default_indexed_color,
};

/// What changed since the last [`crate::Companion::take_changes`]; the app re-reads those.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, uniffi::Enum)]
pub enum Change {
    Devices,
    NeedsYou,
    /// A Device's Workspaces, Panes or Agents.
    Device {
        address: String,
    },
    Terminal {
        address: String,
        pane: u64,
    },
}

/// A result that is not state: each is delivered once, in order, and none survives a
/// reconnect (ADR 0039).
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum Event {
    /// A failed operation, for one message the person can read and copy (ADR 0020).
    Error { address: String, message: String },
    /// The text a Copy of the selection returned.
    Copied { text: String },
    /// An OSC 52 copy from the Pane on screen (ADR 0007, ADR 0041); only the newest
    /// waiting one is kept.
    Clipboard { text: String },
}

/// A paired Device as the app keeps it: an address without its invite, and its name.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SavedDevice {
    pub address: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum DeviceStatus {
    /// The app is in the background, or has not resumed yet.
    Suspended,
    Connecting,
    Connected,
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DeviceView {
    pub address: String,
    pub label: String,
    pub status: DeviceStatus,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct WorkspaceView {
    pub id: u64,
    pub name: String,
    pub panes: Vec<PaneView>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct PaneView {
    pub id: u64,
    /// The title its program set, else its Agent's name.
    pub title: Option<String>,
    pub agent: Option<AgentView>,
    /// It rang the bell and nobody has looked since.
    pub attention: bool,
    pub exited: bool,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AgentView {
    pub label: String,
    pub status: AgentStatus,
    pub blocked_on: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AgentStatus {
    Unknown,
    Idle,
    Working,
    Blocked,
    /// Went idle after work this phone did not watch (ADR 0041).
    Done,
}

impl From<AgentDisplayState> for AgentStatus {
    fn from(state: AgentDisplayState) -> Self {
        match state {
            AgentDisplayState::Unknown => Self::Unknown,
            AgentDisplayState::Idle => Self::Idle,
            AgentDisplayState::Working => Self::Working,
            AgentDisplayState::Blocked => Self::Blocked,
            AgentDisplayState::Done => Self::Done,
        }
    }
}

/// One "Needs you" row (ADR 0024): a Blocked Agent and what it waits for.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct NeedsYouItem {
    pub address: String,
    pub device: String,
    pub workspace: String,
    pub pane: u64,
    pub agent: String,
    pub blocked_on: Option<String>,
}

/// A Pane's screen as the Server's grid has it, colours resolved to `0xRRGGBB`.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TerminalScreen {
    pub revision: u64,
    pub rows: u16,
    pub columns: u16,
    /// Row by row; a wide character's second cell is empty.
    pub cells: Vec<ScreenCell>,
    pub cursor: Option<ScreenCursor>,
    pub background: u32,
    /// The program tracks the mouse: a tap is a click, a swipe is wheel motion.
    pub mouse_tracking: bool,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ScreenCell {
    pub text: String,
    pub foreground: u32,
    pub background: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub link: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct ScreenCursor {
    pub row: u16,
    pub column: u16,
    pub color: u32,
}

/// The keys the row above the soft keyboard adds (ADR 0041), and text.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum Key {
    Enter,
    Escape,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Character { text: String },
}

// The cell flags the Server sends: alacritty's bits.
const INVERSE: u16 = 1 << 0;
const BOLD: u16 = 1 << 1;
const ITALIC: u16 = 1 << 2;
const UNDERLINE: u16 = 1 << 3;
const WIDE_CHAR_SPACER: u16 = 1 << 6;
const HIDDEN: u16 = 1 << 8;
const LEADING_WIDE_CHAR_SPACER: u16 = 1 << 10;

pub(crate) fn screen(view: &TerminalView) -> TerminalScreen {
    let cells = view
        .cells
        .iter()
        .map(|cell| {
            let mut foreground = color(cell.foreground, true);
            let mut background = color(cell.background, false);
            if cell.flags & INVERSE != 0 {
                std::mem::swap(&mut foreground, &mut background);
            }
            let blank = cell.flags & (HIDDEN | WIDE_CHAR_SPACER | LEADING_WIDE_CHAR_SPACER) != 0;
            ScreenCell {
                text: if blank {
                    String::new()
                } else {
                    cell.text.to_string()
                },
                foreground,
                background,
                bold: cell.flags & BOLD != 0,
                italic: cell.flags & ITALIC != 0,
                underline: cell.flags & UNDERLINE != 0,
                link: cell.hyperlink.as_ref().map(ToString::to_string),
            }
        })
        .collect();
    TerminalScreen {
        revision: view.revision,
        rows: view.size.rows,
        columns: view.size.columns,
        cells,
        cursor: view.cursor.map(|cursor| ScreenCursor {
            row: cursor.row,
            column: cursor.column,
            color: DEFAULT_CURSOR_COLOR,
        }),
        background: DEFAULT_BACKGROUND_COLOR,
        mouse_tracking: view.mouse_tracking != condr_core::TerminalMouseTracking::None,
    }
}

/// The default scheme's colour for `color`; the desktop GUI maps the same names onto the
/// scheme the person picked.
// ponytail: one fixed scheme; follow the desktop's Color Scheme once the app has settings.
fn color(color: TerminalColor, foreground: bool) -> u32 {
    match color {
        TerminalColor::Rgb { red, green, blue } => {
            (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
        }
        TerminalColor::Indexed(index) => default_indexed_color(index),
        TerminalColor::Named(index @ 0..=15) => DEFAULT_ANSI_COLORS[usize::from(index)],
        TerminalColor::Named(index @ 259..=266) => DEFAULT_ANSI_COLORS[usize::from(index - 259)],
        TerminalColor::Named(257) => DEFAULT_BACKGROUND_COLOR,
        TerminalColor::Named(258) => DEFAULT_CURSOR_COLOR,
        TerminalColor::Named(_) if foreground => DEFAULT_FOREGROUND_COLOR,
        TerminalColor::Named(_) => DEFAULT_BACKGROUND_COLOR,
    }
}
