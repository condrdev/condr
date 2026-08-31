use super::*;

pub(super) struct UserInputPermit {
    bytes: usize,
    pending_bytes: Arc<AtomicUsize>,
    pending_entries: Arc<AtomicUsize>,
}

impl Drop for UserInputPermit {
    fn drop(&mut self) {
        self.pending_bytes.fetch_sub(self.bytes, Ordering::AcqRel);
        self.pending_entries.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct QueuedInput {
    pub(super) bytes: Vec<u8>,
    _user_permit: Option<UserInputPermit>,
}

#[derive(Clone)]
pub(super) struct TerminalInput {
    sender: mpsc::SyncSender<QueuedInput>,
    pending_bytes: Arc<AtomicUsize>,
    pending_entries: Arc<AtomicUsize>,
    accepting: Arc<AtomicBool>,
}

impl TerminalInput {
    pub(super) fn channel() -> (Self, mpsc::Receiver<QueuedInput>) {
        let (sender, receiver) =
            mpsc::sync_channel(INPUT_QUEUE_CAPACITY + TERMINAL_REPLY_QUEUE_RESERVE);
        (
            Self {
                sender,
                pending_bytes: Arc::new(AtomicUsize::new(0)),
                pending_entries: Arc::new(AtomicUsize::new(0)),
                accepting: Arc::new(AtomicBool::new(true)),
            },
            receiver,
        )
    }

    pub(super) fn try_write(&self, bytes: Vec<u8>) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if !self.accepting.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal writer stopped",
            ));
        }
        let bytes_len = bytes.len();
        self.pending_entries
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                (pending < INPUT_QUEUE_CAPACITY).then(|| pending + 1)
            })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "terminal input queue is full")
            })?;
        self.pending_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                pending
                    .checked_add(bytes_len)
                    .filter(|total| *total <= MAX_PENDING_INPUT_BYTES)
            })
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "terminal input byte budget is full",
                )
            })
            .inspect_err(|_| {
                self.pending_entries.fetch_sub(1, Ordering::AcqRel);
            })?;
        let input = QueuedInput {
            bytes,
            _user_permit: Some(UserInputPermit {
                bytes: bytes_len,
                pending_bytes: Arc::clone(&self.pending_bytes),
                pending_entries: Arc::clone(&self.pending_entries),
            }),
        };
        if !self.accepting.load(Ordering::Acquire) {
            drop(input);
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal writer stopped",
            ));
        }
        self.sender.try_send(input).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                io::Error::new(io::ErrorKind::WouldBlock, "terminal input queue is full")
            }
            mpsc::TrySendError::Disconnected(_) => {
                io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped")
            }
        })
    }

    pub(super) fn write_terminal_reply(&self, bytes: Vec<u8>) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if !self.accepting.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal writer stopped",
            ));
        }
        self.sender
            .send(QueuedInput {
                bytes,
                _user_permit: None,
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped"))
    }

    pub(super) fn stop(&self) {
        self.accepting.store(false, Ordering::Release);
    }
}

#[derive(Default)]
pub(super) struct ResizeState {
    pending: Option<TerminalSize>,
    stopping: bool,
}

#[derive(Default)]
pub(super) struct ResizeControl {
    state: Mutex<ResizeState>,
    wake: Condvar,
}

impl ResizeControl {
    pub(super) fn request(&self, size: TerminalSize) -> io::Result<()> {
        let mut state = self.state.lock().expect("terminal resize lock poisoned");
        if state.stopping {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal resizer stopped",
            ));
        }
        state.pending = Some(size);
        self.wake.notify_one();
        Ok(())
    }

    pub(super) fn stop(&self) {
        let mut state = self.state.lock().expect("terminal resize lock poisoned");
        state.stopping = true;
        state.pending = None;
        self.wake.notify_all();
    }

    pub(super) fn next(&self) -> Option<TerminalSize> {
        let mut state = self.state.lock().expect("terminal resize lock poisoned");
        while state.pending.is_none() && !state.stopping {
            state = self
                .wake
                .wait(state)
                .expect("terminal resize lock poisoned");
        }
        (!state.stopping).then(|| state.pending.take()).flatten()
    }
}

