use std::borrow::Cow;

use super::*;
use alacritty_terminal::vte::ansi::Rgb;

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

pub(super) struct ControlInputPermit {
    pending_entries: Arc<AtomicUsize>,
}

impl Drop for ControlInputPermit {
    fn drop(&mut self) {
        self.pending_entries.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct QueuedInput {
    pub(super) bytes: Vec<u8>,
    submit_split: Option<usize>,
    _user_permit: Option<UserInputPermit>,
    _control_permit: Option<ControlInputPermit>,
    terminal_replies: Option<Arc<Mutex<TerminalReplyState>>>,
}

#[derive(Default)]
pub(super) struct TerminalReplyState {
    pending: VecDeque<Vec<u8>>,
    token_queued: bool,
}

pub(super) struct TerminalInputReceiver {
    receiver: mpsc::Receiver<QueuedInput>,
    terminal_replies: Arc<Mutex<TerminalReplyState>>,
    accepting: Arc<AtomicBool>,
}

impl TerminalInputReceiver {
    fn recv_timeout(&self, timeout: Duration) -> Result<QueuedInput, mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }

    #[cfg(test)]
    pub(super) fn recv(&self) -> Result<QueuedInput, mpsc::RecvError> {
        self.receiver.recv()
    }
}

impl Drop for TerminalInputReceiver {
    fn drop(&mut self) {
        self.accepting.store(false, Ordering::Release);
        let mut replies = self
            .terminal_replies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        replies.pending.clear();
        replies.token_queued = false;
    }
}

#[derive(Clone)]
pub(super) struct TerminalInput {
    sender: mpsc::SyncSender<QueuedInput>,
    pending_bytes: Arc<AtomicUsize>,
    pending_entries: Arc<AtomicUsize>,
    pending_control_entries: Arc<AtomicUsize>,
    terminal_replies: Arc<Mutex<TerminalReplyState>>,
    accepting: Arc<AtomicBool>,
}

impl TerminalInput {
    pub(super) fn channel() -> (Self, TerminalInputReceiver) {
        let (sender, receiver) = mpsc::sync_channel(
            INPUT_QUEUE_CAPACITY + TERMINAL_REPLY_QUEUE_RESERVE + TERMINAL_CONTROL_QUEUE_RESERVE,
        );
        let terminal_replies = Arc::new(Mutex::new(TerminalReplyState::default()));
        let accepting = Arc::new(AtomicBool::new(true));
        (
            Self {
                sender,
                pending_bytes: Arc::new(AtomicUsize::new(0)),
                pending_entries: Arc::new(AtomicUsize::new(0)),
                pending_control_entries: Arc::new(AtomicUsize::new(0)),
                terminal_replies: Arc::clone(&terminal_replies),
                accepting: Arc::clone(&accepting),
            },
            TerminalInputReceiver {
                receiver,
                terminal_replies,
                accepting,
            },
        )
    }

    pub(super) fn try_write(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.try_write_user(bytes, None)
    }

    pub(super) fn try_submit(&self, mut text: Vec<u8>, enter: Vec<u8>) -> io::Result<()> {
        let split = text.len();
        text.extend(enter);
        self.try_write_user(text, Some(split))
    }

