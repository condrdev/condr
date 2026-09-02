use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use condr_core::protocol::RuntimeEpoch;
use condr_core::{
    PaneId, TerminalColor, TerminalCursorShape, TerminalModifiers, TerminalMouseButton,
    TerminalMouseEvent, TerminalMousePosition, TerminalMouseTracking, TerminalMouseWheel,
    TerminalPosition, TerminalSelection, TerminalSide, TerminalSize, TerminalView,
};
use gpui::{
    App, BorderStyle, Bounds, ClipboardItem, ContentMask, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InputHandler, InspectorElementId, IntoElement, LayoutId, Modifiers as GpuiModifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, ShapedLine, Size, StrikethroughStyle, Style, TextAlign,
    TextInputConfiguration, TextRun, TextStyle, TouchPhase, UTF16Selection, UnderlineStyle, Window,
    fill, outline, point, px, relative, rgb, size,
};
use smol_str::SmolStr;

use crate::{Condr, ConnectionKey};

const INVERSE: u16 = 1 << 0;
const BOLD: u16 = 1 << 1;
const ITALIC: u16 = 1 << 2;
const WIDE_CHAR_SPACER: u16 = 1 << 6;
const DIM: u16 = 1 << 7;
const HIDDEN: u16 = 1 << 8;
const STRIKEOUT: u16 = 1 << 9;
const LEADING_WIDE_CHAR_SPACER: u16 = 1 << 10;
const UNDERCURL: u16 = 1 << 12;
const ALL_UNDERLINES: u16 = 0b0111_1000_0000_1000;

