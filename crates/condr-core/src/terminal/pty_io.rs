use super::*;

pub(super) type Terminal = Vt;

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
        revision,
        updates,
        reported_cwd,
        notices,
        size,
        cursor_settle,
    } = io;
    let mut bytes = [0; 16 * 1024];
    let result = loop {
        match reader.read(&mut bytes) {
            Ok(0) => break Ok(()),
            Ok(read) => {
                let mut terminal_guard = terminal.lock().expect("terminal state lock poisoned");
                // The VT answers queries in stream order, so an OSC 7501 query's answer
                // still leaves ahead of the DA1 reply a program uses as its sentinel.
                let events = terminal_guard.write(&bytes[..read]);
                if CURSOR_POSITION_SETTLE_ENABLED {
                    // Observe after every read like herdr does, so a position that only
                    // lasts between two reads never counts as held, whatever the frame rate.
                    let size = *size.lock().expect("terminal size lock poisoned");
                    cursor_settle
                        .lock()
                        .expect("cursor settle lock poisoned")
                        .observe(terminal_guard.cursor_position(size), Instant::now());
                }
                drop(terminal_guard);
                let replies = apply_terminal_events(events, &notices, &reported_cwd);
                if let Err(error) = write_terminal_replies(&input, replies) {
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
