use super::*;

/// What the VT reported for `stream`, applied as the PTY reader applies it.
fn applied(stream: &[u8]) -> (Vec<u8>, SharedTerminalNotices, Mutex<ReportedCwd>) {
    let mut vt = Vt::new(TerminalSize::new(4, 20)).unwrap();
    let notices = SharedTerminalNotices::default();
    let reported = Mutex::new(ReportedCwd::default());
    let replies = apply_terminal_events(vt.write(stream), &notices, &reported);
    (replies, notices, reported)
}

#[test]
fn osc_seven_and_iterm_cwd_reports_are_decoded() {
    assert_eq!(
        reported_cwd_of("file://localhost/tmp/with%20space"),
        Some(PathBuf::from("/tmp/with space"))
    );
    assert_eq!(reported_cwd_of("file://elsewhere/nope"), None);
    assert_eq!(
        reported_cwd_of("file:///plain"),
        Some(PathBuf::from("/plain"))
    );
    assert_eq!(reported_cwd_of("/iterm"), Some(PathBuf::from("/iterm")));
    // ConEmu's 9;9 may quote the path.
    assert_eq!(
        reported_cwd_of("\"/quoted\""),
        Some(PathBuf::from("/quoted"))
    );
    assert_eq!(reported_cwd_of(""), None);
}

#[test]
fn osc_seven_accepts_this_host_and_escaped_paths_but_rejects_foreign_hosts() {
    let host = sysinfo::System::host_name().expect("test host has a name");
    #[cfg(unix)]
    let expected = PathBuf::from("/C:/with space/#%");
    #[cfg(windows)]
    let expected = PathBuf::from("C:\\with space\\#%");
    for authority in [
        String::new(),
        "LOCALHOST".into(),
        host.to_uppercase(),
        format!("{host}."),
    ] {
        let uri = format!("file://{authority}/C:/with%20space/%23%25");
        assert_eq!(reported_cwd_of(&uri), Some(expected.clone()), "{uri}");
    }
    for uri in [
        "file://foreign.invalid/nope",
        "file:///bad%xx",
        "file:///nul%00path",
    ] {
        assert_eq!(reported_cwd_of(uri), None, "{uri}");
    }
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
    let (_, _, reported) = applied(format!("{sequence}{sequence}").as_bytes());
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
fn agent_events_and_program_reports_are_queued_and_the_query_answered_first() {
    let event = AgentEvent::new(AgentKind::Codex, AgentEventKind::Stop, None, None);
    let mut stream = event.encode();
    stream.extend_from_slice(
        b"\x1b]7501;?\x1b\\\x1b[c\x1b]7501;state=working:app=codex\x07\x1b]7501;state=napping\x07\x1b]777;notify;elsewhere;{}\x07",
    );
    let (replies, notices, _) = applied(&stream);
    let replies = String::from_utf8(replies).unwrap();
    let (status, da1) = replies.split_once("\x1b[?62;22c").expect("a DA1 reply");
    assert!(
        status.starts_with("\x1b]7501;?"),
        "the 7501 answer leads: {replies:?}"
    );
    assert!(da1.is_empty());
    assert_eq!(
        notices.lock().unwrap().agent_events,
        [
            AgentNotice::Hook(event),
            AgentNotice::Status(ProgramStatus {
                state: crate::ProgramState::Working,
                id: None,
                kind: None,
                app: Some("codex".into()),
                msg: None,
            }),
        ]
    );
}

/// `condr agent-hook` writes to the terminal the agent draws on, so its event can land
/// between the bytes of one of the agent's characters or inside one of its escape
/// sequences, however the PTY reader then cuts the stream; the agent's output must show
/// as if the event had not been there (ADR 0051).
#[test]
fn a_hook_event_inside_the_agents_output_leaves_that_output_whole() {
    let started = AgentEvent::new(AgentKind::Claude, AgentEventKind::SessionStart, None, None);
    let asked = AgentEvent::new(AgentKind::Claude, AgentEventKind::QuestionAsked, None, None);
    // "─" cut after its second byte, then a cursor move `ESC [ 3 C` cut before its `C`.
    let mut stream = b"\xe2\x94".to_vec();
    stream.extend(started.encode());
    stream.extend_from_slice(b"\x80\x1b[3");
    stream.extend(asked.encode());
    stream.extend_from_slice(b"Cx");
    let size = TerminalSize::new(4, 20);
    for cut in 0..=stream.len() {
        let mut vt = Vt::new(size).unwrap();
        let notices = SharedTerminalNotices::default();
        let reported = Mutex::new(ReportedCwd::default());
        for read in [&stream[..cut], &stream[cut..]] {
            apply_terminal_events(vt.write(read), &notices, &reported);
        }
        assert_eq!(vt.visible_text(size), "─   x", "read cut at byte {cut}");
        assert_eq!(
            notices.lock().unwrap().agent_events,
            [
                AgentNotice::Hook(started.clone()),
                AgentNotice::Hook(asked.clone())
            ],
            "read cut at byte {cut}"
        );
    }
}

/// Only a whole event of the hook's is cut out; anything else stays the library's, so
/// output is never held back longer than an event can be.
#[test]
fn only_a_whole_hook_event_is_cut_out() {
    let opener = format!("\x1b]{AGENT_EVENT_OSC_PREFIX}");
    let event = format!("x{opener}{{}}\x1b\\y");
    assert_eq!(
        next_hook_cut(event.as_bytes()),
        Some(HookCut::Event {
            start: 1,
            body: opener.len() + 1..opener.len() + 3,
            end: event.len() - 1,
        })
    );
    assert_eq!(
        next_hook_cut(format!("x{opener}{{").as_bytes()),
        Some(HookCut::Open(1))
    );
    assert_eq!(next_hook_cut(b"x\x1b]77"), Some(HookCut::Open(1)));
    // Longer than any event the hook writes, or ended by another escape: not one.
    let long = format!("{opener}{}", "a".repeat(MAX_AGENT_EVENT_BYTES));
    assert_eq!(next_hook_cut(long.as_bytes()), None);
    assert_eq!(next_hook_cut(format!("{opener}{{\x1b[m").as_bytes()), None);
    assert_eq!(next_hook_cut(b"\x1b]777;notify;elsewhere;{}\x07"), None);
}

#[test]
fn prompt_starts_and_resets_only_reach_an_agent_that_reports_program_status() {
    let (_, notices, _) = applied(b"\x1b]0;title\x07\x1b]133;A;cl=m\x07\x1bc");
    let notices = notices.lock().unwrap();
    assert!(
        notices.agent_events.is_empty(),
        "a plain shell's prompts are its own"
    );
    assert_eq!(notices.title, None, "RIS clears the title");
    assert!(notices.title_changed);
}
