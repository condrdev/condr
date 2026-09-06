use super::*;

struct RecordingWriter(Arc<Mutex<Vec<u8>>>);

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BlockingRecordingWriter {
    writer: RecordingWriter,
    started: Option<mpsc::Sender<()>>,
    release: mpsc::Receiver<()>,
}

impl Write for BlockingRecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(started) = self.started.take() {
            started.send(()).unwrap();
            self.release.recv().unwrap();
        }
        self.writer.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

struct FailingWriter;

struct ObservedWriter(mpsc::Sender<(Vec<u8>, Instant)>);

impl Write for ObservedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.send((bytes.to_vec(), Instant::now())).unwrap();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn submission_delays_enter_without_interleaving_other_input() {
    let (input, receiver) = TerminalInput::channel();
    let (written, writes) = mpsc::channel();
    input
        .try_submit(b"prompt".to_vec(), b"\r".to_vec())
        .unwrap();
    let io = thread::spawn(move || {
        io_loop(TerminalIoLoop {
            writer: Box::new(ObservedWriter(written)),
            input: receiver,
            stopping: Arc::new(AtomicBool::new(false)),
        })
    });
    let (text, pasted_at) = writes.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(text, b"prompt");
    input.try_write(b"next".to_vec()).unwrap();
    drop(input);
    let (enter, submitted_at) = writes.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(enter, b"\r");
    assert!(submitted_at.duration_since(pasted_at) >= Duration::from_millis(300));
    assert_eq!(
        writes.recv_timeout(Duration::from_secs(2)).unwrap().0,
        b"next"
    );
    io.join().unwrap().unwrap();
}

#[test]
fn stopping_cancels_the_enter_of_a_pending_submission() {
    let (input, receiver) = TerminalInput::channel();
    let stopping = Arc::new(AtomicBool::new(false));
    let (written, writes) = mpsc::channel();
    input
        .try_submit(b"prompt".to_vec(), b"\r".to_vec())
        .unwrap();
    let io = thread::spawn({
        let stopping = Arc::clone(&stopping);
        move || {
            io_loop(TerminalIoLoop {
                writer: Box::new(ObservedWriter(written)),
                input: receiver,
                stopping,
            })
        }
    });
    assert_eq!(
        writes.recv_timeout(Duration::from_secs(2)).unwrap().0,
        b"prompt"
    );
    stopping.store(true, Ordering::Release);
    drop(input);
    io.join().unwrap().unwrap();
    assert!(writes.try_recv().is_err());
}

impl Write for FailingWriter {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "write failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn frame_test_view(revision: u64, text: &str) -> TerminalView {
    let mut cells = text
        .chars()
        .map(|character| TerminalCell {
            text: SmolStr::from(character.to_string()),
            ..blank_cell()
        })
        .collect::<Vec<_>>();
    let columns = u16::try_from(cells.len()).unwrap();
    if cells.is_empty() {
        cells.push(blank_cell());
    }
    TerminalView {
        selection: None,
        revision,
        size: TerminalSize::new(1, columns.max(1)),
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cells,
        cursor: None,
    }
}

