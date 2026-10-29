//! The render state: a snapshot of the viewport's rows, cells and cursor, with which rows
//! changed since it was last marked clean. One per terminal, since an update consumes the
//! terminal's own dirty marks.

use super::{Error, Rgb, Terminal, check, sys};
use std::mem::{MaybeUninit, size_of};
use std::ptr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dirty {
    Clean,
    /// Some rows changed; each says whether it did.
    Partial,
    /// Every row must be read again.
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorStyle {
    Bar,
    Block,
    Underline,
    BlockHollow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    /// Column and row in the viewport, or `None` when the cursor is scrolled out of it.
    pub position: Option<(u16, u16)>,
    pub visible: bool,
    pub blinking: bool,
    pub style: CursorStyle,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Palette(u8),
    Rgb(Rgb),
}

impl Color {
    fn from_sys(color: sys::GhosttyStyleColor) -> Self {
        // SAFETY: the tag says which member of the union is set.
        unsafe {
            match color.tag {
                sys::GhosttyStyleColorTag::GHOSTTY_STYLE_COLOR_PALETTE => {
                    Self::Palette(color.value.palette)
                }
                sys::GhosttyStyleColorTag::GHOSTTY_STYLE_COLOR_RGB => {
                    Self::Rgb(Rgb::from_sys(color.value.rgb))
                }
                _ => Self::Default,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Style {
    pub foreground: Color,
    pub background: Color,
    pub underline_color: Color,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

impl Style {
    fn from_sys(style: &sys::GhosttyStyle) -> Self {
        let underline = match sys::GhosttySgrUnderline(style.underline) {
            sys::GhosttySgrUnderline::GHOSTTY_SGR_UNDERLINE_SINGLE => Underline::Single,
            sys::GhosttySgrUnderline::GHOSTTY_SGR_UNDERLINE_DOUBLE => Underline::Double,
            sys::GhosttySgrUnderline::GHOSTTY_SGR_UNDERLINE_CURLY => Underline::Curly,
            sys::GhosttySgrUnderline::GHOSTTY_SGR_UNDERLINE_DOTTED => Underline::Dotted,
            sys::GhosttySgrUnderline::GHOSTTY_SGR_UNDERLINE_DASHED => Underline::Dashed,
            _ => Underline::None,
        };
        Self {
            foreground: Color::from_sys(style.fg_color),
            background: Color::from_sys(style.bg_color),
            underline_color: Color::from_sys(style.underline_color),
            bold: style.bold,
            italic: style.italic,
            faint: style.faint,
            blink: style.blink,
            inverse: style.inverse,
            invisible: style.invisible,
            strikethrough: style.strikethrough,
            overline: style.overline,
            underline,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellWidth {
    Narrow,
    Wide,
    /// The second cell of a wide character.
    SpacerTail,
    /// The last cell of a row a wide character wrapped away from.
    SpacerHead,
}

/// One viewport cell, borrowed for the duration of a visit.
#[derive(Clone, Copy, Debug)]
pub struct Cell<'a> {
    pub x: u16,
    /// Its grapheme cluster, empty when it holds no text.
    pub text: &'a str,
    pub style: Style,
    pub width: CellWidth,
    pub hyperlink: bool,
}

pub struct RenderState {
    raw: sys::GhosttyRenderState,
    rows: sys::GhosttyRenderStateRowIterator,
    cells: sys::GhosttyRenderStateRowCells,
    text: Vec<u8>,
}

// SAFETY: as for `Terminal`: the render state is only touched through `&mut self` or
// `&self`, kept beside its terminal under one mutex.
unsafe impl Send for RenderState {}

impl RenderState {
    pub fn new() -> Result<Self, Error> {
        let mut state = Self {
            raw: ptr::null_mut(),
            rows: ptr::null_mut(),
            cells: ptr::null_mut(),
            text: vec![0; 64],
        };
        // SAFETY: default allocator; a handle is set only by a call that succeeded, and
        // `Drop` frees whichever were created.
        unsafe {
            check(sys::ghostty_render_state_new(ptr::null(), &mut state.raw))?;
            check(sys::ghostty_render_state_row_iterator_new(
                ptr::null(),
                &mut state.rows,
            ))?;
            check(sys::ghostty_render_state_row_cells_new(
                ptr::null(),
                &mut state.cells,
            ))?;
        }
        Ok(state)
    }

    /// Takes the terminal's current viewport and what changed since the last update.
    pub fn update(&mut self, terminal: &Terminal) -> Result<(), Error> {
        // SAFETY: both live; the caller holds the terminal's lock, so nothing else runs on it.
        check(unsafe { sys::ghostty_render_state_update(self.raw, terminal.raw) })
    }

    fn get<T>(&self, data: sys::GhosttyRenderStateData, out: &mut T) -> Result<(), Error> {
        // SAFETY: each caller passes the output type the header names for its data.
        check(unsafe { sys::ghostty_render_state_get(self.raw, data, (out as *mut T).cast()) })
    }

    pub fn dirty(&self) -> Dirty {
        let mut dirty = sys::GhosttyRenderStateDirty::GHOSTTY_RENDER_STATE_DIRTY_FULL;
        let _ = self.get(
            sys::GhosttyRenderStateData::GHOSTTY_RENDER_STATE_DATA_DIRTY,
            &mut dirty,
        );
        match dirty {
            sys::GhosttyRenderStateDirty::GHOSTTY_RENDER_STATE_DIRTY_FALSE => Dirty::Clean,
            sys::GhosttyRenderStateDirty::GHOSTTY_RENDER_STATE_DIRTY_PARTIAL => Dirty::Partial,
            _ => Dirty::Full,
        }
    }

    /// Marks the state read; rows a later update touches turn dirty again.
    pub fn mark_clean(&mut self) {
        let clean = sys::GhosttyRenderStateDirty::GHOSTTY_RENDER_STATE_DIRTY_FALSE;
        // SAFETY: the option takes a `GhosttyRenderStateDirty`.
        let _ = unsafe {
            sys::ghostty_render_state_set(
                self.raw,
                sys::GhosttyRenderStateOption::GHOSTTY_RENDER_STATE_OPTION_DIRTY,
                (&raw const clean).cast(),
            )
        };
    }

    pub fn cursor(&self) -> Cursor {
        let cursor = MaybeUninit::<sys::GhosttyRenderStateCursor>::zeroed();
        // SAFETY: a zeroed sized struct is a valid out value the call fills.
        let mut cursor = unsafe { cursor.assume_init() };
        cursor.size = size_of::<sys::GhosttyRenderStateCursor>();
        let _ = self.get(
            sys::GhosttyRenderStateData::GHOSTTY_RENDER_STATE_DATA_CURSOR,
            &mut cursor,
        );
        use sys::GhosttyRenderStateCursorVisualStyle as Visual;
        Cursor {
            position: cursor
                .viewport_has_value
                .then_some((cursor.viewport_x, cursor.viewport_y)),
            visible: cursor.visible,
            blinking: cursor.blinking,
            style: match cursor.visual_style {
                Visual::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK => CursorStyle::Block,
                Visual::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE => {
                    CursorStyle::Underline
                }
                Visual::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW => {
                    CursorStyle::BlockHollow
                }
                _ => CursorStyle::Bar,
            },
        }
    }

    /// Visits the viewport's rows from the top.
    pub fn for_each_row(&mut self, mut visit: impl FnMut(&mut Row<'_>)) -> Result<(), Error> {
        let mut rows = self.rows;
        self.get(
            sys::GhosttyRenderStateData::GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
            &mut rows,
        )?;
        let mut y = 0;
        // SAFETY: the iterator was just populated from this state.
        while unsafe { sys::ghostty_render_state_row_iterator_next(self.rows) } {
            visit(&mut Row {
                rows: self.rows,
                cells: self.cells,
                text: &mut self.text,
                y,
            });
            y += 1;
        }
        Ok(())
    }
}

impl Drop for RenderState {
    fn drop(&mut self) {
        // SAFETY: created in `new` (a null handle is ignored) and freed only here.
        unsafe {
            if !self.cells.is_null() {
                sys::ghostty_render_state_row_cells_free(self.cells);
            }
            if !self.rows.is_null() {
                sys::ghostty_render_state_row_iterator_free(self.rows);
            }
            if !self.raw.is_null() {
                sys::ghostty_render_state_free(self.raw);
            }
        }
    }
}

/// The row a visit is at.
pub struct Row<'a> {
    rows: sys::GhosttyRenderStateRowIterator,
    cells: sys::GhosttyRenderStateRowCells,
    text: &'a mut Vec<u8>,
    pub y: u16,
}

impl Row<'_> {
    fn get<T>(&self, data: sys::GhosttyRenderStateRowData, out: &mut T) -> Result<(), Error> {
        // SAFETY: the iterator is at this row; each caller passes the header's output type.
        check(unsafe { sys::ghostty_render_state_row_get(self.rows, data, (out as *mut T).cast()) })
    }

    pub fn dirty(&self) -> bool {
        let mut dirty = true;
        let _ = self.get(
            sys::GhosttyRenderStateRowData::GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY,
            &mut dirty,
        );
        dirty
    }

    pub fn mark_clean(&mut self) {
        let clean = false;
        // SAFETY: the option takes a `bool`.
        let _ = unsafe {
            sys::ghostty_render_state_row_set(
                self.rows,
                sys::GhosttyRenderStateRowOption::GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY,
                (&raw const clean).cast(),
            )
        };
    }

    /// Whether the row's text continues on the next row (a soft wrap).
    pub fn wrapped(&self) -> bool {
        let mut row: sys::GhosttyRow = 0;
        let mut wrapped = false;
        if self
            .get(
                sys::GhosttyRenderStateRowData::GHOSTTY_RENDER_STATE_ROW_DATA_RAW,
                &mut row,
            )
            .is_ok()
        {
            // SAFETY: a row value the state just gave, read for a `bool`.
            let _ = unsafe {
                sys::ghostty_row_get(
                    row,
                    sys::GhosttyRowData::GHOSTTY_ROW_DATA_WRAP,
                    (&raw mut wrapped).cast(),
                )
            };
        }
        wrapped
    }

    /// The selected columns of this row, inclusive.
    pub fn selection(&self) -> Option<(u16, u16)> {
        let mut selection = sys::GhosttyRenderStateRowSelection {
            size: size_of::<sys::GhosttyRenderStateRowSelection>(),
            start_x: 0,
            end_x: 0,
        };
        self.get(
            sys::GhosttyRenderStateRowData::GHOSTTY_RENDER_STATE_ROW_DATA_SELECTION,
            &mut selection,
        )
        .ok()?;
        Some((selection.start_x, selection.end_x))
    }

    /// Visits the row's cells from the left.
    pub fn for_each_cell(&mut self, mut visit: impl FnMut(Cell<'_>)) -> Result<(), Error> {
        let mut cells = self.cells;
        self.get(
            sys::GhosttyRenderStateRowData::GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
            &mut cells,
        )?;
        let mut x = 0;
        // SAFETY: the cells were just populated from this row.
        while unsafe { sys::ghostty_render_state_row_cells_next(cells) } {
            let cell = read_cell(cells, self.text);
            visit(Cell {
                x,
                text: cell.0,
                style: cell.1,
                width: cell.2,
                hyperlink: cell.3,
            });
            x += 1;
        }
        Ok(())
    }
}

fn cells_get<T>(
    cells: sys::GhosttyRenderStateRowCells,
    data: sys::GhosttyRenderStateRowCellsData,
    out: &mut T,
) -> bool {
    // SAFETY: the cells are at a cell; each caller passes the header's output type.
    check(unsafe { sys::ghostty_render_state_row_cells_get(cells, data, (out as *mut T).cast()) })
        .is_ok()
}

fn read_cell(
    cells: sys::GhosttyRenderStateRowCells,
    text: &mut Vec<u8>,
) -> (&str, Style, CellWidth, bool) {
    use sys::GhosttyRenderStateRowCellsData as Data;
    let mut raw: sys::GhosttyCell = 0;
    let (mut width, mut hyperlink) = (sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_NARROW, false);
    let mut content = sys::GhosttyCellContentTag::GHOSTTY_CELL_CONTENT_CODEPOINT;
    if cells_get(
        cells,
        Data::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
        &mut raw,
    ) {
        // SAFETY: a cell value the state just gave, each read for its header type.
        unsafe {
            sys::ghostty_cell_get(
                raw,
                sys::GhosttyCellData::GHOSTTY_CELL_DATA_WIDE,
                (&raw mut width).cast(),
            );
            sys::ghostty_cell_get(
                raw,
                sys::GhosttyCellData::GHOSTTY_CELL_DATA_HAS_HYPERLINK,
                (&raw mut hyperlink).cast(),
            );
            sys::ghostty_cell_get(
                raw,
                sys::GhosttyCellData::GHOSTTY_CELL_DATA_CONTENT_TAG,
                (&raw mut content).cast(),
            );
        }
    }
    let mut style = Style::default();
    let mut styled = false;
    if cells_get(
        cells,
        Data::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_HAS_STYLING,
        &mut styled,
    ) && styled
    {
        let raw_style = MaybeUninit::<sys::GhosttyStyle>::zeroed();
        // SAFETY: a zeroed sized struct is a valid out value the call fills.
        let mut raw_style = unsafe { raw_style.assume_init() };
        raw_style.size = size_of::<sys::GhosttyStyle>();
        if cells_get(
            cells,
            Data::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
            &mut raw_style,
        ) {
            style = Style::from_sys(&raw_style);
        }
    }
    // A cell with no text that only paints a background (an erase in a coloured region)
    // keeps that colour in its content rather than a style.
    match content {
        sys::GhosttyCellContentTag::GHOSTTY_CELL_CONTENT_BG_COLOR_PALETTE => {
            let mut index: sys::GhosttyColorPaletteIndex = 0;
            // SAFETY: as above, for a palette index.
            unsafe {
                sys::ghostty_cell_get(
                    raw,
                    sys::GhosttyCellData::GHOSTTY_CELL_DATA_COLOR_PALETTE,
                    (&raw mut index).cast(),
                )
            };
            style.background = Color::Palette(index);
        }
        sys::GhosttyCellContentTag::GHOSTTY_CELL_CONTENT_BG_COLOR_RGB => {
            let mut rgb = sys::GhosttyColorRgb { r: 0, g: 0, b: 0 };
            // SAFETY: as above, for an RGB value.
            unsafe {
                sys::ghostty_cell_get(
                    raw,
                    sys::GhosttyCellData::GHOSTTY_CELL_DATA_COLOR_RGB,
                    (&raw mut rgb).cast(),
                )
            };
            style.background = Color::Rgb(Rgb::from_sys(rgb));
        }
        _ => {}
    }
    let len = loop {
        let mut buffer = sys::GhosttyBuffer {
            ptr: text.as_mut_ptr(),
            cap: text.len(),
            len: 0,
        };
        // SAFETY: the buffer describes `text`, which outlives the call.
        let result = unsafe {
            sys::ghostty_render_state_row_cells_get(
                cells,
                Data::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
                (&raw mut buffer).cast(),
            )
        };
        if result == sys::GhosttyResult::GHOSTTY_OUT_OF_SPACE && buffer.len > text.len() {
            text.resize(buffer.len, 0);
            continue;
        }
        break if check(result).is_ok() { buffer.len } else { 0 };
    };
    let width = match width {
        sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_WIDE => CellWidth::Wide,
        sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_SPACER_TAIL => CellWidth::SpacerTail,
        sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_SPACER_HEAD => CellWidth::SpacerHead,
        _ => CellWidth::Narrow,
    };
    (
        std::str::from_utf8(&text[..len]).unwrap_or(""),
        style,
        width,
        hyperlink,
    )
}
