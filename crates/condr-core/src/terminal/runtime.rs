mod mouse;
#[cfg(test)]
mod tests;

use super::*;
use std::panic::{self, AssertUnwindSafe};

const CHILD_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct TerminalRuntime {
    pub(super) terminal: Arc<Mutex<Terminal>>,
    pub(super) master: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
    pub(super) process: ProcessProbe,
    pub(super) last_known_cwd: Arc<Mutex<Option<PathBuf>>>,
    reported_cwd: Arc<Mutex<ReportedCwd>>,
    notices: SharedTerminalNotices,
    size: Arc<Mutex<TerminalSize>>,
    revision: Arc<AtomicU64>,
    damage_baseline: Arc<Mutex<Option<TerminalDamageBaseline>>>,
    cursor_settle: Arc<Mutex<CursorSettle>>,
    update_sender: mpsc::Sender<TerminalUpdate>,
    updates: Option<mpsc::Receiver<TerminalUpdate>>,
    input: TerminalInput,
    reported_mouse_press: Mutex<Option<ReportedMousePress>>,
    /// Whether the program reads kitty key reports unasked; the Server sets it from the
    /// detected agent.
    reads_kitty_keys: AtomicBool,
    resize: Arc<ResizeControl>,
    child: SharedChild,
    process_shutdown: ProcessShutdownState,
    reader: Option<JoinHandle<io::Result<()>>>,
    writer: Option<JoinHandle<io::Result<()>>>,
    resizer: Option<JoinHandle<io::Result<()>>>,
    writer_stopping: Arc<AtomicBool>,
    #[cfg(unix)]
    reader_cancel: Option<UnixStream>,
    #[cfg(unix)]
    writer_cancel: Option<UnixStream>,
}

#[derive(Clone, Copy)]
struct ReportedMousePress {
    button: TerminalMouseButton,
    position: TerminalMousePosition,
    modifiers: TerminalModifiers,
}

