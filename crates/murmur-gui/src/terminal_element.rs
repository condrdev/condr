use std::ops::Range;

use gpui::{
    App, BorderStyle, Bounds, ClipboardItem, ContentMask, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InputHandler, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    ShapedLine, Size, StrikethroughStyle, Style, TextAlign, TextInputConfiguration, TextRun,
    UTF16Selection, UnderlineStyle, Window, fill, outline, point, px, relative, rgb, size,
};
use gpui_component::ActiveTheme as _;
use murmur_core::{
    PaneId, TerminalColor, TerminalCursorShape, TerminalPosition, TerminalSide, TerminalSize,
    TerminalView,
};

use crate::{ConnectionKey, Murmur};

const INVERSE: u16 = 1 << 0;
const BOLD: u16 = 1 << 1;
const ITALIC: u16 = 1 << 2;
const UNDERLINE: u16 = 1 << 3;
const WIDE_CHAR_SPACER: u16 = 1 << 6;
const DIM: u16 = 1 << 7;
const HIDDEN: u16 = 1 << 8;
const STRIKEOUT: u16 = 1 << 9;
const LEADING_WIDE_CHAR_SPACER: u16 = 1 << 10;
const ALL_UNDERLINES: u16 = 0b0111_1000_0000_1000;

pub(crate) struct TerminalElement {
    view: Entity<Murmur>,
    focus_handle: FocusHandle,
    connection_key: ConnectionKey,
    pane_id: PaneId,
    terminal: TerminalView,
    marked_text: Option<String>,
}

// Terminals accept IME text, but printable chords must reach keybindings first.
struct TerminalInputHandler {
    inner: ElementInputHandler<Murmur>,
}

impl TerminalInputHandler {
    fn new(bounds: Bounds<Pixels>, view: Entity<Murmur>) -> Self {
        Self {
            inner: ElementInputHandler::new(bounds, view),
        }
    }
}

impl InputHandler for TerminalInputHandler {
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        self.inner
            .selected_text_range(ignore_disabled_input, window, cx)
    }

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.inner.marked_text_range(window, cx)
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        self.inner.text_for_range(range, adjusted_range, window, cx)
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.replace_text_in_range(range, text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .replace_and_mark_text_in_range(range, new_text, new_selected_range, window, cx);
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.inner.unmark_text(window, cx);
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut App) {
        self.inner.paste(item, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        self.inner.bounds_for_range(range, window, cx)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        self.inner.character_index_for_point(point, window, cx)
    }

    fn element_bounds(&mut self, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        self.inner.element_bounds(window, cx)
    }

    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.inner.accepts_text_input(window, cx)
    }

    fn prefers_ime_for_printable_keys(&mut self, _: &mut Window, _: &mut App) -> bool {
        false
    }

    fn text_input_configuration(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> TextInputConfiguration {
        self.inner.text_input_configuration(window, cx)
    }
}

struct ShapedCell {
    origin: Point<Pixels>,
    line: ShapedLine,
}

pub(crate) struct PrepaintState {
    hitbox: Hitbox,
    quads: Vec<PaintQuad>,
    cells: Vec<ShapedCell>,
    cell_size: Size<Pixels>,
}

