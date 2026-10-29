use super::*;

/// Feeds `stream` to a VT and applies what it reported, as the PTY reader does.
fn feed(vt: &mut Vt, notices: &SharedTerminalNotices, stream: &[u8]) -> Vec<u8> {
    apply_terminal_events(
        vt.write(stream),
        notices,
        &Mutex::new(ReportedCwd::default()),
    )
}

#[test]
fn terminal_color_replies_use_the_render_palette_and_preserve_order() {
    let mut vt = Vt::new(TerminalSize::new(4, 12)).unwrap();
    let replies = feed(
        &mut vt,
        &SharedTerminalNotices::default(),
        b"\x1b]11;?\x07\x1b[5n\x1b]4;1;#123456\x07\x1b]4;1;?\x07",
    );
    assert_eq!(
        String::from_utf8(replies).unwrap(),
        "\x1b]11;rgb:0d0d/1111/1717\x07\x1b[0n\x1b]4;1;rgb:1212/3434/5656\x07"
    );
}

#[test]
fn title_and_bell_notices_are_recorded_once_per_change_and_counted() {
    let mut vt = Vt::new(TerminalSize::new(4, 12)).unwrap();
    let notices = SharedTerminalNotices::default();
    let probe = TerminalNoticeProbe {
        notices: Arc::clone(&notices),
    };

    feed(
        &mut vt,
        &notices,
        b"\x1b]2;\xe2\x9c\xb3 fix tests\x07\x07\x07\x1b]52;c;aGVsbG8=\x07\x1b]52;c;d29ybGQ=\x07",
    );
    assert_eq!(
        probe.take(),
        TerminalNoticeBatch {
            title: Some(Some("\u{2733} fix tests".into())),
            bells: 2,
            clipboard: Some("world".into()),
            progress: None,
            notifications: Vec::new(),
            finished_commands: Vec::new(),
        }
    );
    assert!(probe.take().is_empty());

    // A spinner frame is part of the title, as Windows Terminal shows it.
    feed(&mut vt, &notices, b"\x1b]2;\xe2\x9c\xbb fix tests\x07");
    assert_eq!(probe.take().title, Some(Some("\u{273b} fix tests".into())));

    feed(&mut vt, &notices, b"\x1b]2;\x07");
    assert_eq!(probe.take().title, Some(None));

    feed(&mut vt, &notices, b"\x1b]52;c;\x07");
    assert_eq!(probe.take().clipboard.as_deref(), Some(""));
}