impl TerminalRuntime {
    /// Starts an interactive shell in `cwd`. `program` is the configured shell; `None`
    /// or blank means the system default, see [`default_shell_program`]. `pane` tells
    /// programs inside the shell which Server and Pane they run in.
    pub fn spawn_shell(
        cwd: impl AsRef<Path>,
        size: TerminalSize,
        program: Option<&str>,
        pane: Option<&PaneEnvironment>,
    ) -> io::Result<Self> {
        let cwd = cwd.as_ref();
        let metadata = std::fs::metadata(cwd).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "cannot use Terminal working directory {}: {error}",
                    cwd.display()
                ),
            )
        })?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!(
                    "Terminal working directory is not a directory: {}",
                    cwd.display()
                ),
            ));
        }
        let program = program
            .map(str::trim)
            .filter(|program| !program.is_empty())
            .map_or_else(default_shell_program, str::to_owned);
        let mut command = shell_command(&program);
        if let Some(pane) = pane {
            pane.apply(&mut command);
            if let Some(directory) = &pane.shell_integration {
                shell_integration::apply(&mut command, &program, directory);
            }
        }
        command.cwd(cwd);
        Self::spawn(command, size)
    }

    pub fn spawn(mut command: CommandBuilder, size: TerminalSize) -> io::Result<Self> {
        let size = size.validate()?;
        let initial_cwd = command
            .get_cwd()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .and_then(resolve_initial_cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        // Condr names no terminal program (ADR 0051), so a Server started from another
        // terminal must not pass that one's name on to its Panes.
        for inherited in [
            "TERM_PROGRAM",
            "TERM_PROGRAM_VERSION",
            "LC_TERMINAL",
            "LC_TERMINAL_VERSION",
        ] {
            command.env_remove(inherited);
        }
        #[cfg(target_os = "macos")]
        shell::set_missing_locale(&mut command);

        let pair = native_pty_system()
            .openpty(size.into())
            .map_err(other_error)?;
        let reader = pair.master.try_clone_reader().map_err(other_error)?;
        #[cfg(unix)]
        let (reader, reader_cancel, exit_signal) = {
            let poll_fd = duplicate_master_fd(&*pair.master)?;
            let (cancel, cancel_reader) = UnixStream::pair()?;
            let exit_signal = cancel.try_clone()?;
            let reader: Box<dyn Read + Send> = Box::new(UnixPtyReader {
                reader,
                poll_fd,
                cancel: cancel_reader,
                drain_reads: None,
            });
            (reader, cancel, exit_signal)
        };
        #[cfg(unix)]
        let (writer, writer_cancel) = unix_pty_writer(&*pair.master)?;
        #[cfg(not(unix))]
        let writer = pair.master.take_writer().map_err(other_error)?;
        #[cfg(windows)]
        let job = Job::new()?;
        // Children inherit "ignore Ctrl+C", which a new process group or a parent's
        // SetConsoleCtrlHandler(NULL, TRUE) sets (an agent's shell tool starting the Server,
        // say); the shell would pass it to every program it runs. This is the Windows side
        // of portable-pty resetting signal dispositions before exec on Unix.
        #[cfg(windows)]
        #[allow(unsafe_code)]
        // SAFETY: a null handler only clears this process's flag, which the shell inherits.
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(None, 0);
        }
        let child = pair.slave.spawn_command(command).map_err(other_error)?;
        // ponytail: portable-pty starts the shell running, so a child it starts before this
        // line escapes the Job; shells take far longer than that. Creating it inside the Job
        // needs PROC_THREAD_ATTRIBUTE_JOB_LIST in portable-pty's CreateProcessW.
        #[cfg(windows)]
        let child = {
            let mut child = child;
            if let Err(error) = child
                .as_raw_handle()
                .ok_or_else(|| io::Error::other("the shell has no process handle"))
                .and_then(|handle| job.assign(handle))
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            child
        };
        let shell_pid = child.process_id();
        #[cfg(not(windows))]
        let process = ProcessProbe::new(shell_pid);
        #[cfg(windows)]
        let process = ProcessProbe::in_job(shell_pid, job);
        drop(pair.slave);
        let master = Arc::new(Mutex::new(pair.master));

        let (input, input_receiver) = TerminalInput::channel();
        let current_size = Arc::new(Mutex::new(size));
        let notices = SharedTerminalNotices::default();
        let terminal = Arc::new(Mutex::new(Vt::new(size)?));
        let revision = Arc::new(AtomicU64::new(0));
        let damage_baseline = Arc::new(Mutex::new(None));
        let cursor_settle = Arc::new(Mutex::new(CursorSettle::default()));
        let reported_cwd = Arc::new(Mutex::new(ReportedCwd::default()));
        let last_known_cwd = Arc::new(Mutex::new(initial_cwd));
        let (update_sender, updates) = mpsc::channel();

        let resize = Arc::new(ResizeControl::default());
        let writer_stopping = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(Some(child)));
        // Ownership is complete before starting any thread. A failed stage drops this
        // runtime through the same process/I/O cleanup as a normally closed Pane.
        let mut runtime = Self {
            terminal,
            master: Some(master),
            process,
            last_known_cwd,
            reported_cwd,
            notices,
            size: current_size,
            revision,
            damage_baseline,
            cursor_settle,
            update_sender,
            updates: Some(updates),
            input,
            reported_mouse_press: Mutex::new(None),
            reads_kitty_keys: AtomicBool::new(false),
            resize,
            child,
            process_shutdown: ProcessShutdownState::default(),
            reader: None,
            writer: None,
            resizer: None,
            writer_stopping,
            #[cfg(unix)]
            reader_cancel: Some(reader_cancel),
            #[cfg(unix)]
            writer_cancel: Some(writer_cancel),
        };

        let writer_stop = Arc::clone(&runtime.writer_stopping);
        runtime.writer = Some(runtime.start_thread("writer", move || {
            io_loop(TerminalIoLoop {
                writer,
                input: input_receiver,
                stopping: writer_stop,
            })
        })?);

        let resize_control = Arc::clone(&runtime.resize);
        let resize_master = Arc::downgrade(runtime.master.as_ref().expect("new PTY has a master"));
        let resize_terminal = Arc::clone(&runtime.terminal);
        let resize_size = Arc::clone(&runtime.size);
        let resize_revision = Arc::clone(&runtime.revision);
        let resize_updates = runtime.update_sender.clone();
        let (resize_notices, resize_cwd, resize_input) = (
            Arc::clone(&runtime.notices),
            Arc::clone(&runtime.reported_cwd),
            runtime.input.clone(),
        );
        runtime.resizer = Some(runtime.start_thread("resizer", move || {
            resize_loop(
                resize_control,
                resize_master,
                resize_terminal,
                resize_size,
                resize_revision,
                resize_updates,
                // What the resize reported goes out as the reader's reports do.
                |events| {
                    let replies = apply_terminal_events(events, &resize_notices, &resize_cwd);
                    let _ = write_terminal_replies(&resize_input, replies);
                },
            )
        })?);

        let reader_terminal = Arc::clone(&runtime.terminal);
        let reader_revision = Arc::clone(&runtime.revision);
        let reader_updates = runtime.update_sender.clone();
        let reader_reported_cwd = Arc::clone(&runtime.reported_cwd);
        let reader_notices = Arc::clone(&runtime.notices);
        let reader_input = runtime.input.clone();
        let reader_size = Arc::clone(&runtime.size);
        let reader_cursor_settle = Arc::clone(&runtime.cursor_settle);
        runtime.reader = Some(runtime.start_thread("reader", move || {
            read_loop(TerminalReadLoop {
                reader,
                terminal: reader_terminal,
                input: reader_input,
                revision: reader_revision,
                updates: reader_updates,
                reported_cwd: reader_reported_cwd,
                notices: reader_notices,
                size: reader_size,
                cursor_settle: reader_cursor_settle,
            })
        })?);

        #[cfg(not(unix))]
        let exit_signal = runtime.update_sender.clone();
        let child = Arc::downgrade(&runtime.child);
        runtime.start_thread("child-watch", move || child_exit_watch(child, exit_signal))?;
        Ok(runtime)
    }

    /// Every thread the Pane owns. A panic on one ends the Pane like its program exiting,
    /// then the thread; other threads find the locks it held poisoned (ADR 0053).
    fn start_thread<T: Send + 'static>(
        &self,
        name: &'static str,
        run: impl FnOnce() -> T + Send + 'static,
    ) -> io::Result<JoinHandle<T>> {
        #[cfg(test)]
        tests::before_thread_start(self, name)?;
        let exited = self.update_sender.clone();
        thread::Builder::new()
            .name(format!("condr-pty-{name}"))
            .spawn(move || {
                panic::catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|panic| {
                    tracing::error!("the terminal {name} panicked; ending its Pane");
                    let _ = exited.send(TerminalUpdate::Exited);
                    panic::resume_unwind(panic)
                })
            })
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) -> io::Result<()> {
        self.scroll_to_bottom();
        self.clear_selection();
        self.input.try_write(bytes.into())
    }

    /// Answers the program's colour queries with `colors` from now on (ADR 0054), and tells
    /// a program that subscribed to scheme changes when dark turned light or back.
    pub fn set_colors(&self, colors: &TerminalColors) -> io::Result<()> {
        let report = lock_pane(&self.terminal)?.set_colors(colors);
        match report {
            Some(report) => self.input.try_write_control(report),
            None => Ok(()),
        }
    }

    /// Paste and delayed Enter share one queue entry, so another client cannot split them.
    pub fn submit(&self, text: &str) -> io::Result<()> {
        let (bytes, enter) = {
            let mut terminal = lock_pane(&self.terminal)?;
            let paste = terminal.encode_paste(text);
            let enter = terminal.encode_key(
                &TerminalKey::Enter,
                TerminalModifiers::default(),
                TerminalKeyEventKind::Press,
                false,
            )?;
            (paste, enter)
        };
        self.scroll_to_bottom();
        self.clear_selection();
        self.input.try_submit(bytes, enter)
    }

    pub fn set_reads_kitty_keys(&self, reads: bool) {
        self.reads_kitty_keys.store(reads, Ordering::Relaxed);
    }

    fn clear_selection(&self) {
        let Ok(mut terminal) = lock_pane(&self.terminal) else {
            return;
        };
        if terminal.clear_selection() {
            drop(terminal);
            publish_view(&self.revision, &self.update_sender);
        }
    }

    pub fn take_updates(&mut self) -> Option<mpsc::Receiver<TerminalUpdate>> {
        self.updates.take()
    }

    pub fn notice_probe(&self) -> TerminalNoticeProbe {
        TerminalNoticeProbe {
            notices: Arc::clone(&self.notices),
        }
    }

    pub fn cwd_probe(&self) -> TerminalCwdProbe {
        TerminalCwdProbe {
            #[cfg(unix)]
            master: self.master.as_ref().map(Arc::downgrade),
            process: if lock_child(&self.child).is_ok_and(|child| child.is_some()) {
                self.process.clone()
            } else {
                ProcessProbe::new(None)
            },
            last_known_cwd: Arc::clone(&self.last_known_cwd),
            reported_cwd: Arc::clone(&self.reported_cwd),
        }
    }

    pub fn execute(&self, command: TerminalCommand) -> io::Result<Option<String>> {
        match command {
            TerminalCommand::Key {
                key,
                modifiers,
                kind,
            } => {
                let modes = lock_pane(&self.terminal)?.modes();
                let only_shift = modifiers
                    == TerminalModifiers {
                        shift: true,
                        ..TerminalModifiers::default()
                    };
                // Cmd or Ctrl with Up or Down jumps between the shell's prompts while it
                // waits at one (ADR 0055); anywhere else the program gets the key.
                let prompt_jump = matches!(key, TerminalKey::Up | TerminalKey::Down)
                    && [
                        TerminalModifiers {
                            platform: true,
                            ..TerminalModifiers::default()
                        },
                        TerminalModifiers {
                            control: true,
                            ..TerminalModifiers::default()
                        },
                    ]
                    .contains(&modifiers);
                let jumped = if prompt_jump && kind != TerminalKeyEventKind::Release {
                    let prompts = if key == TerminalKey::Up { -1 } else { 1 };
                    lock_pane(&self.terminal)?.scroll_prompts(prompts)
                } else if prompt_jump {
                    // The press jumped, so the program never saw it or its release.
                    lock_pane(&self.terminal)?.scroll_prompts(0).map(|_| false)
                } else {
                    None
                };
                if let Some(moved) = jumped {
                    if moved {
                        publish_view(&self.revision, &self.update_sender);
                    }
                } else if only_shift
                    && !modes.contains(TermMode::ALT_SCREEN)
                    && matches!(key, TerminalKey::PageUp | TerminalKey::PageDown)
                {
                    // The press scrolled, so the program never saw it or its release.
                    if kind != TerminalKeyEventKind::Release {
                        self.scroll(match key {
                            TerminalKey::PageUp => TerminalScroll::PageUp,
                            TerminalKey::PageDown => TerminalScroll::PageDown,
                            _ => unreachable!(),
                        });
                    }
                } else {
                    let bytes = lock_pane(&self.terminal)?.encode_key(
                        &key,
                        modifiers,
                        kind,
                        self.reads_kitty_keys.load(Ordering::Relaxed),
                    )?;
                    // Letting go of a key is not input: it neither scrolls to the bottom
                    // nor clears the selection. A key that encodes to nothing (an
                    // unreported release, Cmd without kitty) does nothing at all.
                    if kind == TerminalKeyEventKind::Release {
                        self.input.try_write(bytes)?;
                    } else if !bytes.is_empty() {
                        self.write(bytes)?;
                    }
                }
                Ok(None)
            }
            TerminalCommand::Text(text) => {
                let bytes = lock_pane(&self.terminal)?.encode_text(&text);
                self.write(bytes)?;
                Ok(None)
            }
            TerminalCommand::Paste(text) => {
                let bytes = lock_pane(&self.terminal)?.encode_paste(&text);
                self.write(bytes)?;
                Ok(None)
            }
            TerminalCommand::Submit(text) => {
                self.submit(&text)?;
                Ok(None)
            }
            TerminalCommand::Mouse(event) => {
                let lossy = !matches!(event, TerminalMouseEvent::Button { .. });
                match self.handle_mouse(event) {
                    // Motion and wheel reports are a stream the next one supersedes: a
                    // program too busy to read them loses them, as in any terminal.
                    Err(error) if lossy && error.kind() == io::ErrorKind::WouldBlock => {}
                    result => result?,
                }
                Ok(None)
            }
            TerminalCommand::Focus(focused) => {
                let report = lock_pane(&self.terminal)?.focus_report(focused);
                if !focused {
                    self.release_mouse()?;
                }
                if let Some(report) = report {
                    self.input.try_write_control(report)?;
                }
                Ok(None)
            }
            TerminalCommand::Resize(size) => {
                self.request_resize(size)?;
                Ok(None)
            }
            TerminalCommand::Scroll(scroll) => {
                self.scroll(scroll);
                Ok(None)
            }
            TerminalCommand::Select(selection) => {
                self.select(selection);
                Ok(None)
            }
            TerminalCommand::SelectAt {
                position,
                display_offset,
                unit,
            } => {
                self.select_at(position, display_offset, unit);
                Ok(None)
            }
            TerminalCommand::Copy { selection } => Ok(self.copy_range(selection)),
        }
    }

    /// Validates a resize and replaces any older resize that has not started yet.
    pub fn request_resize(&self, size: TerminalSize) -> io::Result<()> {
        let size = size.validate()?;
        self.resize.request(size)
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn size(&self) -> TerminalSize {
        pane_size(&self.size)
    }

    pub fn view(&self) -> TerminalView {
        self.view_source().view()
    }

    pub fn view_source(&self) -> TerminalViewSource {
        TerminalViewSource {
            terminal: Arc::clone(&self.terminal),
            size: Arc::clone(&self.size),
            revision: Arc::clone(&self.revision),
            damage_baseline: Arc::clone(&self.damage_baseline),
            cursor_settle: Arc::clone(&self.cursor_settle),
        }
    }

    pub fn visible_text(&self) -> String {
        let size = self.size();
        lock_pane(&self.terminal).map_or_else(
            |_| String::new(),
            |mut terminal| terminal.visible_text(size),
        )
    }

    /// The last `lines` rows of the active screen and its scrollback, oldest first,
    /// with trailing blank rows dropped. Ignores the viewport scroll position.
    pub fn recent_text(&self, lines: usize) -> String {
        lock_pane(&self.terminal)
            .map_or_else(|_| String::new(), |terminal| terminal.recent_text(lines))
    }

    /// A Server launch failure is visible in the same Pane as native CLI failures.
    pub fn print_notice(&self, message: &str) {
        let message: String = message.chars().filter(|ch| !ch.is_control()).collect();
        let Ok(mut terminal) = lock_pane(&self.terminal) else {
            return;
        };
        terminal.write(format!("\r\n[Condr] {message}\r\n").as_bytes());
        drop(terminal);
        publish_view(&self.revision, &self.update_sender);
    }

    pub fn agent_probe(&self) -> Option<TerminalAgentProbe> {
        self.master.as_ref()?;
        Some(TerminalAgentProbe {
            #[cfg(unix)]
            master: Arc::downgrade(self.master.as_ref().expect("checked Terminal PTY master")),
            process: self.process.clone(),
            notices: Arc::clone(&self.notices),
            revision: Arc::clone(&self.revision),
            activity_revision: None,
            #[cfg(unix)]
            last_foreground_group: None,
        })
    }

    pub fn restore_agent(&self, resume: crate::AgentResume) {
        if let Ok(mut notices) = lock_pane(&self.notices) {
            notices.agent.restore(resume);
        }
    }

    /// Freeze process tracking before shutdown so stopping our own processes cannot
    /// turn the last live conversation into an ordinary agent exit.
    pub fn prepare_agent_shutdown(&self) {
        if let Ok(mut notices) = lock_pane(&self.notices) {
            notices.agent_stopping = true;
        }
    }

    /// Returns the last hook decision even if its monitor has not committed it yet.
    /// After closing, merges tail hooks using the identity frozen before signalling.
    pub fn agent_resume(&self) -> Option<Option<crate::AgentResume>> {
        let mut notices = lock_pane(&self.notices).ok()?;
        if notices.agent_stopping {
            for event in std::mem::take(&mut notices.agent_events) {
                event.apply(&mut notices.agent);
            }
        }
        notices.agent.resume()
    }

    pub fn scroll(&self, scroll: TerminalScroll) {
        let Ok(mut terminal) = lock_pane(&self.terminal) else {
            return;
        };
        if terminal.scroll(scroll) {
            drop(terminal);
            publish_view(&self.revision, &self.update_sender);
        }
    }

    pub fn copy_range(&self, selection: Option<TerminalSelection>) -> Option<String> {
        lock_pane(&self.terminal).ok()?.selection_text(selection)
    }

    /// Hands the selection to the VT, which keeps it on its text as output scrolls and
    /// the screen reflows and drops it on a screen swap, so the published viewport
    /// selection stays on the text the user picked (ADR 0008, ADR 0051).
    fn select(&self, selection: Option<TerminalSelection>) {
        let Ok(mut terminal) = lock_pane(&self.terminal) else {
            return;
        };
        terminal.select(selection);
        drop(terminal);
        publish_view(&self.revision, &self.update_sender);
    }

    /// A word or line selection anchored at one cell, which the VT expands across soft
    /// wraps.
    fn select_at(
        &self,
        position: TerminalPosition,
        display_offset: u32,
        unit: TerminalSelectionUnit,
    ) {
        let Ok(mut terminal) = lock_pane(&self.terminal) else {
            return;
        };
        terminal.select_at(position, display_offset, unit);
        drop(terminal);
        publish_view(&self.revision, &self.update_sender);
    }

    fn scroll_to_bottom(&self) {
        let Ok(mut terminal) = lock_pane(&self.terminal) else {
            return;
        };
        if terminal.scroll(TerminalScroll::Bottom) {
            drop(terminal);
            publish_view(&self.revision, &self.update_sender);
        }
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = {
            let mut child = lock_child(&self.child)?;
            let status = child
                .as_mut()
                .ok_or_else(|| io::Error::other("terminal child exit was already reported"))?
                .wait()?;
            *child = None;
            status
        };
        self.finish_io()?;
        Ok(status)
    }

    pub fn shutdown(&mut self) -> io::Result<ExitStatus> {
        let mut child = lock_child(&self.child)?
            .take()
            .ok_or_else(|| io::Error::other("terminal child exit was already reported"))?;
        let status = self.stop_process_and_io(Some(&mut *child))?;
        status.ok_or_else(|| io::Error::other("terminal child could not be reaped"))
    }

    /// Stops the Terminal runtime and releases its PTY resources. Repeated calls are harmless.
    pub fn close(&mut self) -> io::Result<()> {
        if lock_child(&self.child).is_ok_and(|child| child.is_some()) {
            self.shutdown().map(drop)
        } else {
            self.stop_process_and_io(None).map(drop)
        }
    }

    fn stop_process_and_io(
        &mut self,
        child: Option<&mut (dyn Child + Send + Sync)>,
    ) -> io::Result<Option<ExitStatus>> {
        let process = self.process.clone();
        let requires_child_status = child.is_some();
        let mut child = child;
        self.request_write_stop();
        let process_result = attempt_process_tree_shutdown(
            &process,
            &mut child,
            requires_child_status,
            &mut self.process_shutdown,
        );
        self.request_reader_stop();
        let io_result = self.finish_io();
        combine_cleanup_results(process_result, io_result, "stop terminal I/O")?;
        Ok(self.process_shutdown.status.take())
    }

    fn request_write_stop(&mut self) {
        self.input.stop();
        self.resize.stop();
        self.writer_stopping.store(true, Ordering::Release);
        #[cfg(unix)]
        if let Some(cancel) = self.writer_cancel.take() {
            let _ = cancel.shutdown(Shutdown::Both);
        }
    }

    fn request_reader_stop(&mut self) {
        #[cfg(unix)]
        if let Some(cancel) = self.reader_cancel.take() {
            let _ = cancel.shutdown(Shutdown::Both);
        }
    }

    fn finish_io(&mut self) -> io::Result<()> {
        self.request_write_stop();
        self.request_reader_stop();
        #[cfg(not(unix))]
        self.master.take();
        let writer_result = join(&mut self.writer, "terminal writer");
        let resize_result = join(&mut self.resizer, "terminal resizer");
        let reader_result = join(&mut self.reader, "terminal reader");
        #[cfg(unix)]
        self.master.take();
        let writer_and_resize =
            combine_cleanup_results(writer_result, resize_result, "stop terminal resizer");
        combine_cleanup_results(writer_and_resize, reader_result, "stop terminal reader")
    }
}

