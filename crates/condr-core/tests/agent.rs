use condr_core::*;
use std::time::Instant;

fn info<'a>(name: &'a str, argv: &'a [String]) -> ProcessInfo<'a> {
    ProcessInfo {
        name,
        argv: Some(argv),
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn new_agents_are_identified_through_native_commands_and_npm_launchers() {
    for kind in AgentKind::ALL {
        assert_eq!(AgentKind::parse_label(kind.id()), Some(kind));
        let executable = format!("C:\\tools\\{}.exe", kind.executable());
        assert_eq!(
            identify_agent_process(info("runtime", &args(&[&executable]))),
            Some(kind)
        );
    }
    for (kind, package) in [
        (AgentKind::Pi, "@earendil-works/pi-coding-agent"),
        (AgentKind::Pi, "@mariozechner/pi-coding-agent"),
        (AgentKind::Omp, "@oh-my-pi/pi-coding-agent"),
        (AgentKind::Copilot, "@github/copilot"),
    ] {
        let script = format!("/usr/lib/node_modules/{package}/dist/cli.js");
        assert_eq!(
            identify_agent_process(info("node", &args(&["node", &script]))),
            Some(kind)
        );
    }
    assert_eq!(
        AgentKind::parse_label("agent"),
        None,
        "generic command names cannot identify Cursor"
    );
    assert_eq!(
        AgentResume {
            kind: AgentKind::Copilot,
            session_id: "conversation".into()
        }
        .args(),
        ["--resume=conversation"]
    );
    assert!(!AgentKind::Copilot.reports_at_startup());
    assert!(!AgentKind::Cursor.reports_at_startup());
}

#[test]
fn a_delayed_grok_completion_cannot_finish_a_newer_prompt() {
    let mut detector = AgentDetector::new();
    detector.observe_process(ProcessProbeResult::Agent(AgentKind::Grok), Instant::now());
    let event = |kind, prompt: &str| {
        let mut event = AgentEvent::new(AgentKind::Grok, kind, None, Some("root".into()));
        event.prompt_id = Some(prompt.into());
        let bytes = event.encode();
        AgentEvent::decode(&bytes[2..bytes.len() - 1]).unwrap()
    };
    detector.observe_event(&event(AgentEventKind::PromptSubmit, "first"));
    detector.observe_event(&event(AgentEventKind::PromptSubmit, "second"));
    assert_eq!(
        detector.observe_event(&event(AgentEventKind::Interrupt, "first")),
        AgentPublish::Nothing
    );
    assert_eq!(
        detector.observe_event(&event(AgentEventKind::Stop, "second")),
        AgentPublish::Snapshot(AgentSnapshot {
            kind: AgentKind::Grok,
            session_id: Some("root".into()),
            state: AgentState::Idle,
            blocked_on: None,
        })
    );
}

#[test]
fn identifies_agents_directly_and_through_wrappers() {
    assert_eq!(
        identify_agent_process(info("claude", &args(&["claude"]))),
        Some(AgentKind::Claude)
    );
    assert_eq!(
        identify_agent_process(info(
            "node",
            &args(&["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js"])
        )),
        Some(AgentKind::Codex)
    );
    assert_eq!(
        identify_agent_process(info(
            "node",
            &args(&[
                "node",
                "--no-warnings",
                "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"
            ])
        )),
        Some(AgentKind::Claude)
    );
    assert_eq!(
        identify_agent_process(info("node", &args(&["node", "/usr/local/bin/opencode"]))),
        Some(AgentKind::OpenCode)
    );
    assert_eq!(
        identify_agent_process(info(
            "cmd.exe",
            &args(&[
                "cmd.exe",
                "/d",
                "/s",
                "/c",
                "\"C:\\Users\\me\\AppData\\Roaming\\npm\\claude.cmd\""
            ])
        )),
        Some(AgentKind::Claude)
    );
    assert_eq!(
        identify_agent_process(info(
            "pwsh.exe",
            &args(&["pwsh.exe", "-NoLogo", "-Command", "& codex --model o3"])
        )),
        Some(AgentKind::Codex)
    );
    assert_eq!(
        identify_agent_process(info("pwsh.exe", &args(&["pwsh.exe", "-NoLogo"]))),
        None
    );
    assert_eq!(
        identify_agent_process(info("python3", &args(&["python3", "-m", "claude"]))),
        None
    );
    assert_eq!(
        identify_agent_process(info("node", &args(&["node", "-e", "claude"]))),
        None
    );
    assert_eq!(
        identify_agent_process(info("vim", &args(&["vim", "claude"]))),
        None
    );
    assert_eq!(AgentKind::parse_label("Codex.exe"), Some(AgentKind::Codex));
    assert_eq!(
        AgentKind::parse_label("C:\\tools\\claude-code.cmd"),
        Some(AgentKind::Claude)
    );
}

#[test]
fn the_process_named_after_the_agent_wins_over_wrappers() {
    let node = args(&["node", "/opt/node_modules/@openai/codex/bin/codex.js"]);
    let claude = args(&["claude"]);
    assert_eq!(
        identify_agent_among([info("node", &node), info("claude", &claude)]),
        Some(AgentKind::Claude)
    );
    assert_eq!(
        identify_agent_among([info("node", &node)]),
        Some(AgentKind::Codex)
    );
    assert_eq!(identify_agent_among([info("bash", &args(&["bash"]))]), None);
}

#[test]
fn a_new_agent_is_unknown_until_it_reports_and_a_restart_is_a_new_agent() {
    let mut detector = AgentDetector::new();
    let now = Instant::now();
    assert_eq!(
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now),
        AgentPublish::Snapshot(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Unknown,
            blocked_on: None,
        })
    );
    assert_eq!(
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now),
        AgentPublish::Nothing
    );
    assert_eq!(
        detector.observe_process(ProcessProbeResult::ShellOnly, now),
        AgentPublish::Cleared
    );
    assert_eq!(detector.agent(), None);
    assert!(matches!(
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now),
        AgentPublish::Snapshot(_)
    ));
    // A different kind in the same job is a fresh agent, not a state change.
    assert_eq!(
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Claude), now),
        AgentPublish::Snapshot(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Claude,
            state: AgentState::Unknown,
            blocked_on: None,
        })
    );
}