#[test]
fn pending_resize_keeps_only_the_latest_request() {
    let resize = ResizeControl::default();
    resize.request(TerminalSize::new(10, 40)).unwrap();
    let latest = TerminalSize::new(20, 80);
    resize.request(latest).unwrap();

    assert_eq!(resize.next(), Some(latest));
    resize.stop();
    assert_eq!(resize.next(), None);
    assert_eq!(
        resize
            .request(TerminalSize::new(30, 120))
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn terminal_control_has_capacity_reserved_from_user_input_and_replies() {
    let (input, receiver) = TerminalInput::channel();
    for _ in 0..INPUT_QUEUE_CAPACITY {
        input.try_write(vec![b'u']).unwrap();
    }
    assert_eq!(
        input.try_write(vec![b'x']).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );

    input.write_terminal_reply(b"reply".to_vec()).unwrap();
    for _ in 0..TERMINAL_CONTROL_QUEUE_RESERVE {
        input.try_write_control(b"release".to_vec()).unwrap();
    }
    assert_eq!(
        input
            .try_write_control(b"extra release".to_vec())
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    for _ in 0..INPUT_QUEUE_CAPACITY {
        assert_eq!(receiver.recv().unwrap().bytes, b"u");
    }
    assert_eq!(receiver.recv().unwrap().bytes, b"reply");
    for _ in 0..TERMINAL_CONTROL_QUEUE_RESERVE {
        assert_eq!(receiver.recv().unwrap().bytes, b"release");
    }
}

#[test]
fn terminal_replies_do_not_wait_for_the_writer() {
    let (input, receiver) = TerminalInput::channel();
    input.write_terminal_reply(b"first".to_vec()).unwrap();
    let written = Arc::new(Mutex::new(Vec::new()));
    let (started, writer_started) = mpsc::channel();
    let (release_writer, release) = mpsc::channel();
    let io = thread::spawn({
        let written = Arc::clone(&written);
        move || {
            io_loop(TerminalIoLoop {
                writer: Box::new(BlockingRecordingWriter {
                    writer: RecordingWriter(written),
                    started: Some(started),
                    release,
                }),
                input: receiver,
                stopping: Arc::new(AtomicBool::new(false)),
            })
        }
    });
    writer_started.recv_timeout(Duration::from_secs(1)).unwrap();

    let second_input = input.clone();
    let (completed, completion) = mpsc::channel();
    let reply = thread::spawn(move || {
        let result = second_input.write_terminal_reply(b"second".to_vec());
        completed.send(result).unwrap();
    });

    let result = completion.recv_timeout(Duration::from_secs(1));
    release_writer.send(()).unwrap();
    reply.join().unwrap();

    assert!(
        matches!(result, Ok(Ok(()))),
        "terminal reader waited for the PTY writer: {result:?}"
    );
    drop(input);
    io.join().unwrap().unwrap();
    assert_eq!(*written.lock().unwrap(), b"firstsecond");
}

#[test]
fn terminal_reply_writer_failure_disconnects_future_replies() {
    let (input, receiver) = TerminalInput::channel();
    input.try_write(b"user input".to_vec()).unwrap();
    input.write_terminal_reply(b"first".to_vec()).unwrap();
    input.write_terminal_reply(b"second".to_vec()).unwrap();

    let error = io_loop(TerminalIoLoop {
        writer: Box::new(FailingWriter),
        input: receiver,
        stopping: Arc::new(AtomicBool::new(false)),
    })
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(
        input
            .write_terminal_reply(b"after failure".to_vec())
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn terminal_color_replies_use_the_render_palette_and_preserve_order() {
    let size = TerminalSize::new(4, 12);
    let shared_size = Arc::new(Mutex::new(size));
    let (event_proxy, pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&shared_size));
    let terminal = Arc::new(Mutex::new(Term::new(Config::default(), &size, event_proxy)));
    let (input, receiver) = TerminalInput::channel();
    let mut parser: Processor = Processor::new();

    parser.advance(
        &mut *terminal.lock().expect("terminal state lock poisoned"),
        b"\x1b]11;?\x07\x1b[5n\x1b]4;1;#123456\x07\x1b]4;1;?\x07",
    );
    flush_terminal_replies(&terminal, &input, &pending_replies).unwrap();

    assert_eq!(
        receiver.recv().unwrap().bytes,
        b"\x1b]11;rgb:0d0d/1111/1717\x07\x1b[0n\x1b]4;1;rgb:1212/3434/5656\x07"
    );
}

#[test]
fn title_and_bell_notices_are_recorded_once_per_change_and_counted() {
    let size = TerminalSize::new(4, 12);
    let shared_size = Arc::new(Mutex::new(size));
    let (event_proxy, _pending_replies, notices) =
        TerminalEventProxy::new(Arc::clone(&shared_size));
    let probe = TerminalNoticeProbe { notices };
    let mut terminal = Term::new(Config::default(), &size, event_proxy);
    let mut parser: Processor = Processor::new();

    parser.advance(
        &mut terminal,
        b"\x1b]2;\xe2\x9c\xb3 fix tests\x07\x07\x07\x1b]52;c;aGVsbG8=\x07\x1b]52;c;d29ybGQ=\x07",
    );
    assert_eq!(
        probe.take(),
        TerminalNoticeBatch {
            title: Some(Some("\u{2733} fix tests".into())),
            bells: 2,
            clipboard: Some("world".into()),
        }
    );
    assert!(probe.take().is_empty());

    // A spinner frame is part of the title, as Windows Terminal shows it.
    parser.advance(&mut terminal, b"\x1b]2;\xe2\x9c\xbb fix tests\x07");
    assert_eq!(probe.take().title, Some(Some("\u{273b} fix tests".into())));

    parser.advance(&mut terminal, b"\x1b]2;\x07");
    assert_eq!(probe.take().title, Some(None));

    parser.advance(&mut terminal, b"\x1b]52;c;\x07");
    assert_eq!(probe.take().clipboard.as_deref(), Some(""));
}

#[test]
fn terminal_reply_failure_still_publishes_exit_and_keeps_tail_notices() {
    let size = TerminalSize::new(4, 12);
    let shared_size = Arc::new(Mutex::new(size));
    let (event_proxy, pending_replies, notices) = TerminalEventProxy::new(Arc::clone(&shared_size));
    let probe = TerminalNoticeProbe { notices };
    let terminal = Arc::new(Mutex::new(Term::new(Config::default(), &size, event_proxy)));
    let (input, input_receiver) = TerminalInput::channel();
    drop(input_receiver);
    let (updates, update_receiver) = mpsc::channel();

    let error = read_loop(TerminalReadLoop {
        reader: Box::new(io::Cursor::new(
            b"\x1b]2;tail title\x07\x07\x1b]52;c;dGFpbCBjb3B5\x07\x1b[5n".to_vec(),
        )),
        terminal,
        input,
        pending_replies,
        revision: Arc::new(AtomicU64::new(0)),
        updates,
        reported_cwd: Arc::new(Mutex::new(ReportedCwd::default())),
        notices: SharedTerminalNotices::default(),
        size: shared_size,
        cursor_settle: Arc::new(Mutex::new(Default::default())),
    })
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(update_receiver.recv().unwrap(), TerminalUpdate::Exited);
    assert!(update_receiver.try_recv().is_err());
    assert_eq!(
        probe.take(),
        TerminalNoticeBatch {
            title: Some(Some("tail title".into())),
            bells: 1,
            clipboard: Some("tail copy".into()),
        }
    );
}

#[test]
fn decscusr_shape_and_blink_reach_the_snapshot_cursor() {
    let size = TerminalSize::new(1, 4);
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let mut terminal = Term::new(Config::default(), &size, event_proxy);
    let mut parser: Processor = Processor::new();

    let cursor = |terminal: &Terminal| snapshot_terminal(terminal, size, 1).cursor.unwrap();
    assert_eq!(cursor(&terminal).shape, TerminalCursorShape::Block);
    assert!(!cursor(&terminal).blinking, "default cursor stays steady");

    parser.advance(&mut terminal, b"\x1b[5 q");
    assert_eq!(cursor(&terminal).shape, TerminalCursorShape::Beam);
    assert!(cursor(&terminal).blinking);

    parser.advance(&mut terminal, b"\x1b[4 q");
    assert_eq!(cursor(&terminal).shape, TerminalCursorShape::Underline);
    assert!(!cursor(&terminal).blinking);

    parser.advance(&mut terminal, b"\x1b[?25l");
    assert_eq!(cursor(&terminal).shape, TerminalCursorShape::Hidden);
}

#[test]
fn osc8_hyperlinks_are_carried_per_cell() {
    let size = TerminalSize::new(1, 4);
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let mut terminal = Term::new(Config::default(), &size, event_proxy);
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut terminal,
        b"\x1b]8;;https://example.com/a-long-shared-target\x1b\\ab\x1b]8;;\x1b\\c",
    );

    let view = snapshot_terminal(&terminal, size, 1);
    let link = |column| view.cell(0, column).unwrap().hyperlink.clone();
    assert_eq!(
        link(0).as_deref(),
        Some("https://example.com/a-long-shared-target")
    );
    assert_eq!(
        link(1).as_deref(),
        Some("https://example.com/a-long-shared-target")
    );
    assert_eq!(
        link(0).unwrap().as_ptr(),
        link(1).unwrap().as_ptr(),
        "linked cells in one snapshot must share long URI storage"
    );
    assert_eq!(link(2), None);
}

#[test]
fn terminal_hyperlinks_stay_within_the_frame_byte_budget() {
    let size = TerminalSize::new(1, 1);
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let terminal = Term::new(Config::default(), &size, event_proxy);
    let mut hyperlinks = SnapshotHyperlinks::default();
    hyperlinks.bytes = MAX_TERMINAL_HYPERLINK_BYTES - 3;
    let mut cell = Cell::default();
    cell.set_hyperlink(Some(Hyperlink::new(Some("first"), "abc".into())));

    assert_eq!(
        terminal_cell(&cell, terminal.colors(), &mut hyperlinks)
            .hyperlink
            .as_deref(),
        Some("abc")
    );

    cell.set_hyperlink(Some(Hyperlink::new(Some("new-id"), "abc".into())));
    assert_eq!(
        terminal_cell(&cell, terminal.colors(), &mut hyperlinks)
            .hyperlink
            .as_deref(),
        Some("abc")
    );

    cell.set_hyperlink(Some(Hyperlink::new(Some("second"), "d".into())));
    assert_eq!(
        terminal_cell(&cell, terminal.colors(), &mut hyperlinks).hyperlink,
        None
    );

    cell.set_hyperlink(Some(Hyperlink::new(
        Some("oversized"),
        "x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES + 1),
    )));
    assert_eq!(
        terminal_cell(&cell, terminal.colors(), &mut hyperlinks).hyperlink,
        None
    );
}

#[test]
fn terminal_titles_are_sanitized() {
    assert_eq!(
        sanitize_terminal_title("  plain  ").as_deref(),
        Some("plain")
    );
    assert_eq!(
        sanitize_terminal_title("\u{280b} task").as_deref(),
        Some("\u{280b} task")
    );
    assert_eq!(
        sanitize_terminal_title("\u{280b}").as_deref(),
        Some("\u{280b}")
    );
    assert_eq!(sanitize_terminal_title("★task").as_deref(), Some("★task"));
    assert_eq!(sanitize_terminal_title("a\x08b\r\n").as_deref(), Some("ab"));
    assert_eq!(sanitize_terminal_title("").as_deref(), None);
    assert_eq!(
        sanitize_terminal_title(&"x".repeat(1000)).map(|title| title.len()),
        Some(MAX_TERMINAL_TITLE_CHARS)
    );
}

#[test]
fn cleanup_errors_do_not_hide_the_primary_shutdown_failure() {
    let error = combine_cleanup_results::<()>(
        Err(io::Error::other("process cleanup failed")),
        Err(io::Error::other("I/O cleanup failed")),
        "stop terminal I/O",
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("process cleanup failed"), "{message}");
    assert!(message.contains("I/O cleanup failed"), "{message}");
}

#[test]
fn resize_worker_failure_stops_accepting_requests() {
    let initial_size = TerminalSize::new(5, 20);
    let requested_size = TerminalSize::new(10, 40);
    let control = Arc::new(ResizeControl::default());
    control.request(requested_size).unwrap();
    let current_size = Arc::new(Mutex::new(initial_size));
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&current_size));
    let terminal = Arc::new(Mutex::new(Term::new(
        Config::default(),
        &initial_size,
        event_proxy,
    )));
    let (updates, _update_receiver) = mpsc::channel();

    let error = resize_loop(
        Arc::clone(&control),
        Weak::<Mutex<Box<dyn MasterPty + Send>>>::new(),
        terminal,
        current_size,
        Arc::new(AtomicU64::new(0)),
        updates,
    )
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(
        control.request(requested_size).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn terminal_is_resized_before_the_pty() {
    let initial_size = TerminalSize::new(5, 20);
    let requested_size = TerminalSize::new(10, 40);
    let current_size = Arc::new(Mutex::new(initial_size));
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&current_size));
    let terminal = Arc::new(Mutex::new(Term::new(
        Config::default(),
        &initial_size,
        event_proxy,
    )));
    let revision = AtomicU64::new(0);
    let (updates, _update_receiver) = mpsc::channel();
    let mut size_seen_by_pty = None;

    resize_terminal_and_pty(
        &terminal,
        &current_size,
        &revision,
        &updates,
        requested_size,
        |terminal, _| {
            size_seen_by_pty = Some((terminal.screen_lines(), terminal.columns()));
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(
        size_seen_by_pty,
        Some((
            usize::from(requested_size.rows),
            usize::from(requested_size.columns)
        ))
    );
}

#[test]
fn failed_pty_resize_still_publishes_the_new_vt_size() {
    let initial_size = TerminalSize::new(5, 20);
    let requested_size = TerminalSize::new(10, 40);
    let current_size = Arc::new(Mutex::new(initial_size));
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&current_size));
    let terminal = Arc::new(Mutex::new(Term::new(
        Config::default(),
        &initial_size,
        event_proxy,
    )));
    let revision = AtomicU64::new(0);
    let (updates, update_receiver) = mpsc::channel();

    let error = resize_terminal_and_pty(
        &terminal,
        &current_size,
        &revision,
        &updates,
        requested_size,
        |_, _| Err(io::Error::other("PTY resize failed")),
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "PTY resize failed");
    assert_eq!(*current_size.lock().unwrap(), requested_size);
    let terminal = terminal.lock().unwrap();
    assert_eq!(terminal.screen_lines(), usize::from(requested_size.rows));
    assert_eq!(terminal.columns(), usize::from(requested_size.columns));
    drop(terminal);
    assert_eq!(revision.load(Ordering::Acquire), 1);
    assert_eq!(update_receiver.recv().unwrap(), TerminalUpdate::View(1));
}

#[cfg(unix)]
#[test]
fn unix_pty_probe_uses_the_child_as_the_expected_session_leader() {
    let probe = ProcessProbe::new(Some(12_345));
    assert_eq!(probe.session_id, Some(12_345));
}

#[test]
fn process_exit_checks_reject_a_reused_pid_identity() {
    let system = System::new_all();
    let pid = Pid::from_u32(std::process::id());
    let started_at = system.process(pid).unwrap().start_time();
    assert!(!process_tree_exited(
        &[OwnedProcess { pid, started_at }],
        None,
        false,
        false
    ));
    assert!(process_tree_exited(
        &[OwnedProcess {
            pid,
            started_at: started_at.saturating_add(1),
        }],
        None,
        false,
        false
    ));
}

#[test]
fn process_refresh_keeps_retained_identity_after_shell_disappears() {
    let system = System::new_all();
    let pid = Pid::from_u32(std::process::id());
    let identity = OwnedProcess {
        pid,
        started_at: system.process(pid).unwrap().start_time(),
    };
    let mut owned = vec![identity];

    refresh_owned_processes(ProcessProbe::new(None), &mut owned);

    assert_eq!(owned, [identity]);
    assert!(!process_tree_exited(&owned, None, false, false));
}

#[test]
fn cwd_probe_rejects_a_reused_shell_pid_identity() {
    let probe = ProcessProbe::new(Some(std::process::id()));
    let started_at = probe
        .shell_started_at
        .expect("the current process has a birth identity");
    assert!(probe.cwd().is_some());

    let reused = ProcessProbe {
        shell_started_at: Some(started_at.wrapping_add(1)),
        ..probe
    };
    assert_eq!(reused.cwd(), None);
}

#[test]
fn legacy_keys_cover_cursor_modes_modifiers_and_controls() {
    let none = TerminalModifiers::default();
    let cases: &[(TerminalKey, TerminalModifiers, bool, &[u8])] = &[
        (TerminalKey::Up, none, false, b"\x1b[A"),
        (TerminalKey::Up, none, true, b"\x1bOA"),
        (
            TerminalKey::Left,
            TerminalModifiers {
                shift: true,
                control: true,
                ..none
            },
            true,
            b"\x1b[1;6D",
        ),
        (
            TerminalKey::Left,
            TerminalModifiers {
                control: true,
                platform: true,
                ..none
            },
            false,
            b"\x1b[1;5D",
        ),
        (
            TerminalKey::Character("c".into()),
            TerminalModifiers {
                control: true,
                ..none
            },
            false,
            b"\x03",
        ),
        (
            TerminalKey::Character("x".into()),
            TerminalModifiers { alt: true, ..none },
            false,
            b"\x1bx",
        ),
        (
            TerminalKey::Backspace,
            TerminalModifiers {
                control: true,
                ..none
            },
            false,
            b"\x08",
        ),
        (TerminalKey::Function(12), none, false, b"\x1b[24~"),
        (TerminalKey::Function(13), none, false, b"\x1b[25~"),
        (TerminalKey::Function(20), none, false, b"\x1b[34~"),
        (TerminalKey::Enter, none, false, b"\r"),
        (
            TerminalKey::Enter,
            TerminalModifiers { alt: true, ..none },
            false,
            b"\x1b\r",
        ),
        // Only exactly Shift+Enter is LF; Ctrl+Shift+Enter falls back to CR (herdr).
        (
            TerminalKey::Enter,
            TerminalModifiers {
                shift: true,
                ..none
            },
            false,
            b"\n",
        ),
        (
            TerminalKey::Enter,
            TerminalModifiers {
                control: true,
                shift: true,
                ..none
            },
            false,
            b"\r",
        ),
        (
            TerminalKey::Enter,
            TerminalModifiers {
                shift: true,
                ..none
            },
            false,
            b"\n",
        ),
    ];
    for (key, modifiers, application_cursor, expected) in cases {
        assert_eq!(
            encode_key(key, *modifiers, *application_cursor).unwrap(),
            *expected,
            "{key:?} with {modifiers:?}"
        );
    }

    let control_aliases = [
        ("2", 0),
        ("3", 27),
        ("4", 28),
        ("5", 29),
        ("6", 30),
        ("7", 31),
        ("/", 31),
        ("-", 31),
    ];
    for (text, expected) in control_aliases {
        assert_eq!(
            encode_key(
                &TerminalKey::Character(text.into()),
                TerminalModifiers {
                    control: true,
                    ..none
                },
                false,
            )
            .unwrap(),
            [expected],
            "Ctrl+{text}"
        );
    }

    assert!(encode_key(&TerminalKey::Function(21), none, false).is_err());
}

#[test]
fn server_selection_follows_scrolled_output_and_still_copies_it() {
    let size = TerminalSize::new(3, 10);
    let (event_proxy, _replies, _notices) = TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let mut terminal = Term::new(Config::default(), &size, event_proxy);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut terminal, b"alpha\r\nbeta\r\ngamma");
    let position = |row, column, side| TerminalPosition { row, column, side };
    let selection = TerminalSelection {
        start: position(1, 0, TerminalSide::Left),
        end: position(1, 3, TerminalSide::Right),
        display_offset: 0,
    };

    terminal.selection = Some(super::runtime::grid_selection(&terminal, selection));
    assert_eq!(viewport_selection(&terminal, size), Some(selection));
    assert_eq!(terminal.selection_to_string().as_deref(), Some("beta"));

    parser.advance(&mut terminal, b"\r\ndelta");
    let moved = viewport_selection(&terminal, size).unwrap();
    assert_eq!((moved.start.row, moved.end.row), (0, 0));
    assert_eq!(terminal.selection_to_string().as_deref(), Some("beta"));

    parser.advance(&mut terminal, b"\r\nepsilon");
    // Scrolled out of view: still reported (so a copy is offered) but matches no cell.
    let hidden = viewport_selection(&terminal, size).unwrap();
    assert_eq!((hidden.start.row, hidden.end.row), (size.rows, size.rows));
    assert!(hidden.selected_cell_range(size.columns).is_some());
    assert!((0..size.rows).all(|row| !hidden.contains_cell(row, 0, size.columns)));
    assert_eq!(terminal.selection_to_string().as_deref(), Some("beta"));

    terminal.selection = None;
    assert_eq!(viewport_selection(&terminal, size), None);
}

