use super::*;

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

/// The writer stops before the reader on close, so a query landing in that window ends the
/// reader cleanly rather than as a failed close.
#[test]
fn terminal_reply_to_a_stopped_writer_ends_the_reader_cleanly_and_keeps_tail_notices() {
    let size = TerminalSize::new(4, 12);
    let shared_size = Arc::new(Mutex::new(size));
    let (event_proxy, pending_replies, notices) = TerminalEventProxy::new(Arc::clone(&shared_size));
    let probe = TerminalNoticeProbe { notices };
    let terminal = Arc::new(Mutex::new(Term::new(Config::default(), &size, event_proxy)));
    let (input, input_receiver) = TerminalInput::channel();
    drop(input_receiver);
    let (updates, update_receiver) = mpsc::channel();

    read_loop(TerminalReadLoop {
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
    .unwrap();

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
