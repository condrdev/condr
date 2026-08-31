use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use condr_core::protocol::RuntimeEpoch;
use condr_core::{
    PaneId, TerminalColor, TerminalCursorShape, TerminalPosition, TerminalSelection, TerminalSide,
    TerminalSize, TerminalView,
};
use gpui::{
    App, BorderStyle, Bounds, ClipboardItem, ContentMask, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InputHandler, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    ShapedLine, Size, StrikethroughStyle, Style, TextAlign, TextInputConfiguration, TextRun,
    TextStyle, UTF16Selection, UnderlineStyle, Window, fill, outline, point, px, relative, rgb,
    size,
};
use smol_str::SmolStr;

use crate::{Condr, ConnectionKey};

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

#[derive(Clone, PartialEq)]
struct TerminalPalette {
    background: Hsla,
    foreground: Hsla,
    cursor: Hsla,
    selection: Hsla,
    normal: [Hsla; 8],
    bright: [Hsla; 8],
}

impl TerminalPalette {
    // Fixed dark palette until terminal theme configuration is available.
    fn temporary_dark() -> Self {
        Self {
            background: rgb(0x0d1117).into(),
            foreground: rgb(0xc9d1d9).into(),
            cursor: rgb(0xf0f6fc).into(),
            selection: rgb(0x264f78).into(),
            normal: [
                rgb(0x484f58).into(),
                rgb(0xff7b72).into(),
                rgb(0x3fb950).into(),
                rgb(0xd29922).into(),
                rgb(0x58a6ff).into(),
                rgb(0xbc8cff).into(),
                rgb(0x39c5cf).into(),
                rgb(0xb1bac4).into(),
            ],
            bright: [
                rgb(0x6e7681).into(),
                rgb(0xffa198).into(),
                rgb(0x56d364).into(),
                rgb(0xe3b341).into(),
                rgb(0x79c0ff).into(),
                rgb(0xd2a8ff).into(),
                rgb(0x56d4dd).into(),
                rgb(0xffffff).into(),
            ],
        }
    }
}

pub(crate) struct TerminalElement {
    view: Entity<Condr>,
    props: TerminalElementProps,
}

pub(crate) struct TerminalElementProps {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) connection_key: ConnectionKey,
    pub(crate) pane_id: PaneId,
    pub(crate) terminal: TerminalView,
    pub(crate) marked_text: Option<String>,
    pub(crate) selection: Option<TerminalSelection>,
    pub(crate) runtime_epoch: Option<RuntimeEpoch>,
    pub(crate) render_cache: Rc<RefCell<TerminalRenderCache>>,
}

#[derive(Clone, PartialEq)]
struct TerminalRenderCacheKey {
    runtime_epoch: Option<RuntimeEpoch>,
    size: TerminalSize,
    style: TextStyle,
    font_size: Pixels,
    palette: TerminalPalette,
}

#[derive(Clone, PartialEq)]
struct TerminalCellShapeKey {
    text: SmolStr,
    foreground: Hsla,
    flags: u16,
}

struct CachedTerminalCell {
    key: TerminalCellShapeKey,
    line: ShapedLine,
}

#[derive(Default)]
pub(crate) struct TerminalRenderCache {
    key: Option<TerminalRenderCacheKey>,
    cells: Vec<Option<CachedTerminalCell>>,
    #[cfg(all(test, feature = "test-support"))]
    shaped_cells: usize,
}

#[cfg(all(test, feature = "test-support"))]
impl TerminalRenderCache {
    pub(crate) fn shaped_cells(&self) -> usize {
        self.shaped_cells
    }
}

impl TerminalRenderCache {
    fn prepare(&mut self, key: TerminalRenderCacheKey, cell_count: usize) {
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.cells.clear();
            self.cells.resize_with(cell_count, || None);
        }
    }

    fn shaped_line(
        &mut self,
        index: usize,
        text: &SmolStr,
        foreground: Hsla,
        flags: u16,
        shape: impl FnOnce() -> ShapedLine,
    ) -> ShapedLine {
        if let Some(cached) = self.cells[index].as_ref()
            && cached.key.text == *text
            && cached.key.foreground == foreground
            && cached.key.flags == flags
        {
            return cached.line.clone();
        }

        let line = shape();
        self.cells[index] = Some(CachedTerminalCell {
            key: TerminalCellShapeKey {
                text: text.clone(),
                foreground,
                flags,
            },
            line: line.clone(),
        });
        #[cfg(all(test, feature = "test-support"))]
        {
            self.shaped_cells += 1;
        }
        line
    }
}

// Terminals accept IME text, but printable chords must reach keybindings first.
struct TerminalInputHandler {
    inner: ElementInputHandler<Condr>,
}