#[test]
fn kitty_keyboard_protocol_follows_the_negotiated_flags() {
    let none = TerminalModifiers::default();
    let shift = TerminalModifiers {
        shift: true,
        ..none
    };
    let control = TerminalModifiers {
        control: true,
        ..none
    };
    let alt = TerminalModifiers { alt: true, ..none };
    let control_shift = TerminalModifiers {
        control: true,
        shift: true,
        ..none
    };
    let disambiguate = TermMode::DISAMBIGUATE_ESC_CODES;
    let events = disambiguate | TermMode::REPORT_EVENT_TYPES;
    let all = disambiguate
        | TermMode::REPORT_ALTERNATE_KEYS
        | TermMode::REPORT_ALL_KEYS_AS_ESC
        | TermMode::REPORT_ASSOCIATED_TEXT;
    let ch = |text: &str| TerminalKey::Character(text.into());
    let cases: &[(TerminalKey, TerminalModifiers, TermMode, &[u8])] = &[
        // Legacy stays legacy without any kitty flag.
        (TerminalKey::Enter, shift, TermMode::empty(), b"\n"),
        // herdr parity under disambiguation.
        (TerminalKey::Enter, shift, disambiguate, b"\x1b[13;2u"),
        (TerminalKey::Enter, alt, disambiguate, b"\x1b[13;3u"),
        (TerminalKey::Backspace, alt, disambiguate, b"\x1b[127;3u"),
        (ch("a"), control_shift, disambiguate, b"\x1b[97;6u"),
        (ch("L"), control_shift, disambiguate, b"\x1b[108;6u"),
        (ch("c"), control, disambiguate, b"\x1b[99;5u"),
        (ch("L"), shift, disambiguate, b"L"),
        (ch("a"), none, disambiguate, b"a"),
        (TerminalKey::Enter, none, disambiguate, b"\r"),
        (TerminalKey::Tab, none, disambiguate, b"\t"),
        (TerminalKey::Backspace, none, disambiguate, b"\x7f"),
        (TerminalKey::Up, alt, disambiguate, b"\x1b[1;3A"),
        (TerminalKey::Function(5), shift, disambiguate, b"\x1b[15;2~"),
        // Escape follows the spec rather than herdr.
        (TerminalKey::Escape, none, disambiguate, b"\x1b[27u"),
        // Event types add the press suffix and put legacy-form keys in kitty form.
        (ch("c"), control, events, b"\x1b[99;5:1u"),
        (TerminalKey::Left, none, events, b"\x1b[1;1:1D"),
        (TerminalKey::Function(3), control, events, b"\x1b[13;5:1~"),
        // Without a modifier field only the plain legacy sequences are valid.
        (TerminalKey::Up, none, all, b"\x1b[A"),
        (TerminalKey::Function(1), none, all, b"\x1b[P"),
        (TerminalKey::Function(3), none, all, b"\x1b[13~"),
        (
            TerminalKey::Function(3),
            control,
            disambiguate,
            b"\x1b[13;5~",
        ),
        (
            TerminalKey::Function(13),
            none,
            disambiguate,
            b"\x1b[57376u",
        ),
        (TerminalKey::Function(20), control, all, b"\x1b[57383;5u"),
        (TerminalKey::Function(3), none, TermMode::empty(), b"\x1bOR"),
        (TerminalKey::Function(5), none, all, b"\x1b[15~"),
        // Alternate keys alone do not disambiguate Escape.
        (
            TerminalKey::Escape,
            none,
            TermMode::REPORT_ALTERNATE_KEYS,
            b"\x1b",
        ),
        (TerminalKey::Enter, none, events, b"\r"),
        // Report-all reports text keys with alternate and associated text.
        (ch("a"), none, all, b"\x1b[97;1;97u"),
        (ch("L"), shift, all, b"\x1b[108:76;2;76u"),
        (ch("c"), control, all, b"\x1b[99;5u"),
        (TerminalKey::Enter, none, all, b"\x1b[13u"),
        (
            ch("c"),
            control,
            all | TermMode::REPORT_EVENT_TYPES,
            b"\x1b[99;5:1u",
        ),
        (TerminalKey::BackTab, none, all, b"\x1b[9;2u"),
    ];
    for (key, modifiers, modes, expected) in cases {
        assert_eq!(
            encode_key_in_mode(key, *modifiers, *modes).unwrap(),
            *expected,
            "{key:?} {modifiers:?} {modes:?}"
        );
    }

    assert_eq!(encode_text_in_mode("a", disambiguate), b"a");
    assert_eq!(encode_text_in_mode("A", all), b"\x1b[97:65;2;65u");
    assert_eq!(encode_text_in_mode("ab", all), b"ab");
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
fn terminal_cell_text_keeps_short_content_unchanged() {
    let mut cell = Cell {
        c: 'e',
        ..Cell::default()
    };
    cell.push_zerowidth('\u{301}');
    cell.push_zerowidth('\u{327}');

    assert_eq!(terminal_cell_text(&cell), "e\u{301}\u{327}");
}

#[test]
fn terminal_cell_text_truncates_long_combining_sequences() {
    let mut cell = Cell {
        c: 'a',
        ..Cell::default()
    };
    for _ in 0..200 {
        cell.push_zerowidth('\u{301}');
    }

    let text = terminal_cell_text(&cell);
    assert_eq!(text.len(), 255);
    assert_eq!(text.chars().count(), 128);
    assert_eq!(text.chars().next(), Some('a'));
    assert!(text.chars().skip(1).all(|character| character == '\u{301}'));
}

#[test]
fn terminal_cell_text_truncation_preserves_utf8_boundaries_and_base() {
    let mut cell = Cell {
        c: '\u{754c}',
        ..Cell::default()
    };
    for _ in 0..100 {
        cell.push_zerowidth('\u{1d165}');
    }

    let text = terminal_cell_text(&cell);
    assert_eq!(text.len(), 255);
    assert_eq!(text.chars().count(), 64);
    assert_eq!(text.chars().next(), Some('\u{754c}'));
    assert_eq!(std::str::from_utf8(text.as_bytes()).unwrap(), text.as_str());
}

#[test]
fn osc_cwd_parser_handles_fragmented_st_and_bel_sequences() {
    let mut parser = OscCwdParser::default();
    let mut reported = Vec::new();

    parser.advance(b"noise\x1b]9", |osc| {
        if let OscReport::Cwd(cwd) = osc {
            reported.push(cwd)
        }
    });
    parser.advance(b";9;C:\\work space\x1b", |osc| {
        if let OscReport::Cwd(cwd) = osc {
            reported.push(cwd)
        }
    });
    assert!(reported.is_empty());
    parser.advance(b"\\tail\x1b]9;9;\"D:\\quoted\"\x07", |osc| {
        if let OscReport::Cwd(cwd) = osc {
            reported.push(cwd)
        }
    });

    assert_eq!(
        reported,
        [PathBuf::from(r"C:\work space"), PathBuf::from(r"D:\quoted")]
    );
}

#[test]
fn osc_cwd_parser_recovers_after_malformed_prefixes() {
    let mut parser = OscCwdParser::default();
    let mut reported = Vec::new();

    parser.advance(b"\x1b]9;8;ignored\x07\x1b]9;9;/valid\x07", |osc| {
        if let OscReport::Cwd(cwd) = osc {
            reported.push(cwd)
        }
    });

    assert_eq!(reported, [PathBuf::from("/valid")]);
}

#[test]
fn osc_nine_parser_reports_progress_parameters_verbatim() {
    let mut parser = OscCwdParser::default();
    let mut reported = Vec::new();

    parser.advance(
        b"\x1b]9;4;3;0\x07\x1b]9;4;0\x1b\\\x1b]9;2;ignored\x07",
        |osc| reported.push(osc),
    );

    assert_eq!(
        reported,
        [
            OscReport::Progress("4;3;0".into()),
            OscReport::Progress("4;0".into()),
        ]
    );
}

#[test]
fn osc_inside_a_control_string_is_payload_until_st() {
    let mut parser = OscCwdParser::default();
    let mut reported = Vec::new();

    // A DCS carrying a BEL and a fake OSC, split across reads, then a real OSC.
    parser.advance(b"\x1bPq\x07\x1b]7;file:///fake\x07 more", |osc| {
        reported.push(osc)
    });
    parser.advance(b"\x1b\\\x1b]7;file:///real\x07", |osc| reported.push(osc));

    assert_eq!(reported, [OscReport::Cwd(PathBuf::from("/real"))]);
}

#[test]
fn osc_seven_and_iterm_cwd_reports_are_decoded() {
    let mut parser = OscCwdParser::default();
    let mut reported = Vec::new();

    parser.advance(
        b"\x1b]7;file://localhost/tmp/with%20space\x07\x1b]7;file://elsewhere/nope\x07\x1b]7;file:///plain\x1b\\\x1b]1337;CurrentDir=/iterm\x07\x1b]1337;RemoteHost=x\x07",
        |osc| reported.push(osc),
    );

    assert_eq!(
        reported,
        [
            OscReport::Cwd(PathBuf::from("/tmp/with space")),
            OscReport::Cwd(PathBuf::from("/plain")),
            OscReport::Cwd(PathBuf::from("/iterm")),
        ]
    );
}

#[test]
fn osc_seven_accepts_this_host_and_escaped_paths_but_rejects_foreign_hosts() {
    let host = sysinfo::System::host_name().expect("test host has a name");
    let mut parser = OscCwdParser::default();
    let mut reported = Vec::new();
    for authority in [
        String::new(),
        "LOCALHOST".into(),
        host.to_uppercase(),
        format!("{host}."),
    ] {
        let uri = format!("\x1b]7;file://{authority}/C:/with%20space/%23%25\x07");
        parser.advance(uri.as_bytes(), |osc| reported.push(osc));
    }
    #[cfg(unix)]
    let expected = PathBuf::from("/C:/with space/#%");
    #[cfg(windows)]
    let expected = PathBuf::from("C:\\with space\\#%");
    assert_eq!(reported, vec![OscReport::Cwd(expected); 4]);
    reported.clear();
    for uri in [
        "file://foreign.invalid/nope",
        "file:///bad%xx",
        "file:///nul%00path",
    ] {
        parser.advance(format!("\x1b]7;{uri}\x07").as_bytes(), |osc| {
            reported.push(osc)
        });
    }
    assert!(reported.is_empty());
}

#[test]
fn parsed_cwd_reports_advance_the_observation_generation() {
    let directory = std::env::temp_dir().join(format!(
        "condr-terminal-cwd-generation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let sequence = format!("\x1b]9;9;{}\x07", directory.display());
    let reported = Mutex::new(ReportedCwd::default());
    let mut parser = OscCwdParser::default();

    parser.advance(sequence.as_bytes(), |osc| {
        if let OscReport::Cwd(cwd) = osc {
            record_reported_cwd(&reported, cwd)
        }
    });
    parser.advance(sequence.as_bytes(), |osc| {
        if let OscReport::Cwd(cwd) = osc {
            record_reported_cwd(&reported, cwd)
        }
    });

    assert_eq!(
        *reported.lock().unwrap(),
        ReportedCwd {
            cwd: Some(directory.clone()),
            generation: 2,
        }
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn powershell_cwd_hook_syncs_win32_before_reporting() {
    let sync = WINDOWS_POWERSHELL_CWD_HOOK
        .find("[Environment]::CurrentDirectory = $loc.ProviderPath")
        .unwrap();
    let report = WINDOWS_POWERSHELL_CWD_HOOK.find("]9;9;").unwrap();
    assert!(sync < report);
}

#[test]
fn cwd_probe_keeps_the_last_successful_observation_when_sources_disappear() {
    let directory = std::env::temp_dir().join(format!(
        "condr-terminal-cwd-cache-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let initial = directory.join("initial");
    let observed = directory.join("observed");
    std::fs::create_dir_all(&initial).unwrap();
    std::fs::create_dir_all(&observed).unwrap();
    let reported = Arc::new(Mutex::new(ReportedCwd {
        cwd: Some(observed.clone()),
        generation: 7,
    }));
    let probe = TerminalCwdProbe {
        #[cfg(unix)]
        master: None,
        process: ProcessProbe::new(None),
        last_known_cwd: Arc::new(Mutex::new(Some(initial))),
        reported_cwd: Arc::clone(&reported),
    };

    assert_eq!(probe.observe(), (Some(observed.clone()), 7));
    reported.lock().unwrap().cwd = None;
    assert_eq!(probe.observe(), (Some(observed), 7));

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn sparse_terminal_frame_round_trips_from_the_committed_baseline() {
    let mut previous = frame_test_view(7, "abcdef");
    let mut current = frame_test_view(8, "abXdef");
    current.cursor = Some(TerminalCursor {
        row: 0,
        column: 3,
        shape: TerminalCursorShape::Beam,
        blinking: false,
    });

    let frame = TerminalView::frame_from(Some(&previous), &current).unwrap();
    assert!(matches!(
        &frame,
        TerminalViewFrame::Delta(TerminalViewDelta { runs, .. })
            if runs.len() == 1 && runs[0].start == 2 && runs[0].cells.len() == 1
    ));
    previous.apply_frame(frame).unwrap();
    assert_eq!(previous, current);
}

#[test]
fn terminal_frame_wire_interns_hyperlinks_and_round_trips() {
    let uri = "https://example.com/a-long-target-that-must-not-repeat";
    let mut view = frame_test_view(7, "abcdef");
    for cell in &mut view.cells[1..4] {
        cell.hyperlink = Some(uri.into());
    }
    let delta = TerminalViewDelta {
        selection: None,
        base_revision: 7,
        revision: 8,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: view.cells[1..4].to_vec(),
        }],
    };

    for frame in [
        TerminalViewFrame::Full(view),
        TerminalViewFrame::Delta(delta),
    ] {
        let encoded = bincode::serialize(&frame).unwrap();
        assert_eq!(
            encoded
                .windows(uri.len())
                .filter(|bytes| *bytes == uri.as_bytes())
                .count(),
            1
        );
        assert_eq!(
            bincode::deserialize::<TerminalViewFrame>(&encoded).unwrap(),
            frame
        );
    }
}

#[test]
fn link_free_deltas_take_the_budget_fast_path_without_changing_results() {
    let mut budgeted = frame_test_view(1, "abc");
    let mut plain = budgeted.clone();
    let mut hyperlinks = TerminalHyperlinkBudget::new(&mut budgeted);
    let delta = TerminalViewDelta {
        selection: None,
        base_revision: 1,
        revision: 2,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: vec![frame_test_view(2, "x").cells.remove(0)],
        }],
    };

    let returned = hyperlinks
        .apply_delta(&mut budgeted, delta.clone())
        .unwrap();
    plain
        .apply_frame(TerminalViewFrame::Delta(delta.clone()))
        .unwrap();
    assert_eq!(returned, delta);
    assert_eq!(budgeted, plain);

    // A stale delta is still rejected before anything is applied.
    assert!(hyperlinks.apply_delta(&mut budgeted, delta).is_err());
    assert_eq!(budgeted, plain);
}

#[test]
fn retained_hyperlinks_share_storage_across_sparse_deltas() {
    let uri = "https://example.com/a-long-target-shared-across-deltas";
    let mut view = frame_test_view(1, "ab");
    view.cells[0].hyperlink = Some(SmolStr::new(uri));
    let mut hyperlinks = TerminalHyperlinkBudget::new(&mut view);
    let incoming = SmolStr::new(uri);
    assert_ne!(
        view.cells[0].hyperlink.as_ref().unwrap().as_ptr(),
        incoming.as_ptr()
    );

    let mut cell = frame_test_view(2, "b").cells.remove(0);
    cell.hyperlink = Some(incoming);
    hyperlinks
        .apply_delta(
            &mut view,
            TerminalViewDelta {
                selection: None,
                base_revision: 1,
                revision: 2,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 1,
                    cells: vec![cell],
                }],
            },
        )
        .unwrap();

    assert_eq!(
        view.cells[0].hyperlink.as_ref().unwrap().as_ptr(),
        view.cells[1].hyperlink.as_ref().unwrap().as_ptr(),
        "retained cells must share one long URI allocation across deltas"
    );
}

#[test]
fn terminal_frame_wire_drops_hyperlinks_over_budget_without_losing_text() {
    let budgeted_links = MAX_TERMINAL_HYPERLINK_BYTES / MAX_TERMINAL_HYPERLINK_URI_BYTES;
    let mut view = frame_test_view(7, &"x".repeat(budgeted_links + 2));
    view.cells[0].hyperlink = Some("x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES + 1).into());
    for (index, cell) in view.cells[1..=budgeted_links].iter_mut().enumerate() {
        let prefix = format!("https://example.com/{index}/");
        cell.hyperlink = Some(
            format!(
                "{prefix}{}",
                "x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES - prefix.len())
            )
            .into(),
        );
    }
    let prefix = "https://example.com/overflow/";
    view.cells.last_mut().unwrap().hyperlink = Some(
        format!(
            "{prefix}{}",
            "x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES - prefix.len())
        )
        .into(),
    );
    let delta = TerminalViewDelta {
        selection: None,
        base_revision: 6,
        revision: 7,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 0,
            cells: view.cells.clone(),
        }],
    };

    for frame in [
        TerminalViewFrame::Full(view),
        TerminalViewFrame::Delta(delta),
    ] {
        let mut projected = frame.clone();
        assert!(projected.normalize_hyperlinks_for_wire());
        let encoded = bincode::serialize(&frame).unwrap();
        assert!(encoded.len() <= crate::protocol::MAX_CHUNKED_RECORD_SIZE);
        let decoded = bincode::deserialize::<TerminalViewFrame>(&encoded).unwrap();
        assert_eq!(decoded, projected);
        let cells = match &decoded {
            TerminalViewFrame::Full(view) => &view.cells,
            TerminalViewFrame::Delta(delta) => &delta.runs[0].cells,
        };

        assert!(cells.iter().all(|cell| cell.text == "x"));
        assert!(cells[0].hyperlink.is_none());
        assert!(
            cells[1..=budgeted_links]
                .iter()
                .all(|cell| cell.hyperlink.is_some())
        );
        assert!(cells.last().unwrap().hyperlink.is_none());
    }
}

