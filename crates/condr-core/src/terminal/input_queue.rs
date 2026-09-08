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

pub(super) struct TerminalIoLoop {
    pub(super) writer: Box<dyn Write + Send>,
    pub(super) input: TerminalInputReceiver,
    pub(super) stopping: Arc<AtomicBool>,
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
