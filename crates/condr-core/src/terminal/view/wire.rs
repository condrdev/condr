use super::*;
use crate::protocol::pb::{self, WireError, WireResult, enum_or, malformed, narrow, required};

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

/// A cell color as one varint: the kind in the low two bits, its value above them.
fn encode_color(color: TerminalColor) -> u32 {
    match color {
        TerminalColor::Named(value) => u32::from(value) << 2 | 1,
        TerminalColor::Indexed(value) => u32::from(value) << 2 | 2,
        TerminalColor::Rgb { red, green, blue } => {
            (u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue)) << 2 | 3
        }
    }
}

fn decode_color(value: u32) -> WireResult<TerminalColor> {
    let payload = value >> 2;
    Ok(match value & 3 {
        1 => TerminalColor::Named(narrow(payload, "named color")?),
        2 => TerminalColor::Indexed(narrow(payload, "indexed color")?),
        3 if payload <= 0xff_ffff => TerminalColor::Rgb {
            red: (payload >> 16) as u8,
            green: (payload >> 8) as u8,
            blue: payload as u8,
        },
        _ => return Err(malformed("terminal cell color is not a known kind")),
    })
}

/// Writes cells into columns, interning their hyperlinks. `flags` and `hyperlink` stay
/// empty until a cell needs them, since most frames have neither.
fn encode_cells<'a>(
    cells: impl ExactSizeIterator<Item = &'a TerminalCell>,
    hyperlinks: &mut WireHyperlinks,
) -> pb::TerminalCells {
    let count = cells.len();
    let mut columns = pb::TerminalCells {
        text: String::with_capacity(count),
        text_len: Vec::with_capacity(count),
        foreground: Vec::with_capacity(count),
        background: Vec::with_capacity(count),
        flags: Vec::new(),
        hyperlink: Vec::new(),
    };
    let (mut any_flags, mut any_hyperlink) = (false, false);
    for (index, cell) in cells.enumerate() {
        columns.text.push_str(&cell.text);
        columns.text_len.push(cell.text.len() as u32);
        columns.foreground.push(encode_color(cell.foreground));
        columns.background.push(encode_color(cell.background));
        if cell.flags != 0 && !any_flags {
            any_flags = true;
            columns.flags.reserve(count);
            columns.flags.resize(index, 0);
        }
        if any_flags {
            columns.flags.push(u32::from(cell.flags));
        }
        let hyperlink = cell
            .hyperlink
            .as_ref()
            .and_then(|hyperlink| hyperlinks.intern(hyperlink))
            .map_or(0, |index| u32::from(index) + 1);
        if hyperlink != 0 && !any_hyperlink {
            any_hyperlink = true;
            columns.hyperlink.reserve(count);
            columns.hyperlink.resize(index, 0);
        }
        if any_hyperlink {
            columns.hyperlink.push(hyperlink);
        }
    }
    columns
}

/// Reads cells back out of their columns one at a time, checking every column against the
/// cell count, every text slice against character boundaries and the per-cell byte limit,
/// and every hyperlink against the frame's table. Nothing here can panic on peer input.
struct CellReader<'a> {
    columns: &'a pb::TerminalCells,
    hyperlinks: &'a [SmolStr],
    index: usize,
    offset: usize,
}

impl<'a> CellReader<'a> {
    fn new(
        columns: &'a pb::TerminalCells,
        hyperlinks: &'a [SmolStr],
        count: usize,
    ) -> WireResult<Self> {
        let optional = |column: &[u32]| column.is_empty() || column.len() == count;
        if columns.text_len.len() != count
            || columns.foreground.len() != count
            || columns.background.len() != count
            || !optional(&columns.flags)
            || !optional(&columns.hyperlink)
        {
            return Err(malformed(
                "terminal cell columns disagree on the cell count",
            ));
        }
        Ok(Self {
            columns,
            hyperlinks,
            index: 0,
            offset: 0,
        })
    }

