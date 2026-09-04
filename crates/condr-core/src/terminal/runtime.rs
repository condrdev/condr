use super::*;

pub struct TerminalRuntime {
    terminal: Arc<Mutex<Terminal>>,
    master: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
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
        let terminal_config = Config {
            // Copy only: a program may fill the user's clipboard (like tmux `set-clipboard on`),
            // never read it. See ADR 0007.
            osc52: Osc52::OnlyCopy,
            ..Config::default()
        };
        let terminal = Arc::new(Mutex::new(Term::new(terminal_config, &size, event_proxy)));
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
            TerminalCommand::Copy { selection } => Ok(self.copy_range(selection)),
        }
    }

    fn handle_mouse(&self, event: TerminalMouseEvent) -> io::Result<()> {
        let (modes, display_offset, screen_lines) = {
            let terminal = self.terminal.lock().expect("terminal state lock poisoned");
            (
                *terminal.mode(),
                terminal.grid().display_offset(),
                terminal.screen_lines(),
            )
        };
        let shifted_wheel = matches!(
            event,
            TerminalMouseEvent::Wheel { modifiers, .. } if modifiers.shift
        );
        if !shifted_wheel && modes.intersects(TermMode::MOUSE_MODE) {
            return self.handle_application_mouse(event, modes, display_offset, screen_lines);
        }
        if let TerminalMouseEvent::Button {
            button,
            pressed: false,
            ..
        } = event
        {
            self.clear_reported_mouse_press(button);
        }

        let TerminalMouseEvent::Wheel {
            direction, amount, ..
        } = event
        else {
            return Ok(());
        };
        if amount == 0 || (!shifted_wheel && modes.intersects(TermMode::MOUSE_MODE)) {
            return Ok(());
        }

        if !shifted_wheel && modes.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let key = match direction {
                TerminalMouseWheel::Up => TerminalKey::Up,
                TerminalMouseWheel::Down => TerminalKey::Down,
                TerminalMouseWheel::Left | TerminalMouseWheel::Right => return Ok(()),
            };
            let bytes = encode_key(
                &key,
                TerminalModifiers::default(),
                modes.contains(TermMode::APP_CURSOR),
            )?
            .repeat(usize::from(amount.min(MAX_MOUSE_WHEEL_STEPS)));
            return self.input.try_write(bytes);
        }

        let lines = match direction {
            TerminalMouseWheel::Up => i32::from(amount),
            TerminalMouseWheel::Down => -i32::from(amount),
            TerminalMouseWheel::Left | TerminalMouseWheel::Right => return Ok(()),
        };
        self.scroll(TerminalScroll::Lines(lines));
        Ok(())
    }

    fn handle_application_mouse(
        &self,
        event: TerminalMouseEvent,
        modes: TermMode,
        display_offset: usize,
        screen_lines: usize,
    ) -> io::Result<()> {
        match event {
            TerminalMouseEvent::Button {
                button,
                pressed: true,
                position,
                modifiers,
            } => {
                let Some(position) = live_mouse_position(position, display_offset, screen_lines)
                else {
                    return Ok(());
                };
                let event = TerminalMouseEvent::Button {
                    button,
                    pressed: true,
                    position,
                    modifiers,
                };
                if self.write_mouse_report(event, modes)? {
                    *self
                        .reported_mouse_press
                        .lock()
                        .expect("reported mouse press lock poisoned") = Some(ReportedMousePress {
                        button,
                        position,
                        modifiers,
                    });
                }
            }
            TerminalMouseEvent::Button {
                button,
                pressed: false,
                position,
                modifiers,
            } => {
                let mut reported = self
                    .reported_mouse_press
                    .lock()
                    .expect("reported mouse press lock poisoned");
                let Some(press) = *reported else {
                    return Ok(());
                };
                if press.button != button {
                    return Ok(());
                }
                let position = live_mouse_position(position, display_offset, screen_lines)
                    .unwrap_or(press.position);
                let event = TerminalMouseEvent::Button {
                    button,
                    pressed: false,
                    position,
                    modifiers,
                };
                if let Some(bytes) = encode_mouse(event, modes) {
                    self.input.try_write_control(bytes)?;
                }
                *reported = None;
            }
            TerminalMouseEvent::Motion {
                button: Some(button),
                position,
                modifiers,
            } => {
                let Some(position) = live_mouse_position(position, display_offset, screen_lines)
                else {
                    return Ok(());
                };
                let mut reported = self
                    .reported_mouse_press
                    .lock()
                    .expect("reported mouse press lock poisoned");
                let Some(press) = reported.as_mut() else {
                    return Ok(());
                };
                if press.button != button {
                    return Ok(());
                }
                let event = TerminalMouseEvent::Motion {
                    button: Some(button),
                    position,
                    modifiers,
                };
                if let Some(bytes) = encode_mouse(event, modes) {
                    self.input.try_write(bytes)?;
                    press.position = position;
                    press.modifiers = modifiers;
                }
            }
            TerminalMouseEvent::Motion {
                button: None,
                position,
                modifiers,
            } => {
                let Some(position) = live_mouse_position(position, display_offset, screen_lines)
                else {
                    return Ok(());
                };
                self.write_mouse_report(
                    TerminalMouseEvent::Motion {
                        button: None,
                        position,
                        modifiers,
                    },
                    modes,
                )?;
            }
            TerminalMouseEvent::Wheel {
                direction,
                amount,
                position,
                modifiers,
            } => {
                let Some(position) = live_mouse_position(position, display_offset, screen_lines)
                else {
                    return Ok(());
                };
                self.write_mouse_report(
                    TerminalMouseEvent::Wheel {
                        direction,
                        amount,
                        position,
                        modifiers,
                    },
                    modes,
                )?;
            }
        }
        Ok(())
    }

    fn write_mouse_report(&self, event: TerminalMouseEvent, modes: TermMode) -> io::Result<bool> {
        let Some(bytes) = encode_mouse(event, modes) else {
            return Ok(false);
        };
        self.input.try_write(bytes)?;
        Ok(true)
    }

    fn clear_reported_mouse_press(&self, button: TerminalMouseButton) {
        let mut reported = self
            .reported_mouse_press
            .lock()
            .expect("reported mouse press lock poisoned");
        if reported.is_some_and(|press| press.button == button) {
            *reported = None;
        }
    }

    pub fn release_mouse(&self) -> io::Result<()> {
        let modes = *self
            .terminal
            .lock()
            .expect("terminal state lock poisoned")
            .mode();
        let mut reported = self
            .reported_mouse_press
            .lock()
            .expect("reported mouse press lock poisoned");
        let Some(press) = *reported else {
            return Ok(());
        };
        if let Some(bytes) = encode_mouse(
            TerminalMouseEvent::Button {
                button: press.button,
                pressed: false,
                position: press.position,
                modifiers: press.modifiers,
            },
            modes,
        ) {
            self.input.try_write_control(bytes)?;
        }
        *reported = None;
        Ok(())
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

    pub fn bottom_text(&self) -> String {
        let terminal = self.terminal.lock().expect("terminal state lock poisoned");
        bottom_text(&terminal)
    }

    pub fn agent_probe(&self) -> Option<TerminalAgentProbe> {
        self.master.as_ref()?;
        Some(TerminalAgentProbe {
            terminal: Arc::clone(&self.terminal),
            #[cfg(unix)]
            master: Arc::downgrade(self.master.as_ref().expect("checked Terminal PTY master")),
            process: self.process,
            notices: Arc::clone(&self.notices),
            revision: Arc::clone(&self.revision),
            detector: AgentDetector::new(),
            screen_revision: None,
            activity_revision: None,
            #[cfg(unix)]
            last_foreground_group: None,
        })
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

const CHILD_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

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

/// What a Pane's shell and everything it starts learn about their surroundings, as
/// environment variables. herdr does the same with `HERDR_*`; a `condr` CLI or an agent
/// hook running inside the Pane uses these to reach the Server and name its Pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneEnvironment {
    pub pane_id: PaneId,
    /// The Server's endpoint as `CONDR_SOCKET_PATH`: a local socket or pipe path, or
    /// `tcp://host:port`.
    pub socket_path: String,
}

impl PaneEnvironment {
    pub const ENV: &'static str = "CONDR_ENV";
    pub const PANE_ID: &'static str = "CONDR_PANE_ID";
    pub const SOCKET_PATH: &'static str = "CONDR_SOCKET_PATH";

    /// Sets the `CONDR_*` variables and puts the directory holding this executable in
    /// front of `PATH`, so a portable or `cargo run` build has `condr` on the Pane's PATH
    /// without an install step.
    pub fn apply(&self, command: &mut CommandBuilder) {
        command.env(Self::ENV, "1");
        command.env(Self::PANE_ID, self.pane_id.as_u64().to_string());
        command.env(Self::SOCKET_PATH, &self.socket_path);
        if let Some(path) = prepend_executable_directory(
            command
                .get_env("PATH")
                .map(ToOwned::to_owned)
                .or_else(|| std::env::var_os("PATH")),
        ) {
            command.env("PATH", path);
        }
    }
}

fn prepend_executable_directory(path: Option<std::ffi::OsString>) -> Option<std::ffi::OsString> {
    let directory = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let rest = path.unwrap_or_default();
    let mut entries = vec![directory.clone()];
    entries.extend(
        std::env::split_paths(&rest)
            .filter(|entry| !entry.as_os_str().is_empty() && *entry != directory),
    );
    std::env::join_paths(entries).ok()
}

/// The shell a terminal runs when none is configured: `$SHELL`, then the password
/// database, on Unix (as herdr and Zed do); on Windows PowerShell 7 when installed,
/// else Windows PowerShell, else `%ComSpec%`, in Zed's order.
pub fn default_shell_program() -> String {
    #[cfg(windows)]
    {
        // ponytail: PATH lookup only; Zed also probes Program Files, MSIX, scoop and
        // dotnet tools for pwsh. Add those when a real install goes unnoticed.
        if is_on_path("pwsh.exe") {
            return "pwsh.exe".to_owned();
        }
        if is_on_path("powershell.exe") {
            return "powershell.exe".to_owned();
        }
        std::env::var("ComSpec")
            .ok()
            .map(|comspec| comspec.trim().to_owned())
            .filter(|comspec| !comspec.is_empty())
            .unwrap_or_else(|| "cmd.exe".to_owned())
    }
    #[cfg(not(windows))]
    CommandBuilder::new_default_prog().get_shell()
}

#[cfg(windows)]
fn is_on_path(file_name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(file_name).is_file())
    })
}

