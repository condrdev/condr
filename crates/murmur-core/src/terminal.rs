use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};
pub use portable_pty::CommandBuilder;
use portable_pty::{Child, ExitStatus, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

const MAX_TERMINAL_CELLS: usize = 65_536;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub rows: u16,
    pub columns: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalCursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalColor {
    Named(u16),
    Indexed(u8),
    Rgb { red: u8, green: u8, blue: u8 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalCell {
    pub text: String,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub flags: u16,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalCursor {
    pub row: u16,
    pub column: u16,
    pub shape: TerminalCursorShape,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalView {
    pub revision: u64,
    pub size: TerminalSize,
    pub display_offset: u32,
    pub cells: Vec<TerminalCell>,
    pub cursor: Option<TerminalCursor>,
}

impl TerminalView {
    pub fn cell(&self, row: u16, column: u16) -> Option<&TerminalCell> {
        if row >= self.size.rows || column >= self.size.columns {
            return None;
        }
        self.cells
            .get(usize::from(row) * usize::from(self.size.columns) + usize::from(column))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub platform: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalKey {
    Character(String),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Function(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalScroll {
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalPosition {
    pub row: u16,
    pub column: u16,
    pub side: TerminalSide,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminalCommand {
    Key {
        key: TerminalKey,
        modifiers: TerminalModifiers,
    },
    Text(String),
    Paste(String),
    Resize(TerminalSize),
    Scroll(TerminalScroll),
    Select {
        start: TerminalPosition,
        end: TerminalPosition,
    },
    ClearSelection,
    Copy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalUpdate {
    View(u64),
    Exited,
}

impl TerminalSize {
    pub const fn new(rows: u16, columns: u16) -> Self {
        Self {
            rows,
            columns,
            cell_width: 0,
            cell_height: 0,
        }
    }

    pub const fn with_cell_size(mut self, width: u16, height: u16) -> Self {
        self.cell_width = width;
        self.cell_height = height;
        self
    }

    fn validate(self) -> io::Result<Self> {
        if self.rows == 0 || self.columns == 0 {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal rows and columns must be non-zero",
            ))
        } else if usize::from(self.rows) * usize::from(self.columns) > MAX_TERMINAL_CELLS {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal grid exceeds the maximum cell count",
            ))
        } else {
            Ok(self)
        }
    }

    fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.columns,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.columns)
    }
}

impl From<TerminalSize> for PtySize {
    fn from(size: TerminalSize) -> Self {
        Self {
            rows: size.rows,
            cols: size.columns,
            pixel_width: size.columns.saturating_mul(size.cell_width),
            pixel_height: size.rows.saturating_mul(size.cell_height),
        }
    }
}

enum IoCommand {
    Write(Vec<u8>),
    Resize(TerminalSize, mpsc::Sender<io::Result<()>>),
    Shutdown,
}

#[derive(Clone)]
struct TerminalEventProxy {
    io: mpsc::Sender<IoCommand>,
    size: Arc<Mutex<TerminalSize>>,
}

impl EventListener for TerminalEventProxy {
    fn send_event(&self, event: Event) {
        let bytes = match event {
            Event::PtyWrite(text) => Some(text.into_bytes()),
            Event::TextAreaSizeRequest(formatter) => {
                let size = *self.size.lock().expect("terminal size lock poisoned");
                Some(formatter(size.window_size()).into_bytes())
            }
            _ => None,
        };
        if let Some(bytes) = bytes {
            let _ = self.io.send(IoCommand::Write(bytes));
        }
    }
}

type Terminal = Term<TerminalEventProxy>;

pub struct TerminalRuntime {
    terminal: Arc<Mutex<Terminal>>,
    size: Arc<Mutex<TerminalSize>>,
    revision: Arc<AtomicU64>,
    update_sender: mpsc::Sender<TerminalUpdate>,
    updates: Option<mpsc::Receiver<TerminalUpdate>>,
    io: mpsc::Sender<IoCommand>,
    child: Option<Box<dyn Child + Send + Sync>>,
    reader: Option<JoinHandle<io::Result<()>>>,
    writer: Option<JoinHandle<io::Result<()>>>,
}

impl TerminalRuntime {
    pub fn spawn_shell(cwd: impl AsRef<Path>, size: TerminalSize) -> io::Result<Self> {
        #[cfg(windows)]
        let mut command = {
            let mut command = CommandBuilder::new("pwsh.exe");
            command.args(["-NoLogo", "-NoProfile", "-NoExit"]);
            command
        };
        #[cfg(not(windows))]
        let mut command = CommandBuilder::new_default_prog();
        command.cwd(cwd.as_ref());
        Self::spawn(command, size)
    }

    pub fn spawn(mut command: CommandBuilder, size: TerminalSize) -> io::Result<Self> {
        let size = size.validate()?;
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");

        let pair = native_pty_system()
            .openpty(size.into())
            .map_err(other_error)?;
        let reader = pair.master.try_clone_reader().map_err(other_error)?;
        let writer = pair.master.take_writer().map_err(other_error)?;
        let mut child = pair.slave.spawn_command(command).map_err(other_error)?;
        drop(pair.slave);

        // ponytail: unbounded per-Terminal input; add backpressure if sustained input outpaces PTY writes.
        let (io, commands) = mpsc::channel();
        let current_size = Arc::new(Mutex::new(size));
        let event_proxy = TerminalEventProxy {
            io: io.clone(),
            size: Arc::clone(&current_size),
        };
        let terminal = Arc::new(Mutex::new(Term::new(Config::default(), &size, event_proxy)));
        let revision = Arc::new(AtomicU64::new(0));
        let (update_sender, updates) = mpsc::channel();

        let writer_terminal = Arc::clone(&terminal);
        let writer_size = Arc::clone(&current_size);
        let writer_revision = Arc::clone(&revision);
        let writer_updates = update_sender.clone();
        let writer_thread = match thread::Builder::new()
            .name("murmur-pty-writer".into())
            .spawn(move || {
                io_loop(
                    pair.master,
                    writer,
                    commands,
                    writer_terminal,
                    writer_size,
                    writer_revision,
                    writer_updates,
                )
            }) {
            Ok(thread) => thread,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };

        let reader_terminal = Arc::clone(&terminal);
        let reader_revision = Arc::clone(&revision);
        let reader_updates = update_sender.clone();
        let reader_thread = match thread::Builder::new()
            .name("murmur-pty-reader".into())
            .spawn(move || read_loop(reader, reader_terminal, reader_revision, reader_updates))
        {
            Ok(thread) => thread,
            Err(error) => {
                let _ = io.send(IoCommand::Shutdown);
                let _ = writer_thread.join();
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };

        Ok(Self {
            terminal,
            size: current_size,
            revision,
            update_sender,
            updates: Some(updates),
            io,
            child: Some(child),
            reader: Some(reader_thread),
            writer: Some(writer_thread),
        })
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) -> io::Result<()> {
        self.scroll_to_bottom();
        self.io
            .send(IoCommand::Write(bytes.into()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped"))
    }

    pub fn take_updates(&mut self) -> Option<mpsc::Receiver<TerminalUpdate>> {
        self.updates.take()
    }

    pub fn execute(&self, command: TerminalCommand) -> io::Result<Option<String>> {
        match command {
            TerminalCommand::Key { key, modifiers } => {
                let application_cursor = self
                    .terminal
                    .lock()
                    .expect("terminal state lock poisoned")
                    .mode()
                    .contains(TermMode::APP_CURSOR);
                self.write(encode_key(&key, modifiers, application_cursor)?)?;
                Ok(None)
            }
            TerminalCommand::Text(text) => {
                self.write(text.into_bytes())?;
                Ok(None)
            }
            TerminalCommand::Paste(text) => {
                let bracketed = self
                    .terminal
                    .lock()
                    .expect("terminal state lock poisoned")
                    .mode()
                    .contains(TermMode::BRACKETED_PASTE);
                self.write(encode_paste(&text, bracketed))?;
                Ok(None)
            }
            TerminalCommand::Resize(size) => {
                self.resize(size)?;
                Ok(None)
            }
            TerminalCommand::Scroll(scroll) => {
                self.scroll(scroll);
                Ok(None)
            }
            TerminalCommand::Select { start, end } => {
                self.select(start, end);
                Ok(None)
            }
            TerminalCommand::ClearSelection => {
                self.clear_selection();
                Ok(None)
            }
            TerminalCommand::Copy => Ok(self.copy_selection()),
        }
    }

    pub fn resize(&self, size: TerminalSize) -> io::Result<()> {
        let size = size.validate()?;
        if *self.size.lock().expect("terminal size lock poisoned") == size {
            return Ok(());
        }
        let (result, response) = mpsc::channel();
        self.io
            .send(IoCommand::Resize(size, result))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped"))?;
        response
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped"))?
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn view(&self) -> TerminalView {
        let size = *self.size.lock().expect("terminal size lock poisoned");
        let terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let content = terminal.renderable_content();
        let display_offset = content.display_offset;
        let selection = content.selection;
        let cursor = content.cursor;
        let colors = content.colors;
        let mut cells = vec![blank_cell(); usize::from(size.rows) * usize::from(size.columns)];

        for cell in content.display_iter {
            let row = cell.point.line.0 + display_offset as i32;
            let Ok(row) = u16::try_from(row) else {
                continue;
            };
            let Ok(column) = u16::try_from(cell.point.column.0) else {
                continue;
            };
            if row >= size.rows || column >= size.columns {
                continue;
            }
            let mut text = String::from(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth);
            }
            let selected = selection
                .is_some_and(|range| range.contains_cell(&cell, cursor.point, cursor.shape));
            cells[usize::from(row) * usize::from(size.columns) + usize::from(column)] =
                TerminalCell {
                    text,
                    foreground: terminal_color(cell.fg, colors),
                    background: terminal_color(cell.bg, colors),
                    flags: cell.flags.bits(),
                    selected,
                };
        }

        let cursor_row = cursor.point.line.0 + display_offset as i32;
        let cursor = u16::try_from(cursor_row).ok().and_then(|row| {
            let column = u16::try_from(cursor.point.column.0).ok()?;
            (row < size.rows && column < size.columns).then_some(TerminalCursor {
                row,
                column,
                shape: cursor.shape.into(),
            })
        });

        TerminalView {
            revision: self.revision(),
            size,
            display_offset: u32::try_from(display_offset).unwrap_or(u32::MAX),
            cells,
            cursor,
        }
    }

    pub fn visible_text(&self) -> String {
        let terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let mut lines = vec![String::new(); terminal.screen_lines()];

        for cell in terminal.renderable_content().display_iter {
            let line = cell.point.line.0 + terminal.grid().display_offset() as i32;
            let Ok(line) = usize::try_from(line) else {
                continue;
            };
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if cell.flags.contains(Flags::HIDDEN) {
                lines[line].push(' ');
            } else {
                lines[line].push(cell.c);
                if let Some(zerowidth) = cell.zerowidth() {
                    lines[line].extend(zerowidth);
                }
            }
        }

        for line in &mut lines {
            line.truncate(line.trim_end_matches(' ').len());
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    pub fn scroll(&self, scroll: TerminalScroll) {
        let scroll = match scroll {
            TerminalScroll::Lines(lines) => Scroll::Delta(lines),
            TerminalScroll::PageUp => Scroll::PageUp,
            TerminalScroll::PageDown => Scroll::PageDown,
            TerminalScroll::Top => Scroll::Top,
            TerminalScroll::Bottom => Scroll::Bottom,
        };
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let before = terminal.grid().display_offset();
        terminal.scroll_display(scroll);
        if terminal.grid().display_offset() != before {
            drop(terminal);
            self.notify_view();
        }
    }

    pub fn select(&self, start: TerminalPosition, end: TerminalPosition) {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let start_side = side(start.side);
        let start = viewport_point(&terminal, start);
        let end_side = side(end.side);
        let end = viewport_point(&terminal, end);
        let mut selection = Selection::new(SelectionType::Simple, start, start_side);
        selection.update(end, end_side);
        terminal.selection = Some(selection);
        drop(terminal);
        self.notify_view();
    }

    pub fn clear_selection(&self) {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        if terminal.selection.take().is_some() {
            drop(terminal);
            self.notify_view();
        }
    }

    pub fn copy_selection(&self) -> Option<String> {
        self.terminal
            .lock()
            .expect("terminal state lock poisoned")
            .selection_to_string()
    }

    fn scroll_to_bottom(&self) {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        if terminal.grid().display_offset() != 0 {
            terminal.scroll_display(Scroll::Bottom);
            drop(terminal);
            self.notify_view();
        }
    }

    fn notify_view(&self) {
        let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.update_sender.send(TerminalUpdate::View(revision));
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("terminal child exit was already reported"))?
            .wait()?;
        self.child = None;
        self.finish_io()?;
        Ok(status)
    }

    pub fn shutdown(&mut self) -> io::Result<ExitStatus> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("terminal child exit was already reported"))?;
        let status = match child.try_wait()? {
            Some(status) => status,
            None => {
                let kill_error = child.kill().err();
                let status = child.wait()?;
                self.child = None;
                let io_result = self.finish_io();
                kill_error.map_or(io_result, Err)?;
                return Ok(status);
            }
        };
        self.child = None;
        self.finish_io()?;
        Ok(status)
    }

    fn finish_io(&mut self) -> io::Result<()> {
        let _ = self.io.send(IoCommand::Shutdown);
        let writer_result = join(&mut self.writer, "terminal writer");
        let reader_result = join(&mut self.reader, "terminal reader");
        writer_result.and(reader_result)
    }
}

impl Drop for TerminalRuntime {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.shutdown();
        } else {
            let _ = self.finish_io();
        }
    }
}

fn read_loop(
    mut reader: Box<dyn Read + Send>,
    terminal: Arc<Mutex<Terminal>>,
    revision: Arc<AtomicU64>,
    updates: mpsc::Sender<TerminalUpdate>,
) -> io::Result<()> {
    let mut parser: Processor = Processor::new();
    let mut bytes = [0; 16 * 1024];
    let result = loop {
        match reader.read(&mut bytes) {
            Ok(0) => break Ok(()),
            Ok(read) => {
                parser.advance(
                    &mut *terminal.lock().expect("terminal state lock poisoned"),
                    &bytes[..read],
                );
                publish_view(&revision, &updates);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Err(error),
        }
    };
    let _ = updates.send(TerminalUpdate::Exited);
    result
}

fn io_loop(
    master: Box<dyn MasterPty + Send>,
    mut writer: Box<dyn Write + Send>,
    commands: mpsc::Receiver<IoCommand>,
    terminal: Arc<Mutex<Terminal>>,
    current_size: Arc<Mutex<TerminalSize>>,
    revision: Arc<AtomicU64>,
    updates: mpsc::Sender<TerminalUpdate>,
) -> io::Result<()> {
    while let Ok(command) = commands.recv() {
        match command {
            IoCommand::Write(bytes) => {
                writer.write_all(&bytes)?;
                writer.flush()?;
            }
            IoCommand::Resize(size, response) => {
                let mut terminal = terminal.lock().expect("terminal state lock poisoned");
                let result = master.resize(size.into()).map_err(other_error).map(|()| {
                    terminal.resize(size);
                    *current_size.lock().expect("terminal size lock poisoned") = size;
                    publish_view(&revision, &updates);
                });
                let _ = response.send(result);
            }
            IoCommand::Shutdown => return Ok(()),
        }
    }
    Ok(())
}

fn publish_view(revision: &AtomicU64, updates: &mpsc::Sender<TerminalUpdate>) {
    let revision = revision.fetch_add(1, Ordering::AcqRel) + 1;
    let _ = updates.send(TerminalUpdate::View(revision));
}

fn blank_cell() -> TerminalCell {
    TerminalCell {
        text: " ".into(),
        foreground: TerminalColor::Named(NamedColor::Foreground as u16),
        background: TerminalColor::Named(NamedColor::Background as u16),
        flags: 0,
        selected: false,
    }
}

fn terminal_color(
    color: Color,
    overrides: &alacritty_terminal::term::color::Colors,
) -> TerminalColor {
    let override_color = match color {
        Color::Named(named) => overrides[named],
        Color::Indexed(index) => overrides[usize::from(index)],
        Color::Spec(_) => None,
    };
    let color = override_color.map_or(color, Color::Spec);
    match color {
        Color::Named(named) => TerminalColor::Named(named as u16),
        Color::Indexed(index) => TerminalColor::Indexed(index),
        Color::Spec(rgb) => TerminalColor::Rgb {
            red: rgb.r,
            green: rgb.g,
            blue: rgb.b,
        },
    }
}

impl From<CursorShape> for TerminalCursorShape {
    fn from(shape: CursorShape) -> Self {
        match shape {
            CursorShape::Block => Self::Block,
            CursorShape::Underline => Self::Underline,
            CursorShape::Beam => Self::Beam,
            CursorShape::HollowBlock => Self::HollowBlock,
            CursorShape::Hidden => Self::Hidden,
        }
    }
}

fn viewport_point(terminal: &Terminal, position: TerminalPosition) -> Point {
    let row = usize::from(position.row).min(terminal.screen_lines().saturating_sub(1));
    let column = usize::from(position.column).min(terminal.columns().saturating_sub(1));
    Point::new(
        Line(row as i32 - terminal.grid().display_offset() as i32),
        Column(column),
    )
}

fn side(side: TerminalSide) -> Side {
    match side {
        TerminalSide::Left => Side::Left,
        TerminalSide::Right => Side::Right,
    }
}

fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let text = text.replace(['\x1b', '\x03'], "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

fn encode_key(
    key: &TerminalKey,
    modifiers: TerminalModifiers,
    application_cursor: bool,
) -> io::Result<Vec<u8>> {
    if let TerminalKey::Character(text) = key {
        let mut bytes = if modifiers.control {
            control_character(text).map_or_else(|| text.as_bytes().to_vec(), |byte| vec![byte])
        } else {
            text.as_bytes().to_vec()
        };
        if modifiers.alt {
            bytes.insert(0, b'\x1b');
        }
        return Ok(bytes);
    }

    let modifier = modifier_code(modifiers);
    let sequence = match key {
        TerminalKey::Enter => "\r".into(),
        TerminalKey::Tab if modifiers.shift => "\x1b[Z".into(),
        TerminalKey::Tab => "\t".into(),
        TerminalKey::BackTab => "\x1b[Z".into(),
        TerminalKey::Backspace => "\x7f".into(),
        TerminalKey::Escape => "\x1b".into(),
        TerminalKey::Up => cursor_sequence('A', modifier, application_cursor),
        TerminalKey::Down => cursor_sequence('B', modifier, application_cursor),
        TerminalKey::Right => cursor_sequence('C', modifier, application_cursor),
        TerminalKey::Left => cursor_sequence('D', modifier, application_cursor),
        TerminalKey::Home => cursor_sequence('H', modifier, application_cursor),
        TerminalKey::End => cursor_sequence('F', modifier, application_cursor),
        TerminalKey::Insert => tilde_sequence(2, modifier),
        TerminalKey::Delete => tilde_sequence(3, modifier),
        TerminalKey::PageUp => tilde_sequence(5, modifier),
        TerminalKey::PageDown => tilde_sequence(6, modifier),
        TerminalKey::Function(number) => function_sequence(*number, modifier)?,
        TerminalKey::Character(_) => unreachable!(),
    };

    let mut bytes = sequence.into_bytes();
    if modifiers.alt
        && matches!(
            key,
            TerminalKey::Enter
                | TerminalKey::Tab
                | TerminalKey::BackTab
                | TerminalKey::Backspace
                | TerminalKey::Escape
        )
    {
        bytes.insert(0, b'\x1b');
    }
    Ok(bytes)
}

fn control_character(text: &str) -> Option<u8> {
    let byte = text.as_bytes().first()?.to_ascii_lowercase();
    match byte {
        b'@' | b' ' => Some(0),
        b'a'..=b'z' => Some(byte - b'a' + 1),
        b'[' => Some(27),
        b'\\' => Some(28),
        b']' => Some(29),
        b'^' => Some(30),
        b'_' => Some(31),
        b'?' => Some(127),
        _ => None,
    }
}

fn modifier_code(modifiers: TerminalModifiers) -> u8 {
    1 + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control)
        + 8 * u8::from(modifiers.platform)
}

fn cursor_sequence(final_byte: char, modifier: u8, application_cursor: bool) -> String {
    if modifier > 1 {
        format!("\x1b[1;{modifier}{final_byte}")
    } else if application_cursor {
        format!("\x1bO{final_byte}")
    } else {
        format!("\x1b[{final_byte}")
    }
}

fn tilde_sequence(code: u8, modifier: u8) -> String {
    if modifier > 1 {
        format!("\x1b[{code};{modifier}~")
    } else {
        format!("\x1b[{code}~")
    }
}

fn function_sequence(number: u8, modifier: u8) -> io::Result<String> {
    let sequence = match number {
        1..=4 if modifier > 1 => {
            let final_byte = char::from(b'P' + number - 1);
            format!("\x1b[1;{modifier}{final_byte}")
        }
        1..=4 => {
            let final_byte = char::from(b'P' + number - 1);
            format!("\x1bO{final_byte}")
        }
        5 => tilde_sequence(15, modifier),
        6 => tilde_sequence(17, modifier),
        7 => tilde_sequence(18, modifier),
        8 => tilde_sequence(19, modifier),
        9 => tilde_sequence(20, modifier),
        10 => tilde_sequence(21, modifier),
        11 => tilde_sequence(23, modifier),
        12 => tilde_sequence(24, modifier),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal function key must be F1 through F12",
            ));
        }
    };
    Ok(sequence)
}

fn join(thread: &mut Option<JoinHandle<io::Result<()>>>, name: &str) -> io::Result<()> {
    match thread.take() {
        Some(thread) => thread
            .join()
            .map_err(|_| io::Error::other(format!("{name} panicked")))?,
        None => Ok(()),
    }
}

fn other_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_keys_cover_cursor_modes_modifiers_and_controls() {
        let none = TerminalModifiers::default();
        assert_eq!(
            encode_key(&TerminalKey::Up, none, false).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(encode_key(&TerminalKey::Up, none, true).unwrap(), b"\x1bOA");
        assert_eq!(
            encode_key(
                &TerminalKey::Left,
                TerminalModifiers {
                    shift: true,
                    control: true,
                    ..TerminalModifiers::default()
                },
                true,
            )
            .unwrap(),
            b"\x1b[1;6D"
        );
        assert_eq!(
            encode_key(
                &TerminalKey::Character("c".into()),
                TerminalModifiers {
                    control: true,
                    ..TerminalModifiers::default()
                },
                false,
            )
            .unwrap(),
            b"\x03"
        );
        assert_eq!(
            encode_key(
                &TerminalKey::Character("x".into()),
                TerminalModifiers {
                    alt: true,
                    ..TerminalModifiers::default()
                },
                false,
            )
            .unwrap(),
            b"\x1bx"
        );
        assert_eq!(
            encode_key(&TerminalKey::Function(12), none, false).unwrap(),
            b"\x1b[24~"
        );
        assert!(encode_key(&TerminalKey::Function(13), none, false).is_err());
    }

    #[test]
    fn paste_respects_bracketed_mode_and_filters_control_markers() {
        assert_eq!(encode_paste("one\r\ntwo\n", false), b"one\rtwo\r");
        assert_eq!(
            encode_paste("one\x1b[201~\x03two\n", true),
            b"\x1b[200~one[201~two\n\x1b[201~"
        );
    }

    #[test]
    fn terminal_size_rejects_unbounded_grid_allocations() {
        assert!(TerminalSize::new(256, 256).validate().is_ok());
        assert!(TerminalSize::new(257, 256).validate().is_err());
    }
}