impl TerminalElement {
    pub(crate) fn new(
        view: Entity<Murmur>,
        focus_handle: FocusHandle,
        connection_key: ConnectionKey,
        pane_id: PaneId,
        terminal: TerminalView,
        marked_text: Option<String>,
    ) -> Self {
        Self {
            view,
            focus_handle,
            connection_key,
            pane_id,
            terminal,
            marked_text,
        }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = style.line_height_in_pixels(window.rem_size()).max(px(1.));
        let measure = window.text_system().shape_line(
            "M".into(),
            font_size,
            &[TextRun {
                len: 1,
                font: style.font(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let cell_size = size(measure.width().max(px(1.)), line_height);
        let rows = ((bounds.size.height / cell_size.height).floor() as u16).max(1);
        let columns = ((bounds.size.width / cell_size.width).floor() as u16).max(1);
        let terminal_size = TerminalSize {
            rows,
            columns,
            cell_width: cell_size.width.as_f32().round() as u16,
            cell_height: cell_size.height.as_f32().round() as u16,
        };
        let view = self.view.clone();
        let connection_key = self.connection_key;
        let pane_id = self.pane_id;
        window.defer(cx, move |_, cx| {
            view.update(cx, |view, cx| {
                view.update_terminal_geometry(connection_key, pane_id, bounds, cell_size);
                view.resize_terminal(connection_key, pane_id, terminal_size, cx);
            });
        });

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let mut quads = vec![fill(bounds, cx.theme().background)];
        let mut cells = Vec::new();

        // ponytail: full visible-grid repaint; add damage tracking only after profiling proves it matters.
        for row in 0..self.terminal.size.rows {
            for column in 0..self.terminal.size.columns {
                let Some(cell) = self.terminal.cell(row, column) else {
                    continue;
                };
                let cell_bounds = Bounds::new(
                    point(
                        bounds.left() + cell_size.width * usize::from(column),
                        bounds.top() + cell_size.height * usize::from(row),
                    ),
                    cell_size,
                );
                let mut foreground = terminal_color(cell.foreground, true, cx);
                let mut background = terminal_color(cell.background, false, cx);
                if cell.flags & INVERSE != 0 {
                    std::mem::swap(&mut foreground, &mut background);
                }
                if cell.selected {
                    background = cx.theme().selection;
                }
                if cell.flags & DIM != 0 {
                    foreground = foreground.opacity(0.65);
                }
                if background != cx.theme().background {
                    quads.push(fill(cell_bounds, background));
                }
                if cell.flags & (HIDDEN | WIDE_CHAR_SPACER | LEADING_WIDE_CHAR_SPACER) != 0
                    || cell.text == " "
                {
                    continue;
                }

                let mut font = style.font();
                if cell.flags & BOLD != 0 {
                    font = font.bold();
                }
                if cell.flags & ITALIC != 0 {
                    font = font.italic();
                }
                let underline =
                    (cell.flags & (UNDERLINE | ALL_UNDERLINES) != 0).then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(foreground),
                        wavy: false,
                    });
                let strikethrough = (cell.flags & STRIKEOUT != 0).then_some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(foreground),
                });
                let line = window.text_system().shape_line(
                    cell.text.clone().into(),
                    font_size,
                    &[TextRun {
                        len: cell.text.len(),
                        font,
                        color: foreground,
                        background_color: None,
                        underline,
                        strikethrough,
                    }],
                    None,
                );
                cells.push(ShapedCell {
                    origin: cell_bounds.origin,
                    line,
                });
            }
        }

        if let Some(cursor) = self.terminal.cursor {
            let cursor_bounds = Bounds::new(
                point(
                    bounds.left() + cell_size.width * usize::from(cursor.column),
                    bounds.top() + cell_size.height * usize::from(cursor.row),
                ),
                cell_size,
            );
            let cursor_color = terminal_color(TerminalColor::Named(258), true, cx);
            match cursor.shape {
                TerminalCursorShape::Block => {
                    quads.push(fill(cursor_bounds, cursor_color.opacity(0.55)))
                }
                TerminalCursorShape::Underline => quads.push(fill(
                    Bounds::new(
                        point(cursor_bounds.left(), cursor_bounds.bottom() - px(2.)),
                        size(cursor_bounds.size.width, px(2.)),
                    ),
                    cursor_color,
                )),
                TerminalCursorShape::Beam => quads.push(fill(
                    Bounds::new(
                        cursor_bounds.origin,
                        size(px(2.), cursor_bounds.size.height),
                    ),
                    cursor_color,
                )),
                TerminalCursorShape::HollowBlock => {
                    quads.push(outline(cursor_bounds, cursor_color, BorderStyle::default()))
                }
                TerminalCursorShape::Hidden => {}
            }

            if let Some(marked_text) = self.marked_text.as_ref().filter(|text| !text.is_empty()) {
                cells.push(ShapedCell {
                    origin: cursor_bounds.origin,
                    line: window.text_system().shape_line(
                        marked_text.clone().into(),
                        font_size,
                        &[TextRun {
                            len: marked_text.len(),
                            font: style.font(),
                            color: style.color,
                            background_color: None,
                            underline: Some(UnderlineStyle {
                                thickness: px(1.),
                                color: Some(style.color),
                                wavy: false,
                            }),
                            strikethrough: None,
                        }],
                        None,
                    ),
                });
            }
        }

        PrepaintState {
            hitbox,
            quads,
            cells,
            cell_size,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus_handle,
            TerminalInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepaint.quads.drain(..) {
                window.paint_quad(quad);
            }
            for cell in prepaint.cells.drain(..) {
                cell.line
                    .paint(
                        cell.origin,
                        prepaint.cell_size.height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
            }
        });

        let hitbox = prepaint.hitbox.clone();
        let view = self.view.clone();
        let terminal_size = self.terminal.size;
        let cell_size = prepaint.cell_size;
        let connection_key = self.connection_key;
        let pane_id = self.pane_id;
        let focus_handle = self.focus_handle.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase.bubble() && event.button == MouseButton::Left && hitbox.is_hovered(window) {
                focus_handle.focus(window, cx);
                let position = terminal_position(event.position, bounds, cell_size, terminal_size);
                view.update(cx, |view, cx| {
                    view.select_pane(connection_key, pane_id, cx);
                    view.begin_selection(connection_key, pane_id, position);
                });
                cx.stop_propagation();
            }
        });

        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase.bubble()
                && event.dragging()
                && view.read(cx).is_selecting(connection_key, pane_id)
            {
                let position = terminal_position(event.position, bounds, cell_size, terminal_size);
                view.update(cx, |view, _| {
                    view.update_selection(connection_key, pane_id, position)
                });
                cx.stop_propagation();
            }
        });

        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase.bubble()
                && event.button == MouseButton::Left
                && view.read(cx).is_selecting(connection_key, pane_id)
            {
                let position = terminal_position(event.position, bounds, cell_size, terminal_size);
                view.update(cx, |view, _| {
                    view.end_selection(connection_key, pane_id, position)
                });
                cx.stop_propagation();
            }
        });

        let hitbox = prepaint.hitbox.clone();
        let view = self.view.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if !phase.bubble() || !hitbox.should_handle_scroll(window) {
                return;
            }
            let delta = match event.delta {
                ScrollDelta::Pixels(delta) => delta.y / cell_size.height,
                ScrollDelta::Lines(delta) => delta.y,
            };
            let lines = if delta.abs() < 1. && delta != 0. {
                delta.signum() as i32
            } else {
                delta.round() as i32
            };
            view.update(cx, |view, _| {
                view.scroll_terminal(connection_key, pane_id, lines)
            });
            cx.stop_propagation();
        });
    }
}

