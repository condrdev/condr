use super::*;
use serde::{Deserializer, Serializer, de::Error as _};
use std::cell::RefCell;
use std::collections::HashMap;

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

#[derive(Serialize, Deserialize)]
struct WireTerminalCell {
    text: SmolStr,
    foreground: TerminalColor,
    background: TerminalColor,
    flags: u16,
    hyperlink: Option<u16>,
}

#[derive(Serialize, Deserialize)]
struct WireTerminalCellRun {
    start: u32,
    cells: Vec<WireTerminalCell>,
}

#[derive(Serialize, Deserialize)]
struct WireTerminalView {
    revision: u64,
    size: TerminalSize,
    display_offset: u32,
    mouse_tracking: TerminalMouseTracking,
    hyperlinks: Vec<SmolStr>,
    cells: Vec<WireTerminalCell>,
    cursor: Option<TerminalCursor>,
    selection: Option<TerminalSelection>,
}

#[derive(Serialize, Deserialize)]
struct WireTerminalViewDelta {
    base_revision: u64,
    revision: u64,
    display_offset: u32,
    mouse_tracking: TerminalMouseTracking,
    cursor: Option<TerminalCursor>,
    selection: Option<TerminalSelection>,
    hyperlinks: Vec<SmolStr>,
    runs: Vec<WireTerminalCellRun>,
}

/// Interns URIs for one wire frame. The pointer cache is sound only because a pass never
/// allocates new strings: every cell keeps its `SmolStr` alive until the pass ends, so a
/// heap address cannot be reused for a different URI mid-pass.
#[derive(Default)]
struct WireHyperlinks {
    values: Vec<SmolStr>,
    indices: HashMap<SmolStr, u16>,
    pointers: HashMap<(usize, usize), Option<u16>>,
    bytes: usize,
}

impl WireHyperlinks {
    fn intern(&mut self, hyperlink: &SmolStr) -> Option<u16> {
        let pointer = heap_string_pointer(hyperlink);
        if let Some(index) = pointer.and_then(|pointer| self.pointers.get(&pointer).copied()) {
            return index;
        }
        let index = self.intern_by_value(hyperlink);
        if let Some(pointer) = pointer {
            self.pointers.insert(pointer, index);
        }
        index
    }

    fn intern_by_value(&mut self, hyperlink: &SmolStr) -> Option<u16> {
        if hyperlink.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES {
            return None;
        }
        if let Some(index) = self.indices.get(hyperlink) {
            return Some(*index);
        }
        let next_bytes = self.bytes.checked_add(hyperlink.len())?;
        if next_bytes > MAX_TERMINAL_HYPERLINK_BYTES {
            return None;
        }
        let index = u16::try_from(self.values.len()).ok()?;
        self.bytes = next_bytes;
        self.values.push(hyperlink.clone());
        self.indices.insert(hyperlink.clone(), index);
        Some(index)
    }

    fn encode_cell(&mut self, cell: &TerminalCell) -> WireTerminalCell {
        let hyperlink = cell
            .hyperlink
            .as_ref()
            .and_then(|hyperlink| self.intern(hyperlink));
        WireTerminalCell {
            text: cell.text.clone(),
            foreground: cell.foreground,
            background: cell.background,
            flags: cell.flags,
            hyperlink,
        }
    }

    fn normalize_cell(&mut self, cell: &mut TerminalCell) -> bool {
        let Some(hyperlink) = cell.hyperlink.take() else {
            return false;
        };
        match self.intern(&hyperlink) {
            Some(index) => {
                cell.hyperlink = Some(self.values[usize::from(index)].clone());
                false
            }
            None => true,
        }
    }
}

fn heap_string_pointer(value: &SmolStr) -> Option<(usize, usize)> {
    value
        .is_heap_allocated()
        .then(|| (value.as_ptr() as usize, value.len()))
}

#[derive(Debug)]
pub struct TerminalHyperlinkBudget {
    by_uri: HashMap<SmolStr, usize>,
    by_pointer: HashMap<(usize, usize), usize>,
    entries: Vec<Option<TrackedHyperlink>>,
    free_entries: Vec<usize>,
    bytes: usize,
}

#[derive(Debug)]
struct TrackedHyperlink {
    uri: SmolStr,
    references: usize,
}