#[test]
fn alacritty_damage_produces_a_sparse_frame_against_the_last_take() {
    let size = TerminalSize::new(4, 12);
    let shared_size = Arc::new(Mutex::new(size));
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&shared_size));
    let terminal = Arc::new(Mutex::new(Term::new(Config::default(), &size, event_proxy)));
    let revision = Arc::new(AtomicU64::new(0));
    let source = TerminalViewSource {
        terminal: Arc::clone(&terminal),
        size: shared_size,
        revision: Arc::clone(&revision),
        damage_baseline: Arc::new(Mutex::new(None)),
        cursor_settle: Arc::new(Mutex::new(Default::default())),
    };

    let TerminalViewFrame::Full(mut retained) = source.take_frame().unwrap() else {
        panic!("the first damage frame must establish a full baseline");
    };
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut *terminal.lock().expect("terminal state lock poisoned"),
        b"\x1b]8;;https://example.com/a-long-shared-target\x1b\\abc\x1b]8;;\x1b\\",
    );
    revision.store(1, Ordering::Release);

    let frame = source.take_frame().unwrap();
    let TerminalViewFrame::Delta(TerminalViewDelta {
        base_revision: 0,
        revision: 1,
        runs,
        ..
    }) = &frame
    else {
        panic!("partial damage must produce a delta");
    };
    assert!(runs.iter().map(|run| run.cells.len()).sum::<usize>() < retained.cells.len());
    let links = runs
        .iter()
        .flat_map(|run| &run.cells)
        .filter_map(|cell| cell.hyperlink.as_ref())
        .collect::<Vec<_>>();
    assert!(links.len() >= 2);
    assert_eq!(
        links[0].as_ptr(),
        links[1].as_ptr(),
        "linked cells in one damage frame must share long URI storage"
    );
    retained.apply_frame(frame).unwrap();
    assert_eq!(retained, source.view());
}