fn terminal_position(
    point: Point<Pixels>,
    bounds: Bounds<Pixels>,
    cell_size: Size<Pixels>,
    terminal_size: TerminalSize,
) -> TerminalPosition {
    let x = (point.x - bounds.left()) / cell_size.width;
    let y = ((point.y - bounds.top()) / cell_size.height).max(0.);
    let side = if x <= 0. {
        TerminalSide::Left
    } else if x >= f32::from(terminal_size.columns) || x.fract() >= 0.5 {
        TerminalSide::Right
    } else {
        TerminalSide::Left
    };
    TerminalPosition {
        row: (y.floor() as u16).min(terminal_size.rows.saturating_sub(1)),
        column: (x.max(0.).floor() as u16).min(terminal_size.columns.saturating_sub(1)),
        side,
    }
}

fn terminal_color(color: TerminalColor, foreground: bool, cx: &App) -> Hsla {
    match color {
        TerminalColor::Rgb { red, green, blue } => {
            rgb((u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)).into()
        }
        TerminalColor::Indexed(index) => indexed_color(index),
        TerminalColor::Named(256 | 267) => cx.theme().foreground,
        TerminalColor::Named(257) => cx.theme().background,
        TerminalColor::Named(258) => cx.theme().primary,
        TerminalColor::Named(index @ 259..=266) => indexed_color((index - 259) as u8).opacity(0.65),
        TerminalColor::Named(268) => cx.theme().foreground.opacity(0.65),
        TerminalColor::Named(index @ 0..=15) => indexed_color(index as u8),
        TerminalColor::Named(_) if foreground => cx.theme().foreground,
        TerminalColor::Named(_) => cx.theme().background,
    }
}

fn indexed_color(index: u8) -> Hsla {
    const ANSI: [u32; 16] = [
        0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666,
        0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
    ];
    if index < 16 {
        return rgb(ANSI[usize::from(index)]).into();
    }
    let (red, green, blue) = if index < 232 {
        let value = index - 16;
        (
            color_cube(value / 36),
            color_cube((value / 6) % 6),
            color_cube(value % 6),
        )
    } else {
        let gray = 8 + (index - 232) * 10;
        (gray, gray, gray)
    };
    rgb((u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)).into()
}

fn color_cube(value: u8) -> u8 {
    if value == 0 { 0 } else { 55 + value * 40 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_points_map_to_clamped_terminal_cells() {
        let bounds = Bounds::new(point(px(10.), px(20.)), size(px(80.), px(40.)));
        let cell_size = size(px(8.), px(10.));
        let terminal_size = TerminalSize::new(4, 10);

        assert_eq!(
            terminal_position(point(px(23.), px(35.)), bounds, cell_size, terminal_size),
            TerminalPosition {
                row: 1,
                column: 1,
                side: TerminalSide::Right,
            }
        );
        assert_eq!(
            terminal_position(point(px(500.), px(500.)), bounds, cell_size, terminal_size),
            TerminalPosition {
                row: 3,
                column: 9,
                side: TerminalSide::Right,
            }
        );
        assert_eq!(
            terminal_position(point(px(0.), px(0.)), bounds, cell_size, terminal_size).side,
            TerminalSide::Left
        );
    }
}