#[test]
fn unidentified_probes_need_six_misses_but_a_bare_shell_needs_one() {
    let mut detector = AgentDetector::new();
    let now = Instant::now();
    detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now);
    for _ in 0..5 {
        assert_eq!(
            detector.observe_process(ProcessProbeResult::Unidentified, now),
            AgentPublish::Nothing
        );
    }
    assert_eq!(
        detector.observe_process(ProcessProbeResult::Unidentified, now),
        AgentPublish::Cleared
    );
}

#[test]
fn hook_events_drive_the_state_and_round_trip_through_the_wire() {
    use AgentEventKind::*;
    let event = |kind, source: Option<&str>| {
        AgentEvent::new(AgentKind::Claude, kind, source.map(str::to_owned), None)
    };
    let apply = |state, kind, source| event(kind, source).apply(state);
    assert_eq!(
        apply(AgentState::Unknown, SessionStart, Some("startup")),
        AgentState::Idle
    );
    assert_eq!(
        apply(AgentState::Working, SessionStart, Some("compact")),
        AgentState::Working
    );
    assert_eq!(
        apply(AgentState::Idle, PromptSubmit, None),
        AgentState::Working
    );
    assert_eq!(
        apply(AgentState::Working, PermissionRequest, None),
        AgentState::Blocked
    );
    assert_eq!(
        apply(AgentState::Working, QuestionAsked, None),
        AgentState::Blocked
    );
    assert_eq!(
        apply(AgentState::Blocked, ToolStart, None),
        AgentState::Working
    );
    assert_eq!(
        apply(AgentState::Blocked, ToolComplete, None),
        AgentState::Working
    );
    assert_eq!(apply(AgentState::Working, Stop, None), AgentState::Idle);
    assert_eq!(
        apply(AgentState::Working, Interrupt, None),
        AgentState::Idle
    );
    assert_eq!(
        apply(AgentState::Blocked, StopFailure, None),
        AgentState::Idle
    );

    let encoded = event(SessionStart, Some("resume")).encode();
    assert_eq!(
        encoded,
        b"\x1b]777;notify;condr://agent;{\"v\":1,\"agent\":\"claude\",\"event\":\"session-start\",\"source\":\"resume\",\"session_id\":null}\x07"
            .to_vec()
    );
    let payload = &encoded[2..encoded.len() - 1];
    assert_eq!(
        AgentEvent::decode(payload),
        Some(event(SessionStart, Some("resume")))
    );
    assert_eq!(
        AgentEvent::decode(br#"777;notify;condr://agent;{"v":1,"agent":"codex","event":"stop"}"#),
        Some(AgentEvent::new(AgentKind::Codex, Stop, None, None))
    );
    assert_eq!(
        AgentEvent::decode(br#"777;notify;condr://agent;{"v":2,"agent":"codex","event":"stop"}"#),
        None
    );
    assert_eq!(
        AgentEvent::decode(br#"777;notify;condr://agent;{"v":1,"agent":"codex","event":"nap"}"#),
        None
    );
    assert_eq!(AgentEvent::decode(b"777;notify;other;{}"), None);
    for kind in AgentEventKind::ALL {
        assert_eq!(AgentEventKind::parse(kind.name()), Some(kind));
    }
}

#[test]
fn a_blocked_agent_says_what_it_waits_for_until_it_moves_on() {
    let mut detector = AgentDetector::new();
    let now = Instant::now();
    detector.observe_process(ProcessProbeResult::Agent(AgentKind::Claude), now);
    let blocked = |detail: Option<&str>, kind| {
        let mut event = AgentEvent::new(AgentKind::Claude, kind, None, Some("root".into()));
        event.detail = detail.map(str::to_owned);
        event
    };
    let snapshot = |state, blocked_on: Option<&str>| {
        AgentPublish::Snapshot(AgentSnapshot {
            session_id: Some("root".into()),
            kind: AgentKind::Claude,
            state,
            blocked_on: blocked_on.map(str::to_owned),
        })
    };
    assert_eq!(
        detector.observe_event(&blocked(
            Some("Bash: cargo test"),
            AgentEventKind::PermissionRequest
        )),
        snapshot(AgentState::Blocked, Some("Bash: cargo test"))
    );
    // A new question while still blocked is a change worth publishing.
    assert_eq!(
        detector.observe_event(&blocked(Some("Which DB?"), AgentEventKind::QuestionAsked)),
        snapshot(AgentState::Blocked, Some("Which DB?"))
    );
    assert_eq!(
        detector.observe_event(&blocked(Some("Which DB?"), AgentEventKind::QuestionAsked)),
        AgentPublish::Nothing
    );
    // A blocking event without a detail clears the stale one.
    assert_eq!(
        detector.observe_event(&blocked(None, AgentEventKind::PermissionRequest)),
        snapshot(AgentState::Blocked, None)
    );
    // Leaving Blocked never carries a detail, even if the event has one.
    assert_eq!(
        detector.observe_event(&blocked(Some("Bash"), AgentEventKind::ToolStart)),
        snapshot(AgentState::Working, None)
    );
    let bytes = blocked(Some("Bash: cargo test"), AgentEventKind::PermissionRequest).encode();
    assert_eq!(
        AgentEvent::decode(&bytes[2..bytes.len() - 1]).and_then(|event| event.detail),
        Some("Bash: cargo test".to_owned())
    );
    // A detail with a control character or over the bound is not accepted off the wire.
    let bytes = blocked(Some("a\x1bb"), AgentEventKind::PermissionRequest).encode();
    assert_eq!(AgentEvent::decode(&bytes[2..bytes.len() - 1]), None);
}

#[test]
fn events_only_count_for_the_agent_the_process_table_shows() {
    let mut detector = AgentDetector::new();
    let now = Instant::now();
    let stop = AgentEvent::new(AgentKind::Codex, AgentEventKind::Stop, None, None);
    // Before any probe, or over a bare shell, events do not seed an agent.
    assert_eq!(detector.observe_event(&stop), AgentPublish::Nothing);
    detector.observe_process(ProcessProbeResult::ShellOnly, now);
    assert_eq!(detector.observe_event(&stop), AgentPublish::Nothing);
    detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now);
    assert_eq!(
        detector.observe_event(&AgentEvent::new(
            AgentKind::Claude,
            AgentEventKind::Stop,
            None,
            None
        )),
        AgentPublish::Nothing
    );
    assert_eq!(
        detector.observe_event(&stop),
        AgentPublish::Snapshot(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Idle,
            blocked_on: None,
        })
    );
    assert_eq!(detector.observe_event(&stop), AgentPublish::Nothing);
    // A confirmed process probe does not reset the reported state.
    assert_eq!(
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now),
        AgentPublish::Nothing
    );
    // A restart is a new agent: back to Unknown, old state gone.
    detector.observe_process(ProcessProbeResult::ShellOnly, now);
    assert_eq!(
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now),
        AgentPublish::Snapshot(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Unknown,
            blocked_on: None,
        })
    );
}