#[test]
fn dense_or_resized_terminal_frames_use_a_full_view() {
    let previous = frame_test_view(1, "abcdef");
    let dense = frame_test_view(2, "XYZWef");
    assert!(matches!(
        TerminalView::frame_from(Some(&previous), &dense),
        Some(TerminalViewFrame::Full(_))
    ));

    let resized = frame_test_view(3, "abcdefg");
    assert!(matches!(
        TerminalView::frame_from(Some(&previous), &resized),
        Some(TerminalViewFrame::Full(_))
    ));
}

#[test]
fn visually_identical_terminal_revision_does_not_create_a_frame() {
    let previous = frame_test_view(1, "same");
    let current = frame_test_view(2, "same");
    assert_eq!(TerminalView::frame_from(Some(&previous), &current), None);
}

#[test]
fn terminal_delta_rejects_revision_gaps_and_invalid_runs() {
    let mut view = frame_test_view(4, "abcd");
    let gap = TerminalViewFrame::Delta(TerminalViewDelta {
        selection: None,
        base_revision: 5,
        revision: 6,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: Vec::new(),
    });
    assert!(matches!(
        view.apply_frame(gap),
        Err(TerminalFrameError::RevisionMismatch {
            expected: 5,
            actual: 4
        })
    ));

    let invalid = TerminalViewFrame::Delta(TerminalViewDelta {
        selection: None,
        base_revision: 4,
        revision: 5,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 4,
            cells: vec![blank_cell()],
        }],
    });
    assert_eq!(
        view.apply_frame(invalid),
        Err(TerminalFrameError::InvalidCellRun)
    );
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
        cells[column].text = ch.to_string().into();
    }
    let view = TerminalView {
        selection: None,
        revision: 1,
        size,
        display_offset: 3,
        mouse_tracking: TerminalMouseTracking::None,
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
        selection: None,
        revision: 1,
        size: TerminalSize::new(3, 10),
        display_offset: 2,
        mouse_tracking: TerminalMouseTracking::None,
        cells: vec![blank_cell(); 30],
        cursor: None,
    };

    let selection = view.line_selection_at(1).unwrap();
    assert_eq!((selection.start.row, selection.start.column), (1, 0));
    assert_eq!((selection.end.row, selection.end.column), (1, 9));
    assert_eq!(selection.display_offset, 2);
}
