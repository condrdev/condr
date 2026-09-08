use super::view::DetectedLinks;
use super::*;

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
    revision: u64,
    size: TerminalSize,
    display_offset: u32,
    /// The cursor as published, after settling; a settle alone must still emit a frame.
    cursor: Option<TerminalCursor>,
    /// The plain-text URLs as published; a cell whose link changed is damage alacritty
    /// cannot know about, since the text it sits in may be untouched.
    links: DetectedLinks,
}

impl TerminalViewSource {
    pub fn view(&self) -> TerminalView {
        let terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let size = *self.size.lock().expect("terminal size lock poisoned");
        let mut view = snapshot_terminal(&terminal, size, self.revision.load(Ordering::Acquire));
        view.cursor = self.settled_cursor(view.cursor);
        view
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// A cursor move is being held back; `take_frame` after [`CURSOR_POSITION_SETTLE`]
    /// publishes it once it has stayed put.
    pub fn cursor_settle_pending(&self) -> bool {
        CURSOR_POSITION_SETTLE_ENABLED
            && self
                .cursor_settle
                .lock()
                .expect("cursor settle lock poisoned")
                .pending()
    }

    pub(super) fn settled_cursor(&self, cursor: Option<TerminalCursor>) -> Option<TerminalCursor> {
        if !CURSOR_POSITION_SETTLE_ENABLED {
            return cursor;
        }
        let now = Instant::now();
        let mut settle = self
            .cursor_settle
            .lock()
            .expect("cursor settle lock poisoned");
        settle.observe(cursor, now);
        settle.reported(cursor, now)
    }

    pub fn take_frame(&self) -> Option<TerminalViewFrame> {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let size = *self.size.lock().expect("terminal size lock poisoned");
        let mut revision = self.revision.load(Ordering::Acquire);
        let damage = match terminal.damage() {
            TermDamage::Full => None,
            TermDamage::Partial(lines) => Some(lines.collect::<Vec<_>>()),
        };
        let mut baseline = self
            .damage_baseline
            .lock()
            .expect("terminal damage baseline lock poisoned");
        let content = terminal.renderable_content();
        let display_offset = u32::try_from(content.display_offset).unwrap_or(u32::MAX);
        let mouse_tracking = TerminalMouseTracking::from_term_mode(*terminal.mode());
        let cursor = terminal_cursor(
            content.cursor.point,
            content.cursor.shape,
            terminal.cursor_style().blinking,
            content.display_offset,
            size,
        );
        let cursor = self.settled_cursor(cursor);
        if baseline
            .as_ref()
            .is_some_and(|baseline| revision <= baseline.revision && baseline.cursor != cursor)
        {
            // Nothing new arrived from the PTY, but the settled cursor moved: give the
            // cursor-only frame its own revision so clients accept it.
            revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        }
        let links = detect_links(&terminal, size);
        let full = || {
            let mut view = snapshot_terminal_with_links(&terminal, size, revision, &links);
            view.cursor = cursor;
            TerminalViewFrame::Full(view)
        };
        let requires_full = baseline.as_ref().is_none_or(|baseline| {
            baseline.size != size || baseline.display_offset != display_offset
        }) || damage.is_none();

        let frame = if baseline
            .as_ref()
            .is_some_and(|baseline| revision <= baseline.revision)
        {
            None
        } else if requires_full {
            Some(full())
        } else {
            let previous = baseline.as_ref().expect("partial damage has a baseline");
            let columns = usize::from(size.columns);
            // Damaged column spans per viewport row: alacritty's, widened by every cell whose
            // detected link changed, which happens when text elsewhere on its line did.
            let mut damaged_rows = std::collections::BTreeMap::new();
            for bounds in damage.expect("full damage was handled") {
                if bounds.line >= usize::from(size.rows) || bounds.left >= columns {
                    continue;
                }
                let right = bounds.right.min(columns - 1);
                if bounds.left <= right {
                    damaged_rows.insert(bounds.line, (bounds.left, right));
                }
            }
            let link_changed = |index: &u32| previous.links.get(index) != links.get(index);
            for index in previous
                .links
                .keys()
                .chain(links.keys())
                .filter(|i| link_changed(i))
            {
                let (row, column) = (*index as usize / columns, *index as usize % columns);
                damaged_rows
                    .entry(row)
                    .and_modify(|(left, right)| {
                        *left = (*left).min(column);
                        *right = (*right).max(column);
                    })
                    .or_insert((column, column));
            }
            let mut runs = Vec::new();
            let mut changed_cells = 0usize;
            let mut hyperlinks = SnapshotHyperlinks::default();
            for (row, (left, right)) in damaged_rows {
                let mut cells = Vec::with_capacity(right - left + 1);
                let line = i32::try_from(row).unwrap_or(i32::MAX)
                    - i32::try_from(content.display_offset).unwrap_or(i32::MAX);
                for column in left..=right {
                    let index = row * columns + column;
                    cells.push(terminal_cell(
                        &terminal.grid()[Point::new(Line(line), Column(column))],
                        content.colors,
                        &mut hyperlinks,
                        links.get(&(index as u32)),
                    ));
                }
                changed_cells += cells.len();
                runs.push(TerminalCellRun {
                    start: u32::try_from(row * columns + left)
                        .expect("terminal cell count fits u32"),
                    cells,
                });
            }
            if changed_cells > usize::from(size.rows) * columns / 2 {
                Some(full())
            } else {
                Some(TerminalViewFrame::Delta(TerminalViewDelta {
                    base_revision: previous.revision,
                    revision,
                    display_offset,
                    mouse_tracking,
                    cursor,
                    selection: viewport_selection(&terminal, size),
                    runs,
                }))
            }
        };

        *baseline = Some(TerminalDamageBaseline {
            revision,
            size,
            display_offset,
            cursor,
            links,
        });
        drop(baseline);
        terminal.reset_damage();
        frame
    }
}
