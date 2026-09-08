use super::*;

#[test]
fn osc_cwd_parser_handles_fragmented_st_and_bel_sequences() {
    let mut parser = OscScanner::default();
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
fn agent_event_sequences_are_cut_out_before_the_vt_and_reported() {
    let mut scanner = OscScanner::default();
    let mut reported = Vec::new();
    let event = AgentEvent::new(AgentKind::Codex, AgentEventKind::Stop, None);
    let mut stream = b"before".to_vec();
    stream.extend(event.encode());
    stream.extend(b"after\x1b]0;title\x07\x1b[31mred");

    // Everything around the event stays in order; only the event is gone.
    let filtered = scanner.advance(&stream, |osc| reported.push(osc));
    assert_eq!(
        &*filtered,
        b"beforeafter\x1b]0;title\x07\x1b[31mred" as &[u8]
    );
    assert_eq!(reported, [OscReport::Agent(event)]);

    // Once released, a non-event OSC is still delivered intact.
    let mut scanner = OscScanner::default();
    let filtered = scanner.advance(b"x\x1b]0;title\x07y\x1b[31m", |_| unreachable!());
    assert!(matches!(filtered, std::borrow::Cow::Borrowed(_)));
    assert_eq!(&*filtered, b"x\x1b]0;title\x07y\x1b[31m" as &[u8]);
}

#[test]
fn an_agent_event_split_across_reads_is_held_back_then_dropped() {
    let mut scanner = OscScanner::default();
    let mut reported = Vec::new();
    let event = AgentEvent::new(AgentKind::Claude, AgentEventKind::PermissionRequest, None);
    let encoded = event.encode();
    let mut vt = Vec::new();
    // One byte at a time: the hardest fragmentation.
    for chunk in encoded.chunks(1) {
        vt.extend_from_slice(&scanner.advance(chunk, |osc| reported.push(osc)));
    }
    vt.extend_from_slice(&scanner.advance(b"tail", |osc| reported.push(osc)));
    assert_eq!(vt, b"tail");
    assert_eq!(reported, [OscReport::Agent(event)]);

    // ST-terminated events and an ESC ESC ] start are handled too.
    let mut scanner = OscScanner::default();
    let mut reported = Vec::new();
    let body = br#"777;notify;condr://agent;{"v":1,"agent":"codex","event":"prompt-submit"}"#;
    let mut stream = b"\x1b\x1b]".to_vec();
    stream.extend_from_slice(body);
    stream.extend_from_slice(b"\x1b\\end");
    let vt = scanner
        .advance(&stream, |osc| reported.push(osc))
        .into_owned();
    assert_eq!(vt, b"\x1bend");
    assert_eq!(
        reported,
        [OscReport::Agent(AgentEvent::new(
            AgentKind::Codex,
            AgentEventKind::PromptSubmit,
            None
        ))]
    );
}

#[test]
fn a_held_sequence_that_turns_out_not_to_be_an_event_reaches_the_vt_in_order() {
    let mut scanner = OscScanner::default();
    // Diverges from the sentinel only at "condr": bytes held across two reads are
    // released in front of what follows.
    let first = scanner
        .advance(b"a\x1b]777;notify;", |_| unreachable!())
        .into_owned();
    assert_eq!(first, b"a");
    let second = scanner
        .advance(b"other;x\x07b", |_| unreachable!())
        .into_owned();
    assert_eq!(second, b"\x1b]777;notify;other;x\x07b");

    // A sentinel with a malformed body is still ours and still dropped.
    let mut scanner = OscScanner::default();
    let vt = scanner
        .advance(
            b"\x1b]777;notify;condr://agent;not json\x07k",
            |_| unreachable!(),
        )
        .into_owned();
    assert_eq!(vt, b"k");

    // An oversized payload is abandoned and released, whatever it began like.
    let mut scanner = OscScanner::default();
    let mut huge = b"\x1b]777;notify;condr://agent;".to_vec();
    huge.extend(std::iter::repeat_n(b'{', MAX_OSC_CWD_BYTES));
    let vt = scanner.advance(&huge, |_| unreachable!()).into_owned();
    assert_eq!(vt, huge);
    let vt = scanner.advance(b"}\x07after", |_| unreachable!());
    assert_eq!(&*vt, b"}\x07after" as &[u8]);
}

#[test]
fn a_bare_escape_ends_a_string_the_way_the_vt_reads_it() {
    let event = AgentEvent::new(AgentKind::Codex, AgentEventKind::Stop, None);
    let encoded = event.encode();

    // A Sixel/kitty image cut off by ^C: the VT recovers at the next ESC, and so must the
    // scanner, or every later agent event would be swallowed as DCS payload.
    let mut scanner = OscScanner::default();
    let mut reported = Vec::new();
    let mut stream = b"\x1bPq#0;2;0;0;0#0!10~-".to_vec();
    stream.extend_from_slice(&encoded);
    stream.extend_from_slice(b"prompt$ ");
    let vt = scanner
        .advance(&stream, |osc| reported.push(osc))
        .into_owned();
    assert_eq!(vt, b"\x1bPq#0;2;0;0;0#0!10~-prompt$ ");
    assert_eq!(reported, [OscReport::Agent(event.clone())]);

    // An unterminated title followed by a CSI: title ends at the ESC, the CSI survives,
    // and the event after them is still cut out.
    let mut scanner = OscScanner::default();
    let mut reported = Vec::new();
    let mut stream = b"\x1b]0;title\x1b[31m".to_vec();
    stream.extend_from_slice(&encoded);
    stream.extend_from_slice(b"red");
    let vt = scanner
        .advance(&stream, |osc| reported.push(osc))
        .into_owned();
    assert_eq!(vt, b"\x1b]0;title\x1b[31mred");
    assert_eq!(reported, [OscReport::Agent(event.clone())]);

    // The same with the ESC as the last byte of one read and the event opening the next.
    let mut scanner = OscScanner::default();
    let mut reported = Vec::new();
    let mut vt = scanner
        .advance(b"\x1b]0;title\x1b", |osc| reported.push(osc))
        .into_owned();
    let mut second = encoded[1..].to_vec();
    second.extend_from_slice(b"x");
    vt.extend_from_slice(&scanner.advance(&second, |osc| reported.push(osc)));
    assert_eq!(vt, b"\x1b]0;titlex");
    assert_eq!(reported, [OscReport::Agent(event.clone())]);

    // An unterminated *event* ended by an escape is dropped up to that escape.
    let mut scanner = OscScanner::default();
    let mut reported = Vec::new();
    let mut stream = encoded[..encoded.len() - 1].to_vec();
    stream.extend_from_slice(b"\x1b[0mz");
    let vt = scanner
        .advance(&stream, |osc| reported.push(osc))
        .into_owned();
    assert_eq!(vt, b"\x1b[0mz");
    assert_eq!(reported, [OscReport::Agent(event)]);
}

#[test]
fn the_scanner_produces_the_same_bytes_and_reports_however_a_stream_is_chunked() {
    // A small deterministic generator: enough to mix events, half events, titles, DCS,
    // stray escapes and text in every order and cut them anywhere.
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let event = AgentEvent::new(AgentKind::Claude, AgentEventKind::PromptSubmit, None).encode();
    let tokens: [&[u8]; 13] = [
        b"text ",
        &event,
        &event[..event.len() / 2],
        b"\x1b]0;title\x07",
        b"\x1b]0;title\x1b\\",
        b"\x1b]0;title",
        b"\x1b]777;notify;other\x07",
        b"\x1bPq~~\x1b\\",
        b"\x1bPq~~",
        b"\x1b[31m",
        b"\x1b",
        b"\x07",
        b"\x1b]777;notify;condr://agent;{bad}\x07",
    ];
    for _ in 0..2000 {
        let mut stream = Vec::new();
        for _ in 0..(next() % 8 + 1) {
            stream.extend_from_slice(tokens[(next() % tokens.len() as u64) as usize]);
        }
        let mut whole = OscScanner::default();
        let mut whole_reports = Vec::new();
        let whole_vt = whole
            .advance(&stream, |osc| whole_reports.push(osc))
            .into_owned();

        let mut chunked = OscScanner::default();
        let mut chunked_reports = Vec::new();
        let mut chunked_vt = Vec::new();
        let mut rest: &[u8] = &stream;
        while !rest.is_empty() {
            let take = ((next() % 9) as usize + 1).min(rest.len());
            let (chunk, tail) = rest.split_at(take);
            chunked_vt.extend_from_slice(&chunked.advance(chunk, |osc| chunked_reports.push(osc)));
            rest = tail;
        }
        // Flush what a trailing partial sequence still holds, in both.
        chunked_vt.extend_from_slice(&chunked.advance(b"\x07\x07", |_| {}));
        let mut whole_vt = whole_vt;
        whole_vt.extend_from_slice(&whole.advance(b"\x07\x07", |_| {}));

        assert_eq!(
            chunked_vt,
            whole_vt,
            "stream {:?} differs when chunked",
            String::from_utf8_lossy(&stream)
        );
        assert_eq!(chunked_reports, whole_reports);
        assert!(
            !String::from_utf8_lossy(&whole_vt).contains("condr://agent"),
            "an event leaked to the VT in {:?}",
            String::from_utf8_lossy(&stream)
        );
    }
}

#[test]
fn osc_cwd_parser_recovers_after_malformed_prefixes() {
    let mut parser = OscScanner::default();
    let mut reported = Vec::new();

    parser.advance(b"\x1b]9;8;ignored\x07\x1b]9;9;/valid\x07", |osc| {
        if let OscReport::Cwd(cwd) = osc {
            reported.push(cwd)
        }
    });

    assert_eq!(reported, [PathBuf::from("/valid")]);
}

#[test]
fn a_control_string_ends_at_st_or_at_the_escape_that_starts_something_else() {
    let mut parser = OscScanner::default();
    let mut reported = Vec::new();

    // BEL is payload inside a DCS; an `ESC ]` ends it and starts an OSC, exactly as the
    // VT reads the same bytes, so the "fake" cwd is as real as the terminal makes it.
    parser.advance(b"\x1bPq\x07\x1bPq\x07 more", |osc| reported.push(osc));
    assert!(reported.is_empty());
    parser.advance(b"\x1b\\\x1b]7;file:///real\x07", |osc| reported.push(osc));
    assert_eq!(reported, [OscReport::Cwd(PathBuf::from("/real"))]);

    let mut parser = OscScanner::default();
    let mut reported = Vec::new();
    parser.advance(b"\x1bPq\x1b]7;file:///after\x07", |osc| reported.push(osc));
    assert_eq!(reported, [OscReport::Cwd(PathBuf::from("/after"))]);
}

#[test]
fn osc_seven_and_iterm_cwd_reports_are_decoded() {
    let mut parser = OscScanner::default();
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
    let mut parser = OscScanner::default();
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
    let mut parser = OscScanner::default();

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