    fn next_cell(&mut self) -> WireResult<TerminalCell> {
        let index = self.index;
        let length = self.columns.text_len[index] as usize;
        if length > MAX_TERMINAL_CELL_TEXT_BYTES {
            return Err(malformed("terminal cell text exceeds the byte limit"));
        }
        let end = self.offset + length;
        let text = self.columns.text.get(self.offset..end).ok_or_else(|| {
            malformed("terminal cell text does not split on character boundaries")
        })?;
        let hyperlink = match self.columns.hyperlink.get(index).copied().unwrap_or(0) {
            0 => None,
            link => Some(
                self.hyperlinks
                    .get(link as usize - 1)
                    .cloned()
                    .ok_or_else(|| {
                        malformed("terminal cell references an invalid hyperlink index")
                    })?,
            ),
        };
        let flags = match self.columns.flags.get(index) {
            Some(&flags) => narrow(flags, "terminal cell flags")?,
            None => 0,
        };
        self.index += 1;
        self.offset = end;
        Ok(TerminalCell {
            text: SmolStr::new(text),
            foreground: decode_color(self.columns.foreground[index])?,
            background: decode_color(self.columns.background[index])?,
            flags,
            hyperlink,
        })
    }

    fn cells(&mut self, count: usize) -> WireResult<Vec<TerminalCell>> {
        let mut cells = Vec::with_capacity(count);
        for _ in 0..count {
            cells.push(self.next_cell()?);
        }
        Ok(cells)
    }

    fn finish(self) -> WireResult<()> {
        if self.offset != self.columns.text.len() {
            return Err(malformed("terminal cell text has bytes no cell covers"));
        }
        Ok(())
    }
}

fn decode_hyperlinks(hyperlinks: Vec<String>) -> WireResult<Vec<SmolStr>> {
    if hyperlinks.len() > MAX_TERMINAL_HYPERLINKS {
        return Err(malformed("terminal hyperlink table exceeds u16 indices"));
    }
    let mut bytes = 0usize;
    for hyperlink in &hyperlinks {
        if hyperlink.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES {
            return Err(malformed("terminal hyperlink URI exceeds the byte limit"));
        }
        bytes += hyperlink.len();
        if bytes > MAX_TERMINAL_HYPERLINK_BYTES {
            return Err(malformed("terminal hyperlink table exceeds the byte limit"));
        }
    }
    Ok(hyperlinks.into_iter().map(SmolStr::from).collect())
}

fn encode_size(size: TerminalSize) -> pb::TerminalSize {
    pb::TerminalSize {
        rows: size.rows.into(),
        columns: size.columns.into(),
        cell_width: size.cell_width.into(),
        cell_height: size.cell_height.into(),
    }
}

pub(crate) fn decode_size(size: pb::TerminalSize) -> WireResult<TerminalSize> {
    Ok(TerminalSize {
        rows: narrow(size.rows, "terminal rows")?,
        columns: narrow(size.columns, "terminal columns")?,
        cell_width: narrow(size.cell_width, "cell width")?,
        cell_height: narrow(size.cell_height, "cell height")?,
    })
}

fn encode_mouse_tracking(tracking: TerminalMouseTracking) -> i32 {
    (match tracking {
        TerminalMouseTracking::None => pb::TerminalMouseTracking::None,
        TerminalMouseTracking::Click => pb::TerminalMouseTracking::Click,
        TerminalMouseTracking::Drag => pb::TerminalMouseTracking::Drag,
        TerminalMouseTracking::Motion => pb::TerminalMouseTracking::Motion,
    }) as i32
}

fn decode_mouse_tracking(raw: i32) -> WireResult<TerminalMouseTracking> {
    use pb::TerminalMouseTracking as Wire;
    Ok(match enum_or(raw, "mouse tracking", Wire::None)? {
        Wire::Unspecified => unreachable!("enum_or rejects 0"),
        Wire::None => TerminalMouseTracking::None,
        Wire::Click => TerminalMouseTracking::Click,
        Wire::Drag => TerminalMouseTracking::Drag,
        Wire::Motion => TerminalMouseTracking::Motion,
    })
}

