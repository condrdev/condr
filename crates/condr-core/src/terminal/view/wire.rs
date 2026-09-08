use super::*;

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
