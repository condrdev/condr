mod snapshot;
mod wire;

use super::*;
use serde::{Deserializer, Serializer, de::Error as _};
use std::cell::RefCell;
use std::collections::HashMap;

pub(super) use snapshot::{
    DetectedLinks, SnapshotHyperlinks, detect_links, publish_view, side, snapshot_terminal,
    snapshot_terminal_with_links, terminal_cell, terminal_cursor, viewport_point,
    viewport_selection,
};
#[cfg(test)]
pub(super) use snapshot::{blank_cell, terminal_cell_text};
pub use wire::TerminalHyperlinkBudget;

const MAX_TERMINAL_HYPERLINKS: usize = u16::MAX as usize + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub rows: u16,
    pub columns: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalCursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalColor {
    Named(u16),
    Indexed(u8),
    Rgb { red: u8, green: u8, blue: u8 },
}

/// The palette used when nothing else decides a color: OSC 4/10/11 replies to programs that
/// ask, and clients without a color scheme of their own. Entries 0–15 are GitHub Dark; the
/// 6×6×6 cube and the grayscale ramp above are the fixed xterm values.
pub const DEFAULT_ANSI_COLORS: [u32; 16] = [
    0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4, 0x6e7681,
    0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff,
];
pub const DEFAULT_FOREGROUND_COLOR: u32 = 0xc9d1d9;
pub const DEFAULT_BACKGROUND_COLOR: u32 = 0x0d1117;
pub const DEFAULT_CURSOR_COLOR: u32 = 0xf0f6fc;

