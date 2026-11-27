//! Kit's scrollbar over a Pane's terminal (ADR 0077).

use condr_core::TerminalView;
use gpui_kit::component::scroll::ScrollbarHandle;
use gpui_kit::{Bounds, Pixels, Point, Size, point, px, size};
use std::cell::RefCell;
use std::rc::Rc;

/// The scrollback as Kit's scrollbar sees it: every row of history and screen at the
/// grid's cell height, with the viewport's top row as the offset. The Server owns the
/// scroll position (ADR 0036), so a drag only asks for a row; the thumb holds where the
/// pointer left it until a frame shows the viewport moved.
#[derive(Clone)]
pub(crate) struct TerminalScrollbar(Rc<RefCell<ScrollbarState>>);

struct ScrollbarState {
    /// The Pane's padding right of the grid, which the bar covers to sit at its edge.
    right_inset: Pixels,
    /// The grid's rows, widened over the right inset.
    viewport: Bounds<Pixels>,
    cell_height: Pixels,
    history_rows: u32,
    top_row: u32,
    held: Option<Pixels>,
    dragging: bool,
    /// The row a drag or a click on the track asked for, not yet sent.
    requested: Option<u32>,
}

impl TerminalScrollbar {
    pub(crate) fn new(right_inset: Pixels) -> Self {
        Self(Rc::new(RefCell::new(ScrollbarState {
            right_inset,
            viewport: Bounds::default(),
            cell_height: px(0.),
            history_rows: 0,
            top_row: 0,
            held: None,
            dragging: false,
            requested: None,
        })))
    }

    /// Called from the terminal element's prepaint, which runs before the scrollbar's.
    pub(crate) fn update(&self, grid: Bounds<Pixels>, cell_height: Pixels, view: &TerminalView) {
        let mut state = self.0.borrow_mut();
        state.viewport = Bounds::new(
            grid.origin,
            size(
                grid.size.width + state.right_inset,
                cell_height * f32::from(view.size.rows),
            ),
        );
        state.cell_height = cell_height;
        let top_row = view.history_rows.saturating_sub(view.display_offset);
        if !state.dragging && (state.top_row, state.history_rows) != (top_row, view.history_rows) {
            state.held = None;
        }
        state.top_row = top_row;
        state.history_rows = view.history_rows;
    }

    /// A drag holds the bar, wherever the pointer went.
    pub(crate) fn dragging(&self) -> bool {
        self.0.borrow().dragging
    }

    pub(crate) fn take_requested_row(&self) -> Option<u32> {
        self.0.borrow_mut().requested.take()
    }
}

impl ScrollbarHandle for TerminalScrollbar {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().viewport
    }

    fn offset(&self) -> Point<Pixels> {
        let state = self.0.borrow();
        let y = state
            .held
            .unwrap_or(-(state.cell_height * state.top_row as f32));
        point(px(0.), y)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let mut state = self.0.borrow_mut();
        if state.cell_height <= px(0.) {
            return;
        }
        let bottom = state.cell_height * state.history_rows as f32;
        let y = offset.y.clamp(-bottom, px(0.));
        state.held = Some(y);
        state.requested = Some((-y / state.cell_height).round() as u32);
    }

    fn content_size(&self) -> Size<Pixels> {
        let state = self.0.borrow();
        let history = state.cell_height * state.history_rows as f32;
        size(
            state.viewport.size.width,
            state.viewport.size.height + history,
        )
    }

    fn start_drag(&self) {
        self.0.borrow_mut().dragging = true;
    }

    fn end_drag(&self) {
        self.0.borrow_mut().dragging = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use condr_core::TerminalSize;

    fn view(history_rows: u32, display_offset: u32) -> TerminalView {
        TerminalView {
            revision: 1,
            size: TerminalSize {
                rows: 10,
                columns: 20,
                cell_width: 8,
                cell_height: 20,
            },
            display_offset,
            history_rows,
            mouse_tracking: Default::default(),
            search: Default::default(),
            cells: Vec::new(),
            cursor: None,
            selection: None,
            images: Vec::new(),
        }
    }

    fn grid() -> Bounds<Pixels> {
        Bounds::new(point(px(5.), px(5.)), size(px(160.), px(205.)))
    }

    #[test]
    fn rows_map_to_pixels_and_a_drag_holds_until_the_viewport_moves() {
        let bar = TerminalScrollbar::new(px(5.));
        bar.update(grid(), px(20.), &view(90, 30));
        assert_eq!(bar.viewport_bounds().size, size(px(165.), px(200.)));
        assert_eq!(bar.content_size().height, px(2000.));
        assert_eq!(bar.offset().y, px(-1200.));

        bar.start_drag();
        assert!(bar.dragging());
        bar.set_offset(point(px(0.), px(-409.)));
        assert_eq!(bar.take_requested_row(), Some(20));
        assert_eq!(bar.take_requested_row(), None);
        // A frame from before the drag reached the Server does not snap the thumb back.
        bar.update(grid(), px(20.), &view(91, 30));
        assert_eq!(bar.offset().y, px(-409.));
        bar.end_drag();
        assert!(!bar.dragging());
        bar.update(grid(), px(20.), &view(91, 71));
        assert_eq!(bar.offset().y, px(-400.));

        // Past either end asks for the end.
        bar.set_offset(point(px(0.), px(-5000.)));
        assert_eq!(bar.take_requested_row(), Some(91));
        bar.set_offset(point(px(0.), px(30.)));
        assert_eq!(bar.take_requested_row(), Some(0));
    }
}