impl TerminalHyperlinkBudget {
    /// Bounds and canonicalizes a full retained view, returning its incremental budget state.
    pub fn new(view: &mut TerminalView) -> Self {
        let mut budget = Self {
            by_uri: HashMap::new(),
            by_pointer: HashMap::new(),
            entries: Vec::new(),
            free_entries: Vec::new(),
            bytes: 0,
        };
        budget.normalize_cells(&mut view.cells);
        budget
    }

    /// Applies a validated delta while keeping retained hyperlink storage bounded.
    pub fn apply_delta(
        &mut self,
        view: &mut TerminalView,
        mut delta: TerminalViewDelta,
    ) -> Result<TerminalViewDelta, TerminalFrameError> {
        view.validate_delta(&delta)?;
        // Common case: no links retained and none arriving, so nothing to release or intern.
        let delta_has_links = delta
            .runs
            .iter()
            .flat_map(|run| &run.cells)
            .any(|cell| cell.hyperlink.is_some());
        if self.by_uri.is_empty() && !delta_has_links {
            view.apply_validated_delta(&delta);
            return Ok(delta);
        }
        for run in &delta.runs {
            let start =
                usize::try_from(run.start).expect("a validated terminal cell run start fits usize");
            for cell in &view.cells[start..start + run.cells.len()] {
                if let Some(hyperlink) = &cell.hyperlink {
                    self.release(hyperlink);
                }
            }
        }
        if delta_has_links {
            self.normalize_cells(delta.runs.iter_mut().flat_map(|run| &mut run.cells));
        }
        view.apply_validated_delta(&delta);
        Ok(delta)
    }

    fn normalize_cells<'a>(
        &mut self,
        cells: impl IntoIterator<Item = &'a mut TerminalCell>,
    ) -> bool {
        let mut aliases = HashMap::new();
        let mut metadata_lost = false;
        for cell in cells {
            let Some(hyperlink) = cell.hyperlink.take() else {
                continue;
            };
            match self.admit(&hyperlink, &mut aliases) {
                Some(index) => {
                    cell.hyperlink = Some(
                        self.entries[index]
                            .as_ref()
                            .expect("admitted terminal hyperlink exists")
                            .uri
                            .clone(),
                    );
                }
                None => metadata_lost = true,
            }
        }
        metadata_lost
    }

    fn admit(
        &mut self,
        uri: &SmolStr,
        aliases: &mut HashMap<(usize, usize), Option<usize>>,
    ) -> Option<usize> {
        let pointer = heap_string_pointer(uri);
        if let Some(index) = pointer.and_then(|pointer| aliases.get(&pointer).copied()) {
            if let Some(index) = index {
                self.retain(index);
            }
            return index;
        }
        if uri.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES {
            if let Some(pointer) = pointer {
                aliases.insert(pointer, None);
            }
            return None;
        }

        let existing = pointer
            .and_then(|pointer| self.by_pointer.get(&pointer).copied())
            .or_else(|| self.by_uri.get(uri).copied());
        if let Some(index) = existing {
            self.retain(index);
            if let Some(pointer) = pointer {
                aliases.insert(pointer, Some(index));
            }
            return Some(index);
        }

        let next_bytes = self.bytes.checked_add(uri.len())?;
        if next_bytes > MAX_TERMINAL_HYPERLINK_BYTES || self.by_uri.len() >= MAX_TERMINAL_HYPERLINKS
        {
            if let Some(pointer) = pointer {
                aliases.insert(pointer, None);
            }
            return None;
        }

        let canonical = uri.clone();
        let index = self.free_entries.pop().unwrap_or(self.entries.len());
        let entry = TrackedHyperlink {
            uri: canonical.clone(),
            references: 1,
        };
        if index == self.entries.len() {
            self.entries.push(Some(entry));
        } else {
            self.entries[index] = Some(entry);
        }
        self.bytes = next_bytes;
        self.by_uri.insert(canonical.clone(), index);
        if let Some(pointer) = heap_string_pointer(&canonical) {
            self.by_pointer.insert(pointer, index);
        }
        if let Some(pointer) = pointer {
            aliases.insert(pointer, Some(index));
        }
        Some(index)
    }

    fn retain(&mut self, index: usize) {
        self.entries[index]
            .as_mut()
            .expect("retained terminal hyperlink exists")
            .references += 1;
    }

    fn release(&mut self, uri: &SmolStr) {
        let index = heap_string_pointer(uri)
            .and_then(|pointer| self.by_pointer.get(&pointer).copied())
            .or_else(|| self.by_uri.get(uri).copied())
            .expect("terminal hyperlink budget matches its retained view");
        let remove = {
            let entry = self.entries[index]
                .as_mut()
                .expect("released terminal hyperlink exists");
            entry.references = entry
                .references
                .checked_sub(1)
                .expect("terminal hyperlink reference count is positive");
            entry.references == 0
        };
        if !remove {
            return;
        }

        let entry = self.entries[index]
            .take()
            .expect("released terminal hyperlink exists");
        self.bytes -= entry.uri.len();
        self.by_uri.remove(&entry.uri);
        if let Some(pointer) = heap_string_pointer(&entry.uri) {
            self.by_pointer.remove(&pointer);
        }
        self.free_entries.push(index);
    }
}