impl TerminalInputHandler {
    fn new(bounds: Bounds<Pixels>, view: Entity<Condr>) -> Self {
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
    pub(crate) fn new(view: Entity<Condr>, props: TerminalElementProps) -> Self {
        Self { view, props }
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
        let connection_key = self.props.connection_key;
        let pane_id = self.props.pane_id;
        if !view.read(cx).terminal_layout_is_current(
            connection_key,
            pane_id,
            bounds,
            cell_size,
            terminal_size,
        ) {
            window.defer(cx, move |_, cx| {
                view.update(cx, |view, cx| {
                    view.update_terminal_geometry(connection_key, pane_id, bounds, cell_size);
                    view.resize_terminal(connection_key, pane_id, terminal_size, cx);
                });
            });
        }

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let palette = TerminalPalette::temporary_dark();
        let mut quads = vec![fill(bounds, palette.background)];
        let mut cells = Vec::with_capacity(self.props.terminal.cells.len());
        let cache_key = TerminalRenderCacheKey {
            runtime_epoch: self.props.runtime_epoch,
            size: self.props.terminal.size,
            style: style.clone(),
            font_size,
            palette: palette.clone(),
        };
        let mut render_cache = self.props.render_cache.borrow_mut();
        render_cache.prepare(cache_key, self.props.terminal.cells.len());

        // Terminal revisions often change only a cursor or spinner cell.
        for row in 0..self.props.terminal.size.rows {
            for column in 0..self.props.terminal.size.columns {
                let Some(cell) = self.props.terminal.cell(row, column) else {
                    continue;
                };
                let cell_bounds = Bounds::new(
                    point(
                        bounds.left() + cell_size.width * usize::from(column),
                        bounds.top() + cell_size.height * usize::from(row),
                    ),
                    cell_size,
                );
                let mut foreground = terminal_color(cell.foreground, true, &palette);
                let mut background = terminal_color(cell.background, false, &palette);
                if cell.flags & INVERSE != 0 {
                    std::mem::swap(&mut foreground, &mut background);
                }
                if cell.flags & DIM != 0 {
                    foreground = foreground.opacity(0.65);
                }
                if background != palette.background {
                    quads.push(fill(cell_bounds, background));
                }
                if cell.flags & (HIDDEN | WIDE_CHAR_SPACER | LEADING_WIDE_CHAR_SPACER) != 0
                    || cell.text == " "
                {
                    continue;
                }

                let cache_index = usize::from(row) * usize::from(self.props.terminal.size.columns)
                    + usize::from(column);
                let line = render_cache.shaped_line(
                    cache_index,
                    &cell.text,
                    foreground,
                    cell.flags,
                    || {
                        let mut font = style.font();
                        if cell.flags & BOLD != 0 {
                            font = font.bold();
                        }
                        if cell.flags & ITALIC != 0 {
                            font = font.italic();
                        }
                        let underline = (cell.flags & (UNDERLINE | ALL_UNDERLINES) != 0).then_some(
                            UnderlineStyle {
                                thickness: px(1.),
                                color: Some(foreground),
                                wavy: false,
                            },
                        );
                        let strikethrough =
                            (cell.flags & STRIKEOUT != 0).then_some(StrikethroughStyle {
                                thickness: px(1.),
                                color: Some(foreground),
                            });
                        window.text_system().shape_line(
                            cell.text.as_str().into(),
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
                        )
                    },
                );
                cells.push(ShapedCell {
                    origin: cell_bounds.origin,
                    line,
                });
            }
        }

        if let Some(selection) = self.props.selection {
            push_selection_quads(
                &mut quads,
                selection,
                self.props.terminal.size,
                bounds,
                cell_size,
                palette.selection,
            );
        }

        if let Some(cursor) = self.props.terminal.cursor {
            let cursor_bounds = Bounds::new(
                point(
                    bounds.left() + cell_size.width * usize::from(cursor.column),
                    bounds.top() + cell_size.height * usize::from(cursor.row),
                ),
                cell_size,
            );
            let cursor_color = terminal_color(TerminalColor::Named(258), true, &palette);
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

            if let Some(marked_text) = self
                .props
                .marked_text
                .as_ref()
                .filter(|text| !text.is_empty())
            {
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
            &self.props.focus_handle,
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
        let terminal_size = self.props.terminal.size;
        let cell_size = prepaint.cell_size;
        let connection_key = self.props.connection_key;
        let pane_id = self.props.pane_id;
        let focus_handle = self.props.focus_handle.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase.bubble() && event.button == MouseButton::Left && hitbox.is_hovered(window) {
                let position = terminal_position(event.position, bounds, cell_size, terminal_size);
                let accepted = view.update(cx, |view, cx| {
                    view.select_pane(connection_key, pane_id, window, cx)
                });
                if accepted {
                    focus_handle.focus(window, cx);
                    view.update(cx, |view, cx| {
                        view.begin_selection(
                            connection_key,
                            pane_id,
                            position,
                            event.click_count,
                            cx,
                        );
                    });
                }
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
                view.update(cx, |view, cx| {
                    view.update_selection(connection_key, pane_id, position, cx)
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
                view.update(cx, |view, cx| {
                    view.end_selection(connection_key, pane_id, position, cx)
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
            view.update(cx, |view, cx| {
                view.scroll_terminal(connection_key, pane_id, lines, cx)
            });
            cx.stop_propagation();
        });
    }
}

fn push_selection_quads(
    quads: &mut Vec<PaintQuad>,
    selection: TerminalSelection,
    terminal_size: TerminalSize,
    bounds: Bounds<Pixels>,
    cell_size: Size<Pixels>,
    color: Hsla,
) {
    let columns = u32::from(terminal_size.columns);
    let cell_count = columns * u32::from(terminal_size.rows);
    let Some(last_cell) = cell_count.checked_sub(1) else {
        return;
    };
    let Some((start, end)) = selection.selected_cell_range(terminal_size.columns) else {
        return;
    };
    let start = start.min(last_cell);
    let end = end.min(last_cell);

    for row in (start / columns)..=(end / columns) {
        let start_column = if row == start / columns {
            start % columns
        } else {
            0
        };
        let end_column = if row == end / columns {
            end % columns
        } else {
            columns - 1
        };
        quads.push(fill(
            Bounds::new(
                point(
                    bounds.left() + cell_size.width * start_column as usize,
                    bounds.top() + cell_size.height * row as usize,
                ),
                size(
                    cell_size.width * (end_column - start_column + 1) as usize,
                    cell_size.height,
                ),
            ),
            color,
        ));
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

fn terminal_color(color: TerminalColor, foreground: bool, palette: &TerminalPalette) -> Hsla {
    match color {
        TerminalColor::Rgb { red, green, blue } => {
            rgb((u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)).into()
        }
        TerminalColor::Indexed(index) => indexed_color(index, palette),
        TerminalColor::Named(256 | 267) => palette.foreground,
        TerminalColor::Named(257) => palette.background,
        TerminalColor::Named(258) => palette.cursor,
        TerminalColor::Named(index @ 259..=266) => {
            palette.normal[usize::from(index - 259)].opacity(0.65)
        }
        TerminalColor::Named(268) => palette.foreground.opacity(0.65),
        TerminalColor::Named(index @ 0..=15) => indexed_color(index as u8, palette),
        TerminalColor::Named(_) if foreground => palette.foreground,
        TerminalColor::Named(_) => palette.background,
    }
}

fn indexed_color(index: u8, palette: &TerminalPalette) -> Hsla {
    if index < 16 {
        return if index < 8 {
            palette.normal[usize::from(index)]
        } else {
            palette.bright[usize::from(index - 8)]
        };
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

    #[cfg(feature = "test-support")]
    #[test]
    fn render_cache_reuses_unchanged_cells_across_terminal_frames() {
        let mut cache = TerminalRenderCache::default();
        let key = TerminalRenderCacheKey {
            runtime_epoch: None,
            size: TerminalSize::new(24, 80),
            style: TextStyle::default(),
            font_size: px(14.),
            palette: TerminalPalette::temporary_dark(),
        };
        let foreground: Hsla = rgb(0xffffff).into();
        cache.prepare(key.clone(), 2);
        cache.shaped_line(
            0,
            &SmolStr::new_inline("x"),
            foreground,
            0,
            ShapedLine::default,
        );
        assert_eq!(cache.shaped_cells(), 1);

        cache.prepare(key, 2);
        cache.shaped_line(0, &SmolStr::new_inline("x"), foreground, 0, || {
            panic!("unchanged cell should reuse its shaped line")
        });
        assert_eq!(cache.shaped_cells(), 1);

        cache.shaped_line(
            0,
            &SmolStr::new_inline("y"),
            foreground,
            0,
            ShapedLine::default,
        );
        assert_eq!(cache.shaped_cells(), 2);
    }

    #[test]
    fn temporary_palette_resolves_terminal_colors_independently_from_ui_theme() {
        let palette = TerminalPalette::temporary_dark();

        assert_eq!(
            terminal_color(TerminalColor::Named(256), true, &palette),
            palette.foreground
        );
        assert_eq!(
            terminal_color(TerminalColor::Named(257), false, &palette),
            palette.background
        );
        assert_eq!(
            terminal_color(TerminalColor::Indexed(1), true, &palette),
            palette.normal[1]
        );
        assert_eq!(
            terminal_color(TerminalColor::Indexed(9), true, &palette),
            palette.bright[1]
        );
        assert_eq!(
            terminal_color(
                TerminalColor::Rgb {
                    red: 0x12,
                    green: 0x34,
                    blue: 0x56,
                },
                true,
                &palette,
            ),
            rgb(0x123456).into()
        );
    }

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

    #[test]
    fn selection_backgrounds_are_compacted_by_row() {
        let mut quads = Vec::new();
        push_selection_quads(
            &mut quads,
            TerminalSelection {
                start: TerminalPosition {
                    row: 0,
                    column: 2,
                    side: TerminalSide::Left,
                },
                end: TerminalPosition {
                    row: 2,
                    column: 7,
                    side: TerminalSide::Right,
                },
                display_offset: 0,
            },
            TerminalSize::new(4, 10),
            Bounds::new(point(px(0.), px(0.)), size(px(100.), px(40.))),
            size(px(10.), px(10.)),
            rgb(0xffffff).into(),
        );

        assert_eq!(quads.len(), 3);
    }
}
