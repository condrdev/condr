use super::*;

#[derive(Clone)]
pub struct TerminalViewSource {
    pub(super) terminal: Arc<Mutex<Terminal>>,
    pub(super) size: Arc<Mutex<TerminalSize>>,
    pub(super) revision: Arc<AtomicU64>,
    pub(super) damage_baseline: Arc<Mutex<Option<TerminalDamageBaseline>>>,
    pub(super) cursor_settle: Arc<Mutex<super::cursor_settle::CursorSettle>>,
}

/// The frame last taken, which the next one is compared with cell by cell.
pub(super) type TerminalDamageBaseline = TerminalView;

/// How often an idle Pane's monitor looks whether its scrollback can be compressed.
pub const TERMINAL_IDLE_CHECK: Duration = Duration::from_secs(10);

impl TerminalViewSource {
    pub fn view(&self) -> TerminalView {
        let Ok(mut terminal) = lock_pane(&self.terminal) else {
            return blank_view(pane_size(&self.size), self.revision.load(Ordering::Acquire));
        };
        // Read under the terminal lock, which a resize holds while it sets both.
        let size = pane_size(&self.size);
        let mut view = terminal.view(size, self.revision.load(Ordering::Acquire), None);
        drop(terminal);
        view.cursor = self.settled_cursor(view.cursor);
        view
    }

    /// When the monitor must look again without new output: a held cursor move or an
    /// open synchronized update times out then.
    pub fn frame_wakeup(&self) -> Option<Duration> {
        let hold = lock_pane(&self.terminal).ok()?.hold_remaining();
        let settle = self
            .cursor_settle_pending()
            .then_some(CURSOR_POSITION_SETTLE);
        hold.into_iter().chain(settle).min()
    }

    /// Compresses the Pane's scrollback once output has stopped since the last look.
    pub fn compress_if_idle(&self) {
        if let Ok(mut terminal) = lock_pane(&self.terminal) {
            terminal.compress_if_idle();
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// A cursor move is being held back; `take_frame` after [`CURSOR_POSITION_SETTLE`]
    /// publishes it once it has stayed put.
    pub fn cursor_settle_pending(&self) -> bool {
        CURSOR_POSITION_SETTLE_ENABLED
            && lock_pane(&self.cursor_settle).is_ok_and(|settle| settle.pending())
    }

    pub(super) fn settled_cursor(&self, cursor: Option<TerminalCursor>) -> Option<TerminalCursor> {
        if !CURSOR_POSITION_SETTLE_ENABLED {
            return cursor;
        }
        let now = Instant::now();
        let Ok(mut settle) = lock_pane(&self.cursor_settle) else {
            return cursor;
        };
        settle.observe(cursor, now);
        settle.reported(cursor, now)
    }

    pub fn take_frame(&self) -> Option<TerminalViewFrame> {
        let mut terminal = lock_pane(&self.terminal).ok()?;
        // A synchronized update shows nothing until it ends or times out (ADR 0051).
        if terminal.hold_remaining().is_some() {
            return None;
        }
        let size = pane_size(&self.size);
        let mut revision = self.revision.load(Ordering::Acquire);
        let mut baseline = lock_pane(&self.damage_baseline).ok()?;
        // Only the rows the VT marks changed are read again; the rest come from the frame
        // last taken, and the comparison below finds the cells that really changed.
        let mut view = terminal.view(size, revision, baseline.as_ref());
        drop(terminal);
        view.cursor = self.settled_cursor(view.cursor);
        if baseline
            .as_ref()
            .is_some_and(|baseline| revision <= baseline.revision && baseline.cursor != view.cursor)
        {
            // Nothing new arrived from the PTY, but the settled cursor moved: give the
            // cursor-only frame its own revision so clients accept it.
            revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
            view.revision = revision;
        }
        let frame = if baseline
            .as_ref()
            .is_some_and(|baseline| revision <= baseline.revision)
        {
            None
        } else {
            TerminalView::frame_from(baseline.as_ref(), &view)
        };
        *baseline = Some(view);
        frame
    }
}

/// What a Pane whose terminal a panic ended shows: a blank grid at its last size
/// (ADR 0053).
fn blank_view(size: TerminalSize, revision: u64) -> TerminalView {
    TerminalView {
        revision,
        size,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cells: vec![super::view::blank_cell(); usize::from(size.columns) * usize::from(size.rows)],
        cursor: None,
        selection: None,
    }
}
