//! The VT behind every Pane: libghostty-vt (ADR 0051), in Condr's terms. It parses what
//! the program writes, holds frames through a synchronized update, and turns the
//! library's render state into the `TerminalView` the wire carries.

use super::view::blank_cell;
use super::*;
use condr_ghostty_vt::{self as ghostty, CellWidth, Color as VtColor, Event, Point, Underline};
use smol_str::SmolStr;
use std::sync::LazyLock;

/// How long a synchronized update (mode 2026) may hold frames before the Server shows
/// what the program drew anyway, so a program that dies inside one cannot freeze its
/// Pane (ADR 0051).
pub(super) const RENDER_HOLD_TIMEOUT: Duration = Duration::from_secs(1);
/// Ghostty's default: history kept in the library's logical bytes, compressed when idle.
const SCROLLBACK_BYTES: usize = 50_000_000;
/// The most wheel steps one event reports, so a flung wheel cannot flood the program.
pub(super) const MAX_MOUSE_WHEEL_STEPS: u16 = 64;
/// alacritty's set without `:`, so a double-click keeps URLs and Windows paths whole.
const WORD_BOUNDARIES: &str = ",│`|\"' ()[]{}<>\t";

/// The modes that route input between the program and the Pane (scrolling, selection),
/// as flags. The library's encoders read the rest themselves.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TermMode(u32);

impl TermMode {
    pub(super) const APP_CURSOR: Self = Self(1);
    pub(super) const ALT_SCREEN: Self = Self(1 << 1);
    pub(super) const ALTERNATE_SCROLL: Self = Self(1 << 2);
    pub(super) const MOUSE_REPORT_CLICK: Self = Self(1 << 3);
    pub(super) const MOUSE_DRAG: Self = Self(1 << 4);
    pub(super) const MOUSE_MOTION: Self = Self(1 << 5);
    pub(super) const MOUSE_MODE: Self =
        Self(Self::MOUSE_REPORT_CLICK.0 | Self::MOUSE_DRAG.0 | Self::MOUSE_MOTION.0);

    pub(super) const fn empty() -> Self {
        Self(0)
    }