/// The shell's file name without `.exe`, lower-cased; both separators count so a
/// Windows path classifies the same on every host.
fn shell_name(program: &str) -> String {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_owned()
}

/// The interactive shell command, with cwd reporting for the shells we know how to
/// hook: bash on Linux through a wrapper rc file, PowerShell on Windows through a
/// prompt hook. Any other shell starts plain.
fn shell_command(program: &str) -> CommandBuilder {
    match shell_name(program).as_str() {
        #[cfg(target_os = "linux")]
        "bash" => {
            let mut command = CommandBuilder::new("/bin/sh");
            command.args(["-c", LINUX_BASH_CWD_WRAPPER, "condr-shell", program]);
            command
        }
        #[cfg(windows)]
        "pwsh" | "powershell" => {
            let mut command = CommandBuilder::new(program);
            command.args([
                "-NoLogo",
                "-NoExit",
                "-Command",
                WINDOWS_POWERSHELL_CWD_HOOK,
            ]);
            command
        }
        _ => CommandBuilder::new(program),
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;

    #[test]
    fn shell_names_ignore_directories_extensions_and_case() {
        assert_eq!(shell_name("/usr/bin/bash"), "bash");
        assert_eq!(
            shell_name(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "pwsh"
        );
        assert_eq!(shell_name("PowerShell.EXE"), "powershell");
        assert_eq!(shell_name("fish"), "fish");
    }

    #[test]
    fn unknown_shells_start_without_extra_arguments() {
        assert_eq!(shell_command("fish").get_argv(), &["fish"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bash_is_wrapped_for_cwd_reporting() {
        let command = shell_command("/usr/local/bin/bash");
        let argv = command.get_argv();
        assert_eq!(argv[0], "/bin/sh");
        assert_eq!(argv.last().unwrap(), "/usr/local/bin/bash");
    }

    #[cfg(windows)]
    #[test]
    fn powershell_gets_the_prompt_hook() {
        for program in [
            "pwsh.exe",
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
        ] {
            let command = shell_command(program);
            let argv = command.get_argv();
            assert_eq!(argv[0], program);
            assert_eq!(argv.last().unwrap(), WINDOWS_POWERSHELL_CWD_HOOK);
        }
        assert_eq!(shell_command("cmd.exe").get_argv(), &["cmd.exe"]);
    }

    #[test]
    fn the_pane_environment_names_the_server_and_pane_and_leads_path() {
        let mut command = CommandBuilder::new("sh");
        command.env("PATH", "/usr/bin");
        PaneEnvironment {
            pane_id: PaneId::from_u64(42),
            socket_path: "/run/condr.sock".into(),
        }
        .apply(&mut command);
        assert_eq!(command.get_env("CONDR_ENV").unwrap(), "1");
        assert_eq!(command.get_env("CONDR_PANE_ID").unwrap(), "42");
        assert_eq!(
            command.get_env("CONDR_SOCKET_PATH").unwrap(),
            "/run/condr.sock"
        );
        let path = command.get_env("PATH").unwrap().to_owned();
        let mut entries = std::env::split_paths(&path);
        assert_eq!(
            entries.next().as_deref(),
            std::env::current_exe().unwrap().parent()
        );
        assert_eq!(entries.next().as_deref(), Some(Path::new("/usr/bin")));
    }

    #[test]
    fn the_default_shell_is_never_blank() {
        assert!(!default_shell_program().trim().is_empty());
    }
}

impl TerminalViewSource {
    pub fn view(&self) -> TerminalView {
        let terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let size = *self.size.lock().expect("terminal size lock poisoned");
        let mut view = snapshot_terminal(&terminal, size, self.revision.load(Ordering::Acquire));
        view.cursor = self.settled_cursor(view.cursor);
        view
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// A cursor move is being held back; `take_frame` after [`CURSOR_POSITION_SETTLE`]
    /// publishes it once it has stayed put.
    pub fn cursor_settle_pending(&self) -> bool {
        CURSOR_POSITION_SETTLE_ENABLED
            && self
                .cursor_settle
                .lock()
                .expect("cursor settle lock poisoned")
                .pending()
    }

    fn settled_cursor(&self, cursor: Option<TerminalCursor>) -> Option<TerminalCursor> {
        if !CURSOR_POSITION_SETTLE_ENABLED {
            return cursor;
        }
        let now = Instant::now();
        let mut settle = self
            .cursor_settle
            .lock()
            .expect("cursor settle lock poisoned");
        settle.observe(cursor, now);
        settle.reported(cursor, now)
    }

    pub fn take_frame(&self) -> Option<TerminalViewFrame> {
        let mut terminal = self.terminal.lock().expect("terminal state lock poisoned");
        let size = *self.size.lock().expect("terminal size lock poisoned");
        let mut revision = self.revision.load(Ordering::Acquire);
        let damage = match terminal.damage() {
            TermDamage::Full => None,
            TermDamage::Partial(lines) => Some(lines.collect::<Vec<_>>()),
        };
        let mut baseline = self
            .damage_baseline
            .lock()
            .expect("terminal damage baseline lock poisoned");
        let content = terminal.renderable_content();
        let display_offset = u32::try_from(content.display_offset).unwrap_or(u32::MAX);
        let mouse_tracking = TerminalMouseTracking::from_term_mode(*terminal.mode());
        let cursor = terminal_cursor(
            content.cursor.point,
            content.cursor.shape,
            terminal.cursor_style().blinking,
            content.display_offset,
            size,
        );
        let cursor = self.settled_cursor(cursor);
        if baseline
            .is_some_and(|baseline| revision <= baseline.revision && baseline.cursor != cursor)
        {
            // Nothing new arrived from the PTY, but the settled cursor moved: give the
            // cursor-only frame its own revision so clients accept it.
            revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        }
        let full = || {
            let mut view = snapshot_terminal(&terminal, size, revision);
            view.cursor = cursor;
            TerminalViewFrame::Full(view)
        };
        let requires_full = baseline.is_none_or(|baseline| {
            baseline.size != size || baseline.display_offset != display_offset
        }) || damage.is_none();

        let frame = if baseline.is_some_and(|baseline| revision <= baseline.revision) {
            None
        } else if requires_full {
            Some(full())
        } else {
            let previous = baseline.expect("partial damage has a baseline");
            let columns = usize::from(size.columns);
            let mut runs = Vec::new();
            let mut changed_cells = 0usize;
            let mut hyperlinks = SnapshotHyperlinks::default();
            for bounds in damage.expect("full damage was handled") {
                if bounds.line >= usize::from(size.rows) || bounds.left >= columns {
                    continue;
                }
                let right = bounds.right.min(columns - 1);
                if bounds.left > right {
                    continue;
                }
                let mut cells = Vec::with_capacity(right - bounds.left + 1);
                let line = i32::try_from(bounds.line).unwrap_or(i32::MAX)
                    - i32::try_from(content.display_offset).unwrap_or(i32::MAX);
                for column in bounds.left..=right {
                    cells.push(terminal_cell(
                        &terminal.grid()[Point::new(Line(line), Column(column))],
                        content.colors,
                        &mut hyperlinks,
                    ));
                }
                changed_cells += cells.len();
                runs.push(TerminalCellRun {
                    start: u32::try_from(bounds.line * columns + bounds.left)
                        .expect("terminal cell count fits u32"),
                    cells,
                });
            }
            if changed_cells > usize::from(size.rows) * columns / 2 {
                Some(full())
            } else {
                Some(TerminalViewFrame::Delta(TerminalViewDelta {
                    base_revision: previous.revision,
                    revision,
                    display_offset,
                    mouse_tracking,
                    cursor,
                    selection: viewport_selection(&terminal, size),
                    runs,
                }))
            }
        };

        *baseline = Some(TerminalDamageBaseline {
            revision,
            size,
            display_offset,
            cursor,
        });
        drop(baseline);
        terminal.reset_damage();
        frame
    }
}

#[derive(Clone)]
pub struct TerminalCwdProbe {
    #[cfg(unix)]
    pub(super) master: Option<Weak<Mutex<Box<dyn MasterPty + Send>>>>,
    pub(super) process: ProcessProbe,
    pub(super) last_known_cwd: Arc<Mutex<Option<PathBuf>>>,
    pub(super) reported_cwd: Arc<Mutex<ReportedCwd>>,
}

impl TerminalCwdProbe {
    pub fn cwd(&self) -> Option<PathBuf> {
        self.observe().0
    }

    pub fn observe(&self) -> (Option<PathBuf>, u64) {
        let mut last_known = self
            .last_known_cwd
            .lock()
            .expect("Terminal cwd lock poisoned");

        #[cfg(unix)]
        let foreground = self
            .master
            .as_ref()
            .and_then(Weak::upgrade)
            .and_then(|master| self.process.foreground_cwd(&master));
        let reported = self
            .reported_cwd
            .lock()
            .expect("Terminal cwd lock poisoned")
            .clone();
        let reported_generation = reported.generation;
        let reported_cwd = reported.cwd.and_then(existing_absolute_directory);

        #[cfg(unix)]
        let observed = foreground.or(reported_cwd).or_else(|| self.process.cwd());
        #[cfg(not(unix))]
        let observed = reported_cwd.or_else(|| self.process.cwd());
        let cwd = if let Some(cwd) = observed {
            *last_known = Some(cwd.clone());
            Some(cwd)
        } else {
            last_known.clone().and_then(existing_absolute_directory)
        };
        (cwd, reported_generation)
    }
}

#[derive(Clone)]
pub struct TerminalNoticeProbe {
    pub(super) notices: SharedTerminalNotices,
}

/// What changed since the previous `take`: `title` is `Some` only when the title changed
/// (to the new title, or `None` when it was reset); `bells` counts BELs, collapsed by the caller.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalNoticeBatch {
    pub title: Option<Option<String>>,
    pub bells: u64,
    /// Latest OSC 52 copy payload, including an empty clipboard clear.
    pub clipboard: Option<String>,
}

impl TerminalNoticeBatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.bells == 0 && self.clipboard.is_none()
    }
}

impl TerminalNoticeProbe {
    pub fn take(&self) -> TerminalNoticeBatch {
        let mut notices = self.notices.lock().expect("terminal notices lock poisoned");
        TerminalNoticeBatch {
            title: std::mem::take(&mut notices.title_changed).then(|| notices.title.clone()),
            bells: std::mem::take(&mut notices.bells),
            clipboard: std::mem::take(&mut notices.clipboard),
        }
    }
}

/// Drives agent detection for one Terminal: the process probe names the agent, the
/// screen and OSC evidence classify it, and the [`AgentDetector`] decides what to
/// publish. Poll it every [`TerminalAgentProbe::poll_interval`].
pub struct TerminalAgentProbe {
    pub(super) terminal: Arc<Mutex<Terminal>>,
    #[cfg(unix)]
    master: Weak<Mutex<Box<dyn MasterPty + Send>>>,
    pub(super) process: ProcessProbe,
    notices: SharedTerminalNotices,
    revision: Arc<AtomicU64>,
    detector: AgentDetector,
    /// The terminal revision the last screen read saw; unchanged means skip the read.
    screen_revision: Option<u64>,
    /// The terminal revision the last tick saw; new output keeps process probing fast.
    activity_revision: Option<u64>,
    #[cfg(unix)]
    last_foreground_group: Option<UnixPid>,
}

impl TerminalAgentProbe {
    pub fn poll_interval(&self) -> Duration {
        self.detector.poll_interval()
    }

    pub fn agent(&self) -> Option<AgentKind> {
        self.detector.agent()
    }

    /// One detection tick. `Some(None)` means the agent left; `Some(Some(_))` is a new
    /// snapshot to publish.
    pub fn poll(&mut self) -> Option<Option<AgentSnapshot>> {
        let now = Instant::now();
        let (osc_title, osc_progress) = self.osc_evidence();
        let foreground_changed = self.foreground_changed();
        let revision = self.revision.load(Ordering::Acquire);
        let output_changed = self.activity_revision != Some(revision);
        self.activity_revision = Some(revision);
        if self
            .detector
            .wants_process_probe(now, foreground_changed, output_changed)
        {
            let result = self.probe_process();
            match self
                .detector
                .observe_process(result, &osc_title, &osc_progress, now)
            {
                AgentPublish::Nothing => {}
                AgentPublish::Snapshot(snapshot) => {
                    self.screen_revision = None;
                    return Some(Some(snapshot));
                }
                AgentPublish::Cleared => return Some(None),
            }
        }
        let screen_changed = self.screen_revision != Some(revision);
        if !self.detector.wants_screen(screen_changed, now) {
            return None;
        }
        let screen = {
            let terminal = self.terminal.lock().expect("terminal state lock poisoned");
            bottom_text(&terminal)
        };
        self.screen_revision = Some(revision);
        match self.detector.observe_screen(
            DetectionInput {
                screen: &screen,
                osc_title: &osc_title,
                osc_progress: &osc_progress,
            },
            now,
        ) {
            AgentPublish::Nothing => None,
            AgentPublish::Snapshot(snapshot) => Some(Some(snapshot)),
            AgentPublish::Cleared => Some(None),
        }
    }

    fn osc_evidence(&self) -> (String, String) {
        let notices = self.notices.lock().expect("terminal notices lock poisoned");
        (
            notices.title.clone().unwrap_or_default(),
            notices.progress.clone().unwrap_or_default(),
        )
    }

    /// A foreground group change (Unix) forces an early process probe, as herdr does.
    fn foreground_changed(&mut self) -> bool {
        #[cfg(unix)]
        {
            let Some(master) = self.master.upgrade() else {
                return false;
            };
            let group = self.process.foreground_group(&master);
            let changed =
                self.last_foreground_group.is_some() && group != self.last_foreground_group;
            self.last_foreground_group = group;
            changed
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    fn probe_process(&self) -> ProcessProbeResult {
        #[cfg(unix)]
        {
            match self.master.upgrade() {
                Some(master) => self.process.probe_agent(&master),
                None => ProcessProbeResult::Unidentified,
            }
        }
        #[cfg(windows)]
        {
            self.process.probe_agent()
        }
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
        let _ = self.close();
    }
}
