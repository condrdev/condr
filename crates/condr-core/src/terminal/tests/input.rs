use super::*;

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
