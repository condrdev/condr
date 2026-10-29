//! libghostty-vt, Ghostty's terminal core, for the Server's terminals (ADR 0051): the
//! bindings script/update-libghostty-vt.sh generates and a thin safe layer over the part
//! Condr calls. Every `unsafe` the VT needs lives in this crate.
#![allow(unsafe_code)]

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::all
)]
mod bindings;
mod key;
mod render;

pub use key::{KeyAction, KeyEncoder, KeyEvent, Mods, PhysicalKey};
pub use render::{
    Cell, CellWidth, Color, Cursor, CursorStyle, Dirty, RenderState, Row, Style, Underline,
};

use bindings as sys;
use std::ffi::c_void;
use std::mem::{MaybeUninit, size_of};
use std::panic::{self, AssertUnwindSafe};
use std::{fmt, ptr, slice};

/// What a libghostty-vt call that did not succeed returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error(i32);

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match sys::GhosttyResult(self.0) {
            sys::GhosttyResult::GHOSTTY_OUT_OF_MEMORY => "out of memory",
            sys::GhosttyResult::GHOSTTY_INVALID_VALUE => "invalid value",
            sys::GhosttyResult::GHOSTTY_OUT_OF_SPACE => "buffer too small",
            sys::GhosttyResult::GHOSTTY_NO_VALUE => "no value",
            sys::GhosttyResult::GHOSTTY_IO_ERROR => "I/O error",
            sys::GhosttyResult::GHOSTTY_LIMIT_EXCEEDED => "limit exceeded",
            sys::GhosttyResult::GHOSTTY_REJECTED => "rejected",
            _ => "unknown result",
        };
        write!(formatter, "libghostty-vt: {reason} ({})", self.0)
    }
}

impl std::error::Error for Error {}