type SharedChild = Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>;

fn lock_child(
    child: &SharedChild,
) -> io::Result<MutexGuard<'_, Option<Box<dyn Child + Send + Sync>>>> {
    lock_pane(child)
}

fn live_mouse_position(
    position: TerminalMousePosition,
    display_offset: usize,
    screen_lines: usize,
) -> Option<TerminalMousePosition> {
    let row = usize::from(position.row).checked_sub(display_offset)?;
    (row < screen_lines).then_some(TerminalMousePosition {
        row: u16::try_from(row).ok()?,
        column: position.column,
    })
}

/// Notices the shell exiting even while a descendant keeps the PTY slave open, like
/// herdr's child watcher. PTY EOF alone would then never come, so the pane would look
/// alive forever and the child would stay unreaped. On Unix the watcher cancels the
/// reader, which drains the remaining bytes and publishes the exit in order; ConPTY has
/// no cancel, so the exit is published directly and `close` releases the reader.
fn child_exit_watch(
    child: Weak<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    #[cfg(unix)] exit_signal: UnixStream,
    #[cfg(not(unix))] exit_signal: mpsc::Sender<TerminalUpdate>,
) {
    loop {
        let Some(child) = child.upgrade() else {
            return;
        };
        {
            let mut child = lock_child(&child).expect("terminal child lock poisoned");
            let Some(child) = child.as_mut() else {
                // Reaped by `wait` or `shutdown`.
                return;
            };
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(_) => return,
            }
        }
        thread::sleep(CHILD_EXIT_POLL_INTERVAL);
    }
    #[cfg(unix)]
    let _ = exit_signal.shutdown(Shutdown::Both);
    #[cfg(not(unix))]
    let _ = exit_signal.send(TerminalUpdate::Exited);
}

impl Drop for TerminalRuntime {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            tracing::warn!("failed to close Terminal runtime: {error}");
        }
    }
}
