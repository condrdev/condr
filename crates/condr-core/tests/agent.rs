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
            kind: AgentKind::Codex,
            state: AgentState::Unknown,
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
            kind: AgentKind::Claude,
            state: AgentState::Unknown,
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
        AgentEvent::new(AgentKind::Claude, kind, source.map(str::to_owned))
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
        b"\x1b]777;notify;condr://agent;{\"v\":1,\"agent\":\"claude\",\"event\":\"session-start\",\"source\":\"resume\"}\x07"
            .to_vec()
    );
    let payload = &encoded[2..encoded.len() - 1];
    assert_eq!(
        AgentEvent::decode(payload),
        Some(event(SessionStart, Some("resume")))
    );
    assert_eq!(
        AgentEvent::decode(br#"777;notify;condr://agent;{"v":1,"agent":"codex","event":"stop"}"#),
        Some(AgentEvent::new(AgentKind::Codex, Stop, None))
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
fn events_only_count_for_the_agent_the_process_table_shows() {
    let mut detector = AgentDetector::new();
    let now = Instant::now();
    let stop = AgentEvent::new(AgentKind::Codex, AgentEventKind::Stop, None);
    // Before any probe, or over a bare shell, events do not seed an agent.
    assert_eq!(detector.observe_event(&stop), AgentPublish::Nothing);
    detector.observe_process(ProcessProbeResult::ShellOnly, now);
    assert_eq!(detector.observe_event(&stop), AgentPublish::Nothing);
    detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now);
    assert_eq!(
        detector.observe_event(&AgentEvent::new(
            AgentKind::Claude,
            AgentEventKind::Stop,
            None
        )),
        AgentPublish::Nothing
    );
    assert_eq!(
        detector.observe_event(&stop),
        AgentPublish::Snapshot(AgentSnapshot {
            kind: AgentKind::Codex,
            state: AgentState::Idle,
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
            kind: AgentKind::Codex,
            state: AgentState::Unknown,
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
    );
    assert_eq!(
        detector.observe_event(&start),
        AgentPublish::Snapshot(AgentSnapshot {
            kind: AgentKind::Claude,
            state: AgentState::Idle,
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