/// The writer stops before the reader on close, so a query landing in that window ends the
/// reader cleanly rather than as a failed close.
#[test]
fn terminal_reply_to_a_stopped_writer_ends_the_reader_cleanly_and_keeps_tail_notices() {
    let size = TerminalSize::new(4, 12);
    let notices = SharedTerminalNotices::default();
    let probe = TerminalNoticeProbe {
        notices: Arc::clone(&notices),
    };
    let (input, input_receiver) = TerminalInput::channel();
    drop(input_receiver);
    let (updates, update_receiver) = mpsc::channel();

    read_loop(TerminalReadLoop {
        reader: Box::new(io::Cursor::new(
            b"\x1b]2;tail title\x07\x07\x1b]52;c;dGFpbCBjb3B5\x07\x1b[5n".to_vec(),
        )),
        terminal: Arc::new(Mutex::new(Vt::new(size).unwrap())),
        input,
        revision: Arc::new(AtomicU64::new(0)),
        updates,
        reported_cwd: Arc::new(Mutex::new(ReportedCwd::default())),
        notices,
        size: Arc::new(Mutex::new(size)),
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
            progress: None,
            notifications: Vec::new(),
            finished_commands: Vec::new(),
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

#[test]
fn progress_is_recorded_once_per_change_and_cleared_by_remove_reset_and_silence() {
    let mut vt = Vt::new(TerminalSize::new(4, 12)).unwrap();
    let notices = SharedTerminalNotices::default();
    let probe = TerminalNoticeProbe {
        notices: Arc::clone(&notices),
    };
    let progress = |state, percent| Some(Some(TerminalProgress { state, percent }));

    feed(&mut vt, &notices, b"\x1b]9;4;1;40\x07");
    assert_eq!(
        probe.take().progress,
        progress(TerminalProgressState::Normal, Some(40))
    );
    feed(&mut vt, &notices, b"\x1b]9;4;1;40\x07");
    assert_eq!(probe.take().progress, None, "the same report again");
    feed(&mut vt, &notices, b"\x1b]9;4;3\x07\x1b]9;4;0\x07");
    assert_eq!(
        probe.take().progress,
        Some(None),
        "set and removed in one read"
    );

    feed(&mut vt, &notices, b"\x1b]9;4;2;70\x07");
    assert_eq!(
        probe.take().progress,
        progress(TerminalProgressState::Error, Some(70))
    );
    feed(&mut vt, &notices, b"\x1bc");
    assert_eq!(probe.take().progress, Some(None), "RIS clears it");

    feed(&mut vt, &notices, b"\x1b]9;4;4;10\x07");
    assert!(probe.take().progress.is_some());
    notices.lock().unwrap().progress_at = Instant::now().checked_sub(PROGRESS_TIMEOUT);
    assert_eq!(
        probe.take().progress,
        Some(None),
        "a silent program loses its bar"
    );
    assert!(probe.take().is_empty());
}

#[test]
fn program_notifications_are_queued_but_condrs_own_agent_events_are_not() {
    let mut vt = Vt::new(TerminalSize::new(4, 12)).unwrap();
    let notices = SharedTerminalNotices::default();
    let probe = TerminalNoticeProbe {
        notices: Arc::clone(&notices),
    };
    let event = AgentEvent::new(AgentKind::Codex, AgentEventKind::Stop, None, None);
    let mut stream = event.encode();
    stream.extend_from_slice(
        b"\x1b]9;Build done\x07\x1b]777;notify;Deploy;  prod \x07\x1b]777;notify;;\x07",
    );
    feed(&mut vt, &notices, &stream);
    assert_eq!(
        probe.take().notifications,
        [
            TerminalNotification {
                title: None,
                body: "Build done".into(),
            },
            TerminalNotification {
                title: Some("Deploy".into()),
                body: "prod".into(),
            },
        ],
        "the hook event is not one, an empty one is dropped"
    );
    assert_eq!(
        notices.lock().unwrap().agent_events,
        [AgentNotice::Hook(event)]
    );

    for index in 0..20 {
        feed(&mut vt, &notices, format!("\x1b]9;n{index}\x07").as_bytes());
    }
    let queued = probe.take().notifications;
    assert_eq!(queued.len(), 8, "a flood keeps the newest");
    assert_eq!(queued.last().unwrap().body, "n19");
}

#[test]
fn a_long_shell_command_is_reported_when_it_ends_unless_an_agent_ran_in_it() {
    let mut vt = Vt::new(TerminalSize::new(4, 12)).unwrap();
    let notices = SharedTerminalNotices::default();
    let probe = TerminalNoticeProbe {
        notices: Arc::clone(&notices),
    };
    let age = |notices: &SharedTerminalNotices| {
        let mut notices = notices.lock().unwrap();
        let command = notices.command.as_mut().expect("a command runs");
        command.started = Instant::now().checked_sub(LONG_COMMAND).unwrap();
    };

    feed(&mut vt, &notices, b"\x1b]133;C\x07\x1b]133;D;0\x07");
    assert!(probe.take().finished_commands.is_empty(), "a short one");

    feed(&mut vt, &notices, b"\x1b]133;C\x07");
    age(&notices);
    feed(&mut vt, &notices, b"\x1b]133;D;2\x07");
    let finished = probe.take().finished_commands;
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].exit_code, Some(2));
    assert!(finished[0].duration >= LONG_COMMAND);

    feed(&mut vt, &notices, b"\x1b]133;C\x07");
    age(&notices);
    notices.lock().unwrap().command.as_mut().unwrap().agent = true;
    feed(&mut vt, &notices, b"\x1b]133;D;0\x07");
    assert!(
        probe.take().finished_commands.is_empty(),
        "an Agent's session"
    );

    feed(&mut vt, &notices, b"\x1b]133;D;0\x07");
    assert!(probe.take().is_empty(), "an end without a start");
}

#[test]
fn a_programs_own_status_percentage_shows_as_its_progress_until_a_report_moves_on() {
    let mut vt = Vt::new(TerminalSize::new(4, 12)).unwrap();
    let notices = SharedTerminalNotices::default();
    let probe = TerminalNoticeProbe {
        notices: Arc::clone(&notices),
    };
    let progress = |state, percent| {
        Some(Some(TerminalProgress {
            state,
            percent: Some(percent),
        }))
    };
    let mut feed = |stream: &[u8]| {
        feed(&mut vt, &notices, stream);
        probe.take().progress
    };

    assert_eq!(
        feed(b"\x1b]7501;state=working:progress=40\x07"),
        progress(TerminalProgressState::Normal, 40)
    );
    assert_eq!(
        feed(b"\x1b]7501;state=blocked:kind=permission:progress=40\x07"),
        progress(TerminalProgressState::Paused, 40)
    );
    assert_eq!(
        feed(b"\x1b]7501;state=working:id=sync:progress=90\x07"),
        None,
        "a child record's progress is not the Pane's"
    );
    assert_eq!(feed(b"\x1b]7501;state=working\x07"), Some(None), "moved on");

    assert_eq!(
        feed(b"\x1b]9;4;1;30\x07"),
        progress(TerminalProgressState::Normal, 30)
    );
    assert_eq!(
        feed(b"\x1b]7501;state=done\x07"),
        None,
        "an OSC 9;4 progress is not the report's to end"
    );
    assert_eq!(
        feed(b"\x1b]7501;state=working:progress=10\x07"),
        progress(TerminalProgressState::Normal, 10)
    );
    assert_eq!(feed(b"\x1b]133;A\x07"), Some(None), "a prompt ends it");

    // A waiting program does not repeat itself: its bar outlives the OSC 9;4 timeout.
    assert_eq!(
        feed(b"\x1b]7501;state=blocked:kind=question:progress=60\x07"),
        progress(TerminalProgressState::Paused, 60)
    );
    notices.lock().unwrap().progress_at = Instant::now().checked_sub(PROGRESS_TIMEOUT);
    assert_eq!(probe.take().progress, None, "still waiting, still shown");
    notices.lock().unwrap().end_status_progress();
    assert_eq!(probe.take().progress, Some(None), "the program left");
}
