//! What Swift and Kotlin read: plain records built from the model when asked, never kept.

use condr_core::{
    AgentDisplayState, DEFAULT_BACKGROUND_COLOR, DEFAULT_CURSOR_COLOR, DEFAULT_FOREGROUND_COLOR,
    TerminalColor, TerminalView, default_indexed_color,
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
    /// The Server no longer knows this phone: it refused the handshake with
    /// `NotAuthorized` or `DeviceRevoked` (ADR 0040). Only pairing again helps.
    Revoked,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DeviceView {
    pub address: String,
    /// The Device id its address names, what Condr Cloud and a push know it by (ADR
    /// 0047); none for an address that does not parse.
    pub device_id: Option<String>,
    pub label: String,
    pub status: DeviceStatus,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct WorkspaceView {
    pub id: u64,
    pub name: String,
    /// The root directory as the Server names it, for the Workspace's heading.
    pub root: String,
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
    /// The Agent's stable id, `claude`, `codex` and so on, or `other` for a kind this build
    /// does not know (ADR 0028); the app picks the mark by it.
    pub kind: String,
    pub label: String,
    pub status: AgentStatus,
    /// What a Blocked Agent waits for, or why an Error one halted (ADR 0024, ADR 0044).
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AgentStatus {
    Unknown,
    Idle,
    Working,
    Blocked,
    /// Finished work nobody has looked at, on this phone or anywhere else (ADR 0052).
    Done,
    /// A turn failed and the Agent halted.
    Error,
}

impl From<AgentDisplayState> for AgentStatus {
    fn from(state: AgentDisplayState) -> Self {
        match state {
            AgentDisplayState::Unknown => Self::Unknown,
            AgentDisplayState::Idle => Self::Idle,
            AgentDisplayState::Working => Self::Working,
            AgentDisplayState::Blocked => Self::Blocked,
            AgentDisplayState::Done => Self::Done,
            AgentDisplayState::Error => Self::Error,
        }
    }
}

/// One "Needs you" row (ADR 0024): a Blocked or Error Agent and what it waits for.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct NeedsYouItem {
    pub address: String,
    pub device: String,
    pub workspace: String,
    pub pane: u64,
    /// The Agent's stable id, as [`AgentView::kind`].
    pub kind: String,
    pub agent: String,
    pub detail: Option<String>,
}

/// A Pane's screen as the Server's grid has it, colours resolved to `0xRRGGBB`.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TerminalScreen {
    pub revision: u64,
    pub rows: u16,
    pub columns: u16,
    /// How many lines above the newest output the viewport sits; 0 at the bottom (ADR
    /// 0036: a Pane has one scroll position, shared by every viewer).
    pub scrolled_lines: u32,
    /// Row by row; a wide character's second cell is empty.
    pub cells: Vec<ScreenCell>,
    pub cursor: Option<ScreenCursor>,
    pub background: u32,
    /// The program tracks the mouse: a tap is a click, a swipe is wheel motion.
    pub mouse_tracking: bool,
    /// Part of the screen is the Pane's selection, shared with every viewer (ADR 0008);
    /// its cells come in the selection's colours.
    pub has_selection: bool,
    /// The kitty graphics images on the screen, in the order to draw them (ADR 0065).
    pub images: Vec<ScreenImage>,
}

/// Which pass of the screen an image is drawn in (ADR 0065).
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Enum)]
pub enum ScreenImageLayer {
    BelowBackground,
    BelowText,
    AboveText,
}

/// One kitty graphics image on the screen (ADR 0065), measured in this screen's cells so
/// the phone draws it at its own; [`crate::Companion::terminal_image`] gives its PNG once
/// fetched, and a `Terminal` change says when.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ScreenImage {
    /// The image's hex SHA-256, what `terminal_image` takes.
    pub key: String,
    pub layer: ScreenImageLayer,
    pub column: f64,
    pub row: f64,
    pub width: f64,
    pub height: f64,
    /// The part of the image drawn, as fractions of its width and height.
    pub source_x: f64,
    pub source_y: f64,
    pub source_width: f64,
    pub source_height: f64,
}