/// Minimum APCA Lc between text and its cell background; Lc 45 is the floor
/// for large fluent text and the common terminal default. Becomes a setting
/// once terminal appearance configuration exists.
const MINIMUM_CONTRAST_LC: f32 = 45.0;

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
    pub(crate) scroll_remainder: Rc<RefCell<Point<f32>>>,
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
    /// Painted after `cells`: the cursor block and the character or IME text
    /// repainted on top of it.
    overlay_quads: Vec<PaintQuad>,
    overlay_cells: Vec<ShapedCell>,
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
        let rows = terminal_grid_extent(bounds.size.height, cell_size.height, 1);
        let columns = terminal_grid_extent(bounds.size.width, cell_size.width, 2);
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
            window.defer(cx, move |window, cx| {
                view.update(cx, |view, cx| {
                    view.update_terminal_geometry(
                        connection_key,
                        pane_id,
                        bounds,
                        cell_size,
                        window,
                    );
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
        let mut block_regions = Vec::new();
        let mut background_regions: Vec<BlockRegion> = Vec::new();
        let mut contrast_memo = ContrastMemo::default();

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
                if background != palette.background {
                    let start_line = i64::from(row) * BLOCK_SUBLINES;
                    let start_col = i64::from(column) * BLOCK_SUBCOLUMNS;
                    let region = BlockRegion {
                        start_line,
                        start_col,
                        end_line: start_line + BLOCK_SUBLINES - 1,
                        end_col: start_col + BLOCK_SUBCOLUMNS - 1,
                        color: background,
                    };
                    match background_regions.last_mut() {
                        Some(last) if last.can_merge_with(&region) => last.merge_with(&region),
                        _ => background_regions.push(region),
                    }
                }
                if cell.flags & (HIDDEN | WIDE_CHAR_SPACER | LEADING_WIDE_CHAR_SPACER) != 0
                    || (cell.text == " " && cell.flags & (ALL_UNDERLINES | STRIKEOUT) == 0)
                {
                    continue;
                }

                let mut characters = cell.text.chars();
                let single_character = match (characters.next(), characters.next()) {
                    (Some(character), None) => Some(character),
                    _ => None,
                };
                let foreground_source = if cell.flags & INVERSE != 0 {
                    cell.background
                } else {
                    cell.foreground
                };
                if !is_app_chosen_exact_color(foreground_source)
                    && !single_character.is_some_and(is_decorative_character)
                {
                    foreground = contrast_memo.ensure(foreground, background);
                }
                if cell.flags & DIM != 0 {
                    foreground = foreground.opacity(0.65);
                }
                if let Some(character) = single_character
                    && collect_block_element_regions(
                        &mut block_regions,
                        character,
                        row,
                        column,
                        foreground,
                    )
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
                        let underline =
                            (cell.flags & ALL_UNDERLINES != 0).then_some(UnderlineStyle {
                                thickness: px(1.),
                                color: Some(foreground),
                                wavy: cell.flags & UNDERCURL != 0,
                            });
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

        // Cell backgrounds paint below the block elements.
        for regions in [background_regions, block_regions] {
            for region in merge_block_regions(regions) {
                let left = bounds.left()
                    + cell_size.width * (region.start_col as f32 / BLOCK_SUBCOLUMNS as f32);
                let top = bounds.top()
                    + cell_size.height * (region.start_line as f32 / BLOCK_SUBLINES as f32);
                let width = cell_size.width
                    * ((region.end_col - region.start_col + 1) as f32 / BLOCK_SUBCOLUMNS as f32);
                let height = cell_size.height
                    * ((region.end_line - region.start_line + 1) as f32 / BLOCK_SUBLINES as f32);
                quads.push(fill(
                    Bounds::new(point(left, top), size(width, height)),
                    region.color,
                ));
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

        let mut overlay_quads = Vec::new();
        let mut overlay_cells = Vec::new();
        if let Some(cursor) = self.props.terminal.cursor {
            let cursor_origin = point(
                bounds.left() + cell_size.width * usize::from(cursor.column),
                bounds.top() + cell_size.height * usize::from(cursor.row),
            );
            let cursor_color = terminal_color(TerminalColor::Named(258), true, &palette);
            // Shape the character under the cursor so a block cursor can
            // repaint it in the background color, and so the cursor grows to
            // cover wide characters instead of splitting them.
            let cursor_cell = self
                .props
                .terminal
                .cell(cursor.row, cursor.column)
                .filter(|cell| !cell.text.trim().is_empty());
            let cursor_text = cursor_cell.map(|cell| {
                window.text_system().shape_line(
                    cell.text.as_str().into(),
                    font_size,
                    &[TextRun {
                        len: cell.text.len(),
                        font: style.font(),
                        color: palette.background,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
            });
            let cursor_width = cursor_text
                .as_ref()
                .map_or(cell_size.width, |line| line.width.max(cell_size.width));
            let cursor_bounds = Bounds::new(cursor_origin, size(cursor_width, cell_size.height));
            // Only the focused pane shows a filled cursor; unfocused panes
            // demote every visible shape to a hollow outline.
            let focused = self.props.focus_handle.is_focused(window);
            let shape = match cursor.shape {
                TerminalCursorShape::Hidden => TerminalCursorShape::Hidden,
                TerminalCursorShape::HollowBlock => TerminalCursorShape::HollowBlock,
                _ if !focused => TerminalCursorShape::HollowBlock,
                shape => shape,
            };
            match shape {
                TerminalCursorShape::Block => {
                    overlay_quads.push(fill(cursor_bounds, cursor_color));
                    if let Some(line) = cursor_text {
                        overlay_cells.push(ShapedCell {
                            origin: cursor_origin,
                            line,
                        });
                    }
                }
                TerminalCursorShape::Underline => overlay_quads.push(fill(
                    Bounds::new(
                        point(cursor_bounds.left(), cursor_bounds.bottom() - px(2.)),
                        size(cursor_bounds.size.width, px(2.)),
                    ),
                    cursor_color,
                )),
                TerminalCursorShape::Beam => overlay_quads.push(fill(
                    Bounds::new(
                        cursor_bounds.origin,
                        size(px(2.), cursor_bounds.size.height),
                    ),
                    cursor_color,
                )),
                TerminalCursorShape::HollowBlock => {
                    overlay_quads.push(outline(cursor_bounds, cursor_color, BorderStyle::default()))
                }
                TerminalCursorShape::Hidden => {}
            }

            if let Some(marked_text) = self
                .props
                .marked_text
                .as_ref()
                .filter(|text| !text.is_empty())
            {
                overlay_cells.push(ShapedCell {
                    origin: cursor_origin,
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
            overlay_quads,
            overlay_cells,
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
            for quad in prepaint.overlay_quads.drain(..) {
                window.paint_quad(quad);
            }
            for cell in prepaint.overlay_cells.drain(..) {
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
        let mouse_tracking = self.props.terminal.mouse_tracking;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble() || !hitbox.is_hovered(window) {
                return;
            }
            let Some(button) = terminal_mouse_button(event.button) else {
                return;
            };
            let position = terminal_position(event.position, bounds, cell_size, terminal_size);
            let accepted = view.update(cx, |view, cx| {
                view.select_pane(connection_key, pane_id, window, cx)
            });
            if !accepted {
                return;
            }
            focus_handle.focus(window, cx);

            if button == TerminalMouseButton::Left
                && (mouse_tracking == TerminalMouseTracking::None || event.modifiers.shift)
            {
                view.update(cx, |view, cx| {
                    view.begin_selection(connection_key, pane_id, position, event.click_count, cx);
                });
                window.capture_pointer(hitbox.id);
                cx.stop_propagation();
                return;
            }
            if event.modifiers.shift {
                cx.stop_propagation();
                return;
            }
            if mouse_tracking != TerminalMouseTracking::None {
                let position = TerminalMousePosition {
                    row: position.row,
                    column: position.column,
                };
                let sent = view.update(cx, |view, cx| {
                    view.start_terminal_mouse_capture(
                        connection_key,
                        pane_id,
                        button,
                        TerminalMouseEvent::Button {
                            button,
                            pressed: true,
                            position,
                            modifiers: terminal_modifiers(event.modifiers),
                        },
                        cx,
                    )
                });
                if sent {
                    window.capture_pointer(hitbox.id);
                    cx.stop_propagation();
                }
            }
        });

        let hitbox = prepaint.hitbox.clone();
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if !phase.bubble() {
                return;
            }
            if view
                .read(cx)
                .terminal_mouse_gesture_owner()
                .is_some_and(|owner| owner != (connection_key, pane_id))
            {
                return;
            }
            if view.read(cx).is_selecting(connection_key, pane_id) {
                let position = terminal_position(event.position, bounds, cell_size, terminal_size);
                if event.dragging() {
                    view.update(cx, |view, cx| {
                        view.update_selection(connection_key, pane_id, position, cx)
                    });
                } else {
                    view.update(cx, |view, cx| {
                        view.end_selection(connection_key, pane_id, position, cx)
                    });
                    window.release_pointer();
                }
                cx.stop_propagation();
                return;
            }

            let captured = view
                .read(cx)
                .terminal_mouse_capture(connection_key, pane_id);
            if let Some(button) = captured {
                let position =
                    terminal_mouse_position(event.position, bounds, cell_size, terminal_size);
                if event.pressed_button.and_then(terminal_mouse_button) != Some(button) {
                    view.update(cx, |view, _| {
                        view.finish_terminal_mouse_capture(
                            connection_key,
                            pane_id,
                            button,
                            TerminalMouseEvent::Button {
                                button,
                                pressed: false,
                                position,
                                modifiers: terminal_modifiers(event.modifiers),
                            },
                        )
                    });
                    window.release_pointer();
                } else if matches!(
                    mouse_tracking,
                    TerminalMouseTracking::Drag | TerminalMouseTracking::Motion
                ) {
                    view.update(cx, |view, _| {
                        view.report_captured_terminal_mouse(
                            connection_key,
                            pane_id,
                            TerminalMouseEvent::Motion {
                                button: Some(button),
                                position,
                                modifiers: terminal_modifiers(event.modifiers),
                            },
                        )
                    });
                }
                cx.stop_propagation();
            } else if !event.modifiers.shift
                && mouse_tracking == TerminalMouseTracking::Motion
                && hitbox.is_hovered(window)
            {
                let position =
                    terminal_mouse_position(event.position, bounds, cell_size, terminal_size);
                let sent = view.update(cx, |view, _| {
                    view.report_terminal_motion(
                        connection_key,
                        pane_id,
                        TerminalMouseEvent::Motion {
                            button: None,
                            position,
                            modifiers: terminal_modifiers(event.modifiers),
                        },
                    )
                });
                if sent {
                    cx.stop_propagation();
                }
            }
        });

        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if !phase.bubble() {
                return;
            }
            if view
                .read(cx)
                .terminal_mouse_gesture_owner()
                .is_some_and(|owner| owner != (connection_key, pane_id))
            {
                return;
            }
            if event.button == MouseButton::Left
                && view.read(cx).is_selecting(connection_key, pane_id)
            {
                let position = terminal_position(event.position, bounds, cell_size, terminal_size);
                view.update(cx, |view, cx| {
                    view.end_selection(connection_key, pane_id, position, cx)
                });
                window.release_pointer();
                cx.stop_propagation();
                return;
            }
            let Some(button) = terminal_mouse_button(event.button) else {
                return;
            };
            let position =
                terminal_mouse_position(event.position, bounds, cell_size, terminal_size);
            let captured = view.update(cx, |view, _| {
                view.finish_terminal_mouse_capture(
                    connection_key,
                    pane_id,
                    button,
                    TerminalMouseEvent::Button {
                        button,
                        pressed: false,
                        position,
                        modifiers: terminal_modifiers(event.modifiers),
                    },
                )
            });
            if captured {
                window.release_pointer();
                cx.stop_propagation();
            }
        });

        let hitbox = prepaint.hitbox.clone();
        let view = self.view.clone();
        let scroll_remainder = self.props.scroll_remainder.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if !phase.bubble() || !hitbox.should_handle_scroll(window) {
                return;
            }
            if matches!(event.touch_phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                *scroll_remainder.borrow_mut() = point(0., 0.);
                return;
            }
            if event.touch_phase == TouchPhase::Started {
                *scroll_remainder.borrow_mut() = point(0., 0.);
            }
            let precise_scroll = matches!(
                event.delta,
                ScrollDelta::Pixels(delta) if delta.x != px(0.) || delta.y != px(0.)
            );
            let (horizontal, vertical) = match event.delta {
                ScrollDelta::Pixels(delta) => {
                    let mut remainder = scroll_remainder.borrow_mut();
                    (
                        accumulated_wheel_steps(&mut remainder.x, delta.x / cell_size.width),
                        accumulated_wheel_steps(&mut remainder.y, delta.y / cell_size.height),
                    )
                }
                ScrollDelta::Lines(delta) => (delta.x, delta.y),
            };
            let position =
                terminal_mouse_position(event.position, bounds, cell_size, terminal_size);
            let modifiers = terminal_modifiers(event.modifiers);
            let directions = [
                (vertical, TerminalMouseWheel::Up, TerminalMouseWheel::Down),
                (
                    horizontal,
                    TerminalMouseWheel::Left,
                    TerminalMouseWheel::Right,
                ),
            ];
            let mut handled = false;
            for (delta, positive, negative) in directions {
                let amount = wheel_steps(delta);
                if amount == 0 {
                    continue;
                }
                handled |= view.update(cx, |view, cx| {
                    view.report_terminal_mouse(
                        connection_key,
                        pane_id,
                        TerminalMouseEvent::Wheel {
                            direction: if delta > 0. { positive } else { negative },
                            amount,
                            position,
                            modifiers,
                        },
                        true,
                        cx,
                    )
                });
            }
            if handled || precise_scroll {
                cx.stop_propagation();
            }
        });
    }
}

// Block characters are described on an integer subcell grid so that the
// regions of neighboring cells line up exactly and can merge: 8 subcolumns
// cover the eighth-width bars and 24 sublines cover eighths, halves and
// thirds of the cell height.
const BLOCK_SUBCOLUMNS: i64 = 8;
const BLOCK_SUBLINES: i64 = 24;

/// A block-element rectangle on the terminal-wide subcell grid, with
/// inclusive extents.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BlockRegion {
    start_line: i64,
    start_col: i64,
    end_line: i64,
    end_col: i64,
    color: Hsla,
}

impl BlockRegion {
    /// Two regions can merge when their union is again a rectangle of one
    /// color: equal line extents with touching or overlapping columns, or
    /// equal column extents with touching or overlapping lines.
    fn can_merge_with(&self, other: &Self) -> bool {
        if self.color != other.color {
            return false;
        }
        let same_lines = self.start_line == other.start_line && self.end_line == other.end_line;
        let same_cols = self.start_col == other.start_col && self.end_col == other.end_col;
        let cols_touch = self.start_col <= other.end_col + 1 && other.start_col <= self.end_col + 1;
        let lines_touch =
            self.start_line <= other.end_line + 1 && other.start_line <= self.end_line + 1;
        (same_lines && cols_touch) || (same_cols && lines_touch)
    }

    fn merge_with(&mut self, other: &Self) {
        self.start_line = self.start_line.min(other.start_line);
        self.start_col = self.start_col.min(other.start_col);
        self.end_line = self.end_line.max(other.end_line);
        self.end_col = self.end_col.max(other.end_col);
    }
}

/// Collapses adjacent same-color regions into fewer, larger rectangles so a
/// logo or QR code drawn from block characters does not cost one quad per
/// subcell.
fn merge_block_regions(regions: Vec<BlockRegion>) -> Vec<BlockRegion> {
    // Cells are visited in row-major order, so one linear pass already
    // collapses the dominant case of horizontal runs (and vertical stacks
    // that end up adjacent in the vector).
    let mut merged: Vec<BlockRegion> = Vec::with_capacity(regions.len());
    for region in regions {
        match merged.last_mut() {
            Some(last) if last.can_merge_with(&region) => last.merge_with(&region),
            _ => merged.push(region),
        }
    }
    // ponytail: the exhaustive fixpoint is quadratic, so screens that still
    // hold thousands of unmergeable regions after the linear pass skip it
    // and just paint more quads; switch to a grid-based merge if profiling
    // ever shows quad count as the bottleneck.
    if merged.len() <= 256 {
        let mut changed = true;
        while changed {
            changed = false;
            let mut i = 0;
            while i < merged.len() {
                let mut j = i + 1;
                while j < merged.len() {
                    if merged[i].can_merge_with(&merged[j]) {
                        let other = merged.remove(j);
                        merged[i].merge_with(&other);
                        changed = true;
                    } else {
                        j += 1;
                    }
                }
                i += 1;
            }
        }
    }
    merged
}

/// Collects the filled rectangles of a block character into `regions`,
/// placed on the subcell grid at the given terminal cell. These characters
/// are painted as quads instead of font glyphs: glyphs only cover the em
/// box, which is shorter than the line height, so glyph-rendered blocks
/// leave horizontal seams and pixel art falls apart. Returns false when the
/// character is not a block element.
fn collect_block_element_regions(
    regions: &mut Vec<BlockRegion>,
    character: char,
    row: u16,
    column: u16,
    color: Hsla,
) -> bool {
    const UPPER_LEFT: u8 = 1;
    const UPPER_RIGHT: u8 = 1 << 1;
    const LOWER_LEFT: u8 = 1 << 2;
    const LOWER_RIGHT: u8 = 1 << 3;
    // U+2596..=U+259F in code point order.
    const QUADRANTS: [u8; 10] = [
        LOWER_LEFT,
        LOWER_RIGHT,
        UPPER_LEFT,
        UPPER_LEFT | LOWER_LEFT | LOWER_RIGHT,
        UPPER_LEFT | LOWER_RIGHT,
        UPPER_LEFT | UPPER_RIGHT | LOWER_LEFT,
        UPPER_LEFT | UPPER_RIGHT | LOWER_RIGHT,
        UPPER_RIGHT,
        UPPER_RIGHT | LOWER_LEFT,
        UPPER_RIGHT | LOWER_LEFT | LOWER_RIGHT,
    ];

    let base_line = i64::from(row) * BLOCK_SUBLINES;
    let base_col = i64::from(column) * BLOCK_SUBCOLUMNS;
    let mut push = |x: i64, y: i64, width: i64, height: i64, color: Hsla| {
        regions.push(BlockRegion {
            start_line: base_line + y,
            start_col: base_col + x,
            end_line: base_line + y + height - 1,
            end_col: base_col + x + width - 1,
            color,
        });
    };
    match character {
        // Upper half.
        '\u{2580}' => push(0, 0, 8, 12, color),
        // Lower one eighth through the full block.
        '\u{2581}'..='\u{2588}' => {
            let eighths = i64::from(character as u32 - 0x2580);
            push(0, 24 - eighths * 3, 8, eighths * 3, color);
        }
        // Left seven eighths down to the left one eighth.
        '\u{2589}'..='\u{258F}' => push(0, 0, i64::from(0x2590 - character as u32), 24, color),
        // Right half.
        '\u{2590}' => push(4, 0, 4, 24, color),
        // Light, medium and dark shades, approximated with the foreground
        // color at reduced opacity instead of the fonts' stipple patterns,
        // trading pattern fidelity for seamless cell coverage.
        '\u{2591}' => push(0, 0, 8, 24, color.opacity(0.25)),
        '\u{2592}' => push(0, 0, 8, 24, color.opacity(0.5)),
        '\u{2593}' => push(0, 0, 8, 24, color.opacity(0.75)),
        // Upper one eighth.
        '\u{2594}' => push(0, 0, 8, 3, color),
        // Right one eighth.
        '\u{2595}' => push(7, 0, 1, 24, color),
        '\u{2596}'..='\u{259F}' => {
            let mask = QUADRANTS[character as usize - 0x2596];
            for (bit, x, y) in [
                (UPPER_LEFT, 0, 0),
                (UPPER_RIGHT, 4, 0),
                (LOWER_LEFT, 0, 12),
                (LOWER_RIGHT, 4, 12),
            ] {
                if mask & bit != 0 {
                    push(x, y, 4, 12, color);
                }
            }
        }
        // Legacy Computing sextants: a 2x3 grid of subcells, with bit
        // `row * 2 + column` filled. The code points enumerate every fill
        // combination in binary order, except the four that already exist
        // as Block Elements: empty, ▌ (0b010101), ▐ (0b101010) and █, so
        // the enumeration skips those values.
        '\u{1FB00}'..='\u{1FB3B}' => {
            let mut mask = character as u32 - 0x1FB00 + 1;
            if mask >= 0b010101 {
                mask += 1;
            }
            if mask >= 0b101010 {
                mask += 1;
            }
            for subrow in 0..3i64 {
                for subcolumn in 0..2i64 {
                    if mask & (1 << (subrow * 2 + subcolumn)) != 0 {
                        push(subcolumn * 4, subrow * 8, 4, 8, color);
                    }
                }
            }
        }
        _ => return false,
    }
    true
}

/// Box-like characters used as seamless visual connectors: adjusting their
/// color for contrast would break the joins with neighboring backgrounds,
/// so they keep their exact colors.
fn is_decorative_character(character: char) -> bool {
    matches!(
        character as u32,
        // Box Drawing, Block Elements and Geometric Shapes.
        0x2500..=0x25FF
        // Legacy Computing sextants.
        | 0x1FB00..=0x1FB3B
        // Powerline separators in the Private Use Area.
        | 0xE0B0..=0xE0BF | 0xE0C0..=0xE0CA | 0xE0CC..=0xE0D7
    )
}

/// Whether the application explicitly picked this color and does not want it
/// adjusted for contrast: 24-bit true color, or a specific entry in the
/// 256-color palette outside the 16 theme-defined ANSI colors.
fn is_app_chosen_exact_color(color: TerminalColor) -> bool {
    match color {
        TerminalColor::Rgb { .. } => true,
        TerminalColor::Indexed(index) => index >= 16,
        TerminalColor::Named(_) => false,
    }
}

/// Per-frame memo for the contrast adjustment: a terminal frame holds few
/// distinct color pairs, so a linear scan beats hashing float colors.
#[derive(Default)]
struct ContrastMemo {
    entries: Vec<(Hsla, Hsla, Hsla)>,
}

impl ContrastMemo {
    fn ensure(&mut self, foreground: Hsla, background: Hsla) -> Hsla {
        if let Some((_, _, adjusted)) = self
            .entries
            .iter()
            .find(|(fg, bg, _)| *fg == foreground && *bg == background)
        {
            return *adjusted;
        }
        let adjusted =
            crate::apca::ensure_minimum_contrast(foreground, background, MINIMUM_CONTRAST_LC);
        if self.entries.len() < 256 {
            self.entries.push((foreground, background, adjusted));
        }
        adjusted
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

fn terminal_mouse_position(
    point: Point<Pixels>,
    bounds: Bounds<Pixels>,
    cell_size: Size<Pixels>,
    terminal_size: TerminalSize,
) -> TerminalMousePosition {
    let position = terminal_position(point, bounds, cell_size, terminal_size);
    TerminalMousePosition {
        row: position.row,
        column: position.column,
    }
}

fn terminal_mouse_button(button: MouseButton) -> Option<TerminalMouseButton> {
    match button {
        MouseButton::Left => Some(TerminalMouseButton::Left),
        MouseButton::Middle => Some(TerminalMouseButton::Middle),
        MouseButton::Right => Some(TerminalMouseButton::Right),
        MouseButton::Navigate(_) => None,
    }
}

fn terminal_modifiers(modifiers: GpuiModifiers) -> TerminalModifiers {
    TerminalModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        control: modifiers.control,
        platform: modifiers.platform,
    }
}

fn wheel_steps(delta: f32) -> u16 {
    if delta == 0. {
        0
    } else {
        delta.abs().round().max(1.) as u16
    }
}

fn accumulated_wheel_steps(remainder: &mut f32, delta: f32) -> f32 {
    if delta == 0. {
        return 0.;
    }
    if *remainder != 0. && remainder.signum() != delta.signum() {
        *remainder = 0.;
    }
    *remainder += delta;
    let steps = remainder.trunc();
    *remainder -= steps;
    steps
}

fn terminal_grid_extent(extent: Pixels, cell: Pixels, minimum: u16) -> u16 {
    ((extent / cell).next_up().floor() as u16).max(minimum)
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
    fn block_elements_map_to_cell_filling_regions() {
        let color: Hsla = rgb(0xff7b72).into();
        let regions_for = |character: char| {
            let mut regions = Vec::new();
            assert!(
                collect_block_element_regions(&mut regions, character, 1, 2, color),
                "{character} should be handled as a block element"
            );
            regions
        };
        let region = |start_line: i64, start_col: i64, end_line: i64, end_col: i64| BlockRegion {
            start_line: 24 + start_line,
            start_col: 16 + start_col,
            end_line: 24 + end_line,
            end_col: 16 + end_col,
            color,
        };

        // The full block covers the whole cell, line height included.
        assert_eq!(regions_for('\u{2588}'), [region(0, 0, 23, 7)]);
        // The upper half block starts at the cell top, not at the font baseline.
        assert_eq!(regions_for('\u{2580}'), [region(0, 0, 11, 7)]);
        // The lower quarter block sits flush with the cell bottom.
        assert_eq!(regions_for('\u{2582}'), [region(18, 0, 23, 7)]);
        // The left one-eighth block hugs the left edge.
        assert_eq!(regions_for('\u{258F}'), [region(0, 0, 23, 0)]);
        // ▚ fills exactly its upper-left and lower-right quadrants.
        assert_eq!(
            regions_for('\u{259A}'),
            [region(0, 0, 11, 3), region(12, 4, 23, 7)]
        );
        // ░ approximates its stipple with a translucent full-cell fill.
        assert_eq!(regions_for('\u{2591}')[0].color, color.opacity(0.25));

        // U+1FB00 BLOCK SEXTANT-1 fills only the top-left 2x3 subcell.
        assert_eq!(regions_for('\u{1FB00}'), [region(0, 0, 7, 3)]);
        // U+1FB14 BLOCK SEXTANT-235 straddles the enumeration gap left by ▌.
        assert_eq!(
            regions_for('\u{1FB14}'),
            [
                region(0, 4, 7, 7),
                region(8, 0, 15, 3),
                region(16, 0, 23, 3)
            ]
        );
        // The last sextant fills everything but the top-left subcell.
        let last = regions_for('\u{1FB3B}');
        assert_eq!(last.len(), 5);
        assert!(
            !last.contains(&region(0, 0, 7, 3)),
            "the top-left subcell must stay empty"
        );

        let mut regions = Vec::new();
        assert!(
            !collect_block_element_regions(&mut regions, 'x', 0, 0, color),
            "ordinary text must keep going through font shaping"
        );
        assert!(
            !collect_block_element_regions(&mut regions, '\u{1FB3C}', 0, 0, color),
            "code points after the sextant range must not be treated as sextants"
        );
        assert!(regions.is_empty());
    }

    #[test]
    fn adjacent_block_regions_merge_into_larger_rectangles() {
        let orange: Hsla = rgb(0xff7b72).into();
        let blue: Hsla = rgb(0x58a6ff).into();
        let mut regions = Vec::new();
        // A row of three full blocks, a stacked full block below the first,
        // and a differently colored block at the end of the row.
        for column in 0..3 {
            collect_block_element_regions(&mut regions, '\u{2588}', 0, column, orange);
        }
        collect_block_element_regions(&mut regions, '\u{2588}', 0, 3, blue);
        collect_block_element_regions(&mut regions, '\u{2588}', 1, 0, orange);

        let merged = merge_block_regions(regions);
        assert_eq!(
            merged,
            [
                BlockRegion {
                    start_line: 0,
                    start_col: 0,
                    end_line: 23,
                    end_col: 23,
                    color: orange,
                },
                BlockRegion {
                    start_line: 0,
                    start_col: 24,
                    end_line: 23,
                    end_col: 31,
                    color: blue,
                },
                BlockRegion {
                    start_line: 24,
                    start_col: 0,
                    end_line: 47,
                    end_col: 7,
                    color: orange,
                },
            ]
        );

        // Two ▀ over two ▄ of the same color merge into one full-width band
        // via the quadratic fixpoint pass.
        let mut regions = Vec::new();
        collect_block_element_regions(&mut regions, '\u{2584}', 0, 0, orange);
        collect_block_element_regions(&mut regions, '\u{2584}', 0, 1, orange);
        collect_block_element_regions(&mut regions, '\u{2580}', 1, 0, orange);
        collect_block_element_regions(&mut regions, '\u{2580}', 1, 1, orange);
        assert_eq!(
            merge_block_regions(regions),
            [BlockRegion {
                start_line: 12,
                start_col: 0,
                end_line: 35,
                end_col: 15,
                color: orange,
            }]
        );
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

    #[test]
    fn terminal_grid_keeps_two_columns_at_narrow_widths() {
        assert_eq!(terminal_grid_extent(px(1.), px(10.), 2), 2);
        assert_eq!(terminal_grid_extent(px(20.), px(10.), 2), 2);
        assert_eq!(terminal_grid_extent(px(30.), px(10.), 2), 3);
        assert_eq!(terminal_grid_extent(px(1.), px(10.), 1), 1);
    }

    #[test]
    fn pixel_scroll_accumulates_whole_lines_and_drops_stale_direction() {
        let mut remainder = 0.;
        assert_eq!(accumulated_wheel_steps(&mut remainder, 0.4), 0.);
        assert_eq!(accumulated_wheel_steps(&mut remainder, 0.4), 0.);
        assert_eq!(accumulated_wheel_steps(&mut remainder, 0.4), 1.);
        assert!((remainder - 0.2).abs() < f32::EPSILON * 4.);

        assert_eq!(accumulated_wheel_steps(&mut remainder, -0.6), 0.);
        assert!((remainder + 0.6).abs() < f32::EPSILON * 4.);
    }
}