fn encode_cursor(cursor: TerminalCursor) -> pb::TerminalCursor {
    use pb::TerminalCursorShape as Wire;
    pb::TerminalCursor {
        row: cursor.row.into(),
        column: cursor.column.into(),
        shape: match cursor.shape {
            TerminalCursorShape::Block => Wire::Block,
            TerminalCursorShape::Underline => Wire::Underline,
            TerminalCursorShape::Beam => Wire::Beam,
            TerminalCursorShape::HollowBlock => Wire::HollowBlock,
            TerminalCursorShape::Hidden => Wire::Hidden,
        } as i32,
        blinking: cursor.blinking,
    }
}

fn decode_cursor(cursor: pb::TerminalCursor) -> WireResult<TerminalCursor> {
    use pb::TerminalCursorShape as Wire;
    Ok(TerminalCursor {
        row: narrow(cursor.row, "cursor row")?,
        column: narrow(cursor.column, "cursor column")?,
        shape: match enum_or(cursor.shape, "cursor shape", Wire::Block)? {
            Wire::Unspecified => unreachable!("enum_or rejects 0"),
            Wire::Block => TerminalCursorShape::Block,
            Wire::Underline => TerminalCursorShape::Underline,
            Wire::Beam => TerminalCursorShape::Beam,
            Wire::HollowBlock => TerminalCursorShape::HollowBlock,
            Wire::Hidden => TerminalCursorShape::Hidden,
        },
        blinking: cursor.blinking,
    })
}

pub(crate) fn encode_position(position: TerminalPosition) -> pb::TerminalPosition {
    pb::TerminalPosition {
        row: position.row.into(),
        column: position.column.into(),
        side: match position.side {
            TerminalSide::Left => pb::TerminalSide::Left,
            TerminalSide::Right => pb::TerminalSide::Right,
        } as i32,
    }
}

pub(crate) fn decode_position(position: pb::TerminalPosition) -> WireResult<TerminalPosition> {
    Ok(TerminalPosition {
        row: narrow(position.row, "selection row")?,
        column: narrow(position.column, "selection column")?,
        side: match enum_or(position.side, "selection side", pb::TerminalSide::Left)? {
            pb::TerminalSide::Unspecified => unreachable!("enum_or rejects 0"),
            pb::TerminalSide::Left => TerminalSide::Left,
            pb::TerminalSide::Right => TerminalSide::Right,
        },
    })
}

pub(crate) fn encode_selection(selection: TerminalSelection) -> pb::TerminalSelection {
    pb::TerminalSelection {
        start: Some(encode_position(selection.start)),
        end: Some(encode_position(selection.end)),
        display_offset: selection.display_offset,
    }
}

pub(crate) fn decode_selection(selection: pb::TerminalSelection) -> WireResult<TerminalSelection> {
    Ok(TerminalSelection {
        start: decode_position(required(selection.start, "selection start")?)?,
        end: decode_position(required(selection.end, "selection end")?)?,
        display_offset: selection.display_offset,
    })
}

impl From<&TerminalView> for pb::TerminalView {
    fn from(view: &TerminalView) -> Self {
        let mut hyperlinks = WireHyperlinks::default();
        let cells = encode_cells(view.cells.iter(), &mut hyperlinks);
        Self {
            revision: view.revision,
            size: Some(encode_size(view.size)),
            display_offset: view.display_offset,
            mouse_tracking: encode_mouse_tracking(view.mouse_tracking),
            hyperlinks: hyperlinks.values.into_iter().map(String::from).collect(),
            cells: Some(cells),
            cursor: view.cursor.map(encode_cursor),
            selection: view.selection.map(encode_selection),
        }
    }
}

impl TryFrom<pb::TerminalView> for TerminalView {
    type Error = WireError;