    fn try_write_user(&self, bytes: Vec<u8>, submit_split: Option<usize>) -> io::Result<()> {
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
            submit_split,
            _user_permit: Some(UserInputPermit {
                bytes: bytes_len,
                pending_bytes: Arc::clone(&self.pending_bytes),
                pending_entries: Arc::clone(&self.pending_entries),
            }),
            _control_permit: None,
            terminal_replies: None,
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
        let mut replies = self
            .terminal_replies
            .lock()
            .expect("terminal reply queue lock poisoned");
        if !self.accepting.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal writer stopped",
            ));
        }
        if replies.token_queued {
            replies.pending.push_back(bytes);
            return Ok(());
        }
        replies.token_queued = true;
        let input = QueuedInput {
            bytes,
            submit_split: None,
            _user_permit: None,
            _control_permit: None,
            terminal_replies: Some(Arc::clone(&self.terminal_replies)),
        };
        if !self.accepting.load(Ordering::Acquire) {
            replies.token_queued = false;
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal writer stopped",
            ));
        }
        match self.sender.try_send(input) {
            Ok(()) => Ok(()),
            Err(error) => {
                replies.token_queued = false;
                Err(match error {
                    mpsc::TrySendError::Full(_) => {
                        io::Error::new(io::ErrorKind::WouldBlock, "terminal reply queue is full")
                    }
                    mpsc::TrySendError::Disconnected(_) => {
                        io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped")
                    }
                })
            }
        }
    }

    pub(super) fn try_write_control(&self, bytes: Vec<u8>) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if !self.accepting.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal writer stopped",
            ));
        }
        self.pending_control_entries
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                (pending < TERMINAL_CONTROL_QUEUE_RESERVE).then(|| pending + 1)
            })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "terminal control queue is full")
            })?;
        let input = QueuedInput {
            bytes,
            submit_split: None,
            _user_permit: None,
            _control_permit: Some(ControlInputPermit {
                pending_entries: Arc::clone(&self.pending_control_entries),
            }),
            terminal_replies: None,
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
                io::Error::new(io::ErrorKind::WouldBlock, "terminal control queue is full")
            }
            mpsc::TrySendError::Disconnected(_) => {
                io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped")
            }
        })
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
    pub(super) size: Arc<Mutex<TerminalSize>>,
    pub(super) pending_replies: PendingTerminalReplies,
    pub(super) notices: SharedTerminalNotices,
}

/// Out-of-band signals the VT emits alongside grid changes: the OSC 0/2 title, BEL and
/// OSC 52 copies. The monitor drains them with [`TerminalNoticeProbe::take`]; bells are
/// counted, not queued, and only the latest clipboard payload is retained.
#[derive(Debug, Default)]
pub(super) struct TerminalNotices {
    pub(super) title: Option<String>,
    pub(super) title_changed: bool,
    pub(super) bells: u64,
    pub(super) clipboard: Option<String>,
    /// Hook events since the agent probe last drained them, oldest first.
    pub(super) agent_events: Vec<AgentEvent>,
}

pub(super) type SharedTerminalNotices = Arc<Mutex<TerminalNotices>>;

/// More queued hook events than this means nobody is draining them; the oldest go.
const MAX_QUEUED_AGENT_EVENTS: usize = 64;

impl TerminalNotices {
    fn record_agent_event(&mut self, event: AgentEvent) {
        if self.agent_events.len() == MAX_QUEUED_AGENT_EVENTS {
            self.agent_events.remove(0);
        }
        self.agent_events.push(event);
    }

    fn record_title(&mut self, title: Option<String>) {
        if self.title != title {
            self.title = title;
            self.title_changed = true;
        }
    }

    fn record_clipboard(&mut self, text: String) {
        if text.len() <= MAX_PENDING_CLIPBOARD_BYTES {
            self.clipboard = Some(text);
        }
    }
}

/// Drops control characters, trims, and caps the length. Empty becomes `None`. Spinner
/// glyphs stay: Claude Code ("✳ Claude Code") and Codex ("⠼ condr") animate them as
/// their activity indicator, and Windows Terminal shows them as is.
pub(super) fn sanitize_terminal_title(raw: &str) -> Option<String> {
    let title = raw
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let title = title
        .trim()
        .chars()
        .take(MAX_TERMINAL_TITLE_CHARS)
        .collect::<String>();
    (!title.is_empty()).then_some(title)
}

pub(super) enum PendingTerminalReply {
    Bytes(Vec<u8>),
    Color(usize, Arc<dyn Fn(Rgb) -> String + Sync + Send + 'static>),
}

pub(super) type PendingTerminalReplies = Arc<Mutex<Vec<PendingTerminalReply>>>;

impl TerminalEventProxy {
    pub(super) fn new(
        size: Arc<Mutex<TerminalSize>>,
    ) -> (Self, PendingTerminalReplies, SharedTerminalNotices) {
        let pending_replies = Arc::new(Mutex::new(Vec::new()));
        let notices = SharedTerminalNotices::default();
        (
            Self {
                size,
                pending_replies: Arc::clone(&pending_replies),
                notices: Arc::clone(&notices),
            },
            pending_replies,
            notices,
        )
    }