/// The xterm-256 color at `index` as `0xRRGGBB`, with [`DEFAULT_ANSI_COLORS`] for 0–15.
pub const fn default_indexed_color(index: u8) -> u32 {
    let (red, green, blue) = match index {
        0..=15 => return DEFAULT_ANSI_COLORS[index as usize],
        16..=231 => {
            let value = index - 16;
            (
                color_cube(value / 36),
                color_cube((value / 6) % 6),
                color_cube(value % 6),
            )
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    };
    ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}

const fn color_cube(value: u8) -> u8 {
    if value == 0 { 0 } else { 55 + value * 40 }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalCell {
    pub text: SmolStr,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub flags: u16,
    /// OSC 8 target URI. Terminal view wire formats intern these per frame.
    pub hyperlink: Option<SmolStr>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalCursor {
    pub row: u16,
    pub column: u16,
    pub shape: TerminalCursorShape,
    /// DECSCUSR 1/3/5 or DECSET 12 asked for a blinking cursor; the GUI drives the phase.
    pub blinking: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalMouseTracking {
    #[default]
    None,
    Click,
    Drag,
    Motion,
}

impl TerminalMouseTracking {
    pub(super) fn from_term_mode(mode: TermMode) -> Self {
        if mode.contains(TermMode::MOUSE_MOTION) {
            Self::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            Self::Drag
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            Self::Click
        } else {
            Self::None
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalView {
    pub revision: u64,
    pub size: TerminalSize,
    pub display_offset: u32,
    pub mouse_tracking: TerminalMouseTracking,
    pub cells: Vec<TerminalCell>,
    pub cursor: Option<TerminalCursor>,
    /// The Server-tracked selection clipped to this viewport. alacritty keeps it attached
    /// to its text while output scrolls, which a viewport-anchored Client copy cannot.
    pub selection: Option<TerminalSelection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalCellRun {
    pub start: u32,
    pub cells: Vec<TerminalCell>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalViewDelta {
    pub base_revision: u64,
    pub revision: u64,
    pub display_offset: u32,
    pub mouse_tracking: TerminalMouseTracking,
    pub cursor: Option<TerminalCursor>,
    pub selection: Option<TerminalSelection>,
    pub runs: Vec<TerminalCellRun>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalViewFrame {
    Full(TerminalView),
    Delta(TerminalViewDelta),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalFrameError {
    RevisionMismatch { expected: u64, actual: u64 },
    NonMonotonicRevision { base: u64, revision: u64 },
    InvalidCellRun,
}

impl std::fmt::Display for TerminalFrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RevisionMismatch { expected, actual } => write!(
                formatter,
                "terminal frame expects revision {expected}, but the client has {actual}"
            ),
            Self::NonMonotonicRevision { base, revision } => write!(
                formatter,
                "terminal frame revision {revision} does not advance baseline {base}"
            ),
            Self::InvalidCellRun => {
                formatter.write_str("terminal frame contains an invalid cell run")
            }
        }
    }
}

impl std::error::Error for TerminalFrameError {}

impl TerminalView {
    pub fn cell(&self, row: u16, column: u16) -> Option<&TerminalCell> {
        if row >= self.size.rows || column >= self.size.columns {
            return None;
        }
        self.cells
            .get(usize::from(row) * usize::from(self.size.columns) + usize::from(column))
    }

    pub fn frame_from(previous: Option<&Self>, current: &Self) -> Option<TerminalViewFrame> {
        let Some(previous) = previous else {
            return Some(TerminalViewFrame::Full(current.clone()));
        };
        if previous.size != current.size
            || previous.cells.len() != current.cells.len()
            || current.revision <= previous.revision
        {
            return Some(TerminalViewFrame::Full(current.clone()));
        }

        let mut runs = Vec::new();
        let mut changed_cells = 0usize;
        let mut index = 0usize;
        while index < current.cells.len() {
            if previous.cells[index] == current.cells[index] {
                index += 1;
                continue;
            }
            let start = index;
            while index < current.cells.len() && previous.cells[index] != current.cells[index] {
                index += 1;
            }
            changed_cells += index - start;
            runs.push(TerminalCellRun {
                start: u32::try_from(start).expect("terminal cell count fits u32"),
                cells: current.cells[start..index].to_vec(),
            });
        }

        let metadata_changed = previous.display_offset != current.display_offset
            || previous.mouse_tracking != current.mouse_tracking
            || previous.cursor != current.cursor
            || previous.selection != current.selection;
        if changed_cells == 0 && !metadata_changed {
            return None;
        }
        if changed_cells > current.cells.len() / 2 {
            return Some(TerminalViewFrame::Full(current.clone()));
        }

        Some(TerminalViewFrame::Delta(TerminalViewDelta {
            base_revision: previous.revision,
            revision: current.revision,
            display_offset: current.display_offset,
            mouse_tracking: current.mouse_tracking,
            cursor: current.cursor,
            selection: current.selection,
            runs,
        }))
    }

    pub fn apply_frame(&mut self, frame: TerminalViewFrame) -> Result<(), TerminalFrameError> {
        self.validate_frame(&frame)?;
        match frame {
            TerminalViewFrame::Full(view) => {
                *self = view;
                Ok(())
            }
            TerminalViewFrame::Delta(delta) => {
                self.apply_validated_delta(&delta);
                Ok(())
            }
        }
    }

    pub fn validate_frame(&self, frame: &TerminalViewFrame) -> Result<(), TerminalFrameError> {
        match frame {
            TerminalViewFrame::Full(_) => Ok(()),
            TerminalViewFrame::Delta(delta) => self.validate_delta(delta),
        }
    }

    /// Checks that `delta` applies cleanly to this view without touching it.
    pub fn validate_delta(&self, delta: &TerminalViewDelta) -> Result<(), TerminalFrameError> {
        if delta.revision <= delta.base_revision {
            return Err(TerminalFrameError::NonMonotonicRevision {
                base: delta.base_revision,
                revision: delta.revision,
            });
        }
        if self.revision != delta.base_revision {
            return Err(TerminalFrameError::RevisionMismatch {
                expected: delta.base_revision,
                actual: self.revision,
            });
        }
        let mut previous_end = 0usize;
        for run in &delta.runs {
            let start =
                usize::try_from(run.start).map_err(|_| TerminalFrameError::InvalidCellRun)?;
            let end = start
                .checked_add(run.cells.len())
                .filter(|end| *end <= self.cells.len())
                .ok_or(TerminalFrameError::InvalidCellRun)?;
            if run.cells.is_empty() || start < previous_end {
                return Err(TerminalFrameError::InvalidCellRun);
            }
            previous_end = end;
        }
        Ok(())
    }

    fn apply_validated_delta(&mut self, delta: &TerminalViewDelta) {
        for run in &delta.runs {
            let start =
                usize::try_from(run.start).expect("a validated terminal cell run start fits usize");
            let end = start + run.cells.len();
            self.cells[start..end].clone_from_slice(&run.cells);
        }
        self.revision = delta.revision;
        self.display_offset = delta.display_offset;
        self.mouse_tracking = delta.mouse_tracking;
        self.cursor = delta.cursor;
        self.selection = delta.selection;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalPosition {
    pub row: u16,
    pub column: u16,
    pub side: TerminalSide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalSelection {
    pub start: TerminalPosition,
    pub end: TerminalPosition,
    pub display_offset: u32,
}

impl TerminalSelection {
    pub fn contains_cell(self, row: u16, column: u16, columns: u16) -> bool {
        let Some((start, end)) = self.selected_cell_range(columns) else {
            return false;
        };
        let cell = u32::from(row) * u32::from(columns) + u32::from(column);
        (start..=end).contains(&cell)
    }

    pub fn selected_cell_range(self, columns: u16) -> Option<(u32, u32)> {
        if columns == 0 {
            return None;
        }

        let mut start = self.start;
        let mut end = self.end;
        start.column = start.column.min(columns - 1);
        end.column = end.column.min(columns - 1);
        if (start.row, start.column) > (end.row, end.column) {
            std::mem::swap(&mut start, &mut end);
        }

        if start == end
            || (start.row == end.row
                && start.column.checked_add(1) == Some(end.column)
                && start.side == TerminalSide::Right
                && end.side == TerminalSide::Left)
        {
            return None;
        }

        let different_cells = (start.row, start.column) != (end.row, end.column);
        let mut start_cell = u32::from(start.row) * u32::from(columns) + u32::from(start.column);
        let mut end_cell = u32::from(end.row) * u32::from(columns) + u32::from(end.column);
        if different_cells && start.side == TerminalSide::Right {
            start_cell += 1;
        }
        if different_cells && end.side == TerminalSide::Left {
            end_cell = end_cell.saturating_sub(1);
        }

        (start_cell <= end_cell).then_some((start_cell, end_cell))
    }
}

/// What a multi-click selects around its anchor. The Server expands it with alacritty's
/// semantic search, which follows soft-wrapped lines and matching brackets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalSelectionUnit {
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalUpdate {
    View(u64),
    Exited,
}

impl TerminalSize {
    pub const fn new(rows: u16, columns: u16) -> Self {
        Self {
            rows,
            columns,
            cell_width: 0,
            cell_height: 0,
        }
    }

    pub const fn with_cell_size(mut self, width: u16, height: u16) -> Self {
        self.cell_width = width;
        self.cell_height = height;
        self
    }

    pub(super) fn validate(self) -> io::Result<Self> {
        if self.rows == 0 || self.columns == 0 {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal rows and columns must be non-zero",
            ))
        } else if usize::from(self.rows) * usize::from(self.columns) > MAX_TERMINAL_CELLS {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal grid exceeds the maximum cell count",
            ))
        } else {
            Ok(self)
        }
    }

    pub(super) fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.columns,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.columns)
    }
}

impl From<TerminalSize> for PtySize {
    fn from(size: TerminalSize) -> Self {
        Self {
            rows: size.rows,
            cols: size.columns,
            pixel_width: size.columns.saturating_mul(size.cell_width),
            pixel_height: size.rows.saturating_mul(size.cell_height),
        }
    }
}

/// Set alongside `hyperlink` on cells whose link the Server found in plain text rather than
/// received via OSC 8, so clients can underline those only on hover. The one flag bit
/// alacritty leaves free.
pub const DETECTED_LINK_FLAG: u16 = 1 << 15;