pub(crate) fn image_key(key: &condr_core::TerminalImageKey) -> String {
    key.0.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn parse_image_key(key: &str) -> Option<condr_core::TerminalImageKey> {
    let bytes = key.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        out[index] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(condr_core::TerminalImageKey(out))
}

/// The view's images in cells, from the pixels of the VT's cells.
fn screen_images(view: &TerminalView) -> Vec<ScreenImage> {
    let (cell_width, cell_height) = (
        f64::from(view.size.cell_width),
        f64::from(view.size.cell_height),
    );
    if cell_width == 0. || cell_height == 0. {
        return Vec::new();
    }
    view.images
        .iter()
        .filter(|image| {
            image.image_width > 0
                && image.image_height > 0
                && image.source_width > 0
                && image.source_height > 0
        })
        .map(|image| {
            let (width, height) = (f64::from(image.image_width), f64::from(image.image_height));
            ScreenImage {
                key: image_key(&image.image),
                layer: match image.layer {
                    condr_core::TerminalImageLayer::BelowBackground => {
                        ScreenImageLayer::BelowBackground
                    }
                    condr_core::TerminalImageLayer::BelowText => ScreenImageLayer::BelowText,
                    condr_core::TerminalImageLayer::AboveText => ScreenImageLayer::AboveText,
                },
                column: f64::from(image.column) + f64::from(image.offset_x) / cell_width,
                row: f64::from(image.row) + f64::from(image.offset_y) / cell_height,
                width: f64::from(image.width) / cell_width,
                height: f64::from(image.height) / cell_height,
                source_x: f64::from(image.source_x) / width,
                source_y: f64::from(image.source_y) / height,
                source_width: f64::from(image.source_width) / width,
                source_height: f64::from(image.source_height) / height,
            }
        })
        .collect()
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
    /// Shift-Tab, which Agents bind to switching their mode.
    BackTab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Character {
        text: String,
    },
}

/// The default scheme's selection, GitHub Dark Default's: what the desktop draws there too.
const SELECTION_BACKGROUND: u32 = 0xe6edf3;
const SELECTION_TEXT: u32 = 0x0d1117;

pub(crate) fn screen(view: &TerminalView) -> TerminalScreen {
    let columns = view.size.columns.max(1);
    let cells = view
        .cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let mut foreground = color(cell.foreground, true);
            let mut background = color(cell.background, false);
            if cell.flags & condr_core::CELL_INVERSE != 0 {
                std::mem::swap(&mut foreground, &mut background);
            }
            if cell.flags & condr_core::CELL_FAINT != 0 {
                foreground = faint(foreground, background);
            }
            let (row, column) = (index / usize::from(columns), index % usize::from(columns));
            if view.selection.is_some_and(|selection| {
                selection.contains_cell(row as u16, column as u16, columns)
            }) {
                foreground = SELECTION_TEXT;
                background = SELECTION_BACKGROUND;
            }
            let blank = cell.is_spacer() || cell.flags & condr_core::CELL_INVISIBLE != 0;
            ScreenCell {
                text: if blank {
                    String::new()
                } else {
                    cell.text.to_string()
                },
                foreground,
                background,
                bold: cell.flags & condr_core::CELL_BOLD != 0,
                italic: cell.flags & condr_core::CELL_ITALIC != 0,
                underline: cell.flags & condr_core::CELL_UNDERLINE != 0,
                link: cell.hyperlink.as_ref().map(ToString::to_string),
            }
        })
        .collect();
    TerminalScreen {
        revision: view.revision,
        rows: view.size.rows,
        columns: view.size.columns,
        scrolled_lines: view.display_offset,
        cells,
        cursor: view.cursor.map(|cursor| ScreenCursor {
            row: cursor.row,
            column: cursor.column,
            color: DEFAULT_CURSOR_COLOR,
        }),
        background: DEFAULT_BACKGROUND_COLOR,
        mouse_tracking: view.mouse_tracking != condr_core::TerminalMouseTracking::None,
        has_selection: view.selection.is_some(),
        images: screen_images(view),
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
        TerminalColor::Default if foreground => DEFAULT_FOREGROUND_COLOR,
        TerminalColor::Default => DEFAULT_BACKGROUND_COLOR,
    }
}

/// Faint text, as the desktop draws it: the foreground at 65% over the background.
fn faint(foreground: u32, background: u32) -> u32 {
    let channel = |shift: u32| {
        let (front, back) = ((foreground >> shift) & 0xff, (background >> shift) & 0xff);
        ((front * 65 + back * 35) / 100) << shift
    };
    channel(16) | channel(8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use condr_core::{TerminalMouseTracking, TerminalSize};

    #[test]
    fn the_screen_carries_how_far_the_viewport_is_scrolled() {
        let view = TerminalView {
            revision: 3,
            size: TerminalSize::new(2, 2),
            display_offset: 7,
            mouse_tracking: TerminalMouseTracking::None,
            cells: Vec::new(),
            cursor: None,
            selection: None,
            images: Vec::new(),
        };
        let screen = screen(&view);
        assert_eq!(screen.scrolled_lines, 7);
        assert_eq!((screen.rows, screen.columns), (2, 2));
    }
}