impl WireTerminalCell {
    fn decode(self, hyperlinks: &[SmolStr]) -> Result<TerminalCell, &'static str> {
        let hyperlink = self
            .hyperlink
            .map(|index| {
                hyperlinks
                    .get(usize::from(index))
                    .cloned()
                    .ok_or("terminal cell references an invalid hyperlink index")
            })
            .transpose()?;
        Ok(TerminalCell {
            text: self.text,
            foreground: self.foreground,
            background: self.background,
            flags: self.flags,
            hyperlink,
        })
    }
}

fn decode_wire_cells(
    cells: Vec<WireTerminalCell>,
    hyperlinks: &[SmolStr],
) -> Result<Vec<TerminalCell>, &'static str> {
    cells
        .into_iter()
        .map(|cell| cell.decode(hyperlinks))
        .collect()
}

fn validate_wire_hyperlinks(hyperlinks: &[SmolStr]) -> Result<(), &'static str> {
    if hyperlinks.len() > MAX_TERMINAL_HYPERLINKS {
        return Err("terminal hyperlink table exceeds u16 indices");
    }
    let mut bytes = 0usize;
    for hyperlink in hyperlinks {
        if hyperlink.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES {
            return Err("terminal hyperlink URI exceeds the byte limit");
        }
        bytes = bytes
            .checked_add(hyperlink.len())
            .ok_or("terminal hyperlink table byte count overflowed")?;
        if bytes > MAX_TERMINAL_HYPERLINK_BYTES {
            return Err("terminal hyperlink table exceeds the byte limit");
        }
    }
    Ok(())
}

impl Serialize for TerminalView {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut hyperlinks = WireHyperlinks::default();
        let cells = self
            .cells
            .iter()
            .map(|cell| hyperlinks.encode_cell(cell))
            .collect();
        WireTerminalView {
            revision: self.revision,
            size: self.size,
            display_offset: self.display_offset,
            mouse_tracking: self.mouse_tracking,
            hyperlinks: hyperlinks.values,
            cells,
            cursor: self.cursor,
            selection: self.selection,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TerminalView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireTerminalView::deserialize(deserializer)?;
        validate_wire_hyperlinks(&wire.hyperlinks).map_err(D::Error::custom)?;
        let cells = decode_wire_cells(wire.cells, &wire.hyperlinks).map_err(D::Error::custom)?;
        Ok(Self {
            revision: wire.revision,
            size: wire.size,
            display_offset: wire.display_offset,
            mouse_tracking: wire.mouse_tracking,
            cells,
            cursor: wire.cursor,
            selection: wire.selection,
        })
    }
}