    fn notices(&self) -> std::sync::MutexGuard<'_, TerminalNotices> {
        self.notices.lock().expect("terminal notices lock poisoned")
    }
}

impl EventListener for TerminalEventProxy {
    fn send_event(&self, event: Event) {
        let reply = match event {
            Event::Title(title) => {
                self.notices().record_title(sanitize_terminal_title(&title));
                return;
            }
            Event::ResetTitle => {
                self.notices().record_title(None);
                return;
            }
            Event::Bell => {
                let mut notices = self.notices();
                notices.bells = notices.bells.saturating_add(1);
                return;
            }
            Event::ClipboardStore(_, text) => {
                self.notices().record_clipboard(text);
                return;
            }
            Event::PtyWrite(text) => Some(PendingTerminalReply::Bytes(text.into_bytes())),
            Event::TextAreaSizeRequest(formatter) => {
                let size = *self.size.lock().expect("terminal size lock poisoned");
                Some(PendingTerminalReply::Bytes(
                    formatter(size.window_size()).into_bytes(),
                ))
            }
            Event::ColorRequest(index, formatter) => {
                Some(PendingTerminalReply::Color(index, formatter))
            }
            _ => None,
        };
        if let Some(reply) = reply {
            self.pending_replies
                .lock()
                .expect("terminal reply queue lock poisoned")
                .push(reply);
        }
    }
}

pub(super) fn flush_terminal_replies(
    terminal: &Arc<Mutex<Terminal>>,
    input: &TerminalInput,
    pending_replies: &PendingTerminalReplies,
) -> io::Result<()> {
    let replies = std::mem::take(
        &mut *pending_replies
            .lock()
            .expect("terminal reply queue lock poisoned"),
    );
    if replies.is_empty() {
        return Ok(());
    }
    let terminal = terminal.lock().expect("terminal state lock poisoned");
    let mut bytes = Vec::new();
    for reply in replies {
        match reply {
            PendingTerminalReply::Bytes(reply) => bytes.extend(reply),
            PendingTerminalReply::Color(index, formatter) => {
                let color = terminal.colors()[index].or_else(|| default_terminal_color(index));
                let Some(color) = color else {
                    continue;
                };
                bytes.extend(formatter(color).bytes());
            }
        }
    }
    drop(terminal);
    input.write_terminal_reply(bytes)
}

fn default_terminal_color(index: usize) -> Option<Rgb> {
    const NORMAL: [Rgb; 8] = [
        rgb(0x48_4f_58),
        rgb(0xff_7b_72),
        rgb(0x3f_b9_50),
        rgb(0xd2_99_22),
        rgb(0x58_a6_ff),
        rgb(0xbc_8c_ff),
        rgb(0x39_c5_cf),
        rgb(0xb1_ba_c4),
    ];
    const BRIGHT: [Rgb; 8] = [
        rgb(0x6e_76_81),
        rgb(0xff_a1_98),
        rgb(0x56d364),
        rgb(0xe3_b3_41),
        rgb(0x79_c0_ff),
        rgb(0xd2_a8_ff),
        rgb(0x56_d4_dd),
        rgb(0xff_ff_ff),
    ];

    match index {
        0..=7 => Some(NORMAL[index]),
        8..=15 => Some(BRIGHT[index - 8]),
        16..=231 => {
            let value = index - 16;
            Some(Rgb {
                r: color_cube(value / 36),
                g: color_cube((value / 6) % 6),
                b: color_cube(value % 6),
            })
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            Some(Rgb {
                r: gray as u8,
                g: gray as u8,
                b: gray as u8,
            })
        }
        index if index == NamedColor::Foreground as usize => Some(rgb(0xc9_d1_d9)),
        index if index == NamedColor::Background as usize => Some(rgb(0x0d_11_17)),
        index if index == NamedColor::Cursor as usize => Some(rgb(0xf0_f6_fc)),
        _ => None,
    }
}