pub(super) struct ResizeWorkerGuard {
    control: Arc<ResizeControl>,
}

impl Drop for ResizeWorkerGuard {
    fn drop(&mut self) {
        self.control.stop();
    }
}

#[derive(Clone)]
pub(super) struct TerminalEventProxy {
    pub(super) input: TerminalInput,
    pub(super) size: Arc<Mutex<TerminalSize>>,
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
            let _ = self.input.write_terminal_reply(bytes);
        }
    }
}

pub(super) type Terminal = Term<TerminalEventProxy>;

pub(super) struct TerminalIoLoop {
    pub(super) writer: Box<dyn Write + Send>,
    pub(super) input: mpsc::Receiver<QueuedInput>,
    pub(super) stopping: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ReportedCwd {
    pub(super) cwd: Option<PathBuf>,
    pub(super) generation: u64,
}

#[cfg(unix)]
pub(super) fn unix_pty_writer(
    master: &dyn MasterPty,
) -> io::Result<(Box<dyn Write + Send>, UnixStream)> {
    let writer = master.take_writer().map_err(other_error)?;
    let poll_fd = duplicate_master_fd(master)?;
    let flags = fcntl(poll_fd.as_raw_fd(), FcntlArg::F_GETFL)
        .map(OFlag::from_bits_truncate)
        .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    fcntl(
        poll_fd.as_raw_fd(),
        FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK),
    )
    .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    let (cancel, cancel_reader) = UnixStream::pair()?;
    Ok((
        Box::new(UnixPtyWriter {
            writer,
            poll_fd,
            cancel: cancel_reader,
        }),
        cancel,
    ))
}

#[cfg(unix)]
pub(super) struct RawMasterFd(RawFd);

#[cfg(unix)]
impl AsRawFd for RawMasterFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

#[cfg(unix)]
pub(super) fn duplicate_master_fd(master: &dyn MasterPty) -> io::Result<FileDescriptor> {
    let raw_fd = master
        .as_raw_fd()
        .ok_or_else(|| io::Error::other("Unix PTY master does not expose a file descriptor"))?;
    FileDescriptor::dup(&RawMasterFd(raw_fd)).map_err(other_error)
}

#[cfg(unix)]
pub(super) struct UnixPtyWriter {
    writer: Box<dyn Write + Send>,
    poll_fd: FileDescriptor,
    cancel: UnixStream,
}

#[cfg(unix)]
impl Write for UnixPtyWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            let mut descriptors = [
                PollFd::new(self.poll_fd.as_fd(), PollFlags::POLLOUT),
                PollFd::new(self.cancel.as_fd(), PollFlags::POLLIN),
            ];
            poll(&mut descriptors, PollTimeout::NONE)
                .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
            let writer_events = descriptors[0].revents().unwrap_or(PollFlags::empty());
            let cancel_events = descriptors[1].revents().unwrap_or(PollFlags::empty());
            if cancel_events.intersects(
                PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL,
            ) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Terminal writer was cancelled",
                ));
            }
            if writer_events.contains(PollFlags::POLLNVAL) {
                return Err(io::Error::other(
                    "Terminal writer descriptor became invalid",
                ));
            }
            if writer_events
                .intersects(PollFlags::POLLOUT | PollFlags::POLLHUP | PollFlags::POLLERR)
            {
                match self.writer.write(bytes) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                    result => return result,
                }
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(unix)]
pub(super) struct UnixPtyReader {
    pub(super) reader: Box<dyn Read + Send>,
    pub(super) poll_fd: FileDescriptor,
    pub(super) cancel: UnixStream,
    pub(super) drain_reads: Option<u8>,
}