    fn try_from(view: pb::TerminalView) -> WireResult<Self> {
        let size = decode_size(required(view.size, "terminal size")?)?;
        let count = usize::from(size.rows) * usize::from(size.columns);
        if count > MAX_TERMINAL_CELLS {
            return Err(malformed("terminal grid exceeds the maximum cell count"));
        }
        let hyperlinks = decode_hyperlinks(view.hyperlinks)?;
        let columns = required(view.cells, "terminal cells")?;
        let mut reader = CellReader::new(&columns, &hyperlinks, count)?;
        let cells = reader.cells(count)?;
        reader.finish()?;
        Ok(Self {
            revision: view.revision,
            size,
            display_offset: view.display_offset,
            mouse_tracking: decode_mouse_tracking(view.mouse_tracking)?,
            cells,
            cursor: view.cursor.map(decode_cursor).transpose()?,
            selection: view.selection.map(decode_selection).transpose()?,
        })
    }
}

impl From<&TerminalViewDelta> for pb::TerminalViewDelta {
    fn from(delta: &TerminalViewDelta) -> Self {
        let mut hyperlinks = WireHyperlinks::default();
        let count = delta.runs.iter().map(|run| run.cells.len()).sum();
        let cells = encode_cells(
            ExactCells {
                cells: delta.runs.iter().flat_map(|run| &run.cells),
                remaining: count,
            },
            &mut hyperlinks,
        );
        Self {
            base_revision: delta.base_revision,
            revision: delta.revision,
            display_offset: delta.display_offset,
            mouse_tracking: encode_mouse_tracking(delta.mouse_tracking),
            cursor: delta.cursor.map(encode_cursor),
            selection: delta.selection.map(encode_selection),
            hyperlinks: hyperlinks.values.into_iter().map(String::from).collect(),
            run_start: delta.runs.iter().map(|run| run.start).collect(),
            run_len: delta
                .runs
                .iter()
                .map(|run| run.cells.len() as u32)
                .collect(),
            cells: Some(cells),
        }
    }
}

/// The cells of every run in order, with the total count `encode_cells` sizes by.
struct ExactCells<I> {
    cells: I,
    remaining: usize,
}

impl<'a, I: Iterator<Item = &'a TerminalCell>> Iterator for ExactCells<I> {
    type Item = &'a TerminalCell;

    fn next(&mut self) -> Option<Self::Item> {
        let cell = self.cells.next()?;
        self.remaining -= 1;
        Some(cell)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, I: Iterator<Item = &'a TerminalCell>> ExactSizeIterator for ExactCells<I> {}

impl TryFrom<pb::TerminalViewDelta> for TerminalViewDelta {
    type Error = WireError;

    /// Checks what can be checked without the view: run and cell counts, and each run's
    /// cells. Order, overlap and bounds against the view are `validate_delta`'s.
    fn try_from(delta: pb::TerminalViewDelta) -> WireResult<Self> {
        if delta.run_start.len() != delta.run_len.len() {
            return Err(malformed("terminal delta run columns disagree"));
        }
        let mut count = 0usize;
        for &length in &delta.run_len {
            count += length as usize;
            if count > MAX_TERMINAL_CELLS {
                return Err(malformed("terminal delta exceeds the maximum cell count"));
            }
        }
        let hyperlinks = decode_hyperlinks(delta.hyperlinks)?;
        let columns = required(delta.cells, "terminal cells")?;
        let mut reader = CellReader::new(&columns, &hyperlinks, count)?;
        let runs = delta
            .run_start
            .iter()
            .zip(&delta.run_len)
            .map(|(&start, &length)| {
                Ok(TerminalCellRun {
                    start,
                    cells: reader.cells(length as usize)?,
                })
            })
            .collect::<WireResult<_>>()?;
        reader.finish()?;
        Ok(Self {
            base_revision: delta.base_revision,
            revision: delta.revision,
            display_offset: delta.display_offset,
            mouse_tracking: decode_mouse_tracking(delta.mouse_tracking)?,
            cursor: delta.cursor.map(decode_cursor).transpose()?,
            selection: delta.selection.map(decode_selection).transpose()?,
            runs,
        })
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
}
