use super::*;

pub(super) type Terminal = Term<TerminalEventProxy>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ReportedCwd {
    pub(super) cwd: Option<PathBuf>,
    pub(super) generation: u64,
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
                    // Closing stops the writer before the reader; a query the shell sent in
                    // that window has no one left to answer it, which is not a failure.
                    break if error.kind() == io::ErrorKind::BrokenPipe {
                        Ok(())
                    } else {
                        Err(error)
                    };
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