fn check(result: sys::GhosttyResult) -> Result<(), Error> {
    if result == sys::GhosttyResult::GHOSTTY_SUCCESS {
        Ok(())
    } else {
        Err(Error(result.0))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn from_u32(rgb: u32) -> Self {
        Self {
            r: (rgb >> 16) as u8,
            g: (rgb >> 8) as u8,
            b: rgb as u8,
        }
    }

    pub const fn to_u32(self) -> u32 {
        (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    fn from_sys(rgb: sys::GhosttyColorRgb) -> Self {
        Self {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
        }
    }

    fn to_sys(self) -> sys::GhosttyColorRgb {
        sys::GhosttyColorRgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

/// A DEC private mode (`?n`) or an ANSI one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mode(u16);

impl Mode {
    pub const fn dec(number: u16) -> Self {
        Self(number & 0x7fff)
    }

    pub const fn ansi(number: u16) -> Self {
        Self(number & 0x7fff | 1 << 15)
    }
}

/// How a terminal starts, and what it answers about itself.
pub struct Options {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u32,
    pub cell_height: u32,
    /// Scrollback kept per screen, in the library's logical bytes.
    pub scrollback_bytes: usize,
    /// The XTVERSION answer.
    pub version: String,
    /// The name XTGETTCAP answers for `TN`, matching `TERM`.
    pub terminfo_name: String,
    /// Whether a resize pulls scrollback back onto the screen; ConPTY keeps its own.
    pub resize_pulls_scrollback: bool,
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub palette: [Rgb; 256],
}

/// What the terminal reported while it parsed a write, in stream order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// An answer to a query the program sent, for its PTY.
    WritePty(Vec<u8>),
    Bell,
    TitleChanged(String),
    /// The working directory a program reported (OSC 7, 9;9 or 1337), as it sent it.
    PwdChanged(String),
    /// Text a program copied (OSC 52 and the like).
    ClipboardWrite(String),
    Notification {
        title: String,
        body: String,
    },
    ProgramStatus(ProgramStatus),
    /// OSC 133 started a primary prompt.
    PromptStart,
    /// RIS reset the terminal.
    Reset,
    /// A synchronized update (mode 2026) began or ended.
    RenderHold(bool),
}

/// One OSC 7501 report, as the library parsed it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramStatus {
    pub state: ProgramState,
    pub kind: Option<BlockedKind>,
    pub id: String,
    pub app: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    Clear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockedKind {
    Permission,
    Question,
    Auth,
}

/// A cell named the way the library addresses it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Point {
    /// From the top of the viewport.
    Viewport { x: u16, y: u32 },
    /// From the top of the scrollback.
    Screen { x: u16, y: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scroll {
    Top,
    Bottom,
    /// Rows down (positive) or up (negative).
    Delta(isize),
}

/// Where the viewport sits in the screen's rows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Scrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

/// The state the terminal's callbacks write into while it parses. The library only asks
/// that calls on a terminal be serialized, so they run on the thread inside `write`.
struct Callbacks {
    events: Vec<Event>,
    size: sys::GhosttySizeReportSize,
    version: Vec<u8>,
    panics: usize,
}

/// One terminal: its screens, scrollback and modes, fed the bytes its program writes.
pub struct Terminal {
    raw: sys::GhosttyTerminal,
    /// Owned through a raw pointer, since the library holds it for the terminal's life.
    callbacks: *mut Callbacks,
}

// SAFETY: libghostty-vt asks only that calls on one terminal be serialized, and every
// method takes `&self` or `&mut self` on a terminal Condr keeps behind a mutex. Ghostty
// itself shares one locked terminal between its IO and render threads.
unsafe impl Send for Terminal {}

impl Terminal {
    pub fn new(options: Options) -> Result<Self, Error> {
        let mut raw = ptr::null_mut();
        // SAFETY: a null allocator is the library's default; `raw` is read only once the
        // call succeeded.
        check(unsafe {
            sys::ghostty_terminal_new(ptr::null(), &mut raw, options.cols, options.rows)
        })?;
        let callbacks = Box::into_raw(Box::new(Callbacks {
            events: Vec::new(),
            size: sys::GhosttySizeReportSize {
                rows: options.rows,
                columns: options.cols,
                cell_width: options.cell_width,
                cell_height: options.cell_height,
            },
            version: options.version.as_bytes().to_vec(),
            panics: 0,
        }));
        let terminal = Self { raw, callbacks };
        terminal.install(&options)?;
        Ok(terminal)
    }

    fn set(&self, option: sys::GhosttyTerminalOption, value: *const c_void) -> Result<(), Error> {
        // SAFETY: each caller passes the value type the header names for its option.
        check(unsafe { sys::ghostty_terminal_set(self.raw, option, value) })
    }

    fn install(&self, options: &Options) -> Result<(), Error> {
        use sys::GhosttyTerminalOption as Opt;
        self.set(Opt::GHOSTTY_TERMINAL_OPT_USERDATA, self.callbacks.cast())?;
        let callbacks: [(Opt, *const c_void); 13] = [
            (
                Opt::GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                write_pty as *const c_void,
            ),
            (Opt::GHOSTTY_TERMINAL_OPT_BELL, bell as *const c_void),
            (
                Opt::GHOSTTY_TERMINAL_OPT_TITLE_CHANGED,
                title_changed as *const c_void,
            ),
            (
                Opt::GHOSTTY_TERMINAL_OPT_PWD_CHANGED,
                pwd_changed as *const c_void,
            ),
            (
                Opt::GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE,
                clipboard_write as *const c_void,
            ),
            (
                Opt::GHOSTTY_TERMINAL_OPT_DESKTOP_NOTIFICATION,
                desktop_notification as *const c_void,
            ),
            (
                Opt::GHOSTTY_TERMINAL_OPT_PROGRAM_STATUS,
                program_status as *const c_void,
            ),
            (
                Opt::GHOSTTY_TERMINAL_OPT_SEMANTIC_PROMPT,
                semantic_prompt as *const c_void,
            ),
            (Opt::GHOSTTY_TERMINAL_OPT_RESET, reset as *const c_void),
            (
                Opt::GHOSTTY_TERMINAL_OPT_RENDER_HOLD,
                render_hold as *const c_void,
            ),
            (Opt::GHOSTTY_TERMINAL_OPT_SIZE, size_report as *const c_void),
            (
                Opt::GHOSTTY_TERMINAL_OPT_DEVICE_ATTRIBUTES,
                device_attributes as *const c_void,
            ),
            (
                Opt::GHOSTTY_TERMINAL_OPT_XTVERSION,
                xtversion as *const c_void,
            ),
        ];
        for (option, callback) in callbacks {
            self.set(option, callback)?;
        }
        let name = sys::GhosttyString {
            ptr: options.terminfo_name.as_ptr(),
            len: options.terminfo_name.len(),
        };
        // The library copies the name, so it need not outlive the call.
        self.set(
            Opt::GHOSTTY_TERMINAL_OPT_TERMINFO_NAME,
            (&raw const name).cast(),
        )?;
        self.set(
            Opt::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
            (&raw const options.scrollback_bytes).cast(),
        )?;
        self.set(
            Opt::GHOSTTY_TERMINAL_OPT_RESIZE_PULL_SCROLLBACK,
            (&raw const options.resize_pulls_scrollback).cast(),
        )?;
        for (option, color) in [
            (
                Opt::GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND,
                options.foreground,
            ),
            (
                Opt::GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND,
                options.background,
            ),
            (Opt::GHOSTTY_TERMINAL_OPT_COLOR_CURSOR, options.cursor),
        ] {
            let color = color.to_sys();
            self.set(option, (&raw const color).cast())?;
        }
        let palette = options.palette.map(Rgb::to_sys);
        self.set(
            Opt::GHOSTTY_TERMINAL_OPT_COLOR_PALETTE,
            (&raw const palette).cast(),
        )?;
        // Graphemes measured as Unicode clusters by default, as Ghostty does (mode 2027).
        let grapheme = sys::GhosttyTerminalModeConfig {
            mode: Mode::dec(2027).0,
            value: true,
        };
        self.set(
            Opt::GHOSTTY_TERMINAL_OPT_MODE_DEFAULT,
            (&raw const grapheme).cast(),
        )?;
        self.set(Opt::GHOSTTY_TERMINAL_OPT_MODE, (&raw const grapheme).cast())
    }

    /// Parses output the program wrote to its PTY and returns what it reported meanwhile.
    pub fn write(&mut self, bytes: &[u8]) -> Vec<Event> {
        // SAFETY: the terminal is live, `&mut self` serializes calls on it, and the
        // pointer and length describe `bytes`.
        unsafe { sys::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
        std::mem::take(&mut self.callbacks_mut().events)
    }

    fn callbacks_mut(&mut self) -> &mut Callbacks {
        // SAFETY: allocated in `new`, freed only in `drop`, and the library touches it only
        // inside calls that `&mut self` already excludes here.
        unsafe { &mut *self.callbacks }
    }

    /// Callbacks whose panic was caught and answered with nothing, since creation.
    pub fn callback_panics(&self) -> usize {
        // SAFETY: as in `callbacks_mut`, read outside any library call.
        unsafe { (*self.callbacks).panics }
    }

    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<(), Error> {
        self.callbacks_mut().size = sys::GhosttySizeReportSize {
            rows,
            columns: cols,
            cell_width,
            cell_height,
        };
        // SAFETY: a live terminal; `&mut self` serializes the call.
        check(unsafe {
            sys::ghostty_terminal_resize(self.raw, cols, rows, cell_width, cell_height)
        })
    }

    fn get<T>(&self, data: sys::GhosttyTerminalData, out: &mut T) -> Result<(), Error> {
        // SAFETY: each caller passes the output type the header names for its data.
        check(unsafe { sys::ghostty_terminal_get(self.raw, data, (out as *mut T).cast()) })
    }

    fn get_value<T: Default>(&self, data: sys::GhosttyTerminalData) -> T {
        let mut value = T::default();
        let _ = self.get(data, &mut value);
        value
    }

    fn get_string(&self, data: sys::GhosttyTerminalData) -> String {
        let mut string = sys::GhosttyString {
            ptr: ptr::null(),
            len: 0,
        };
        if self.get(data, &mut string).is_err() {
            return String::new();
        }
        // SAFETY: the library returned a borrowed string, read before the next call.
        unsafe { string_lossy(string) }
    }

    pub fn cols(&self) -> u16 {
        self.get_value(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_COLS)
    }

    pub fn rows(&self) -> u16 {
        self.get_value(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_ROWS)
    }

    /// The cursor in the active area, from its top left.
    pub fn cursor(&self) -> (u16, u16) {
        (
            self.get_value(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_CURSOR_X),
            self.get_value(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_CURSOR_Y),
        )
    }

    pub fn mode(&self, mode: Mode) -> bool {
        let mut config = sys::GhosttyTerminalModeConfig {
            mode: mode.0,
            value: false,
        };
        self.get(
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_MODE,
            &mut config,
        )
        .is_ok()
            && config.value
    }

    /// The kitty keyboard protocol flags the program pushed.
    pub fn kitty_keyboard_flags(&self) -> u8 {
        self.get_value(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS)
    }

    pub fn alternate_screen(&self) -> bool {
        let mut screen = sys::GhosttyTerminalScreen::GHOSTTY_TERMINAL_SCREEN_PRIMARY;
        let _ = self.get(
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN,
            &mut screen,
        );
        screen == sys::GhosttyTerminalScreen::GHOSTTY_TERMINAL_SCREEN_ALTERNATE
    }

    pub fn scrollbar(&self) -> Scrollbar {
        let mut bar = sys::GhosttyTerminalScrollbar {
            total: 0,
            offset: 0,
            len: 0,
        };
        let _ = self.get(
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_SCROLLBAR,
            &mut bar,
        );
        Scrollbar {
            total: bar.total,
            offset: bar.offset,
            len: bar.len,
        }
    }

    pub fn title(&self) -> String {
        self.get_string(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_TITLE)
    }

    /// The palette as the program left it (OSC 4) and as the terminal started.
    pub fn palette(&self) -> ([Rgb; 256], [Rgb; 256]) {
        let read = |data| {
            let mut palette = [sys::GhosttyColorRgb { r: 0, g: 0, b: 0 }; 256];
            let _ = self.get(data, &mut palette);
            palette.map(Rgb::from_sys)
        };
        (
            read(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_COLOR_PALETTE),
            read(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_COLOR_PALETTE_DEFAULT),
        )
    }

    pub fn scroll_viewport(&mut self, scroll: Scroll) {
        let (tag, delta) = match scroll {
            Scroll::Top => (
                sys::GhosttyTerminalScrollViewportTag::GHOSTTY_SCROLL_VIEWPORT_TOP,
                0,
            ),
            Scroll::Bottom => (
                sys::GhosttyTerminalScrollViewportTag::GHOSTTY_SCROLL_VIEWPORT_BOTTOM,
                0,
            ),
            Scroll::Delta(delta) => (
                sys::GhosttyTerminalScrollViewportTag::GHOSTTY_SCROLL_VIEWPORT_DELTA,
                delta,
            ),
        };
        let behavior = sys::GhosttyTerminalScrollViewport {
            tag,
            value: sys::GhosttyTerminalScrollViewportValue { delta },
        };
        // SAFETY: a live terminal; `&mut self` serializes the call.
        unsafe { sys::ghostty_terminal_scroll_viewport(self.raw, behavior) }
    }

    fn grid_ref(&self, point: Point) -> Option<sys::GhosttyGridRef> {
        let (tag, x, y) = match point {
            Point::Viewport { x, y } => (sys::GhosttyPointTag::GHOSTTY_POINT_TAG_VIEWPORT, x, y),
            Point::Screen { x, y } => (sys::GhosttyPointTag::GHOSTTY_POINT_TAG_SCREEN, x, y),
        };
        let point = sys::GhosttyPoint {
            tag,
            value: sys::GhosttyPointValue {
                coordinate: sys::GhosttyPointCoordinate { x, y },
            },
        };
        let grid_ref = MaybeUninit::<sys::GhosttyGridRef>::zeroed();
        // SAFETY: `grid_ref` is a sized struct the call fills; read only on success.
        let mut grid_ref = unsafe { grid_ref.assume_init() };
        grid_ref.size = size_of::<sys::GhosttyGridRef>();
        // SAFETY: a live terminal and a writable out pointer.
        check(unsafe { sys::ghostty_terminal_grid_ref(self.raw, point, &mut grid_ref) }).ok()?;
        Some(grid_ref)
    }

    fn selection_between(&self, start: Point, end: Point) -> Option<sys::GhosttySelection> {
        Some(sys::GhosttySelection {
            size: size_of::<sys::GhosttySelection>(),
            start: self.grid_ref(start)?,
            end: self.grid_ref(end)?,
            rectangle: false,
        })
    }

    /// Selects from `start` to `end`, inclusive, or clears the selection. The library
    /// keeps it on its text as output scrolls and the screen reflows.
    pub fn select(&mut self, range: Option<(Point, Point)>) {
        let selection = range.and_then(|(start, end)| self.selection_between(start, end));
        let value = selection.as_ref().map_or(ptr::null(), ptr::from_ref);
        let _ = self.set(
            sys::GhosttyTerminalOption::GHOSTTY_TERMINAL_OPT_SELECTION,
            value.cast(),
        );
    }

    pub fn has_selection(&self) -> bool {
        let selection = MaybeUninit::<sys::GhosttySelection>::zeroed();
        // SAFETY: a zeroed sized struct is a valid out value the call may fill.
        let mut selection = unsafe { selection.assume_init() };
        selection.size = size_of::<sys::GhosttySelection>();
        self.get(
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_SELECTION,
            &mut selection,
        )
        .is_ok()
    }

    /// Selects the word under `point`, its edges at `boundaries` or whitespace.
    pub fn select_word(&mut self, point: Point, boundaries: &[u32]) {
        let Some(grid_ref) = self.grid_ref(point) else {
            return;
        };
        let options = sys::GhosttyTerminalSelectWordOptions {
            size: size_of::<sys::GhosttyTerminalSelectWordOptions>(),
            ref_: grid_ref,
            // An empty list as a null pointer: the library's default boundaries.
            boundary_codepoints: if boundaries.is_empty() {
                ptr::null()
            } else {
                boundaries.as_ptr()
            },
            boundary_codepoints_len: boundaries.len(),
        };
        self.select_with(|raw, out| {
            // SAFETY: a live terminal, options that outlive the call and a writable out.
            unsafe { sys::ghostty_terminal_select_word(raw, &options, out) }
        });
    }

    /// Selects the line under `point`, following soft wraps.
    pub fn select_line(&mut self, point: Point) {
        let Some(grid_ref) = self.grid_ref(point) else {
            return;
        };
        let options = sys::GhosttyTerminalSelectLineOptions {
            size: size_of::<sys::GhosttyTerminalSelectLineOptions>(),
            ref_: grid_ref,
            whitespace: ptr::null(),
            whitespace_len: 0,
            semantic_prompt_boundary: false,
        };
        self.select_with(|raw, out| {
            // SAFETY: as in `select_word`.
            unsafe { sys::ghostty_terminal_select_line(raw, &options, out) }
        });
    }

    fn select_with(
        &mut self,
        find: impl FnOnce(sys::GhosttyTerminal, *mut sys::GhosttySelection) -> sys::GhosttyResult,
    ) {
        let selection = MaybeUninit::<sys::GhosttySelection>::zeroed();
        // SAFETY: a zeroed sized struct is a valid out value the call may fill.
        let mut selection = unsafe { selection.assume_init() };
        selection.size = size_of::<sys::GhosttySelection>();
        if check(find(self.raw, &mut selection)).is_ok() {
            let _ = self.set(
                sys::GhosttyTerminalOption::GHOSTTY_TERMINAL_OPT_SELECTION,
                (&raw const selection).cast(),
            );
        }
    }

    /// The selected text, trailing blanks trimmed, `range` instead of the current
    /// selection when given. `unwrap` joins soft-wrapped rows, as Ghostty copies.
    pub fn selection_text(&self, range: Option<(Point, Point)>, unwrap: bool) -> Option<String> {
        let selection = match range {
            Some((start, end)) => Some(self.selection_between(start, end)?),
            None if self.has_selection() => None,
            None => return None,
        };
        let options = sys::GhosttyTerminalSelectionFormatOptions {
            size: size_of::<sys::GhosttyTerminalSelectionFormatOptions>(),
            emit: sys::GhosttyFormatterFormat::GHOSTTY_FORMATTER_FORMAT_PLAIN,
            unwrap,
            trim: true,
            selection: selection.as_ref().map_or(ptr::null(), ptr::from_ref),
        };
        let (mut text, mut len) = (ptr::null_mut(), 0);
        // SAFETY: a live terminal; `text` and `len` are read only once the call succeeded.
        check(unsafe {
            sys::ghostty_terminal_selection_format_alloc(
                self.raw,
                ptr::null(),
                options,
                &mut text,
                &mut len,
            )
        })
        .ok()?;
        // SAFETY: the library allocated `len` bytes at `text` with the default allocator.
        Some(unsafe { take_allocated(text, len) })
    }

    /// The URI of the OSC 8 link at `point`, if it has one.
    pub fn hyperlink(&self, point: Point) -> Option<String> {
        let grid_ref = self.grid_ref(point)?;
        let mut buffer = vec![0u8; 512];
        loop {
            let mut len = 0;
            // SAFETY: the reference is fresh and the buffer is as long as said.
            let result = unsafe {
                sys::ghostty_grid_ref_hyperlink_uri(
                    &grid_ref,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut len,
                )
            };
            if result == sys::GhosttyResult::GHOSTTY_OUT_OF_SPACE && len > buffer.len() {
                buffer.resize(len, 0);
                continue;
            }
            check(result).ok()?;
            buffer.truncate(len);
            return (!buffer.is_empty()).then(|| String::from_utf8_lossy(&buffer).into_owned());
        }
    }

    /// Changes since the last call mean the terminal was busy; compression waits for quiet.
    pub fn compression_activity(&self) -> u64 {
        let mut activity = 0;
        // SAFETY: a live terminal and a writable out value.
        let _ = unsafe { sys::ghostty_terminal_compression_activity(self.raw, &mut activity) };
        activity
    }

    /// Compresses idle scrollback; `true` when nothing is left to do.
    pub fn compress(&mut self) -> bool {
        let mut result =
            sys::GhosttyTerminalCompressionResult::GHOSTTY_TERMINAL_COMPRESSION_RESULT_UNSUPPORTED;
        // SAFETY: a live terminal; `&mut self` serializes the call.
        let _ = unsafe {
            sys::ghostty_terminal_compress(
                self.raw,
                sys::GhosttyTerminalCompressionMode::GHOSTTY_TERMINAL_COMPRESSION_MODE_INCREMENTAL,
                &mut result,
            )
        };
        result != sys::GhosttyTerminalCompressionResult::GHOSTTY_TERMINAL_COMPRESSION_RESULT_PENDING
    }

    /// The screen as plain text, soft-wrapped lines joined and trailing blanks trimmed.
    pub fn plain_text(&self) -> Result<String, Error> {
        let options = sys::GhosttyFormatterTerminalOptions {
            size: size_of::<sys::GhosttyFormatterTerminalOptions>(),
            emit: sys::GhosttyFormatterFormat::GHOSTTY_FORMATTER_FORMAT_PLAIN,
            unwrap: true,
            trim: true,
            extra: sys::GhosttyFormatterTerminalExtra {
                size: size_of::<sys::GhosttyFormatterTerminalExtra>(),
                palette: false,
                modes: false,
                scrolling_region: false,
                tabstops: false,
                pwd: false,
                keyboard: false,
                screen: sys::GhosttyFormatterScreenExtra {
                    size: size_of::<sys::GhosttyFormatterScreenExtra>(),
                    cursor: false,
                    style: false,
                    hyperlink: false,
                    protection: false,
                    kitty_keyboard: false,
                    charsets: false,
                },
            },
            selection: ptr::null(),
        };
        let mut formatter = ptr::null_mut();
        // SAFETY: the terminal outlives the formatter, which is freed below.
        check(unsafe {
            sys::ghostty_formatter_terminal_new(ptr::null(), &mut formatter, self.raw, options)
        })?;
        let (mut text, mut len) = (ptr::null_mut(), 0);
        // SAFETY: a live formatter; `text` and `len` are only read once the call succeeded.
        let result = unsafe {
            sys::ghostty_formatter_format_alloc(formatter, ptr::null(), &mut text, &mut len)
        };
        // SAFETY: created above and not used again.
        unsafe { sys::ghostty_formatter_free(formatter) };
        check(result)?;
        // SAFETY: the library allocated `len` bytes at `text` with the default allocator.
        Ok(unsafe { take_allocated(text, len) })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: created by `new` and freed only here, the terminal before the callbacks
        // it may still call.
        unsafe {
            sys::ghostty_terminal_free(self.raw);
            drop(Box::from_raw(self.callbacks));
        }
    }
}

/// Copies out and frees a buffer the library allocated with its default allocator.
///
/// # Safety
/// `text` must be null or point at `len` bytes the library allocated that way.
unsafe fn take_allocated(text: *mut u8, len: usize) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: the caller's contract.
    let copied = String::from_utf8_lossy(unsafe { slice::from_raw_parts(text, len) }).into_owned();
    // SAFETY: the caller's contract; not used again.
    unsafe { sys::ghostty_free(ptr::null(), text, len) };
    copied
}

/// A borrowed array the library passed, which it may give as a null pointer when empty.
///
/// # Safety
/// `data` must be null or point at `len` readable values that outlive `'a`.
unsafe fn borrowed<'a, T>(data: *const T, len: usize) -> &'a [T] {
    if data.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: the caller's contract.
    unsafe { slice::from_raw_parts(data, len) }
}

/// # Safety
/// `string` must be empty or describe `len` readable bytes.
unsafe fn string_lossy(string: sys::GhosttyString) -> String {
    if string.ptr.is_null() || string.len == 0 {
        return String::new();
    }
    // SAFETY: the caller's contract.
    String::from_utf8_lossy(unsafe { slice::from_raw_parts(string.ptr, string.len) }).into_owned()
}

/// Runs a callback's body on the terminal's `Callbacks`. A panic is caught and answered
/// with `fallback`: unwinding into Zig frames is undefined behaviour, and the library
/// only ever sees a callback that returned (docs/research/ffi-callback-panics.md).
fn guarded<T>(userdata: *mut c_void, fallback: T, body: impl FnOnce(&mut Callbacks) -> T) -> T {
    // SAFETY: the userdata is the terminal's `Callbacks`, alive for as long as the
    // terminal, and only touched here while `write` runs the library.
    let Some(callbacks) = (unsafe { userdata.cast::<Callbacks>().as_mut() }) else {
        return fallback;
    };
    match panic::catch_unwind(AssertUnwindSafe(|| body(&mut *callbacks))) {
        Ok(value) => value,
        Err(_) => {
            callbacks.panics += 1;
            fallback
        }
    }
}

extern "C" fn write_pty(
    _: sys::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    guarded(userdata, (), |callbacks| {
        // SAFETY: the library passes `len` readable bytes for the call's duration.
        let bytes = unsafe { borrowed(data, len) };
        match callbacks.events.last_mut() {
            Some(Event::WritePty(pending)) => pending.extend_from_slice(bytes),
            _ => callbacks.events.push(Event::WritePty(bytes.to_vec())),
        }
    });
}

extern "C" fn bell(_: sys::GhosttyTerminal, userdata: *mut c_void) {
    guarded(userdata, (), |callbacks| callbacks.events.push(Event::Bell));
}

fn read_string(terminal: sys::GhosttyTerminal, data: sys::GhosttyTerminalData) -> String {
    let mut string = sys::GhosttyString {
        ptr: ptr::null(),
        len: 0,
    };
    // SAFETY: reading data, not writing, is allowed inside a callback.
    if check(unsafe { sys::ghostty_terminal_get(terminal, data, (&raw mut string).cast()) })
        .is_err()
    {
        return String::new();
    }
    // SAFETY: a borrowed string, copied before the callback returns.
    unsafe { string_lossy(string) }
}

extern "C" fn title_changed(terminal: sys::GhosttyTerminal, userdata: *mut c_void) {
    guarded(userdata, (), |callbacks| {
        let title = read_string(
            terminal,
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_TITLE,
        );
        callbacks.events.push(Event::TitleChanged(title));
    });
}

extern "C" fn pwd_changed(terminal: sys::GhosttyTerminal, userdata: *mut c_void) {
    guarded(userdata, (), |callbacks| {
        let pwd = read_string(
            terminal,
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_PWD,
        );
        callbacks.events.push(Event::PwdChanged(pwd));
    });
}

extern "C" fn clipboard_write(
    _: sys::GhosttyTerminal,
    userdata: *mut c_void,
    write: *const sys::GhosttyClipboardWrite,
) {
    use sys::GhosttyClipboardWriteResult as Result;
    let result = guarded(
        userdata,
        Result::GHOSTTY_CLIPBOARD_WRITE_RESULT_IO_ERROR,
        |callbacks| {
            // SAFETY: the library passes a request valid for the call's duration.
            let request = unsafe { &*write };
            // SAFETY: `contents` holds `contents_len` entries for the same duration.
            let contents = unsafe { borrowed(request.contents, request.contents_len) };
            // No contents at all is a program clearing the clipboard (an empty OSC 52).
            if contents.is_empty() {
                callbacks.events.push(Event::ClipboardWrite(String::new()));
                return Result::GHOSTTY_CLIPBOARD_WRITE_RESULT_SUCCESS;
            }
            let text = contents.iter().find_map(|content| {
                // SAFETY: borrowed strings, copied before the callback returns.
                let mime = unsafe { string_lossy(content.mime) };
                mime.starts_with("text/plain")
                    .then(|| unsafe { string_lossy(content.data) })
            });
            match text {
                Some(text) => {
                    callbacks.events.push(Event::ClipboardWrite(text));
                    Result::GHOSTTY_CLIPBOARD_WRITE_RESULT_SUCCESS
                }
                None => Result::GHOSTTY_CLIPBOARD_WRITE_RESULT_UNSUPPORTED,
            }
        },
    );
    // SAFETY: as above; every request is answered once, a copy-only clipboard (ADR 0007)
    // remembering nothing.
    unsafe {
        if let Some(reply) = (*write).reply {
            let answer = sys::GhosttyClipboardWriteReply {
                size: size_of::<sys::GhosttyClipboardWriteReply>(),
                result,
                remember: false,
            };
            reply(write, &answer);
        }
    }
}

extern "C" fn desktop_notification(
    _: sys::GhosttyTerminal,
    userdata: *mut c_void,
    notification: *const sys::GhosttyTerminalDesktopNotification,
) {
    guarded(userdata, (), |callbacks| {
        // SAFETY: a notification and strings valid for the call's duration.
        let (title, body) = unsafe {
            (
                string_lossy((*notification).title),
                string_lossy((*notification).body),
            )
        };
        callbacks.events.push(Event::Notification { title, body });
    });
}

extern "C" fn program_status(
    _: sys::GhosttyTerminal,
    userdata: *mut c_void,
    report: *const sys::GhosttyTerminalProgramStatus,
) {
    use sys::GhosttyProgramStatusKind as Kind;
    use sys::GhosttyProgramStatusState as State;
    guarded(userdata, (), |callbacks| {
        // SAFETY: a report and strings valid for the call's duration.
        let report = unsafe { &*report };
        let state = match report.state {
            State::GHOSTTY_PROGRAM_STATUS_STATE_IDLE => ProgramState::Idle,
            State::GHOSTTY_PROGRAM_STATUS_STATE_WORKING => ProgramState::Working,
            State::GHOSTTY_PROGRAM_STATUS_STATE_DONE => ProgramState::Done,
            State::GHOSTTY_PROGRAM_STATUS_STATE_BLOCKED => ProgramState::Blocked,
            State::GHOSTTY_PROGRAM_STATUS_STATE_ERROR => ProgramState::Error,
            State::GHOSTTY_PROGRAM_STATUS_STATE_CLEAR => ProgramState::Clear,
            _ => return,
        };
        let kind = match report.kind {
            Kind::GHOSTTY_PROGRAM_STATUS_KIND_PERMISSION => Some(BlockedKind::Permission),
            Kind::GHOSTTY_PROGRAM_STATUS_KIND_QUESTION => Some(BlockedKind::Question),
            Kind::GHOSTTY_PROGRAM_STATUS_KIND_AUTH => Some(BlockedKind::Auth),
            _ => None,
        };
        // SAFETY: borrowed strings, copied before the callback returns.
        let (id, app, message) = unsafe {
            (
                string_lossy(report.id),
                string_lossy(report.app),
                string_lossy(report.message),
            )
        };
        callbacks.events.push(Event::ProgramStatus(ProgramStatus {
            state,
            kind,
            id,
            app,
            message,
        }));
    });
}

extern "C" fn semantic_prompt(
    _: sys::GhosttyTerminal,
    userdata: *mut c_void,
    event: *const sys::GhosttyTerminalSemanticPrompt,
) {
    guarded(userdata, (), |callbacks| {
        // SAFETY: an event valid for the call's duration.
        let event = unsafe { &*event };
        if event.kind == sys::GhosttySemanticPromptKind::GHOSTTY_SEMANTIC_PROMPT_PROMPT_START
            && event.prompt_kind
                == sys::GhosttySemanticPromptPromptKind::GHOSTTY_SEMANTIC_PROMPT_PROMPT_PRIMARY
        {
            callbacks.events.push(Event::PromptStart);
        }
    });
}

extern "C" fn reset(_: sys::GhosttyTerminal, userdata: *mut c_void) {
    guarded(userdata, (), |callbacks| {
        callbacks.events.push(Event::Reset)
    });
}

extern "C" fn render_hold(_: sys::GhosttyTerminal, userdata: *mut c_void, held: bool) {
    guarded(userdata, (), |callbacks| {
        callbacks.events.push(Event::RenderHold(held))
    });
}

extern "C" fn size_report(
    _: sys::GhosttyTerminal,
    userdata: *mut c_void,
    out: *mut sys::GhosttySizeReportSize,
) -> bool {
    guarded(userdata, false, |callbacks| {
        // SAFETY: a writable out value for the call's duration.
        unsafe { *out = callbacks.size };
        true
    })
}

/// VT220 with ANSI colour, as Ghostty declares itself.
extern "C" fn device_attributes(
    _: sys::GhosttyTerminal,
    userdata: *mut c_void,
    out: *mut sys::GhosttyDeviceAttributes,
) -> bool {
    guarded(userdata, false, |_| {
        let mut features = [0; 64];
        features[0] = 22;
        let attributes = sys::GhosttyDeviceAttributes {
            primary: sys::GhosttyDeviceAttributesPrimary {
                conformance_level: 62,
                features,
                num_features: 1,
            },
            secondary: sys::GhosttyDeviceAttributesSecondary {
                device_type: 1,
                firmware_version: 0,
                rom_cartridge: 0,
            },
            tertiary: sys::GhosttyDeviceAttributesTertiary { unit_id: 0 },
        };
        // SAFETY: a writable out value for the call's duration.
        unsafe { *out = attributes };
        true
    })
}

extern "C" fn xtversion(_: sys::GhosttyTerminal, userdata: *mut c_void) -> sys::GhosttyString {
    guarded(
        userdata,
        sys::GhosttyString {
            ptr: ptr::null(),
            len: 0,
        },
        |callbacks| sys::GhosttyString {
            ptr: callbacks.version.as_ptr(),
            len: callbacks.version.len(),
        },
    )
}

#[cfg(test)]
mod tests;
