use super::*;

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalCell {
    pub text: SmolStr,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub flags: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalCursor {
    pub row: u16,
    pub column: u16,
    pub shape: TerminalCursorShape,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalView {
    pub revision: u64,
    pub size: TerminalSize,
    pub display_offset: u32,
    pub mouse_tracking: TerminalMouseTracking,
    pub cells: Vec<TerminalCell>,
    pub cursor: Option<TerminalCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalCellRun {
    pub start: u32,
    pub cells: Vec<TerminalCell>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalViewDelta {
    pub base_revision: u64,
    pub revision: u64,
    pub display_offset: u32,
    pub mouse_tracking: TerminalMouseTracking,
    pub cursor: Option<TerminalCursor>,
    pub runs: Vec<TerminalCellRun>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalViewFrame {
    Full(TerminalView),
    Delta(TerminalViewDelta),
}

#[derive(Clone)]
pub struct TerminalViewSource {
    pub(super) terminal: Arc<Mutex<Terminal>>,
    pub(super) size: Arc<Mutex<TerminalSize>>,
    pub(super) revision: Arc<AtomicU64>,
    pub(super) damage_baseline: Arc<Mutex<Option<TerminalDamageBaseline>>>,
}

#[derive(Clone, Copy)]
pub(super) struct TerminalDamageBaseline {
    pub(super) revision: u64,
    pub(super) size: TerminalSize,
    pub(super) display_offset: u32,
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
            || previous.cursor != current.cursor;
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
            runs,
        }))
    }

    pub fn apply_frame(&mut self, frame: TerminalViewFrame) -> Result<(), TerminalFrameError> {
        match frame {
            TerminalViewFrame::Full(view) => {
                *self = view;
                Ok(())
            }
            TerminalViewFrame::Delta(delta) => {
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
                    let start = usize::try_from(run.start)
                        .map_err(|_| TerminalFrameError::InvalidCellRun)?;
                    let end = start
                        .checked_add(run.cells.len())
                        .filter(|end| *end <= self.cells.len())
                        .ok_or(TerminalFrameError::InvalidCellRun)?;
                    if run.cells.is_empty() || start < previous_end {
                        return Err(TerminalFrameError::InvalidCellRun);
                    }
                    previous_end = end;
                }
                for run in delta.runs {
                    let start = usize::try_from(run.start)
                        .map_err(|_| TerminalFrameError::InvalidCellRun)?;
                    let end = start + run.cells.len();
                    self.cells[start..end].clone_from_slice(&run.cells);
                }
                self.revision = delta.revision;
                self.display_offset = delta.display_offset;
                self.mouse_tracking = delta.mouse_tracking;
                self.cursor = delta.cursor;
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub platform: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalMouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalMouseWheel {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalMousePosition {
    pub row: u16,
    pub column: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalMouseEvent {
    Button {
        button: TerminalMouseButton,
        pressed: bool,
        position: TerminalMousePosition,
        modifiers: TerminalModifiers,
    },
    Motion {
        button: Option<TerminalMouseButton>,
        position: TerminalMousePosition,
        modifiers: TerminalModifiers,
    },
    Wheel {
        direction: TerminalMouseWheel,
        amount: u16,
        position: TerminalMousePosition,
        modifiers: TerminalModifiers,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalKey {
    Character(String),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Function(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalScroll {
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
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

impl TerminalView {
    pub fn word_selection_at(&self, row: u16, column: u16) -> Option<TerminalSelection> {
        if row >= self.size.rows || column >= self.size.columns {
            return None;
        }
        let clicked = self.semantic_character(row, column)?;
        if is_word_separator(clicked) {
            return None;
        }

        let mut start = column;
        while start > 0
            && self
                .semantic_character(row, start - 1)
                .is_some_and(|ch| !is_word_separator(ch))
        {
            start -= 1;
        }

        let mut end = column;
        while end + 1 < self.size.columns
            && self
                .semantic_character(row, end + 1)
                .is_some_and(|ch| !is_word_separator(ch))
        {
            end += 1;
        }

        while start <= end
            && self
                .semantic_character(row, start)
                .is_some_and(is_leading_token_wrapper)
        {
            start += 1;
        }
        while start <= end
            && self
                .semantic_character(row, end)
                .is_some_and(is_trailing_token_wrapper)
        {
            if end == 0 {
                return None;
            }
            end -= 1;
        }
        if !(start..=end).contains(&column) {
            return None;
        }

        Some(TerminalSelection {
            start: TerminalPosition {
                row,
                column: start,
                side: TerminalSide::Left,
            },
            end: TerminalPosition {
                row,
                column: end,
                side: TerminalSide::Right,
            },
            display_offset: self.display_offset,
        })
    }

    pub fn line_selection_at(&self, row: u16) -> Option<TerminalSelection> {
        let end = self.size.columns.checked_sub(1)?;
        (row < self.size.rows).then_some(TerminalSelection {
            start: TerminalPosition {
                row,
                column: 0,
                side: TerminalSide::Left,
            },
            end: TerminalPosition {
                row,
                column: end,
                side: TerminalSide::Right,
            },
            display_offset: self.display_offset,
        })
    }

    fn semantic_character(&self, row: u16, column: u16) -> Option<char> {
        let cell = self.cell(row, column)?;
        let flags = Flags::from_bits_retain(cell.flags);
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            return column
                .checked_sub(1)
                .and_then(|column| self.cell(row, column))
                .and_then(|cell| cell.text.chars().next());
        }
        cell.text.chars().next()
    }
}

fn is_word_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '|' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '!'
        )
}

fn is_leading_token_wrapper(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '<' | '"' | '\'' | '`')
}

fn is_trailing_token_wrapper(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '>' | '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '!' | '?'
    )
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalCommand {
    Key {
        key: TerminalKey,
        modifiers: TerminalModifiers,
    },
    Text(String),
    Paste(String),
    Mouse(TerminalMouseEvent),
    Focus(bool),
    Resize(TerminalSize),
    Scroll(TerminalScroll),
    Copy {
        selection: TerminalSelection,
    },
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

pub(super) fn publish_view(revision: &AtomicU64, updates: &mpsc::Sender<TerminalUpdate>) {
    let revision = revision.fetch_add(1, Ordering::AcqRel) + 1;
    let _ = updates.send(TerminalUpdate::View(revision));
}

pub(super) fn snapshot_terminal(
    terminal: &Terminal,
    size: TerminalSize,
    revision: u64,
) -> TerminalView {
    let content = terminal.renderable_content();
    let display_offset = content.display_offset;
    let cursor = terminal_cursor(
        content.cursor.point,
        content.cursor.shape,
        display_offset,
        size,
    );
    let mut cells = vec![blank_cell(); usize::from(size.rows) * usize::from(size.columns)];

    for cell in content.display_iter {
        let row = cell.point.line.0 + display_offset as i32;
        let Ok(row) = u16::try_from(row) else {
            continue;
        };
        let Ok(column) = u16::try_from(cell.point.column.0) else {
            continue;
        };
        if row >= size.rows || column >= size.columns {
            continue;
        }
        cells[usize::from(row) * usize::from(size.columns) + usize::from(column)] =
            terminal_cell(cell.cell, content.colors);
    }

    TerminalView {
        revision,
        size,
        display_offset: u32::try_from(display_offset).unwrap_or(u32::MAX),
        mouse_tracking: TerminalMouseTracking::from_term_mode(*terminal.mode()),
        cells,
        cursor,
    }
}

pub(super) fn terminal_cell(
    cell: &Cell,
    colors: &alacritty_terminal::term::color::Colors,
) -> TerminalCell {
    let text = terminal_cell_text(cell);
    TerminalCell {
        text,
        foreground: terminal_color(cell.fg, colors),
        background: terminal_color(cell.bg, colors),
        flags: cell.flags.bits(),
    }
}

pub(super) fn terminal_cell_text(cell: &Cell) -> SmolStr {
    let mut text = SmolStrBuilder::new();
    text.push(cell.c);
    let mut text_bytes = cell.c.len_utf8();
    if let Some(zerowidth) = cell.zerowidth() {
        for &character in zerowidth {
            if text_bytes + character.len_utf8() > MAX_TERMINAL_CELL_TEXT_BYTES {
                break;
            }
            text.push(character);
            text_bytes += character.len_utf8();
        }
    }
    text.finish()
}

pub(super) fn terminal_cursor(
    point: Point,
    shape: CursorShape,
    display_offset: usize,
    size: TerminalSize,
) -> Option<TerminalCursor> {
    let row = point.line.0 + display_offset as i32;
    u16::try_from(row).ok().and_then(|row| {
        let column = u16::try_from(point.column.0).ok()?;
        (row < size.rows && column < size.columns).then_some(TerminalCursor {
            row,
            column,
            shape: shape.into(),
        })
    })
}

pub(super) fn blank_cell() -> TerminalCell {
    TerminalCell {
        text: SmolStr::new_static(" "),
        foreground: TerminalColor::Named(NamedColor::Foreground as u16),
        background: TerminalColor::Named(NamedColor::Background as u16),
        flags: 0,
    }
}

fn terminal_color(
    color: Color,
    overrides: &alacritty_terminal::term::color::Colors,
) -> TerminalColor {
    let override_color = match color {
        Color::Named(named) => overrides[named],
        Color::Indexed(index) => overrides[usize::from(index)],
        Color::Spec(_) => None,
    };
    let color = override_color.map_or(color, Color::Spec);
    match color {
        Color::Named(named) => TerminalColor::Named(named as u16),
        Color::Indexed(index) => TerminalColor::Indexed(index),
        Color::Spec(rgb) => TerminalColor::Rgb {
            red: rgb.r,
            green: rgb.g,
            blue: rgb.b,
        },
    }
}

impl From<CursorShape> for TerminalCursorShape {
    fn from(shape: CursorShape) -> Self {
        match shape {
            CursorShape::Block => Self::Block,
            CursorShape::Underline => Self::Underline,
            CursorShape::Beam => Self::Beam,
            CursorShape::HollowBlock => Self::HollowBlock,
            CursorShape::Hidden => Self::Hidden,
        }
    }
}

pub(super) fn viewport_point(
    terminal: &Terminal,
    position: TerminalPosition,
    display_offset: u32,
) -> Point {
    let row = usize::from(position.row).min(terminal.screen_lines().saturating_sub(1));
    let column = usize::from(position.column).min(terminal.columns().saturating_sub(1));
    Point::new(
        Line(row as i32 - i32::try_from(display_offset).unwrap_or(i32::MAX)),
        Column(column),
    )
}

pub(super) fn side(side: TerminalSide) -> Side {
    match side {
        TerminalSide::Left => Side::Left,
        TerminalSide::Right => Side::Right,
    }
}