#[cfg(unix)]
impl Read for UnixPtyReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut descriptors = [
                PollFd::new(self.poll_fd.as_fd(), PollFlags::POLLIN),
                PollFd::new(self.cancel.as_fd(), PollFlags::POLLIN),
            ];
            let timeout = if self.drain_reads.is_some() {
                PollTimeout::ZERO
            } else {
                PollTimeout::NONE
            };
            poll(&mut descriptors, timeout)
                .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
            let pty_events = descriptors[0].revents().unwrap_or(PollFlags::empty());
            let cancel_events = descriptors[1].revents().unwrap_or(PollFlags::empty());
            let readable =
                pty_events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR);
            let cancelled = cancel_events
                .intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR);

            if cancelled && self.drain_reads.is_none() {
                self.drain_reads = Some(MAX_CANCEL_DRAIN_READS);
            }
            if let Some(remaining) = self.drain_reads.as_mut() {
                if !readable || *remaining == 0 {
                    return Ok(0);
                }
                match self.reader.read(bytes) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                    result => {
                        *remaining -= 1;
                        return result;
                    }
                }
            }
            if readable {
                match self.reader.read(bytes) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                    result => return result,
                }
            }
            if pty_events.contains(PollFlags::POLLNVAL)
                || cancel_events.contains(PollFlags::POLLNVAL)
            {
                return Err(io::Error::other(
                    "Terminal reader descriptor became invalid",
                ));
            }
        }
    }
}