impl Serialize for TerminalViewDelta {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut hyperlinks = WireHyperlinks::default();
        let runs = self
            .runs
            .iter()
            .map(|run| WireTerminalCellRun {
                start: run.start,
                cells: run
                    .cells
                    .iter()
                    .map(|cell| hyperlinks.encode_cell(cell))
                    .collect(),
            })
            .collect();
        WireTerminalViewDelta {
            base_revision: self.base_revision,
            revision: self.revision,
            display_offset: self.display_offset,
            mouse_tracking: self.mouse_tracking,
            cursor: self.cursor,
            selection: self.selection,
            hyperlinks: hyperlinks.values,
            runs,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TerminalViewDelta {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireTerminalViewDelta::deserialize(deserializer)?;
        validate_wire_hyperlinks(&wire.hyperlinks).map_err(D::Error::custom)?;
        let runs = wire
            .runs
            .into_iter()
            .map(|run| {
                Ok(TerminalCellRun {
                    start: run.start,
                    cells: decode_wire_cells(run.cells, &wire.hyperlinks)?,
                })
            })
            .collect::<Result<_, &'static str>>()
            .map_err(D::Error::custom)?;
        Ok(Self {
            base_revision: wire.base_revision,
            revision: wire.revision,
            display_offset: wire.display_offset,
            mouse_tracking: wire.mouse_tracking,
            cursor: wire.cursor,
            selection: wire.selection,
            runs,
        })
    }
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
    pub(super) cursor_settle: Arc<Mutex<super::cursor_settle::CursorSettle>>,
}

#[derive(Clone)]
pub(super) struct TerminalDamageBaseline {
    pub(super) revision: u64,
    pub(super) size: TerminalSize,
    pub(super) display_offset: u32,
    /// The cursor as published, after settling; a settle alone must still emit a frame.
    pub(super) cursor: Option<TerminalCursor>,
    /// The plain-text URLs as published; a cell whose link changed is damage alacritty
    /// cannot know about, since the text it sits in may be untouched.
    pub(super) links: DetectedLinks,
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
    /// Applies the hyperlink limits used by the wire codec and reports metadata loss.
    pub fn normalize_hyperlinks_for_wire(&mut self) -> bool {
        let mut hyperlinks = WireHyperlinks::default();
        let mut changed = false;
        for cell in &mut self.cells {
            changed |= hyperlinks.normalize_cell(cell);
        }
        changed
    }

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

impl TerminalViewDelta {
    fn normalize_hyperlinks_for_wire(&mut self) -> bool {
        let mut hyperlinks = WireHyperlinks::default();
        let mut changed = false;
        for cell in self.runs.iter_mut().flat_map(|run| &mut run.cells) {
            changed |= hyperlinks.normalize_cell(cell);
        }
        changed
    }
}

impl TerminalViewFrame {
    /// Applies the hyperlink limits used by the wire codec and reports metadata loss.
    pub fn normalize_hyperlinks_for_wire(&mut self) -> bool {
        match self {
            Self::Full(view) => view.normalize_hyperlinks_for_wire(),
            Self::Delta(delta) => delta.normalize_hyperlinks_for_wire(),
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

/// What a multi-click selects around its anchor. The Server expands it with alacritty's
/// semantic search, which follows soft-wrapped lines and matching brackets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalSelectionUnit {
    Word,
    Line,
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
    /// Replaces the Server-tracked selection; `None` clears it.
    Select(Option<TerminalSelection>),
    /// Replaces the Server-tracked selection with the word or line around a viewport cell.
    SelectAt {
        position: TerminalPosition,
        display_offset: u32,
        unit: TerminalSelectionUnit,
    },
    /// Copies the Server-tracked selection, or an explicit viewport range.
    Copy {
        selection: Option<TerminalSelection>,
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

/// Set alongside `hyperlink` on cells whose link the Server found in plain text rather than
/// received via OSC 8, so clients can underline those only on hover. The one flag bit
/// alacritty leaves free.
pub const DETECTED_LINK_FLAG: u16 = 1 << 15;

/// Lowercase throughout, which makes alacritty's search case-insensitive. A URL runs from a
/// known scheme to whitespace or a quote/bracket; [`trimmed_url_len`] drops what prose adds.
const URL_PATTERN: &str = r#"(https?://|file://|ftp://|git://|gemini://|gopher://|zed://|ipfs:|ipns:|magnet:|mailto:|news:|ssh:)[^\s"'<>`]+"#;

thread_local! {
    static URL_SEARCH: RefCell<RegexSearch> =
        RefCell::new(RegexSearch::new(URL_PATTERN).expect("the URL pattern compiles"));
}

/// Plain-text URLs in the viewport, keyed by viewport cell index.
pub(super) type DetectedLinks = HashMap<u32, SmolStr>;

/// Finds plain-text URLs with alacritty's grid search, which follows soft-wrapped lines and
/// steps over wide characters, then trims trailing punctuation and unbalanced brackets.
pub(super) fn detect_links(terminal: &Terminal, size: TerminalSize) -> DetectedLinks {
    let mut links = DetectedLinks::new();
    let (Some(last_row), Some(last_column)) =
        (size.rows.checked_sub(1), size.columns.checked_sub(1))
    else {
        return links;
    };
    let display_offset = i32::try_from(terminal.grid().display_offset()).unwrap_or(i32::MAX);
    let start = Point::new(Line(-display_offset), Column(0));
    let end = Point::new(
        Line(i32::from(last_row) - display_offset),
        Column(usize::from(last_column)),
    );
    let columns = u32::from(size.columns);
    let spacer = Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER;
    URL_SEARCH.with_borrow_mut(|regex| {
        for found in RegexIter::new(start, end, Direction::Right, terminal, regex) {
            // Every cell of the match in order. Spacers carry no character but stay linked so
            // the run has no holes.
            let mut cells = Vec::new();
            let mut point = *found.start();
            loop {
                let cell = &terminal.grid()[point];
                let row = u32::try_from(point.line.0 + display_offset).unwrap_or(u32::MAX);
                let character = (!cell.flags.intersects(spacer)).then_some(cell.c);
                cells.push((row * columns + point.column.0 as u32, character));
                if point == *found.end() {
                    break;
                }
                point = point.add(terminal, Boundary::None, 1);
            }
            let chars = cells
                .iter()
                .filter_map(|(_, character)| *character)
                .collect::<Vec<_>>();
            let keep = trimmed_url_len(&chars);
            if keep == 0 {
                continue;
            }
            let uri = SmolStr::from(chars[..keep].iter().collect::<String>());
            let mut kept = 0;
            for (index, character) in cells {
                if character.is_some() {
                    if kept == keep {
                        break;
                    }
                    kept += 1;
                }
                links.insert(index, uri.clone());
            }
        }
    });
    links
}

/// The URL's length without trailing punctuation and closing brackets that have no opener:
/// prose puts those after a URL far more often than a URL ends with them.
fn trimmed_url_len(chars: &[char]) -> usize {
    let mut end = chars.len();
    loop {
        match chars[..end].last() {
            Some('.' | ',' | ';' | ':' | '!' | '?') => end -= 1,
            Some(&close @ (')' | ']' | '}')) => {
                let open = match close {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                let span = &chars[..end];
                let count = |wanted| span.iter().filter(|&&c| c == wanted).count();
                if count(open) < count(close) {
                    end -= 1;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    end
}

pub(super) fn snapshot_terminal(
    terminal: &Terminal,
    size: TerminalSize,
    revision: u64,
) -> TerminalView {
    let links = detect_links(terminal, size);
    snapshot_terminal_with_links(terminal, size, revision, &links)
}

pub(super) fn snapshot_terminal_with_links(
    terminal: &Terminal,
    size: TerminalSize,
    revision: u64,
    links: &DetectedLinks,
) -> TerminalView {
    let content = terminal.renderable_content();
    let display_offset = content.display_offset;
    let cursor = terminal_cursor(
        content.cursor.point,
        content.cursor.shape,
        terminal.cursor_style().blinking,
        display_offset,
        size,
    );
    let mut cells = vec![blank_cell(); usize::from(size.rows) * usize::from(size.columns)];
    let mut hyperlinks = SnapshotHyperlinks::default();

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
        let index = usize::from(row) * usize::from(size.columns) + usize::from(column);
        cells[index] = terminal_cell(
            cell.cell,
            content.colors,
            &mut hyperlinks,
            links.get(&(index as u32)),
        );
    }

    TerminalView {
        revision,
        size,
        display_offset: u32::try_from(display_offset).unwrap_or(u32::MAX),
        mouse_tracking: TerminalMouseTracking::from_term_mode(*terminal.mode()),
        cells,
        cursor,
        selection: viewport_selection(terminal, size),
    }
}

/// The Server-tracked selection as inclusive viewport cells, or `None` when there is none.
/// A selection that exists but is entirely scrolled out of view is reported one row past
/// the grid (`row == size.rows`): it matches no cell, so nothing is painted, yet the
/// Client still knows a copy would succeed.
pub(super) fn viewport_selection(
    terminal: &Terminal,
    size: TerminalSize,
) -> Option<TerminalSelection> {
    let range = terminal.selection.as_ref()?.to_range(terminal)?;
    let display_offset = i64::try_from(terminal.grid().display_offset()).ok()?;
    let last_row = i64::from(size.rows.checked_sub(1)?);
    let last_column = size.columns.checked_sub(1)?;
    let start_row = i64::from(range.start.line.0) + display_offset;
    let end_row = i64::from(range.end.line.0) + display_offset;
    if end_row < 0 || start_row > last_row {
        let hidden = |column| TerminalPosition {
            row: size.rows,
            column,
            side: TerminalSide::Left,
        };
        return Some(TerminalSelection {
            start: hidden(0),
            end: TerminalPosition {
                side: TerminalSide::Right,
                ..hidden(last_column)
            },
            display_offset: u32::try_from(display_offset).ok()?,
        });
    }
    let (start_row, start_column) = if start_row < 0 {
        (0, 0)
    } else {
        (
            start_row,
            u16::try_from(range.start.column.0).unwrap_or(u16::MAX),
        )
    };
    let (end_row, end_column) = if end_row > last_row {
        (last_row, last_column)
    } else {
        (
            end_row,
            u16::try_from(range.end.column.0).unwrap_or(u16::MAX),
        )
    };
    Some(TerminalSelection {
        start: TerminalPosition {
            row: u16::try_from(start_row).ok()?,
            column: start_column.min(last_column),
            side: TerminalSide::Left,
        },
        end: TerminalPosition {
            row: u16::try_from(end_row).ok()?,
            column: end_column.min(last_column),
            side: TerminalSide::Right,
        },
        display_offset: u32::try_from(display_offset).ok()?,
    })
}

#[derive(Default)]
pub(super) struct SnapshotHyperlinks {
    values: std::collections::HashSet<SmolStr>,
    pointers: HashMap<(usize, usize), Option<SmolStr>>,
    pub(super) bytes: usize,
}

impl SnapshotHyperlinks {
    fn intern(&mut self, uri: &str) -> Option<SmolStr> {
        let pointer = (uri.as_ptr() as usize, uri.len());
        if let Some(uri) = self.pointers.get(&pointer) {
            return uri.clone();
        }
        let canonical = if uri.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES {
            None
        } else if let Some(uri) = self.values.get(uri) {
            Some(uri.clone())
        } else {
            let next_bytes = self.bytes.checked_add(uri.len())?;
            if next_bytes > MAX_TERMINAL_HYPERLINK_BYTES {
                None
            } else {
                let uri = SmolStr::new(uri);
                self.bytes = next_bytes;
                self.values.insert(uri.clone());
                Some(uri)
            }
        };
        self.pointers.insert(pointer, canonical.clone());
        canonical
    }
}

pub(super) fn terminal_cell(
    cell: &Cell,
    colors: &alacritty_terminal::term::color::Colors,
    hyperlinks: &mut SnapshotHyperlinks,
    detected_link: Option<&SmolStr>,
) -> TerminalCell {
    let text = terminal_cell_text(cell);
    let mut flags = cell.flags.bits();
    // An OSC 8 link is the program's word; a detected one only fills in where it said nothing.
    let hyperlink = match (cell.hyperlink(), detected_link) {
        (Some(link), _) => hyperlinks.intern(link.uri()),
        (None, Some(uri)) => {
            let uri = hyperlinks.intern(uri);
            if uri.is_some() {
                flags |= DETECTED_LINK_FLAG;
            }
            uri
        }
        (None, None) => None,
    };
    TerminalCell {
        text,
        foreground: terminal_color(cell.fg, colors),
        background: terminal_color(cell.bg, colors),
        flags,
        hyperlink,
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
    blinking: bool,
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
            blinking,
        })
    })
}

pub(super) fn blank_cell() -> TerminalCell {
    TerminalCell {
        text: SmolStr::new_static(" "),
        foreground: TerminalColor::Named(NamedColor::Foreground as u16),
        background: TerminalColor::Named(NamedColor::Background as u16),
        flags: 0,
        hyperlink: None,
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
