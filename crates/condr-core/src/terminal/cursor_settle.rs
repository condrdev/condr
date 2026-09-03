//! Holds the reported cursor position steady while a program repaints.
//!
//! ConPTY re-renders its own frame on top of the program's output and splits it across
//! reads, so a snapshot taken mid-frame sees the cursor wherever the paint pass left it.
//! Ported from herdr's `CursorPositionSettleState` (Apache-2.0, `src/pane/cursor.rs`): a
//! new position is only reported once it has held for [`CURSOR_POSITION_SETTLE`], and a
//! cursor that keeps moving is accepted after [`CURSOR_POSITION_MAX_HOLD`] so a busy
//! program still tracks. Hiding is trusted immediately; revealing waits like a move.

use std::time::{Duration, Instant};

use super::view::{TerminalCursor, TerminalCursorShape};

pub const CURSOR_POSITION_SETTLE: Duration = Duration::from_millis(20);
const CURSOR_POSITION_MAX_HOLD: Duration = Duration::from_millis(100);
/// Only ConPTY splits and re-renders frames this way; Unix PTYs pass output through.
pub(super) const CURSOR_POSITION_SETTLE_ENABLED: bool = cfg!(windows);

#[derive(Debug, Default)]
pub(super) struct CursorSettle {
    settled: Option<TerminalCursor>,
    candidate: Option<TerminalCursor>,
    pending_since: Option<Instant>,
}

impl CursorSettle {
    pub(super) fn observe(&mut self, current: Option<TerminalCursor>, now: Instant) {
        let Some(current) = current else {
            self.reset(None);
            return;
        };
        if !visible(current) {
            self.reset(Some(current));
            return;
        }
        let Some(settled) = self.settled else {
            self.reset(Some(current));
            return;
        };
        if same_position(settled, current) && visible(settled) {
            self.reset(Some(current));
            return;
        }
        let Some(candidate) = self.candidate else {
            self.candidate = Some(current);
            self.pending_since = Some(now);
            return;
        };
        // The hold is measured from the first unsettled observation, so a cursor that
        // never stops moving is still adopted within the cap.
        let held = now.duration_since(self.pending_since.unwrap_or(now));
        if held >= CURSOR_POSITION_MAX_HOLD
            || (same_position(candidate, current) && held >= CURSOR_POSITION_SETTLE)
        {
            self.reset(Some(current));
        } else {
            self.candidate = Some(current);
        }
    }

    fn reset(&mut self, settled: Option<TerminalCursor>) {
        self.settled = settled;
        self.candidate = None;
        self.pending_since = None;
    }

    /// The cursor to publish for `current`: its shape and blink, at the settled position.
    pub(super) fn reported(
        &self,
        current: Option<TerminalCursor>,
        now: Instant,
    ) -> Option<TerminalCursor> {
        let current = current?;
        let Some(candidate) = self.candidate else {
            return Some(current);
        };
        if now.duration_since(self.pending_since.unwrap_or(now)) >= CURSOR_POSITION_SETTLE {
            return Some(merge(candidate, current));
        }
        Some(match self.settled {
            Some(settled) => merge(settled, current),
            None => TerminalCursor {
                shape: TerminalCursorShape::Hidden,
                ..candidate
            },
        })
    }

    pub(super) fn pending(&self) -> bool {
        self.candidate.is_some()
    }
}

fn visible(cursor: TerminalCursor) -> bool {
    cursor.shape != TerminalCursorShape::Hidden
}

fn same_position(left: TerminalCursor, right: TerminalCursor) -> bool {
    left.row == right.row && left.column == right.column
}