pub(super) fn read_loop(
    mut reader: Box<dyn Read + Send>,
    terminal: Arc<Mutex<Terminal>>,
    revision: Arc<AtomicU64>,
    updates: mpsc::Sender<TerminalUpdate>,
    reported_cwd: Arc<Mutex<ReportedCwd>>,
) -> io::Result<()> {
    let mut parser: Processor = Processor::new();
    let mut cwd_parser = OscCwdParser::default();
    let mut bytes = [0; 16 * 1024];
    let result = loop {
        match reader.read(&mut bytes) {
            Ok(0) => break Ok(()),
            Ok(read) => {
                cwd_parser.advance(&bytes[..read], |cwd| {
                    record_reported_cwd(&reported_cwd, cwd);
                });
                let mut terminal = terminal.lock().expect("terminal state lock poisoned");
                parser.advance(&mut *terminal, &bytes[..read]);
                publish_view(&revision, &updates);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Err(error),
        }
    };
    let _ = updates.send(TerminalUpdate::Exited);
    result
}

pub(super) fn record_reported_cwd(reported_cwd: &Mutex<ReportedCwd>, cwd: PathBuf) {
    let Some(cwd) = existing_absolute_directory(cwd) else {
        return;
    };
    let mut reported = reported_cwd.lock().expect("Terminal cwd lock poisoned");
    reported.cwd = Some(cwd);
    reported.generation = reported.generation.wrapping_add(1);
}

#[derive(Default)]
pub(super) struct OscCwdParser {
    state: OscCwdState,
    payload: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
pub(super) enum OscCwdState {
    #[default]
    Ground,
    Prefix(usize),
    Payload,
    PayloadEscape,
    Discard,
    DiscardEscape,
}

impl OscCwdParser {
    const PREFIX: &'static [u8] = b"\x1b]9;9;";

    pub(super) fn advance(&mut self, bytes: &[u8], mut report: impl FnMut(PathBuf)) {
        for &byte in bytes {
            match self.state {
                OscCwdState::Ground => {
                    if byte == Self::PREFIX[0] {
                        self.state = OscCwdState::Prefix(1);
                    }
                }
                OscCwdState::Prefix(matched) => {
                    if byte == Self::PREFIX[matched] {
                        let matched = matched + 1;
                        if matched == Self::PREFIX.len() {
                            self.payload.clear();
                            self.state = OscCwdState::Payload;
                        } else {
                            self.state = OscCwdState::Prefix(matched);
                        }
                    } else if byte == Self::PREFIX[0] {
                        self.state = OscCwdState::Prefix(1);
                    } else {
                        self.state = OscCwdState::Ground;
                    }
                }
                OscCwdState::Payload => match byte {
                    0x07 => self.finish(&mut report),
                    0x1b => self.state = OscCwdState::PayloadEscape,
                    _ => self.push_payload(byte),
                },
                OscCwdState::PayloadEscape => match byte {
                    b'\\' => self.finish(&mut report),
                    0x07 => {
                        self.push_payload(0x1b);
                        if matches!(self.state, OscCwdState::Payload) {
                            self.finish(&mut report);
                        }
                    }
                    0x1b => {
                        self.push_payload(0x1b);
                        if matches!(self.state, OscCwdState::Payload) {
                            self.state = OscCwdState::PayloadEscape;
                        }
                    }
                    _ => {
                        self.push_payload(0x1b);
                        if matches!(self.state, OscCwdState::Payload) {
                            self.push_payload(byte);
                        }
                    }
                },
                OscCwdState::Discard => match byte {
                    0x07 => self.reset(),
                    0x1b => self.state = OscCwdState::DiscardEscape,
                    _ => {}
                },
                OscCwdState::DiscardEscape => match byte {
                    b'\\' => self.reset(),
                    0x1b => {}
                    _ => self.state = OscCwdState::Discard,
                },
            }
        }
    }

    pub(super) fn push_payload(&mut self, byte: u8) {
        if self.payload.len() == MAX_OSC_CWD_BYTES {
            self.payload.clear();
            self.state = OscCwdState::Discard;
        } else {
            self.payload.push(byte);
            self.state = OscCwdState::Payload;
        }
    }

    pub(super) fn finish(&mut self, report: &mut impl FnMut(PathBuf)) {
        if let Ok(payload) = std::str::from_utf8(&self.payload) {
            let payload = payload
                .strip_prefix('"')
                .and_then(|payload| payload.strip_suffix('"'))
                .unwrap_or(payload);
            report(PathBuf::from(payload));
        }
        self.reset();
    }

    pub(super) fn reset(&mut self) {
        self.payload.clear();
        self.state = OscCwdState::Ground;
    }
}

pub(super) fn io_loop(io: TerminalIoLoop) -> io::Result<()> {
    let TerminalIoLoop {
        mut writer,
        input,
        stopping,
    } = io;
    while !stopping.load(Ordering::Acquire) {
        let queued = match input.recv_timeout(IO_CONTROL_POLL_INTERVAL) {
            Ok(queued) => queued,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        if stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Err(error) = writer
            .write_all(&queued.bytes)
            .and_then(|()| writer.flush())
        {
            if stopping.load(Ordering::Acquire) {
                return Ok(());
            }
            return Err(error);
        }
    }
    Ok(())
}

pub(super) fn resize_loop(
    control: Arc<ResizeControl>,
    master: Weak<Mutex<Box<dyn MasterPty + Send>>>,
    terminal: Arc<Mutex<Terminal>>,
    current_size: Arc<Mutex<TerminalSize>>,
    revision: Arc<AtomicU64>,
    updates: mpsc::Sender<TerminalUpdate>,
) -> io::Result<()> {
    let _exit = ResizeWorkerGuard {
        control: Arc::clone(&control),
    };
    while let Some(size) = control.next() {
        master
            .upgrade()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "PTY was closed"))?
            .lock()
            .expect("PTY master lock poisoned")
            .resize(size.into())
            .map_err(other_error)?;
        let mut terminal = terminal.lock().expect("terminal state lock poisoned");
        terminal.resize(size);
        *current_size.lock().expect("terminal size lock poisoned") = size;
        drop(terminal);
        publish_view(&revision, &updates);
    }
    Ok(())
}
