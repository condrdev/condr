use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(unix)]
use nix::unistd::{Pid as UnixPid, getpgid};

use crate::{AgentKind, AgentSnapshot, AgentState, classify_agent, identify_agent_process};

const MAX_TERMINAL_CELLS: usize = 65_536;
const PROCESS_REFRESH_INTERVAL: Duration = Duration::from_millis(250);

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalSelection {
    pub start: TerminalPosition,
    pub end: TerminalPosition,
    pub display_offset: u32,
}

impl TerminalSelection {
    pub fn contains_cell(self, row: u16, column: u16, columns: u16) -> bool {
        let Some((start, end)) = self.selected_cell_range(columns) else {
            return false;
        };
        let cell = u32::from(row) * u32::from(columns) + u32::from(column);
        (start..=end).contains(&cell)
    }

    pub fn selected_cell_range(self, columns: u16) -> Option<(u32, u32)> {
        if columns == 0 {
            return None;
        }

        let mut start = self.start;
        let mut end = self.end;
        start.column = start.column.min(columns - 1);
        end.column = end.column.min(columns - 1);
        if (start.row, start.column) > (end.row, end.column) {
            std::mem::swap(&mut start, &mut end);
        }

        if start == end
            || (start.row == end.row
                && start.column.checked_add(1) == Some(end.column)
                && start.side == TerminalSide::Right
                && end.side == TerminalSide::Left)
        {
            return None;
        }

        let different_cells = (start.row, start.column) != (end.row, end.column);
        let mut start_cell = u32::from(start.row) * u32::from(columns) + u32::from(start.column);
        let mut end_cell = u32::from(end.row) * u32::from(columns) + u32::from(end.column);
        if different_cells && start.side == TerminalSide::Right {
            start_cell += 1;
        }
        if different_cells && end.side == TerminalSide::Left {
            end_cell = end_cell.saturating_sub(1);
        }

        (start_cell <= end_cell).then_some((start_cell, end_cell))
    }
}

impl TerminalView {
    pub fn word_selection_at(&self, row: u16, column: u16) -> Option<TerminalSelection> {
        if row >= self.size.rows || column >= self.size.columns {
            return None;
        }
        let clicked = self.semantic_character(row, column)?;
        if is_word_separator(clicked) {
            return None;
        }

        let mut start = column;
        while start > 0
            && self
                .semantic_character(row, start - 1)
                .is_some_and(|ch| !is_word_separator(ch))
        {
            start -= 1;
        }

        let mut end = column;
        while end + 1 < self.size.columns
            && self
                .semantic_character(row, end + 1)
                .is_some_and(|ch| !is_word_separator(ch))
        {
            end += 1;
        }

        while start <= end
            && self
                .semantic_character(row, start)
                .is_some_and(is_leading_token_wrapper)
        {
            start += 1;
        }
        while start <= end
            && self
                .semantic_character(row, end)
                .is_some_and(is_trailing_token_wrapper)
        {
            if end == 0 {
                return None;
            }
            end -= 1;
        }
        if !(start..=end).contains(&column) {
            return None;
        }

        Some(TerminalSelection {
            start: TerminalPosition {
                row,
                column: start,
                side: TerminalSide::Left,
            },
            end: TerminalPosition {
                row,
                column: end,
                side: TerminalSide::Right,
            },
            display_offset: self.display_offset,
        })
    }

    pub fn line_selection_at(&self, row: u16) -> Option<TerminalSelection> {
        let end = self.size.columns.checked_sub(1)?;
        (row < self.size.rows).then_some(TerminalSelection {
            start: TerminalPosition {
                row,
                column: 0,
                side: TerminalSide::Left,
            },
            end: TerminalPosition {
                row,
                column: end,
                side: TerminalSide::Right,
            },
            display_offset: self.display_offset,
        })
    }

    fn semantic_character(&self, row: u16, column: u16) -> Option<char> {
        let cell = self.cell(row, column)?;
        let flags = Flags::from_bits_retain(cell.flags);
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            return column
                .checked_sub(1)
                .and_then(|column| self.cell(row, column))
                .and_then(|cell| cell.text.chars().next());
        }
        cell.text.chars().next()
    }
}