#[test]
fn an_agent_behind_an_opaque_launcher_is_named_by_its_own_events() {
    let mut detector = AgentDetector::new();
    let now = Instant::now();
    // `mise exec -- claude`: something runs in front of the shell, nothing Condr can name.
    detector.observe_process(ProcessProbeResult::Unidentified, now);
    let start = AgentEvent::new(
        AgentKind::Claude,
        AgentEventKind::SessionStart,
        Some("startup".into()),
        None,
    );
    assert_eq!(
        detector.observe_event(&start),
        AgentPublish::Snapshot(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Claude,
            state: AgentState::Idle,
            blocked_on: None,
        })
    );
    assert_eq!(detector.agent(), Some(AgentKind::Claude));
    // The table keeps failing to name it; that is expected, not an exit.
    for _ in 0..10 {
        assert_eq!(
            detector.observe_process(ProcessProbeResult::Unidentified, now),
            AgentPublish::Nothing
        );
    }
    // The table naming it later confirms rather than restarts it.
    assert_eq!(
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Claude), now),
        AgentPublish::Nothing
    );
    // A bare shell still means it is gone.
    assert_eq!(
        detector.observe_process(ProcessProbeResult::ShellOnly, now),
        AgentPublish::Cleared
    );
}

#[test]
fn native_conversations_update_even_without_a_state_change_and_reset_on_exit() {
    let mut detector = AgentDetector::new();
    let now = Instant::now();
    detector.observe_process(ProcessProbeResult::Agent(AgentKind::Claude), now);
    let event = |id: &str, source: &str| {
        AgentEvent::new(
            AgentKind::Claude,
            AgentEventKind::SessionStart,
            Some(source.into()),
            Some(id.into()),
        )
    };
    for (id, source) in [
        ("first", "startup"),
        ("second", "clear"),
        ("third", "resume"),
    ] {
        let event = event(id, source);
        let bytes = event.encode();
        assert_eq!(
            AgentEvent::decode(&bytes[2..bytes.len() - 1]),
            Some(event.clone())
        );
        assert_eq!(
            detector.observe_event(&event),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Claude,
                state: AgentState::Idle,
                session_id: Some(id.into()),
                blocked_on: None,
            })
        );
    }
    // A late hook without an ID retains the current one; a forged sibling cannot replace it.
    assert_eq!(
        detector.observe_event(&AgentEvent::new(
            AgentKind::Codex,
            AgentEventKind::Stop,
            None,
            Some("wrong".into())
        )),
        AgentPublish::Nothing
    );
    assert_eq!(
        detector.observe_event(&AgentEvent::new(
            AgentKind::Claude,
            AgentEventKind::PromptSubmit,
            None,
            None
        )),
        AgentPublish::Snapshot(AgentSnapshot {
            kind: AgentKind::Claude,
            state: AgentState::Working,
            session_id: Some("third".into()),
            blocked_on: None,
        })
    );
    assert_eq!(
        detector.observe_process(ProcessProbeResult::ShellOnly, now),
        AgentPublish::Cleared
    );
    assert_eq!(
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Claude), now),
        AgentPublish::Snapshot(AgentSnapshot {
            kind: AgentKind::Claude,
            state: AgentState::Unknown,
            session_id: None,
            blocked_on: None,
        })
    );
    for id in ["--last", "../conversation", "a;exit"] {
        let bytes = event(id, "resume").encode();
        assert_eq!(AgentEvent::decode(&bytes[2..bytes.len() - 1]), None);
    }
}