/// `position` says where; `current` says how it looks. Hidden on either side hides it.
fn merge(position: TerminalCursor, current: TerminalCursor) -> TerminalCursor {
    TerminalCursor {
        row: position.row,
        column: position.column,
        shape: if visible(position) {
            current.shape
        } else {
            TerminalCursorShape::Hidden
        },
        blinking: current.blinking,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(row: u16, column: u16, shape: TerminalCursorShape) -> Option<TerminalCursor> {
        Some(TerminalCursor {
            row,
            column,
            shape,
            blinking: false,
        })
    }

    fn position(cursor: Option<TerminalCursor>) -> (u16, u16) {
        let cursor = cursor.unwrap();
        (cursor.row, cursor.column)
    }

    const BLOCK: TerminalCursorShape = TerminalCursorShape::Block;
    const BEAM: TerminalCursorShape = TerminalCursorShape::Beam;
    const HIDDEN: TerminalCursorShape = TerminalCursorShape::Hidden;
    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn holds_a_position_change_until_the_quiet_window_passes() {
        let now = Instant::now();
        let mut settle = CursorSettle::default();
        settle.observe(cursor(0, 1, BLOCK), now);
        settle.observe(cursor(5, 20, BLOCK), now + MS);

        assert_eq!(
            position(settle.reported(cursor(5, 20, BLOCK), now + 2 * MS)),
            (0, 1)
        );
        assert_eq!(
            position(settle.reported(cursor(5, 20, BLOCK), now + CURSOR_POSITION_SETTLE + MS)),
            (5, 20)
        );
        assert!(settle.pending(), "reading must not mutate the settle state");
    }

    #[test]
    fn caps_continuous_movement_from_the_first_pending_observation() {
        let now = Instant::now();
        let mut settle = CursorSettle::default();
        settle.observe(cursor(0, 1, BLOCK), now);
        settle.observe(cursor(0, 2, BLOCK), now + MS);
        settle.observe(cursor(0, 3, BLOCK), now + CURSOR_POSITION_MAX_HOLD + MS);

        assert!(!settle.pending());
        assert_eq!(
            settle.reported(cursor(0, 3, BLOCK), now + CURSOR_POSITION_MAX_HOLD + 2 * MS),
            cursor(0, 3, BLOCK)
        );
    }

    #[test]
    fn shape_and_blink_follow_the_current_cursor_while_the_position_is_held() {
        let now = Instant::now();
        let mut settle = CursorSettle::default();
        settle.observe(cursor(0, 1, BLOCK), now);
        settle.observe(cursor(0, 2, BEAM), now + MS);

        let held = settle.reported(cursor(0, 2, BEAM), now + 2 * MS).unwrap();
        assert_eq!((held.row, held.column, held.shape), (0, 1, BEAM));
    }

    #[test]
    fn hides_immediately_and_waits_to_reveal() {
        let now = Instant::now();
        let mut settle = CursorSettle::default();
        settle.observe(cursor(0, 1, BLOCK), now);
        settle.observe(cursor(0, 1, HIDDEN), now + MS);
        assert_eq!(
            settle.reported(cursor(0, 1, HIDDEN), now + 2 * MS),
            cursor(0, 1, HIDDEN)
        );

        settle.observe(cursor(0, 1, BLOCK), now + 3 * MS);
        assert_eq!(
            settle.reported(cursor(0, 1, BLOCK), now + 4 * MS),
            cursor(0, 1, HIDDEN)
        );
        settle.observe(cursor(0, 1, BLOCK), now + 3 * MS + CURSOR_POSITION_SETTLE);
        assert_eq!(
            settle.reported(cursor(0, 1, BLOCK), now + 4 * MS + CURSOR_POSITION_SETTLE),
            cursor(0, 1, BLOCK)
        );
    }

    #[test]
    fn a_missing_cursor_clears_the_history() {
        let now = Instant::now();
        let mut settle = CursorSettle::default();
        settle.observe(cursor(0, 1, BLOCK), now);
        settle.observe(cursor(0, 2, BLOCK), now + MS);
        settle.observe(None, now + 2 * MS);

        assert!(!settle.pending());
        assert_eq!(settle.reported(None, now + 3 * MS), None);
        assert_eq!(
            settle.reported(cursor(0, 7, BLOCK), now + 3 * MS),
            cursor(0, 7, BLOCK)
        );
    }
}