fn is_word_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '|' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '!'
        )
}

fn is_leading_token_wrapper(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '<' | '"' | '\'' | '`')
}

fn is_trailing_token_wrapper(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '>' | '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '!' | '?'
    )
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
    Copy {
        selection: TerminalSelection,
    },
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
    master: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
    process: ProcessProbe,
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
            command.args(["-NoLogo", "-NoExit"]);
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
        let shell_pid = child.process_id();
        drop(pair.slave);
        let master = Arc::new(Mutex::new(pair.master));

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
        let writer_master = Arc::clone(&master);
        let writer_size = Arc::clone(&current_size);
        let writer_revision = Arc::clone(&revision);
        let writer_updates = update_sender.clone();
        let writer_thread = match thread::Builder::new()
            .name("murmur-pty-writer".into())
            .spawn(move || {
                io_loop(
                    writer_master,
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
            master: Some(master),
            process: ProcessProbe::new(shell_pid),
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
            TerminalCommand::Copy { selection } => Ok(self.copy_range(selection)),
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
            cells[usize::from(row) * usize::from(size.columns) + usize::from(column)] =
                TerminalCell {
                    text,
                    foreground: terminal_color(cell.fg, colors),
                    background: terminal_color(cell.bg, colors),
                    flags: cell.flags.bits(),
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

    pub fn bottom_text(&self) -> String {
        let terminal = self.terminal.lock().expect("terminal state lock poisoned");
        bottom_text(&terminal)
    }

    pub fn agent_snapshot(&self, previous: Option<AgentSnapshot>) -> Option<AgentSnapshot> {
        self.agent_probe().snapshot(previous)
    }

    pub fn agent_probe(&self) -> TerminalAgentProbe {
        TerminalAgentProbe {
            terminal: Arc::clone(&self.terminal),
            #[cfg(unix)]
            master: Arc::clone(
                self.master
                    .as_ref()
                    .expect("live Terminal has a PTY master"),
            ),
            process: self.process,
        }
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

    pub fn copy_range(&self, selection: TerminalSelection) -> Option<String> {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let start_side = side(selection.start.side);
        let start = viewport_point(&terminal, selection.start, selection.display_offset);
        let end_side = side(selection.end.side);
        let end = viewport_point(&terminal, selection.end, selection.display_offset);
        let previous = terminal.selection.take();
        let mut range = Selection::new(SelectionType::Simple, start, start_side);
        range.update(end, end_side);
        terminal.selection = Some(range);
        let text = terminal.selection_to_string();
        terminal.selection = previous;
        text
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
        self.master.take();
        let reader_result = join(&mut self.reader, "terminal reader");
        writer_result.and(reader_result)
    }
}

#[derive(Clone)]
pub struct TerminalAgentProbe {
    terminal: Arc<Mutex<Terminal>>,
    #[cfg(unix)]
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    process: ProcessProbe,
}

impl TerminalAgentProbe {
    pub fn snapshot(&self, previous: Option<AgentSnapshot>) -> Option<AgentSnapshot> {
        #[cfg(unix)]
        let kind = self.process.agent_kind(&self.master)?;
        #[cfg(windows)]
        let kind = self.process.agent_kind()?;
        let previous = previous
            .filter(|snapshot| snapshot.kind == kind)
            .map_or(AgentState::Unknown, |snapshot| snapshot.state);
        let terminal = self.terminal.lock().expect("terminal state lock poisoned");
        Some(AgentSnapshot {
            kind,
            state: classify_agent(kind, &bottom_text(&terminal), previous),
        })
    }
}

fn bottom_text(terminal: &Terminal) -> String {
    let mut lines = Vec::with_capacity(terminal.screen_lines());
    for row in 0..terminal.screen_lines() {
        let row = &terminal.grid()[Line(row as i32)];
        let mut line = String::new();
        for column in 0..terminal.columns() {
            let cell = &row[Column(column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if cell.flags.contains(Flags::HIDDEN) {
                line.push(' ');
            } else {
                line.push(cell.c);
                if let Some(zerowidth) = cell.zerowidth() {
                    line.extend(zerowidth);
                }
            }
        }
        line.truncate(line.trim_end_matches(' ').len());
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
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
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
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
                let result = master
                    .lock()
                    .expect("PTY master lock poisoned")
                    .resize(size.into())
                    .map_err(other_error)
                    .map(|()| {
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

#[derive(Clone, Copy)]
struct ProcessProbe {
    shell_pid: Option<u32>,
}

struct ProcessSnapshot {
    system: System,
    refresh_kind: ProcessRefreshKind,
    refreshed_at: Option<Instant>,
}

impl ProcessProbe {
    fn new(shell_pid: Option<u32>) -> Self {
        Self { shell_pid }
    }

    #[cfg(unix)]
    fn agent_kind(&self, master: &Mutex<Box<dyn MasterPty + Send>>) -> Option<AgentKind> {
        let foreground_group = master
            .lock()
            .expect("PTY master lock poisoned")
            .process_group_leader()
            .map(UnixPid::from_raw)
            .or_else(|| {
                let shell_pid = i32::try_from(self.shell_pid?).ok()?;
                getpgid(Some(UnixPid::from_raw(shell_pid))).ok()
            })?;
        let processes = process_snapshot();
        let system = &processes.system;

        let leader = Pid::from_u32(u32::try_from(foreground_group.as_raw()).ok()?);
        if let Some(kind) = system.process(leader).and_then(|process| {
            identify_process(process.name().to_string_lossy().as_ref(), process.cmd())
        }) {
            return Some(kind);
        }

        system.processes().iter().find_map(|(pid, process)| {
            let pid = i32::try_from(pid.as_u32()).ok()?;
            (getpgid(Some(UnixPid::from_raw(pid))).ok()? == foreground_group).then(|| {
                identify_process(process.name().to_string_lossy().as_ref(), process.cmd())
            })?
        })
    }

    #[cfg(windows)]
    fn agent_kind(&self) -> Option<AgentKind> {
        let shell_pid = Pid::from_u32(self.shell_pid?);
        let processes = process_snapshot();
        let system = &processes.system;
        let candidates = system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                descendant_depth(system, *pid, shell_pid).and_then(|_| {
                    identify_process(process.name().to_string_lossy().as_ref(), process.cmd())
                        .map(|kind| (*pid, kind))
                })
            })
            .collect::<Vec<_>>();
        root_agent(&candidates, |ancestor, descendant| {
            descendant_depth(system, descendant, ancestor).is_some()
        })
    }
}

#[cfg(any(windows, test))]
fn root_agent<T: Copy + Eq>(
    candidates: &[(T, AgentKind)],
    is_ancestor: impl Fn(T, T) -> bool,
) -> Option<AgentKind> {
    let mut roots = candidates.iter().filter(|(candidate, _)| {
        candidates
            .iter()
            .all(|(other, _)| is_ancestor(*candidate, *other))
    });
    let (_, kind) = roots.next()?;
    roots.next().is_none().then_some(*kind)
}

fn process_snapshot() -> std::sync::MutexGuard<'static, ProcessSnapshot> {
    static SNAPSHOT: OnceLock<Mutex<ProcessSnapshot>> = OnceLock::new();
    let mut snapshot = SNAPSHOT
        .get_or_init(|| {
            Mutex::new(ProcessSnapshot {
                system: System::new(),
                refresh_kind: ProcessRefreshKind::new()
                    .with_cmd(UpdateKind::Always)
                    .with_exe(UpdateKind::Always),
                refreshed_at: None,
            })
        })
        .lock()
        .expect("process snapshot lock poisoned");
    let now = Instant::now();
    if snapshot
        .refreshed_at
        .is_none_or(|refreshed| now.duration_since(refreshed) >= PROCESS_REFRESH_INTERVAL)
    {
        let refresh_kind = snapshot.refresh_kind;
        snapshot
            .system
            .refresh_processes_specifics(ProcessesToUpdate::All, refresh_kind);
        snapshot.refreshed_at = Some(now);
    }
    snapshot
}

fn identify_process(name: &str, argv: &[std::ffi::OsString]) -> Option<AgentKind> {
    let argv = argv
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    identify_agent_process(name, &argv)
}

#[cfg(windows)]
fn descendant_depth(system: &System, mut pid: Pid, ancestor: Pid) -> Option<usize> {
    for depth in 0..32 {
        if pid == ancestor {
            return Some(depth);
        }
        pid = system.process(pid)?.parent()?;
    }
    None
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

fn viewport_point(terminal: &Terminal, position: TerminalPosition, display_offset: u32) -> Point {
    let row = usize::from(position.row).min(terminal.screen_lines().saturating_sub(1));
    let column = usize::from(position.column).min(terminal.columns().saturating_sub(1));
    Point::new(
        Line(row as i32 - i32::try_from(display_offset).unwrap_or(i32::MAX)),
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

    #[test]
    fn windows_agent_selection_requires_one_candidate_to_own_the_tree() {
        let is_ancestor = |ancestor, mut descendant| {
            while ancestor != descendant {
                descendant = match descendant {
                    3 => 2,
                    2 => 1,
                    _ => return false,
                };
            }
            true
        };
        assert_eq!(
            root_agent(
                &[(2, AgentKind::Claude), (3, AgentKind::Codex)],
                is_ancestor
            ),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            root_agent(
                &[(2, AgentKind::Claude), (4, AgentKind::Codex)],
                is_ancestor
            ),
            None
        );
    }

    #[test]
    fn terminal_selection_matches_simple_cell_boundary_semantics() {
        let position = |row, column, side| TerminalPosition { row, column, side };
        let selection = |start, end| TerminalSelection {
            start,
            end,
            display_offset: 0,
        };

        let empty = selection(
            position(0, 1, TerminalSide::Left),
            position(0, 1, TerminalSide::Left),
        );
        assert!(!empty.contains_cell(0, 1, 10));

        let one_cell = selection(
            position(0, 1, TerminalSide::Left),
            position(0, 1, TerminalSide::Right),
        );
        assert!(one_cell.contains_cell(0, 1, 10));

        let middle_cell = selection(
            position(0, 1, TerminalSide::Right),
            position(0, 3, TerminalSide::Left),
        );
        assert!(!middle_cell.contains_cell(0, 1, 10));
        assert!(middle_cell.contains_cell(0, 2, 10));
        assert!(!middle_cell.contains_cell(0, 3, 10));

        let reversed = selection(
            position(1, 1, TerminalSide::Right),
            position(0, 8, TerminalSide::Left),
        );
        assert!(reversed.contains_cell(0, 8, 10));
        assert!(reversed.contains_cell(1, 1, 10));
    }

    #[test]
    fn terminal_view_selects_words_by_display_column() {
        let text = "run https://example.com/a?q=1, next";
        let size = TerminalSize::new(1, 40);
        let mut cells = vec![blank_cell(); usize::from(size.columns)];
        for (column, ch) in text.chars().enumerate() {
            cells[column].text = ch.to_string();
        }
        let view = TerminalView {
            revision: 1,
            size,
            display_offset: 3,
            cells,
            cursor: None,
        };

        let selection = view.word_selection_at(0, 12).unwrap();
        assert_eq!(selection.start.column, 4);
        assert_eq!(selection.end.column, 28);
        assert_eq!(selection.display_offset, 3);
        assert!(view.word_selection_at(0, 3).is_none());
        assert!(view.word_selection_at(0, 29).is_none());

        let mut punctuation = view.clone();
        punctuation.cells[0].text = ".".into();
        punctuation.cells[1].text = " ".into();
        assert!(punctuation.word_selection_at(0, 0).is_none());
    }

    #[test]
    fn terminal_view_selects_a_complete_line() {
        let view = TerminalView {
            revision: 1,
            size: TerminalSize::new(3, 10),
            display_offset: 2,
            cells: vec![blank_cell(); 30],
            cursor: None,
        };

        let selection = view.line_selection_at(1).unwrap();
        assert_eq!((selection.start.row, selection.start.column), (1, 0));
        assert_eq!((selection.end.row, selection.end.column), (1, 9));
        assert_eq!(selection.display_offset, 2);
    }
}