#[test]
fn a_conversation_reference_round_trips_without_copying_it_to_new_panes() {
    use condr_core::{AgentResume, Session, SessionSnapshot, SplitDirection};
    for (kind, flag) in [
        (AgentKind::Claude, "--resume"),
        (AgentKind::Codex, "resume"),
        (AgentKind::OpenCode, "--session"),
        (AgentKind::Pi, "--session"),
        (AgentKind::Omp, "--session"),
        (AgentKind::Antigravity, "--conversation"),
        (AgentKind::Grok, "--resume"),
        (AgentKind::Cursor, "--resume"),
        (AgentKind::Kimi, "--session"),
    ] {
        let mut session = Session::new();
        session.create_workspace(std::env::temp_dir()).unwrap();
        let pane = session.workspaces()[0].tabs()[0]
            .focused_pane()
            .unwrap()
            .id();
        let resume = AgentResume {
            kind,
            session_id: "ses_123-abc".into(),
        };
        assert_eq!(resume.args(), [flag, "ses_123-abc"]);
        assert!(session.set_pane_agent_resume(pane, Some(resume.clone())));
        let other = session
            .split_pane(pane, SplitDirection::Horizontal, 0.5)
            .unwrap();
        let restored = Session::restore(
            SessionSnapshot::from_bytes(&session.snapshot().to_bytes().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(restored.pane(pane).unwrap().agent_resume(), Some(&resume));
        assert_eq!(restored.pane(other).unwrap().agent_resume(), None);
        assert!(!session.set_pane_agent_resume(
            pane,
            Some(AgentResume {
                kind,
                session_id: "--last".into()
            })
        ));
        assert_eq!(session.pane(pane).unwrap().agent_resume(), Some(&resume));
    }
}

#[test]
fn resume_seeding_never_overrides_a_new_session_and_any_process_exit_clears_it() {
    use condr_core::AgentResume;
    let mut detector = AgentDetector::new();
    let saved = AgentResume {
        kind: AgentKind::Codex,
        session_id: "saved".into(),
    };
    detector.restore(saved.clone());
    detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), Instant::now());
    assert_eq!(detector.resume(), Some(Some(saved)));
    detector.observe_event(&AgentEvent::new(
        AgentKind::Codex,
        AgentEventKind::SessionStart,
        Some("startup".into()),
        None,
    ));
    assert_eq!(
        detector.resume(),
        Some(None),
        "a new session must not recover the old ID"
    );
    detector.observe_event(&AgentEvent::new(
        AgentKind::Codex,
        AgentEventKind::Stop,
        None,
        Some("new".into()),
    ));
    detector.observe_process(ProcessProbeResult::ShellOnly, Instant::now());
    assert_eq!(
        detector.resume(),
        Some(None),
        "every observed exit cancels recovery"
    );
}