const fn rgb(value: u32) -> Rgb {
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

fn color_cube(value: usize) -> u8 {
    if value == 0 {
        0
    } else {
        (55 + value * 40) as u8
    }
}

pub(super) type Terminal = Term<TerminalEventProxy>;

pub(super) struct TerminalIoLoop {
    pub(super) writer: Box<dyn Write + Send>,
    pub(super) input: TerminalInputReceiver,
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

/// Everything the PTY reader thread owns.
pub(super) struct TerminalReadLoop {
    pub(super) reader: Box<dyn Read + Send>,
    pub(super) terminal: Arc<Mutex<Terminal>>,
    pub(super) input: TerminalInput,
    pub(super) pending_replies: PendingTerminalReplies,
    pub(super) revision: Arc<AtomicU64>,
    pub(super) updates: mpsc::Sender<TerminalUpdate>,
    pub(super) reported_cwd: Arc<Mutex<ReportedCwd>>,
    pub(super) notices: SharedTerminalNotices,
    pub(super) size: Arc<Mutex<TerminalSize>>,
    pub(super) cursor_settle: Arc<Mutex<CursorSettle>>,
}

pub(super) fn read_loop(io: TerminalReadLoop) -> io::Result<()> {
    let TerminalReadLoop {
        mut reader,
        terminal,
        input,
        pending_replies,
        revision,
        updates,
        reported_cwd,
        notices,
        size,
        cursor_settle,
    } = io;
    let mut parser: Processor = Processor::new();
    let mut scanner = OscScanner::default();
    let mut bytes = [0; 16 * 1024];
    let result = loop {
        match reader.read(&mut bytes) {
            Ok(0) => break Ok(()),
            Ok(read) => {
                let filtered = scanner.advance(&bytes[..read], |osc| match osc {
                    OscReport::Cwd(cwd) => record_reported_cwd(&reported_cwd, cwd),
                    OscReport::Agent(event) => notices
                        .lock()
                        .expect("terminal notices lock poisoned")
                        .record_agent_event(event),
                });
                let mut terminal_guard = terminal.lock().expect("terminal state lock poisoned");
                parser.advance(&mut *terminal_guard, &filtered);
                if CURSOR_POSITION_SETTLE_ENABLED {
                    // Observe after every read like herdr does, so a position that only
                    // lasts between two reads never counts as held, whatever the frame rate.
                    let size = *size.lock().expect("terminal size lock poisoned");
                    let content = terminal_guard.renderable_content();
                    let cursor = terminal_cursor(
                        content.cursor.point,
                        content.cursor.shape,
                        terminal_guard.cursor_style().blinking,
                        content.display_offset,
                        size,
                    );
                    cursor_settle
                        .lock()
                        .expect("cursor settle lock poisoned")
                        .observe(cursor, Instant::now());
                }
                drop(terminal_guard);
                if let Err(error) = flush_terminal_replies(&terminal, &input, &pending_replies) {
                    break Err(error);
                }
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
pub(super) struct OscScanner {
    state: OscState,
    payload: Vec<u8>,
    /// Bytes of a possible agent event held back from the VT across reads.
    held: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
pub(super) enum OscState {
    #[default]
    Ground,
    Prefix(usize),
    Payload,
    PayloadEscape,
    Discard,
    DiscardEscape,
    /// Inside DCS/SOS/PM/APC, which only ST ends; BEL is payload there.
    ControlString,
    ControlStringEscape,
}

/// The OSC payloads Condr reads itself because vte drops them: cwd reports as OSC 7
/// `file://` URIs, ConEmu `9;9;<cwd>` and iTerm2 `1337;CurrentDir=<cwd>`, and the OSC 777
/// agent events `condr agent-hook` writes (ADR 0014). This scanner runs over the raw bytes.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum OscReport {
    Cwd(PathBuf),
    Agent(AgentEvent),
}

/// What the VT gets from one read: the bytes as they came, or a copy with the agent
/// event sequences cut out. Everything from an `ESC` is held back until the scanner
/// knows the sequence is not an agent event, which for anything else is settled within
/// a few bytes, so ordinary output is borrowed, not copied.
struct VtFilter<'a, 'h> {
    bytes: &'a [u8],
    carry: &'h mut Vec<u8>,
    out: Option<Vec<u8>>,
    /// How much of `bytes` is accounted for in `out` (passed or dropped).
    emitted: usize,
    hold_from: Option<usize>,
}

impl<'a, 'h> VtFilter<'a, 'h> {
    fn new(bytes: &'a [u8], carry: &'h mut Vec<u8>) -> Self {
        let hold_from = (!carry.is_empty()).then_some(0);
        Self {
            bytes,
            carry,
            out: None,
            emitted: 0,
            hold_from,
        }
    }

    fn hold(&mut self, at: usize) {
        self.hold_from = Some(at);
    }

    /// Makes `out` real and current up to `upto`.
    fn materialize(&mut self, upto: usize) {
        match &mut self.out {
            Some(out) => out.extend_from_slice(&self.bytes[self.emitted..upto]),
            None => self.out = Some(self.bytes[..upto].to_vec()),
        }
        self.emitted = upto;
    }

    /// The held bytes were not an agent event after all: the VT gets them, in order.
    fn release(&mut self, end: usize) {
        let Some(from) = self.hold_from.take() else {
            return;
        };
        if self.out.is_none() && self.carry.is_empty() {
            return;
        }
        self.materialize(from);
        let out = self.out.as_mut().expect("materialized");
        out.append(self.carry);
        out.extend_from_slice(&self.bytes[from..end]);
        self.emitted = end;
    }

    /// The held bytes were an agent event: the VT never sees them.
    fn discard(&mut self, end: usize) {
        let Some(from) = self.hold_from.take() else {
            return;
        };
        self.materialize(from);
        self.carry.clear();
        self.emitted = end;
    }

    fn finish(mut self) -> Cow<'a, [u8]> {
        if let Some(from) = self.hold_from.take() {
            self.materialize(from);
            self.carry.extend_from_slice(&self.bytes[from..]);
            self.emitted = self.bytes.len();
        }
        match self.out {
            Some(mut out) => {
                out.extend_from_slice(&self.bytes[self.emitted..]);
                Cow::Owned(out)
            }
            None => Cow::Borrowed(self.bytes),
        }
    }
}

impl OscScanner {
    const PREFIX: &'static [u8] = b"\x1b]";

    /// Scans one read, reporting what Condr consumes itself, and returns what the VT
    /// should see.
    pub(super) fn advance<'a>(
        &mut self,
        bytes: &'a [u8],
        mut report: impl FnMut(OscReport),
    ) -> Cow<'a, [u8]> {
        let mut held = std::mem::take(&mut self.held);
        let mut vt = VtFilter::new(bytes, &mut held);
        for (index, &byte) in bytes.iter().enumerate() {
            match self.state {
                OscState::Ground => {
                    if byte == Self::PREFIX[0] {
                        vt.hold(index);
                        self.state = OscState::Prefix(1);
                    }
                }
                OscState::Prefix(matched) => {
                    if byte == Self::PREFIX[matched] {
                        let matched = matched + 1;
                        if matched == Self::PREFIX.len() {
                            self.payload.clear();
                            self.state = OscState::Payload;
                        } else {
                            self.state = OscState::Prefix(matched);
                        }
                    } else if byte == Self::PREFIX[0] {
                        vt.release(index);
                        vt.hold(index);
                        self.state = OscState::Prefix(1);
                    } else if matched == 1 && matches!(byte, b'P' | b'X' | b'^' | b'_') {
                        // DCS/SOS/PM/APC: whatever looks like an OSC inside is payload.
                        vt.release(index + 1);
                        self.state = OscState::ControlString;
                    } else {
                        vt.release(index + 1);
                        self.state = OscState::Ground;
                    }
                }
                OscState::Payload => match byte {
                    0x07 => self.finish(index, &mut vt, &mut report),
                    0x1b => self.state = OscState::PayloadEscape,
                    _ => self.push_payload(byte, index, &mut vt),
                },
                OscState::PayloadEscape => match byte {
                    b'\\' => self.finish(index, &mut vt, &mut report),
                    0x07 => {
                        self.push_payload(0x1b, index, &mut vt);
                        if matches!(self.state, OscState::Payload) {
                            self.finish(index, &mut vt, &mut report);
                        }
                    }
                    0x1b => {
                        self.push_payload(0x1b, index, &mut vt);
                        if matches!(self.state, OscState::Payload) {
                            self.state = OscState::PayloadEscape;
                        }
                    }
                    _ => {
                        self.push_payload(0x1b, index, &mut vt);
                        if matches!(self.state, OscState::Payload) {
                            self.push_payload(byte, index, &mut vt);
                        }
                    }
                },
                OscState::Discard => match byte {
                    0x07 => self.reset(),
                    0x1b => self.state = OscState::DiscardEscape,
                    _ => {}
                },
                OscState::DiscardEscape => match byte {
                    b'\\' => self.reset(),
                    0x1b => {}
                    _ => self.state = OscState::Discard,
                },
                OscState::ControlString => {
                    if byte == 0x1b {
                        self.state = OscState::ControlStringEscape;
                    }
                }
                OscState::ControlStringEscape => match byte {
                    b'\\' => self.reset(),
                    0x1b => {}
                    _ => self.state = OscState::ControlString,
                },
            }
        }
        let filtered = vt.finish();
        self.held = held;
        filtered
    }

    /// Whether the payload so far can still turn out to be an agent event.
    fn could_be_agent_event(&self) -> bool {
        let prefix = AGENT_EVENT_OSC_PREFIX.as_bytes();
        prefix.starts_with(&self.payload) || self.payload.starts_with(prefix)
    }

    fn push_payload(&mut self, byte: u8, index: usize, vt: &mut VtFilter<'_, '_>) {
        if self.payload.len() == MAX_OSC_CWD_BYTES {
            self.payload.clear();
            self.state = OscState::Discard;
            vt.release(index + 1);
        } else {
            self.payload.push(byte);
            self.state = OscState::Payload;
            if !self.could_be_agent_event() {
                vt.release(index + 1);
            }
        }
    }

    fn finish(
        &mut self,
        index: usize,
        vt: &mut VtFilter<'_, '_>,
        report: &mut impl FnMut(OscReport),
    ) {
        if self.payload.starts_with(AGENT_EVENT_OSC_PREFIX.as_bytes()) {
            // Ours, well-formed or not: the terminal has no use for it either way.
            if let Some(event) = AgentEvent::decode(&self.payload) {
                report(OscReport::Agent(event));
            }
            vt.discard(index + 1);
        } else {
            vt.release(index + 1);
            if let Ok(payload) = std::str::from_utf8(&self.payload) {
                if let Some(cwd) = payload.strip_prefix("9;9;") {
                    let cwd = cwd
                        .strip_prefix('"')
                        .and_then(|cwd| cwd.strip_suffix('"'))
                        .unwrap_or(cwd);
                    report(OscReport::Cwd(PathBuf::from(cwd)));
                } else if let Some(cwd) = payload.strip_prefix("7;").and_then(file_uri_cwd) {
                    report(OscReport::Cwd(cwd));
                } else if let Some(cwd) = payload
                    .strip_prefix("1337;CurrentDir=")
                    .filter(|cwd| !cwd.is_empty())
                {
                    report(OscReport::Cwd(PathBuf::from(cwd)));
                }
            }
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.payload.clear();
        self.state = OscState::Ground;
    }
}

/// OSC 7 carries `file://[host]/path`; only this machine's paths are usable.
fn file_uri_cwd(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) => {
            let host = percent_decode(&rest[..slash])?;
            let host = host.trim_end_matches('.');
            if !host.eq_ignore_ascii_case("localhost")
                && !sysinfo::System::host_name().is_some_and(|local| {
                    let local = local.trim_end_matches('.');
                    host.eq_ignore_ascii_case(local)
                        || local
                            .split_once('.')
                            .is_some_and(|(short, _)| host.eq_ignore_ascii_case(short))
                })
            {
                return None;
            }
            &rest[slash..]
        }
        None => return None,
    };
    let path = percent_decode(path)?;
    #[cfg(windows)]
    let path = {
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3 && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
        if drive { &path[1..] } else { path.as_str() }.replace('/', "\\")
    };
    (!path.is_empty() && !path.contains('\0')).then(|| PathBuf::from(path))
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
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
        let QueuedInput {
            mut bytes,
            submit_split,
            _user_permit,
            _control_permit,
            terminal_replies,
        } = queued;
        loop {
            if let Err(error) = write_queued_input(&mut *writer, &bytes, submit_split, &stopping) {
                drop(input);
                if stopping.load(Ordering::Acquire) {
                    return Ok(());
                }
                return Err(error);
            }
            let Some(replies) = terminal_replies.as_ref() else {
                break;
            };
            let mut replies = replies.lock().expect("terminal reply queue lock poisoned");
            match replies.pending.pop_front() {
                Some(next) => bytes = next,
                None => {
                    // Producers append and inspect this flag under the same lock, so a
                    // reply racing this clear will enqueue the next wake token.
                    replies.token_queued = false;
                    break;
                }
            }
        }
    }
    Ok(())
}

// ConPTY delivers large pastes as individual input events; native Codex needs a
// longer settling interval than the bracketed-paste path on Unix.
const SUBMIT_DELAY: Duration = Duration::from_millis(if cfg!(windows) { 1000 } else { 300 });

fn write_queued_input(
    writer: &mut dyn Write,
    bytes: &[u8],
    submit_split: Option<usize>,
    stopping: &AtomicBool,
) -> io::Result<()> {
    let (text, enter) = bytes.split_at(submit_split.unwrap_or(bytes.len()));
    writer.write_all(text)?;
    writer.flush()?;
    if !enter.is_empty() {
        // A TUI's paste detector can absorb an immediate Enter. Keep the whole
        // submission in one queue entry so another client's input cannot interleave.
        let deadline = Instant::now() + SUBMIT_DELAY;
        loop {
            if stopping.load(Ordering::Acquire) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
        writer.write_all(enter)?;
        writer.flush()?;
    }
    Ok(())
}

const PTY_RESIZE_RETRY_DELAY: Duration = Duration::from_millis(50);

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
        let master = master
            .upgrade()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "PTY was closed"))?;
        // A failed ioctl must not kill the worker. The VT grid is already at the new
        // size, so retry the PTY once shortly after instead of leaving the two apart.
        let resize_pty = |pty_size: PtySize| {
            master
                .lock()
                .expect("PTY master lock poisoned")
                .resize(pty_size)
                .map_err(other_error)
        };
        if resize_terminal_and_pty(
            &terminal,
            &current_size,
            &revision,
            &updates,
            size,
            |_, pty_size| resize_pty(pty_size),
        )
        .is_err()
        {
            thread::sleep(PTY_RESIZE_RETRY_DELAY);
            let _ = resize_pty(size.into());
        }
    }
    Ok(())
}

pub(super) fn resize_terminal_and_pty(
    terminal: &Arc<Mutex<Terminal>>,
    current_size: &Arc<Mutex<TerminalSize>>,
    revision: &AtomicU64,
    updates: &mpsc::Sender<TerminalUpdate>,
    size: TerminalSize,
    resize_pty: impl FnOnce(&Terminal, PtySize) -> io::Result<()>,
) -> io::Result<()> {
    let mut terminal = terminal.lock().expect("terminal state lock poisoned");
    terminal.resize(size);
    let resize_result = resize_pty(&terminal, size.into());
    *current_size.lock().expect("terminal size lock poisoned") = size;
    drop(terminal);
    publish_view(revision, updates);
    resize_result
}
