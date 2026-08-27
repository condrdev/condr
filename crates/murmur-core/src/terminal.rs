use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Config;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::Processor;
pub use portable_pty::CommandBuilder;
use portable_pty::{Child, ExitStatus, MasterPty, PtySize, native_pty_system};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    pub rows: u16,
    pub columns: u16,
    pub cell_width: u16,
    pub cell_height: u16,
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
    io: mpsc::Sender<IoCommand>,
    child: Option<Box<dyn Child + Send + Sync>>,
    reader: Option<JoinHandle<io::Result<()>>>,
    writer: Option<JoinHandle<io::Result<()>>>,
}

impl TerminalRuntime {
    pub fn spawn_shell(cwd: impl AsRef<Path>, size: TerminalSize) -> io::Result<Self> {
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

        // ponytail: unbounded per-Terminal input; add backpressure when protocol input is wired.
        let (io, commands) = mpsc::channel();
        let current_size = Arc::new(Mutex::new(size));
        let event_proxy = TerminalEventProxy {
            io: io.clone(),
            size: Arc::clone(&current_size),
        };
        let terminal = Arc::new(Mutex::new(Term::new(Config::default(), &size, event_proxy)));
        let revision = Arc::new(AtomicU64::new(0));

        let writer_terminal = Arc::clone(&terminal);
        let writer_size = Arc::clone(&current_size);
        let writer_revision = Arc::clone(&revision);
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
        let reader_thread = match thread::Builder::new()
            .name("murmur-pty-reader".into())
            .spawn(move || read_loop(reader, reader_terminal, reader_revision))
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
            io,
            child: Some(child),
            reader: Some(reader_thread),
            writer: Some(writer_thread),
        })
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) -> io::Result<()> {
        self.io
            .send(IoCommand::Write(bytes.into()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped"))
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
        let reader_result = join(&mut self.reader, "terminal reader");
        let _ = self.io.send(IoCommand::Shutdown);
        let writer_result = join(&mut self.writer, "terminal writer");
        reader_result.and(writer_result)
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
) -> io::Result<()> {
    let mut parser: Processor = Processor::new();
    let mut bytes = [0; 16 * 1024];
    loop {
        match reader.read(&mut bytes) {
            Ok(0) => return Ok(()),
            Ok(read) => {
                parser.advance(
                    &mut *terminal.lock().expect("terminal state lock poisoned"),
                    &bytes[..read],
                );
                revision.fetch_add(1, Ordering::Release);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn io_loop(
    master: Box<dyn MasterPty + Send>,
    mut writer: Box<dyn Write + Send>,
    commands: mpsc::Receiver<IoCommand>,
    terminal: Arc<Mutex<Terminal>>,
    current_size: Arc<Mutex<TerminalSize>>,
    revision: Arc<AtomicU64>,
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
                    revision.fetch_add(1, Ordering::Release);
                });
                let _ = response.send(result);
            }
            IoCommand::Shutdown => return Ok(()),
        }
    }
    Ok(())
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