    pub(super) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub(super) const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for TermMode {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for TermMode {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

pub(super) struct Vt {
    terminal: ghostty::Terminal,
    render: ghostty::RenderState,
    keys: ghostty::KeyEncoder,
    mouse: ghostty::MouseEncoder,
    /// When the synchronized update now open began.
    pub(super) held_since: Option<Instant>,
    /// The library's activity token at the last idle check.
    compression_seen: Option<u64>,
}

fn vt_colors(colors: &TerminalColors) -> ghostty::Colors {
    ghostty::Colors {
        foreground: rgb(colors.foreground),
        background: rgb(colors.background),
        cursor: rgb(colors.cursor),
        palette: std::array::from_fn(|index| rgb(colors.indexed(index as u8))),
    }
}

fn rgb(value: u32) -> ghostty::Rgb {
    ghostty::Rgb::from_u32(value)
}

impl Vt {
    pub(super) fn new(size: TerminalSize) -> io::Result<Self> {
        let terminal = ghostty::Terminal::new(ghostty::Options {
            cols: size.columns.max(1),
            rows: size.rows.max(1),
            cell_width: u32::from(size.cell_width),
            cell_height: u32::from(size.cell_height),
            scrollback_bytes: SCROLLBACK_BYTES,
            version: format!("Condr {}", env!("CARGO_PKG_VERSION")),
            terminfo_name: "xterm-256color".into(),
            // ConPTY keeps its own screen and does not pull history back on a resize.
            resize_pulls_scrollback: !cfg!(windows),
            colors: vt_colors(&TerminalColors::default()),
        })
        .map_err(other_error)?;
        let render = ghostty::RenderState::new().map_err(other_error)?;
        let keys = ghostty::KeyEncoder::new().map_err(other_error)?;
        let mouse = ghostty::MouseEncoder::new().map_err(other_error)?;
        Ok(Self {
            terminal,
            render,
            keys,
            mouse,
            held_since: None,
            compression_seen: None,
        })
    }

    /// Parses program output and returns what the program reported meanwhile, a
    /// synchronized update's start and end kept here.
    pub(super) fn write(&mut self, bytes: &[u8]) -> Vec<Event> {
        let events = self.terminal.write(bytes);
        self.keep_holds(events)
    }

    fn keep_holds(&mut self, mut events: Vec<Event>) -> Vec<Event> {
        events.retain(|event| match event {
            Event::RenderHold(held) => {
                self.held_since = held.then(Instant::now);
                false
            }
            _ => true,
        });
        events
    }

    /// How long frames stay held for an open synchronized update, if one is.
    pub(super) fn hold_remaining(&self) -> Option<Duration> {
        let remaining = RENDER_HOLD_TIMEOUT.checked_sub(self.held_since?.elapsed())?;
        (!remaining.is_zero()).then_some(remaining)
    }

    /// Answers colour queries with `colors` from now on. Returns the report a program
    /// that subscribed to scheme changes (mode 2031) reads when dark turned light or back.
    pub(super) fn set_colors(&mut self, colors: &TerminalColors) -> Option<Vec<u8>> {
        self.terminal.set_colors(&vt_colors(colors)).ok().flatten()
    }

    /// Resizes the grid and returns what that reported, such as the in-band size report
    /// a program that turned on mode 2048 reads.
    pub(super) fn resize(&mut self, size: TerminalSize) -> Vec<Event> {
        let events = self
            .terminal
            .resize(
                size.columns.max(1),
                size.rows.max(1),
                u32::from(size.cell_width),
                u32::from(size.cell_height),
            )
            .unwrap_or_default();
        self.keep_holds(events)
    }

    pub(super) fn mode(&self, number: u16) -> bool {
        self.terminal.mode(ghostty::Mode::dec(number))
    }

    pub(super) fn modes(&self) -> TermMode {
        let mut modes = TermMode::empty();
        for (number, flag) in [
            (1, TermMode::APP_CURSOR),
            (9, TermMode::MOUSE_REPORT_CLICK),
            (1000, TermMode::MOUSE_REPORT_CLICK),
            (1002, TermMode::MOUSE_DRAG),
            (1003, TermMode::MOUSE_MOTION),
            (1007, TermMode::ALTERNATE_SCROLL),
        ] {
            if self.mode(number) {
                modes |= flag;
            }
        }
        if self.terminal.alternate_screen() {
            modes |= TermMode::ALT_SCREEN;
        }
        modes
    }

    pub(super) fn mouse_tracking(&self) -> TerminalMouseTracking {
        if self.mode(1003) {
            TerminalMouseTracking::Motion
        } else if self.mode(1002) {
            TerminalMouseTracking::Drag
        } else if self.mode(1000) || self.mode(9) {
            TerminalMouseTracking::Click
        } else {
            TerminalMouseTracking::None
        }
    }

    pub(super) fn screen_lines(&self) -> usize {
        usize::from(self.terminal.rows())
    }

    #[cfg(test)]
    pub(super) fn columns(&self) -> usize {
        usize::from(self.terminal.cols())
    }

    /// Rows of history between the viewport's bottom and the active area's.
    pub(super) fn display_offset(&self) -> usize {
        let bar = self.terminal.scrollbar();
        usize::try_from(bar.total.saturating_sub(bar.offset + bar.len)).unwrap_or(usize::MAX)
    }

    /// Scrolls the viewport and reports whether it moved.
    pub(super) fn scroll(&mut self, scroll: TerminalScroll) -> bool {
        let before = self.terminal.scrollbar().offset;
        let page = self.terminal.rows() as isize;
        self.terminal.scroll_viewport(match scroll {
            TerminalScroll::Lines(lines) => ghostty::Scroll::Delta(-(lines as isize)),
            TerminalScroll::PageUp => ghostty::Scroll::Delta(-page),
            TerminalScroll::PageDown => ghostty::Scroll::Delta(page),
            TerminalScroll::Top => ghostty::Scroll::Top,
            TerminalScroll::Bottom => ghostty::Scroll::Bottom,
        });
        self.terminal.scrollbar().offset != before
    }

    /// Moves the viewport by shell prompts (OSC 133) and says whether it moved, or `None`
    /// when the cursor is not at a marked prompt, so the key belongs to the program.
    pub(super) fn scroll_prompts(&mut self, prompts: isize) -> Option<bool> {
        if !self.terminal.cursor_at_prompt() {
            return None;
        }
        let before = self.terminal.scrollbar().offset;
        self.terminal
            .scroll_viewport(ghostty::Scroll::Prompts(prompts));
        Some(self.terminal.scrollbar().offset != before)
    }

    /// A viewport position as the Client saw it, at its display offset, in screen rows.
    fn screen_point(&self, position: TerminalPosition, display_offset: u32) -> Point {
        let bar = self.terminal.scrollbar();
        let rows = self.terminal.rows().max(1);
        let columns = self.terminal.cols().max(1);
        let top = bar
            .total
            .saturating_sub(bar.len + u64::from(display_offset));
        Point::Screen {
            x: position.column.min(columns - 1),
            y: u32::try_from(top + u64::from(position.row.min(rows - 1))).unwrap_or(u32::MAX),
        }
    }

    fn selection_range(&self, selection: TerminalSelection) -> Option<(Point, Point)> {
        let columns = self.terminal.cols().max(1);
        let (start, end) = selection.selected_cell_range(columns)?;
        let position = |cell: u32| TerminalPosition {
            row: u16::try_from(cell / u32::from(columns)).unwrap_or(u16::MAX),
            column: (cell % u32::from(columns)) as u16,
            side: TerminalSide::Left,
        };
        Some((
            self.screen_point(position(start), selection.display_offset),
            self.screen_point(position(end), selection.display_offset),
        ))
    }

    /// Hands the selection to the library, which keeps it on its text as output scrolls
    /// and the screen reflows (ADR 0008, ADR 0051).
    pub(super) fn select(&mut self, selection: Option<TerminalSelection>) {
        let range = selection.and_then(|selection| self.selection_range(selection));
        self.terminal.select(range);
    }

    /// Clears the selection and reports whether there was one.
    pub(super) fn clear_selection(&mut self) -> bool {
        let had = self.terminal.has_selection();
        if had {
            self.terminal.select(None);
        }
        had
    }

    /// A word or line selection anchored at one cell, across soft wraps.
    pub(super) fn select_at(
        &mut self,
        position: TerminalPosition,
        display_offset: u32,
        unit: TerminalSelectionUnit,
    ) {
        static BOUNDARIES: LazyLock<Vec<u32>> =
            LazyLock::new(|| WORD_BOUNDARIES.chars().map(u32::from).collect());
        let point = self.screen_point(position, display_offset);
        match unit {
            TerminalSelectionUnit::Word => self.terminal.select_word(point, &BOUNDARIES),
            TerminalSelectionUnit::Line => self.terminal.select_line(point),
        }
    }

    /// The selected text as Ghostty copies it, or `selection`'s when given.
    pub(super) fn selection_text(&self, selection: Option<TerminalSelection>) -> Option<String> {
        match selection {
            Some(selection) => self
                .terminal
                .selection_text(Some(self.selection_range(selection)?), true),
            None => self.terminal.selection_text(None, true),
        }
    }

    /// The last `lines` rows that end at the last row with any content, reaching into
    /// scrollback when the screen alone holds fewer, trailing blanks trimmed.
    pub(super) fn recent_text(&self, lines: usize) -> String {
        let bar = self.terminal.scrollbar();
        let Some(last) = bar.total.checked_sub(1) else {
            return String::new();
        };
        let wanted = u64::try_from(lines)
            .unwrap_or(u64::MAX)
            .saturating_add(bar.len);
        let range = (
            Point::Screen {
                x: 0,
                y: u32::try_from(last.saturating_sub(wanted)).unwrap_or(0),
            },
            Point::Screen {
                x: self.terminal.cols().saturating_sub(1),
                y: u32::try_from(last).unwrap_or(u32::MAX),
            },
        );
        let text = self
            .terminal
            .selection_text(Some(range), false)
            .unwrap_or_default();
        let rows: Vec<&str> = text.trim_end_matches('\n').lines().collect();
        rows[rows.len().saturating_sub(lines)..].join("\n")
    }

    /// What a key sends to the program for the live modes: libghostty-vt encodes it once
    /// the program negotiated kitty keyboard flags, Condr's legacy encoder otherwise (ADR
    /// 0051). Cmd+C reaches a program that reads kitty keys without having asked in kitty
    /// form, which legacy encoding would drop.
    pub(super) fn encode_key(
        &mut self,
        key: &TerminalKey,
        modifiers: TerminalModifiers,
        kind: TerminalKeyEventKind,
        reads_kitty_keys: bool,
    ) -> io::Result<Vec<u8>> {
        let cmd = TerminalModifiers {
            platform: true,
            ..TerminalModifiers::default()
        };
        let cmd_c = modifiers == cmd && matches!(key, TerminalKey::Character(text) if text == "c");
        let kitty = self.terminal.kitty_keyboard_flags() != 0;
        let kitty_flags = (!kitty && cmd_c && reads_kitty_keys).then_some(KITTY_DISAMBIGUATE);
        // A program that negotiated a keyboard (kitty flags or xterm's modifyOtherKeys)
        // reads keys as Ghostty encodes them; any other reads the classic xterm bytes
        // Condr sends, since Ghostty's own legacy encoding follows fixterms (ADR 0051).
        if !kitty && kitty_flags.is_none() && !self.keys.modify_other_keys(&self.terminal) {
            return encode_legacy_key(key, modifiers, kind, self.mode(1));
        }
        let mut mods = ghostty::Mods {
            shift: modifiers.shift,
            ctrl: modifiers.control,
            alt: modifiers.alt,
            logo: modifiers.platform,
        };
        let (physical, text, unshifted, consumed) = match key {
            TerminalKey::Character(text) => typed_key(text, modifiers.shift),
            TerminalKey::BackTab => {
                mods.shift = true;
                (
                    ghostty::PhysicalKey::Tab,
                    "",
                    None,
                    ghostty::Mods::default(),
                )
            }
            named => (named_key(named), "", None, ghostty::Mods::default()),
        };
        let action = match kind {
            TerminalKeyEventKind::Press => ghostty::KeyAction::Press,
            TerminalKeyEventKind::Repeat => ghostty::KeyAction::Repeat,
            TerminalKeyEventKind::Release => ghostty::KeyAction::Release,
        };
        let event = ghostty::KeyEvent {
            key: physical,
            action,
            mods,
            consumed,
            text,
            unshifted,
        };
        Ok(self.keys.encode(&self.terminal, &event, kitty_flags))
    }

    /// A mouse event as the program's tracking mode and format report it, if they do.
    pub(super) fn encode_mouse(&mut self, event: TerminalMouseEvent) -> Option<Vec<u8>> {
        let (action, button, position, modifiers, repeat) = match event {
            TerminalMouseEvent::Button {
                button,
                pressed,
                position,
                modifiers,
            } => (
                if pressed {
                    ghostty::MouseAction::Press
                } else {
                    ghostty::MouseAction::Release
                },
                Some(mouse_button(button)),
                position,
                modifiers,
                1,
            ),
            TerminalMouseEvent::Motion {
                button,
                position,
                modifiers,
            } => (
                ghostty::MouseAction::Motion,
                button.map(mouse_button),
                position,
                modifiers,
                1,
            ),
            TerminalMouseEvent::Wheel {
                direction,
                amount,
                position,
                modifiers,
            } => (
                ghostty::MouseAction::Press,
                Some(match direction {
                    TerminalMouseWheel::Up => ghostty::MouseButton::WheelUp,
                    TerminalMouseWheel::Down => ghostty::MouseButton::WheelDown,
                    TerminalMouseWheel::Left => ghostty::MouseButton::WheelLeft,
                    TerminalMouseWheel::Right => ghostty::MouseButton::WheelRight,
                }),
                position,
                modifiers,
                amount.min(MAX_MOUSE_WHEEL_STEPS),
            ),
        };
        let report = self.mouse.encode(
            &self.terminal,
            &ghostty::MouseEvent {
                action,
                button,
                mods: ghostty::Mods {
                    shift: modifiers.shift,
                    ctrl: modifiers.control,
                    alt: modifiers.alt,
                    logo: modifiers.platform,
                },
                column: position.column,
                row: position.row,
            },
        );
        (!report.is_empty() && repeat > 0).then(|| report.repeat(usize::from(repeat)))
    }

    /// Pasted text as the program reads it. A CRLF counts as one line break, so a Windows
    /// clipboard does not press Enter twice outside bracketed paste.
    pub(super) fn encode_paste(&self, text: &str) -> Vec<u8> {
        ghostty::paste(&text.replace("\r\n", "\n"), self.mode(2004))
    }

    /// What the program reads when its Pane gains or loses focus, if it asked (mode 1004).
    pub(super) fn focus_report(&self, focused: bool) -> Option<Vec<u8>> {
        self.mode(1004).then(|| ghostty::focus_report(focused))
    }

    /// Typed text. While every key must be reported as an escape code, a single character
    /// goes out as its key; longer commits, such as IME input, stay plain text.
    pub(super) fn encode_text(&mut self, text: &str) -> Vec<u8> {
        let mut chars = text.chars();
        let single = matches!((chars.next(), chars.next()), (Some(_), None));
        if !single || self.terminal.kitty_keyboard_flags() & KITTY_REPORT_ALL == 0 {
            return text.as_bytes().to_vec();
        }
        let (physical, text, unshifted, consumed) = typed_key(text, true);
        let event = ghostty::KeyEvent {
            key: physical,
            action: ghostty::KeyAction::Press,
            mods: consumed,
            consumed,
            text,
            unshifted,
        };
        self.keys.encode(&self.terminal, &event, None)
    }

    /// Where the cursor sits in the viewport and whether it shows, read without an update
    /// of the render state, for the PTY reader's cursor settling.
    pub(super) fn cursor_position(&self, size: TerminalSize) -> Option<TerminalCursor> {
        let (column, row) = self.terminal.cursor();
        let row = row.checked_add(u16::try_from(self.display_offset()).ok()?)?;
        (row < size.rows && column < size.columns).then_some(TerminalCursor {
            row,
            column,
            shape: if self.mode(25) {
                TerminalCursorShape::Block
            } else {
                TerminalCursorShape::Hidden
            },
            blinking: false,
        })
    }

    /// The viewport as the wire carries it. With `previous`, the frame path, the marks are
    /// cleared once read, and only rows the library marks changed are read again unless
    /// the whole state is marked or the size moved; without, every row is read and the
    /// marks are left for the frame path, the render state's one consumer.
    pub(super) fn view(
        &mut self,
        size: TerminalSize,
        revision: u64,
        previous: Option<&TerminalView>,
    ) -> TerminalView {
        let _ = self.render.update(&self.terminal);
        let clearing = previous.is_some();
        let previous = previous.filter(|previous| {
            previous.size == size && self.render.dirty() != ghostty::Dirty::Full
        });
        let narrowing = previous.is_some();
        let columns = usize::from(size.columns);
        let mut cells = previous.map_or_else(
            || vec![blank_cell(); usize::from(size.rows) * columns],
            |previous| previous.cells.clone(),
        );
        let (palette, default_palette) = self.terminal.palette();
        let colors = Colors {
            palette,
            default_palette,
        };
        let mut selected = Vec::new();
        let mut linked = Vec::new();
        let _ = self.render.for_each_row(|row| {
            let y = usize::from(row.y);
            if y >= usize::from(size.rows) {
                return;
            }
            if let Some(range) = row.selection() {
                selected.push((row.y, range));
            }
            if narrowing && !row.dirty() {
                return;
            }
            let wrapped = row.wrapped();
            let row_y = row.y;
            let start = y * columns;
            let _ = row.for_each_cell(|cell| {
                let x = usize::from(cell.x);
                if x >= columns {
                    return;
                }
                let mut mapped = terminal_cell(&cell, &colors);
                if wrapped && x + 1 == columns {
                    mapped.flags |= CELL_WRAPPED;
                }
                if cell.hyperlink {
                    linked.push((start + x, cell.x, row_y));
                }
                cells[start + x] = mapped;
            });
            if clearing {
                row.mark_clean();
            }
        });
        if clearing {
            self.render.mark_clean();
        }
        for (index, x, y) in linked {
            let uri = self
                .terminal
                .hyperlink(Point::Viewport { x, y: u32::from(y) });
            cells[index].hyperlink = uri.map(SmolStr::from);
        }
        let display_offset = u32::try_from(self.display_offset()).unwrap_or(u32::MAX);
        let mut view = TerminalView {
            revision,
            size,
            display_offset,
            mouse_tracking: self.mouse_tracking(),
            cells,
            cursor: self.cursor(size),
            selection: self.viewport_selection(&selected, size, display_offset),
        };
        detect_links(&mut view);
        let mut hyperlinks = Hyperlinks::default();
        for cell in &mut view.cells {
            cell.hyperlink = cell
                .hyperlink
                .take()
                .and_then(|uri| hyperlinks.intern(&uri));
        }
        view
    }

    fn cursor(&self, size: TerminalSize) -> Option<TerminalCursor> {
        let cursor = self.render.cursor();
        let (column, row) = cursor.position?;
        (row < size.rows && column < size.columns).then_some(TerminalCursor {
            row,
            column,
            shape: if !cursor.visible {
                TerminalCursorShape::Hidden
            } else {
                match cursor.style {
                    ghostty::CursorStyle::Block => TerminalCursorShape::Block,
                    ghostty::CursorStyle::Underline => TerminalCursorShape::Underline,
                    ghostty::CursorStyle::Bar => TerminalCursorShape::Beam,
                    ghostty::CursorStyle::BlockHollow => TerminalCursorShape::HollowBlock,
                }
            },
            blinking: cursor.blinking,
        })
    }

    /// The selection as inclusive viewport cells, or `None` when there is none. One that
    /// exists but is entirely scrolled out of view is reported one row past the grid
    /// (`row == size.rows`): it matches no cell, yet the Client knows a copy would work.
    fn viewport_selection(
        &self,
        selected: &[(u16, (u16, u16))],
        size: TerminalSize,
        display_offset: u32,
    ) -> Option<TerminalSelection> {
        let last_column = size.columns.checked_sub(1)?;
        let position = |row, column, side| TerminalPosition {
            row,
            column: u16::min(column, last_column),
            side,
        };
        match (selected.first(), selected.last()) {
            (Some(&(first, (start, _))), Some(&(last, (_, end)))) => Some(TerminalSelection {
                start: position(first, start, TerminalSide::Left),
                end: position(last, end, TerminalSide::Right),
                display_offset,
            }),
            _ if self.terminal.has_selection() => Some(TerminalSelection {
                start: position(size.rows, 0, TerminalSide::Left),
                end: position(size.rows, last_column, TerminalSide::Right),
                display_offset,
            }),
            _ => None,
        }
    }

    /// The screen as plain rows, for tests and `visible_text`.
    pub(super) fn visible_text(&mut self, size: TerminalSize) -> String {
        let view = self.view(size, 0, None);
        let mut lines: Vec<String> = (0..size.rows)
            .map(|row| {
                let text: String = (0..size.columns)
                    .filter_map(|column| view.cell(row, column))
                    .filter(|cell| !cell.is_spacer())
                    .map(|cell| {
                        if cell.flags & CELL_INVISIBLE != 0 {
                            " "
                        } else {
                            cell.text.as_str()
                        }
                    })
                    .collect();
                text.trim_end_matches(' ').to_owned()
            })
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// One bounded step of compressing the scrollback, once no output has arrived since
    /// the last look; `true` while more steps remain and the terminal is still idle.
    pub(super) fn compress_step_if_idle(&mut self) -> bool {
        let activity = self.terminal.compression_activity();
        let idle = self.compression_seen == Some(activity);
        self.compression_seen = Some(activity);
        idle && !self.terminal.compress()
    }

    #[cfg(test)]
    pub(super) fn render_dirty(&self) -> ghostty::Dirty {
        self.render.dirty()
    }

    #[cfg(test)]
    pub(super) fn resident_bytes(&self) -> u64 {
        self.terminal.resident_bytes()
    }
}

/// The links one view carries: each URI stored once and shared by every cell it covers,
/// a URI over its own limit dropped, and the view's URIs together kept within the frame's
/// budget, the text staying either way.
#[derive(Default)]
pub(super) struct Hyperlinks {
    values: std::collections::HashSet<SmolStr>,
    pub(super) bytes: usize,
}

impl Hyperlinks {
    pub(super) fn intern(&mut self, uri: &str) -> Option<SmolStr> {
        if uri.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES {
            return None;
        }
        if let Some(uri) = self.values.get(uri) {
            return Some(uri.clone());
        }
        let bytes = self.bytes.checked_add(uri.len())?;
        if bytes > MAX_TERMINAL_HYPERLINK_BYTES {
            return None;
        }
        let uri = SmolStr::new(uri);
        self.bytes = bytes;
        self.values.insert(uri.clone());
        Some(uri)
    }
}

/// Kitty keyboard flags: disambiguate escape codes, and report every key as one.
const KITTY_DISAMBIGUATE: u8 = 1;
const KITTY_REPORT_ALL: u8 = 8;

/// The US-layout key that types `text`, and whether Shift made it: the protocol carries
/// text and named keys, not physical keys, so the Server derives one (ADR 0051).
fn typed_key(
    text: &str,
    shift_held: bool,
) -> (ghostty::PhysicalKey, &str, Option<char>, ghostty::Mods) {
    let mut chars = text.chars();
    let (Some(typed), None) = (chars.next(), chars.next()) else {
        return (
            ghostty::PhysicalKey::Unidentified,
            text,
            None,
            ghostty::Mods::default(),
        );
    };
    const SHIFTED: [(char, char); 21] = [
        ('~', '`'),
        ('!', '1'),
        ('@', '2'),
        ('#', '3'),
        ('$', '4'),
        ('%', '5'),
        ('^', '6'),
        ('&', '7'),
        ('*', '8'),
        ('(', '9'),
        (')', '0'),
        ('_', '-'),
        ('+', '='),
        ('{', '['),
        ('}', ']'),
        ('|', '\\'),
        (':', ';'),
        ('"', '\''),
        ('<', ','),
        ('>', '.'),
        ('?', '/'),
    ];
    let (base, shifted) = if typed.is_ascii_uppercase() {
        (typed.to_ascii_lowercase(), true)
    } else if let Some(&(_, base)) = SHIFTED.iter().find(|(shifted, _)| *shifted == typed) {
        (base, true)
    } else {
        (typed, false)
    };
    let physical = if base.is_ascii_graphic() || base == ' ' {
        ghostty::PhysicalKey::Char(base)
    } else {
        ghostty::PhysicalKey::Unidentified
    };
    let consumed = ghostty::Mods {
        shift: shifted && shift_held,
        ..ghostty::Mods::default()
    };
    (physical, text, Some(base), consumed)
}

fn mouse_button(button: TerminalMouseButton) -> ghostty::MouseButton {
    match button {
        TerminalMouseButton::Left => ghostty::MouseButton::Left,
        TerminalMouseButton::Middle => ghostty::MouseButton::Middle,
        TerminalMouseButton::Right => ghostty::MouseButton::Right,
    }
}

fn named_key(key: &TerminalKey) -> ghostty::PhysicalKey {
    match key {
        TerminalKey::Enter => ghostty::PhysicalKey::Enter,
        TerminalKey::Tab | TerminalKey::BackTab => ghostty::PhysicalKey::Tab,
        TerminalKey::Backspace => ghostty::PhysicalKey::Backspace,
        TerminalKey::Delete => ghostty::PhysicalKey::Delete,
        TerminalKey::Escape => ghostty::PhysicalKey::Escape,
        TerminalKey::Up => ghostty::PhysicalKey::Up,
        TerminalKey::Down => ghostty::PhysicalKey::Down,
        TerminalKey::Right => ghostty::PhysicalKey::Right,
        TerminalKey::Left => ghostty::PhysicalKey::Left,
        TerminalKey::Home => ghostty::PhysicalKey::Home,
        TerminalKey::End => ghostty::PhysicalKey::End,
        TerminalKey::PageUp => ghostty::PhysicalKey::PageUp,
        TerminalKey::PageDown => ghostty::PhysicalKey::PageDown,
        TerminalKey::Insert => ghostty::PhysicalKey::Insert,
        TerminalKey::Function(number) => ghostty::PhysicalKey::Function(*number),
        TerminalKey::Character(_) => ghostty::PhysicalKey::Unidentified,
    }
}

/// The palette as the program left it and as the terminal started: an entry a program
/// changed (OSC 4) goes out as RGB, an untouched one as its index for the Client's palette.
struct Colors {
    palette: [ghostty::Rgb; 256],
    default_palette: [ghostty::Rgb; 256],
}

impl Colors {
    fn color(&self, color: VtColor) -> TerminalColor {
        match color {
            VtColor::Default => TerminalColor::Default,
            VtColor::Palette(index)
                if self.palette[usize::from(index)] == self.default_palette[usize::from(index)] =>
            {
                TerminalColor::Indexed(index)
            }
            VtColor::Palette(index) => rgb_color(self.palette[usize::from(index)]),
            VtColor::Rgb(rgb) => rgb_color(rgb),
        }
    }
}

fn rgb_color(rgb: ghostty::Rgb) -> TerminalColor {
    TerminalColor::Rgb {
        red: rgb.r,
        green: rgb.g,
        blue: rgb.b,
    }
}

fn terminal_cell(cell: &ghostty::Cell<'_>, colors: &Colors) -> TerminalCell {
    let style = cell.style;
    let mut flags = match cell.width {
        CellWidth::Narrow => 0,
        CellWidth::Wide => CELL_WIDE,
        CellWidth::SpacerTail => CELL_SPACER_TAIL,
        CellWidth::SpacerHead => CELL_SPACER_HEAD,
    };
    flags |= match style.underline {
        Underline::None => 0,
        Underline::Single => CELL_UNDERLINE_SINGLE,
        Underline::Double => CELL_UNDERLINE_DOUBLE,
        Underline::Curly => CELL_UNDERLINE_CURLY,
        Underline::Dotted => CELL_UNDERLINE_DOTTED,
        Underline::Dashed => CELL_UNDERLINE_DASHED,
    };
    for (set, bit) in [
        (style.bold, CELL_BOLD),
        (style.italic, CELL_ITALIC),
        (style.faint, CELL_FAINT),
        (style.blink, CELL_BLINK),
        (style.inverse, CELL_INVERSE),
        (style.invisible, CELL_INVISIBLE),
        (style.strikethrough, CELL_STRIKETHROUGH),
        (style.overline, CELL_OVERLINE),
    ] {
        if set {
            flags |= bit;
        }
    }
    TerminalCell {
        text: cell_text(cell.text),
        foreground: colors.color(style.foreground),
        background: colors.color(style.background),
        underline_color: colors.color(style.underline_color),
        flags,
        hyperlink: None,
    }
}

/// The cell's grapheme cluster, a space when it holds none, cut at a character boundary
/// within the wire's limit.
pub(super) fn cell_text(text: &str) -> SmolStr {
    if text.is_empty() {
        return SmolStr::new_static(" ");
    }
    let mut end = text.len().min(MAX_TERMINAL_CELL_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    SmolStr::new(&text[..end])
}

/// Lowercase throughout, matched case-insensitively. A URL runs from a known scheme to
/// whitespace or a quote/bracket; [`trimmed_url_len`] drops what prose adds.
const URL_PATTERN: &str = r#"(?i)(https?://|file://|ftp://|git://|gemini://|gopher://|zed://|ipfs:|ipns:|magnet:|mailto:|news:|ssh:)[^\s"'<>`]+"#;

/// Marks plain-text URLs in the view: soft-wrapped rows are joined, wide characters'
/// spacers stay linked so the run has no holes, and an OSC 8 link the program set wins.
fn detect_links(view: &mut TerminalView) {
    static URL: LazyLock<regex_automata::meta::Regex> = LazyLock::new(|| {
        regex_automata::meta::Regex::new(URL_PATTERN).expect("the URL pattern compiles")
    });
    let columns = usize::from(view.size.columns);
    let rows = usize::from(view.size.rows);
    // Rows kept from the last frame still carry the links found then.
    for cell in &mut view.cells {
        if cell.flags & DETECTED_LINK_FLAG != 0 {
            cell.flags &= !DETECTED_LINK_FLAG;
            cell.hyperlink = None;
        }
    }
    let mut row = 0;
    while row < rows {
        // One logical line: this row and every row its predecessor wrapped into.
        let first = row;
        while row + 1 < rows && view.cells[(row + 1) * columns - 1].flags & CELL_WRAPPED != 0 {
            row += 1;
        }
        let cells = first * columns..(row + 1) * columns;
        row += 1;
        let mut text = String::new();
        // Each character's cell, and each cell's character when it is not a spacer.
        let mut owners = Vec::new();
        for index in cells.clone() {
            let cell = &view.cells[index];
            if !cell.is_spacer() {
                let character = cell.text.chars().next().unwrap_or(' ');
                owners.push((text.len(), index));
                text.push(character);
            }
        }
        for found in URL.find_iter(text.as_bytes()) {
            let start = owners.partition_point(|&(byte, _)| byte < found.start());
            let end = owners.partition_point(|&(byte, _)| byte < found.end());
            let chars: Vec<char> = text[found.range()].chars().collect();
            let keep = trimmed_url_len(&chars);
            if keep == 0 {
                continue;
            }
            let uri = SmolStr::from(chars[..keep].iter().collect::<String>());
            let first_cell = owners[start].1;
            let last_cell = owners
                .get(start + keep)
                .map_or(owners[end - 1].1 + 1, |&(_, cell)| cell);
            for index in first_cell..last_cell.min(cells.end) {
                let cell = &mut view.cells[index];
                if cell.hyperlink.is_none() {
                    cell.hyperlink = Some(uri.clone());
                    cell.flags |= DETECTED_LINK_FLAG;
                }
            }
        }
    }
}

/// The URL's length without trailing punctuation and closing brackets that have no opener:
/// prose puts those after a URL far more often than a URL ends with them.
pub(super) fn trimmed_url_len(chars: &[char]) -> usize {
    let mut end = chars.len();
    loop {
        match chars[..end].last() {
            Some('.' | ',' | ';' | ':' | '!' | '?') => end -= 1,
            Some(&close @ (')' | ']' | '}')) => {
                let open = match close {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                let span = &chars[..end];
                let count = |wanted| span.iter().filter(|&&c| c == wanted).count();
                if count(open) < count(close) {
                    end -= 1;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    end
}
