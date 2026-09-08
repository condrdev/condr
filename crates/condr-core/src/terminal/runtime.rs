mod mouse;

use super::*;

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
        let mut child = pair.slave.spawn_command(command).map_err(other_error)?;
        let shell_pid = child.process_id();
        let process = ProcessProbe::new(shell_pid);
        drop(pair.slave);
        let master = Arc::new(Mutex::new(pair.master));

        let (input, input_receiver) = TerminalInput::channel();
        let current_size = Arc::new(Mutex::new(size));
        let (event_proxy, pending_replies, notices) =
            TerminalEventProxy::new(Arc::clone(&current_size));
        let terminal = Arc::new(Mutex::new(Term::new(terminal_config(), &size, event_proxy)));
        let revision = Arc::new(AtomicU64::new(0));
        let damage_baseline = Arc::new(Mutex::new(None));
        let cursor_settle = Arc::new(Mutex::new(CursorSettle::default()));
        let reported_cwd = Arc::new(Mutex::new(ReportedCwd::default()));
        let last_known_cwd = Arc::new(Mutex::new(initial_cwd));
        let (update_sender, updates) = mpsc::channel();

        let writer_stopping = Arc::new(AtomicBool::new(false));
        let writer_stop = Arc::clone(&writer_stopping);
        let writer_thread = match thread::Builder::new()
            .name("condr-pty-writer".into())
            .spawn(move || {
                io_loop(TerminalIoLoop {
                    writer,
                    input: input_receiver,
                    stopping: writer_stop,
                })
            }) {
            Ok(thread) => thread,
            Err(error) => {
                let _ = shutdown_process_tree(process, Some(&mut *child));
                return Err(error);
            }
        };

        let resize = Arc::new(ResizeControl::default());
        let resize_control = Arc::clone(&resize);
        let resize_master = Arc::downgrade(&master);
        let resize_terminal = Arc::clone(&terminal);
        let resize_size = Arc::clone(&current_size);
        let resize_revision = Arc::clone(&revision);
        let resize_updates = update_sender.clone();
        let resize_thread = match thread::Builder::new()
            .name("condr-pty-resizer".into())
            .spawn(move || {
                resize_loop(
                    resize_control,
                    resize_master,
                    resize_terminal,
                    resize_size,
                    resize_revision,
                    resize_updates,
                )
            }) {
            Ok(thread) => thread,
            Err(error) => {
                input.stop();
                writer_stopping.store(true, Ordering::Release);
                #[cfg(unix)]
                let _ = writer_cancel.shutdown(Shutdown::Both);
                let _ = writer_thread.join();
                let _ = shutdown_process_tree(process, Some(&mut *child));
                return Err(error);
            }
        };

        let reader_terminal = Arc::clone(&terminal);
        let reader_revision = Arc::clone(&revision);
        let reader_updates = update_sender.clone();
        let reader_reported_cwd = Arc::clone(&reported_cwd);
        let reader_notices = Arc::clone(&notices);
        let reader_input = input.clone();
        let reader_size = Arc::clone(&current_size);
        let reader_cursor_settle = Arc::clone(&cursor_settle);
        let reader_thread = match thread::Builder::new()
            .name("condr-pty-reader".into())
            .spawn(move || {
                read_loop(TerminalReadLoop {
                    reader,
                    terminal: reader_terminal,
                    input: reader_input,
                    pending_replies,
                    revision: reader_revision,
                    updates: reader_updates,
                    reported_cwd: reader_reported_cwd,
                    notices: reader_notices,
                    size: reader_size,
                    cursor_settle: reader_cursor_settle,
                })
            }) {
            Ok(thread) => thread,
            Err(error) => {
                input.stop();
                resize.stop();
                writer_stopping.store(true, Ordering::Release);
                #[cfg(unix)]
                let _ = writer_cancel.shutdown(Shutdown::Both);
                let _ = writer_thread.join();
                let _ = resize_thread.join();
                let _ = shutdown_process_tree(process, Some(&mut *child));
                return Err(error);
            }
        };

        #[cfg(not(unix))]
        let exit_signal = update_sender.clone();
        let child = Arc::new(Mutex::new(Some(child)));
        if let Err(error) = thread::Builder::new()
            .name("condr-pty-child-watch".into())
            .spawn({
                let child = Arc::downgrade(&child);
                move || child_exit_watch(child, exit_signal)
            })
        {
            // Same order as `stop_process_and_io`: the process tree goes first, and on
            // ConPTY the master is released so the reader, which has no cancel, unblocks.
            input.stop();
            resize.stop();
            writer_stopping.store(true, Ordering::Release);
            #[cfg(unix)]
            let _ = writer_cancel.shutdown(Shutdown::Both);
            let mut child = lock_child(&child).take();
            let _ = shutdown_process_tree(process, child.as_deref_mut());
            #[cfg(unix)]
            let _ = reader_cancel.shutdown(Shutdown::Both);
            #[cfg(not(unix))]
            drop(master);
            let _ = writer_thread.join();
            let _ = resize_thread.join();
            let _ = reader_thread.join();
            return Err(error);
        }

        Ok(Self {
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
            resize,
            child,
            process_shutdown: ProcessShutdownState::default(),
            reader: Some(reader_thread),
            writer: Some(writer_thread),
            resizer: Some(resize_thread),
            writer_stopping,
            #[cfg(unix)]
            reader_cancel: Some(reader_cancel),
            #[cfg(unix)]
            writer_cancel: Some(writer_cancel),
        })
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) -> io::Result<()> {
        self.scroll_to_bottom();
        self.clear_selection();
        self.input.try_write(bytes.into())
    }

    /// Paste and delayed Enter share one queue entry, so another client cannot split them.
    pub fn submit(&self, text: &str) -> io::Result<()> {
        let modes = *self
            .terminal
            .lock()
            .expect("terminal state lock poisoned")
            .mode();
        let bytes = encode_paste(text, modes.contains(TermMode::BRACKETED_PASTE));
        let enter = encode_key_in_mode(&TerminalKey::Enter, TerminalModifiers::default(), modes)?;
        self.scroll_to_bottom();
        self.clear_selection();
        self.input.try_submit(bytes, enter)
    }

    fn clear_selection(&self) {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        if terminal.selection.take().is_some() {
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
            process: if lock_child(&self.child).is_some() {
                self.process
            } else {
                ProcessProbe::new(None)
            },
            last_known_cwd: Arc::clone(&self.last_known_cwd),
            reported_cwd: Arc::clone(&self.reported_cwd),
        }
    }

    pub fn execute(&self, command: TerminalCommand) -> io::Result<Option<String>> {
        match command {
            TerminalCommand::Key { key, modifiers } => {
                let modes = *self
                    .terminal
                    .lock()
                    .expect("terminal state lock poisoned")
                    .mode();
                let only_shift = modifiers
                    == TerminalModifiers {
                        shift: true,
                        ..TerminalModifiers::default()
                    };
                if only_shift
                    && !modes.contains(TermMode::ALT_SCREEN)
                    && matches!(key, TerminalKey::PageUp | TerminalKey::PageDown)
                {
                    self.scroll(match key {
                        TerminalKey::PageUp => TerminalScroll::PageUp,
                        TerminalKey::PageDown => TerminalScroll::PageDown,
                        _ => unreachable!(),
                    });
                } else {
                    self.write(encode_key_in_mode(&key, modifiers, modes)?)?;
                }
                Ok(None)
            }
            TerminalCommand::Text(text) => {
                let modes = *self
                    .terminal
                    .lock()
                    .expect("terminal state lock poisoned")
                    .mode();
                self.write(encode_text_in_mode(&text, modes))?;
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
            TerminalCommand::Mouse(event) => {
                self.handle_mouse(event)?;
                Ok(None)
            }
            TerminalCommand::Focus(focused) => {
                let focus_reporting = self
                    .terminal
                    .lock()
                    .expect("terminal state lock poisoned")
                    .mode()
                    .contains(TermMode::FOCUS_IN_OUT);
                if !focused {
                    self.release_mouse()?;
                }
                if focus_reporting {
                    self.input
                        .try_write_control(if focused { b"\x1b[I" } else { b"\x1b[O" }.to_vec())?;
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
        *self.size.lock().expect("terminal size lock poisoned")
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

    /// The last `lines` rows of the active screen and its scrollback, oldest first,
    /// with trailing blank rows dropped. Ignores the viewport scroll position.
    pub fn recent_text(&self, lines: usize) -> String {
        let terminal = self.terminal.lock().expect("terminal state lock poisoned");
        recent_text(&terminal, lines)
    }

    /// A Server launch failure is visible in the same Pane as native CLI failures.
    pub fn print_notice(&self, message: &str) {
        let message: String = message.chars().filter(|ch| !ch.is_control()).collect();
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let mut processor: Processor = Processor::new();
        processor.advance(
            &mut *terminal,
            format!("\r\n[Condr] {message}\r\n").as_bytes(),
        );
        drop(terminal);
        publish_view(&self.revision, &self.update_sender);
    }

    pub fn agent_probe(&self) -> Option<TerminalAgentProbe> {
        self.master.as_ref()?;
        Some(TerminalAgentProbe {
            #[cfg(unix)]
            master: Arc::downgrade(self.master.as_ref().expect("checked Terminal PTY master")),
            process: self.process,
            notices: Arc::clone(&self.notices),
            revision: Arc::clone(&self.revision),
            activity_revision: None,
            #[cfg(unix)]
            last_foreground_group: None,
        })
    }

    pub fn restore_agent(&self, resume: crate::AgentResume) {
        self.notices
            .lock()
            .expect("terminal notices lock poisoned")
            .agent
            .restore(resume);
    }

    /// Freeze process tracking before shutdown so stopping our own processes cannot
    /// turn the last live conversation into an ordinary agent exit.
    pub fn prepare_agent_shutdown(&self) {
        self.notices
            .lock()
            .expect("terminal notices lock poisoned")
            .agent_stopping = true;
    }

    /// Returns the last hook decision even if its monitor has not committed it yet.
    /// After closing, merges tail hooks using the identity frozen before signalling.
    pub fn agent_resume(&self) -> Option<Option<crate::AgentResume>> {
        let mut notices = self.notices.lock().expect("terminal notices lock poisoned");
        if notices.agent_stopping {
            for event in std::mem::take(&mut notices.agent_events) {
                notices.agent.observe_event(&event);
            }
        }
        notices.agent.resume()
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
            publish_view(&self.revision, &self.update_sender);
        }
    }

    pub fn copy_range(&self, selection: Option<TerminalSelection>) -> Option<String> {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let Some(selection) = selection else {
            return terminal.selection_to_string();
        };
        let previous = terminal.selection.take();
        terminal.selection = Some(grid_selection(&terminal, selection));
        let text = terminal.selection_to_string();
        terminal.selection = previous;
        text
    }

    /// Hands the selection to alacritty, which rotates it with scrolled output and drops
    /// it on resize or screen swaps, so the published viewport selection stays on the
    /// text the user picked.
    fn select(&self, selection: Option<TerminalSelection>) {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        terminal.selection = selection.map(|selection| grid_selection(&terminal, selection));
        drop(terminal);
        publish_view(&self.revision, &self.update_sender);
    }

    /// A word or line selection anchored at one cell. alacritty expands it, to the semantic
    /// escape characters in [`terminal_config`] or the matching bracket, across soft wraps.
    fn select_at(
        &self,
        position: TerminalPosition,
        display_offset: u32,
        unit: TerminalSelectionUnit,
    ) {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let point = viewport_point(&terminal, position, display_offset);
        let ty = match unit {
            TerminalSelectionUnit::Word => SelectionType::Semantic,
            TerminalSelectionUnit::Line => SelectionType::Lines,
        };
        terminal.selection = Some(Selection::new(ty, point, side(position.side)));
        drop(terminal);
        publish_view(&self.revision, &self.update_sender);
    }

    fn scroll_to_bottom(&self) {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        if terminal.grid().display_offset() != 0 {
            terminal.scroll_display(Scroll::Bottom);
            drop(terminal);
            publish_view(&self.revision, &self.update_sender);
        }
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = {
            let mut child = lock_child(&self.child);
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
        let mut child = lock_child(&self.child)
            .take()
            .ok_or_else(|| io::Error::other("terminal child exit was already reported"))?;
        let status = self.stop_process_and_io(Some(&mut *child))?;
        status.ok_or_else(|| io::Error::other("terminal child could not be reaped"))
    }

    /// Stops the Terminal runtime and releases its PTY resources. Repeated calls are harmless.
    pub fn close(&mut self) -> io::Result<()> {
        if lock_child(&self.child).is_some() {
            self.shutdown().map(drop)
        } else {
            self.stop_process_and_io(None).map(drop)
        }
    }

    fn stop_process_and_io(
        &mut self,
        child: Option<&mut (dyn Child + Send + Sync)>,
    ) -> io::Result<Option<ExitStatus>> {
        let process = self.process;
        let requires_child_status = child.is_some();
        let mut child = child;
        self.request_write_stop();
        let initial_process_result = attempt_process_tree_shutdown(
            process,
            &mut child,
            requires_child_status,
            &mut self.process_shutdown,
        );
        self.request_reader_stop();
        let io_result = self.finish_io();
        #[cfg(windows)]
        let process_result = if initial_process_result.is_err() {
            // Releasing ConPTY can be the final exit trigger. Reuse the first attempt's
            // process identities so descendants remain visible after the shell exits.
            attempt_process_tree_shutdown(
                process,
                &mut child,
                requires_child_status,
                &mut self.process_shutdown,
            )
        } else {
            initial_process_result
        };
        #[cfg(not(windows))]
        let process_result = initial_process_result;
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

pub(super) fn terminal_config() -> Config {
    Config {
        // Copy only: a program may fill the user's clipboard (like tmux `set-clipboard on`),
        // never read it. See ADR 0007.
        osc52: Osc52::OnlyCopy,
        // alacritty's set without `:`, so a double-click keeps URLs and Windows paths whole.
        semantic_escape_chars: ",│`|\"' ()[]{}<>\t".into(),
        ..Config::default()
    }
}

pub(super) fn grid_selection(terminal: &Terminal, selection: TerminalSelection) -> Selection {
    let start = viewport_point(terminal, selection.start, selection.display_offset);
    let end = viewport_point(terminal, selection.end, selection.display_offset);
    let mut range = Selection::new(SelectionType::Simple, start, side(selection.start.side));
    range.update(end, side(selection.end.side));
    range
}

type SharedChild = Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>;

fn lock_child(child: &SharedChild) -> MutexGuard<'_, Option<Box<dyn Child + Send + Sync>>> {
    child.lock().expect("terminal child lock poisoned")
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
            let mut child = lock_child(&child);
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
        let _ = self.close();
    }
}
