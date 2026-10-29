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
    assert!(
        encode_key(
            &TerminalKey::Character("c".into()),
            TerminalModifiers {
                platform: true,
                ..none
            },
            false,
        )
        .unwrap()
        .is_empty(),
        "Cmd never types text"
    );
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
    let platform = TerminalModifiers {
        platform: true,
        ..none
    };
    let control_shift = TerminalModifiers {
        control: true,
        shift: true,
        ..none
    };
    // Kitty keyboard flags as the program pushes them (`CSI > flags u`).
    let disambiguate = 1u8;
    let events = disambiguate | 2;
    let all = disambiguate | 4 | 8 | 16;
    let legacy = 0u8;
    let ch = |text: &str| TerminalKey::Character(text.into());
    let cases: &[(TerminalKey, TerminalModifiers, u8, &[u8])] = &[
        // Legacy stays legacy without any kitty flag.
        (TerminalKey::Enter, shift, legacy, b"\n"),
        // herdr parity under disambiguation.
        (TerminalKey::Enter, shift, disambiguate, b"\x1b[13;2u"),
        (TerminalKey::Enter, alt, disambiguate, b"\x1b[13;3u"),
        (TerminalKey::Backspace, alt, disambiguate, b"\x1b[127;3u"),
        (ch("a"), control_shift, disambiguate, b"\x1b[97;6u"),
        (ch("L"), control_shift, disambiguate, b"\x1b[108;6u"),
        (ch("c"), control, disambiguate, b"\x1b[99;5u"),
        // Cmd+C reaches a kitty program as super+c; legacy encoding drops it entirely.
        (ch("c"), platform, disambiguate, b"\x1b[99;9u"),
        (ch("c"), platform, legacy, b""),
        (ch("L"), shift, disambiguate, b"L"),
        (ch("a"), none, disambiguate, b"a"),
        (TerminalKey::Enter, none, disambiguate, b"\r"),
        (TerminalKey::Tab, none, disambiguate, b"\t"),
        (TerminalKey::Backspace, none, disambiguate, b"\x7f"),
        (TerminalKey::Up, alt, disambiguate, b"\x1b[1;3A"),
        (TerminalKey::Function(5), shift, disambiguate, b"\x1b[15;2~"),
        // Escape follows the spec rather than herdr.
        (TerminalKey::Escape, none, disambiguate, b"\x1b[27u"),
        // Event types put legacy-form keys in kitty form; a press, the default, is left
        // unnumbered.
        (ch("c"), control, events, b"\x1b[99;5u"),
        (TerminalKey::Left, none, events, b"\x1b[1;1:1D"),
        (TerminalKey::Function(3), control, events, b"\x1b[13;5~"),
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
        (TerminalKey::Function(3), none, legacy, b"\x1bOR"),
        (TerminalKey::Function(5), none, all, b"\x1b[15~"),
        // libghostty-vt reads any kitty flag as kitty mode, alternate keys alone included.
        (TerminalKey::Escape, none, 4, b"\x1b[27u"),
        (TerminalKey::Enter, none, events, b"\r"),
        // Report-all reports text keys with alternate and associated text, a default
        // modifier field left empty.
        (ch("a"), none, all, b"\x1b[97;;97u"),
        (ch("L"), shift, all, b"\x1b[108:76;2;76u"),
        (ch("c"), control, all, b"\x1b[99;5u"),
        (TerminalKey::Enter, none, all, b"\x1b[13u"),
        (ch("c"), control, all | 2, b"\x1b[99;5u"),
        (TerminalKey::BackTab, none, all, b"\x1b[9;2u"),
    ];
    let mut mismatches = Vec::new();
    for (key, modifiers, flags, expected) in cases {
        let found = encode(*flags, key, *modifiers, TerminalKeyEventKind::Press, false);
        if found != *expected {
            mismatches.push(format!(
                "{key:?} {modifiers:?} {flags}: expected {} found {}",
                expected.escape_ascii(),
                found.escape_ascii()
            ));
        }
    }

    use TerminalKeyEventKind::{Release, Repeat};
    let report_text = events | 8 | 16;
    let event_cases: &[(
        TerminalKey,
        TerminalModifiers,
        TerminalKeyEventKind,
        u8,
        &[u8],
    )] = &[
        // Event types number a repeat 2 and a release 3.
        (ch("c"), control, Repeat, events, b"\x1b[99;5:2u"),
        (ch("c"), control, Release, events, b"\x1b[99;5:3u"),
        (TerminalKey::Left, none, Repeat, events, b"\x1b[1;1:2D"),
        (TerminalKey::Left, none, Release, events, b"\x1b[1;1:3D"),
        (TerminalKey::Escape, none, Release, events, b"\x1b[27;1:3u"),
        (ch("c"), platform, Release, events, b"\x1b[99;9:3u"),
        // Enter, Tab and Backspace keep their legacy bytes and have no release until
        // every key is an escape code.
        (TerminalKey::Enter, none, Repeat, events, b"\r"),
        (TerminalKey::Enter, none, Release, events, b""),
        (TerminalKey::Enter, shift, Release, events, b""),
        (TerminalKey::BackTab, none, Release, events, b""),
        (
            TerminalKey::Enter,
            none,
            Release,
            report_text,
            b"\x1b[13;1:3u",
        ),
        // A release carries no associated text.
        (ch("a"), none, Repeat, report_text, b"\x1b[97;1:2;97u"),
        (ch("a"), none, Release, report_text, b"\x1b[97;1:3u"),
        // Without event types a repeat is a press and a release is nothing.
        (ch("c"), control, Repeat, disambiguate, b"\x1b[99;5u"),
        (ch("c"), control, Release, disambiguate, b""),
        (TerminalKey::Escape, none, Release, disambiguate, b""),
        (ch("c"), control, Repeat, legacy, b"\x03"),
        (ch("c"), control, Release, legacy, b""),
        (TerminalKey::Left, none, Repeat, legacy, b"\x1b[D"),
        (TerminalKey::Left, none, Release, legacy, b""),
    ];
    for (key, modifiers, kind, flags, expected) in event_cases {
        let found = encode(*flags, key, *modifiers, *kind, false);
        if found != *expected {
            mismatches.push(format!(
                "{key:?} {modifiers:?} {kind:?} {flags}: expected {} found {}",
                expected.escape_ascii(),
                found.escape_ascii()
            ));
        }
    }
    for (text, flags, expected) in [
        ("a", disambiguate, &b"a"[..]),
        ("A", all, b"\x1b[97:65;2;65u"),
        ("ab", all, b"ab"),
    ] {
        let found = vt_with_flags(flags).encode_text(text);
        if found != expected {
            mismatches.push(format!(
                "text {text} {flags}: expected {} found {}",
                expected.escape_ascii(),
                found.escape_ascii()
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A VT whose program pushed `flags` (`CSI > flags u`); 0 leaves it on legacy keys.
fn vt_with_flags(flags: u8) -> Vt {
    let mut vt = Vt::new(TerminalSize::new(2, 20)).unwrap();
    if flags != 0 {
        vt.write(format!("\x1b[>{flags}u").as_bytes());
    }
    vt
}

fn encode(
    flags: u8,
    key: &TerminalKey,
    modifiers: TerminalModifiers,
    kind: TerminalKeyEventKind,
    reads_kitty_keys: bool,
) -> Vec<u8> {
    vt_with_flags(flags)
        .encode_key(key, modifiers, kind, reads_kitty_keys)
        .unwrap()
}

#[test]
fn cmd_c_reaches_a_program_that_reads_kitty_keys_unasked() {
    use TerminalKeyEventKind::{Press, Release};
    let ch = |text: &str| TerminalKey::Character(text.into());
    let platform = TerminalModifiers {
        platform: true,
        ..TerminalModifiers::default()
    };
    let control = TerminalModifiers {
        control: true,
        ..TerminalModifiers::default()
    };
    let cmd_shift = TerminalModifiers {
        shift: true,
        ..platform
    };
    let legacy = 0u8;
    let encode =
        |key: &str, modifiers, kind, flags, reads| encode(flags, &ch(key), modifiers, kind, reads);
    assert_eq!(encode("c", platform, Press, legacy, true), b"\x1b[99;9u");
    assert_eq!(encode("c", platform, Press, legacy, false), b"");
    // Only Cmd+C changes: other Cmd chords still send nothing, Ctrl+C stays ETX, a
    // release stays nothing, and negotiated flags keep their own encoding.
    assert_eq!(encode("x", platform, Press, legacy, true), b"");
    assert_eq!(encode("c", cmd_shift, Press, legacy, true), b"");
    assert_eq!(encode("c", control, Press, legacy, true), b"\x03");
    assert_eq!(encode("c", platform, Release, legacy, true), b"");
    assert_eq!(encode("c", platform, Press, 1 | 2, true), b"\x1b[99;9u");
}

#[test]
fn paste_respects_bracketed_mode_and_filters_control_markers() {
    assert_eq!(encode_paste("one\r\ntwo\n", false), b"one\rtwo\r");
    assert_eq!(
        encode_paste("one\x1b[201~\x03two\n", true),
        b"\x1b[200~one[201~two\n\x1b[201~"
    );
}
