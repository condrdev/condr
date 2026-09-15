use super::*;

#[test]
fn an_observed_terminal_exit_cannot_be_undone_by_its_final_hook_drain() {
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    state
        .session
        .create_workspace(std::env::temp_dir())
        .unwrap();
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let resume = condr_core::AgentResume {
        kind: condr_core::AgentKind::Codex,
        session_id: "saved".into(),
    };
    state.record_agent_resume(pane_id, Some(resume.clone()));
    assert_eq!(
        state.session.pane(pane_id).unwrap().agent_resume(),
        Some(&resume)
    );
    state.exited_terminals.insert(pane_id);
    state.record_agent_resume(pane_id, None);
    state.record_agent_resume(pane_id, Some(resume));
    assert_eq!(state.session.pane(pane_id).unwrap().agent_resume(), None);
}

#[test]
fn snapshot_paths_use_platform_state_and_endpoint_identity() {
    let socket = snapshot_path_for_endpoint(std::path::Path::new("condr.sock")).unwrap();
    let pipe = snapshot_path_for_endpoint(std::path::Path::new("condr.pipe")).unwrap();
    let state_directory = condr_core::state_directory().unwrap();

    assert_eq!(socket.parent(), Some(state_directory.as_path()));
    assert_eq!(pipe.parent(), Some(state_directory.as_path()));
    assert_eq!(
        socket.extension().and_then(|value| value.to_str()),
        Some("snapshot")
    );
    assert_ne!(socket, pipe);
}

#[cfg(unix)]
#[test]
fn local_endpoint_identity_normalizes_relative_paths() {
    let relative = std::path::Path::new("condr.sock");
    let absolute = std::path::absolute(relative).unwrap();

    assert_eq!(stable_endpoint_id(relative), stable_endpoint_id(&absolute));
}

/// The TCP listener is an extra door to the same Server: its address plays no part in
/// the Server identity or where the Snapshot lives.
#[test]
fn a_tcp_listener_does_not_change_the_server_identity_or_snapshot() {
    let socket = std::path::Path::new("condr.sock");
    let plain = ServerConfig::at_socket(socket);
    let listening = ServerConfig::at_socket(socket).with_listen("127.0.0.1:4242".parse().unwrap());

    assert_eq!(plain.snapshot_path(), listening.snapshot_path());
    assert!(
        ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap())
            .unwrap()
            .snapshot_path()
            .is_none()
    );
    assert_eq!(
        ServerConfig::ephemeral(socket)
            .with_snapshot_path("explicit.snapshot")
            .snapshot_path(),
        Some(std::path::Path::new("explicit.snapshot"))
    );
}

#[test]
fn only_cwd_inheriting_layout_commands_require_process_observation() {
    let mut session = Session::new();
    let workspace_id = session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let tab_id = session.active_workspace().unwrap().active_tab().id();
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();

    assert!(layout_command_needs_cwd_observation(
        &LayoutCommand::CreateTab {
            workspace_id,
            name: None,
            focus: true
        }
    ));
    assert!(layout_command_needs_cwd_observation(
        &LayoutCommand::SplitPane {
            focus: true,
            pane_id,
            direction: condr_core::SplitDirection::Horizontal,
        }
    ));
    assert!(!layout_command_needs_cwd_observation(
        &LayoutCommand::SetSplitRatios {
            tab_id,
            ratios: Vec::new(),
        }
    ));
    assert!(!layout_command_needs_cwd_observation(
        &LayoutCommand::FocusPane { pane_id }
    ));
}

#[test]
fn shutdown_cwd_uses_a_report_committed_while_the_reader_is_joining() {
    let before = PathBuf::from("before");
    let after = PathBuf::from("after");

    assert_eq!(
        shutdown_cwd((Some(before), 4), (Some(after.clone()), 5)),
        Some(after)
    );
}

#[test]
fn shutdown_cwd_keeps_the_platform_probe_priority_without_a_new_report() {
    let before = PathBuf::from("before");
    let after = PathBuf::from("after");
    let selected = shutdown_cwd((Some(before.clone()), 4), (Some(after.clone()), 4));

    #[cfg(windows)]
    assert_eq!(selected, Some(after));
    #[cfg(not(windows))]
    assert_eq!(selected, Some(before));
}

#[cfg(unix)]
#[test]
fn shutdown_cwd_rejects_a_non_utf8_tail_report() {
    use std::os::unix::ffi::OsStringExt as _;

    let before = PathBuf::from("before");
    let invalid = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));

    assert_eq!(
        shutdown_cwd((Some(before.clone()), 4), (Some(invalid), 5)),
        Some(before)
    );
}

fn terminal_test_view(revision: u64, text: &str) -> TerminalView {
    let cells = text
        .chars()
        .map(|character| condr_core::TerminalCell {
            text: character.to_string().into(),
            foreground: condr_core::TerminalColor::Named(0),
            background: condr_core::TerminalColor::Named(0),
            flags: 0,
            hyperlink: None,
        })
        .collect::<Vec<_>>();
    TerminalView {
        selection: None,
        revision,
        size: TerminalSize::new(1, u16::try_from(cells.len()).unwrap()),
        display_offset: 0,
        mouse_tracking: condr_core::TerminalMouseTracking::None,
        cells,
        cursor: None,
    }
}

fn latest_terminal_view(mut view: TerminalView) -> LatestTerminalView {
    let hyperlinks = TerminalHyperlinkBudget::new(&mut view);
    LatestTerminalView {
        view: Arc::new(view),
        hyperlinks,
        producer_frame: None,
    }
}

#[test]
fn bootstrap_retains_a_closing_terminal_without_inventing_an_exit() {
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    state
        .terminal_views
        .insert(pane_id, latest_terminal_view(terminal_test_view(1, "tail")));
    state.closing_terminals.insert(pane_id);
    state
        .terminal_titles
        .insert(pane_id, "still closing".into());
    state.pending_terminal_bells.insert(pane_id);

    let bootstrap = state.bootstrap();
    assert_eq!(bootstrap.terminals.len(), 1);
    assert!(!bootstrap.terminals[0].exited);
    assert_eq!(view_text(&bootstrap.terminals[0].view), "tail");
    assert_eq!(
        bootstrap.terminals[0].title.as_deref(),
        Some("still closing")
    );
    assert!(bootstrap.terminals[0].attention);

    state.exited_terminals.insert(pane_id);
    assert!(state.bootstrap().terminals[0].exited);
}

#[test]
fn terminal_notices_publish_title_changes_once_and_collapse_bells() {
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    state.active_controller = Some(1);
    state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let events = |state: &RuntimeState| {
        state
            .events
            .iter()
            .map(|event| event.event.clone())
            .collect::<Vec<_>>()
    };

    apply_terminal_notices(
        &mut state,
        pane_id,
        TerminalNoticeBatch {
            title: Some(Some("build".into())),
            bells: 7,
            ..TerminalNoticeBatch::default()
        },
    );
    assert_eq!(
        events(&state),
        vec![
            SessionEvent::TerminalTitleChanged {
                pane_id,
                title: Some("build".into()),
            },
            SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: true,
            },
        ]
    );
    assert_eq!(
        state.bootstrap().terminals.len(),
        0,
        "no runtime, so no terminal record"
    );

    // Re-reporting the same title is not a change; a reset is.
    apply_terminal_notices(
        &mut state,
        pane_id,
        TerminalNoticeBatch {
            title: Some(Some("build".into())),
            ..TerminalNoticeBatch::default()
        },
    );
    assert_eq!(events(&state).len(), 2);

    apply_terminal_notices(
        &mut state,
        pane_id,
        TerminalNoticeBatch {
            bells: 1_000,
            ..TerminalNoticeBatch::default()
        },
    );
    assert_eq!(events(&state).len(), 2, "pending bells stay coalesced");

    state.exited_terminals.insert(pane_id);
    state.record_terminal_focus(pane_id, true);
    assert_eq!(
        events(&state).last(),
        Some(&SessionEvent::TerminalAttentionChanged {
            pane_id,
            attention: false,
        }),
        "focusing an exited Pane still acknowledges its final bell"
    );
    state.exited_terminals.remove(&pane_id);
    apply_terminal_notices(
        &mut state,
        pane_id,
        TerminalNoticeBatch {
            bells: 1,
            ..TerminalNoticeBatch::default()
        },
    );
    assert_eq!(events(&state).len(), 3, "focused panes need no attention");
    state.record_terminal_focus(pane_id, false);
    apply_terminal_notices(
        &mut state,
        pane_id,
        TerminalNoticeBatch {
            bells: 1,
            ..TerminalNoticeBatch::default()
        },
    );
    assert_eq!(events(&state).len(), 4, "focus rearms the bell marker");

    state.clear_controller_terminal_state();
    state.active_controller = None;
    assert_eq!(
        events(&state)
            .into_iter()
            .filter(|event| matches!(event, SessionEvent::TerminalAttentionChanged { .. }))
            .collect::<Vec<_>>(),
        vec![
            SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: true,
            },
            SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: false,
            },
            SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: true,
            },
            SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: false,
            },
        ],
        "controller handoff replays every attention marker with a later clear"
    );
    apply_terminal_notices(
        &mut state,
        pane_id,
        TerminalNoticeBatch {
            bells: 1,
            ..TerminalNoticeBatch::default()
        },
    );
    assert_eq!(
        events(&state).len(),
        5,
        "BELs without an active controller do not leave stale attention"
    );

    state.clear_terminal_title(pane_id);
    assert_eq!(
        events(&state).last(),
        Some(&SessionEvent::TerminalTitleChanged {
            pane_id,
            title: None,
        })
    );
    state.clear_terminal_title(pane_id);
    assert_eq!(events(&state).len(), 6);

    // OSC 52 copies fan out to subscribed clients without entering the event history.
    let (writer, receiver) = ClientWriter::channel();
    state.subscribers.insert(
        1,
        ClientSubscriber {
            writer,
            terminal_baselines: std::collections::HashMap::new(),
            pending_terminals: std::collections::HashSet::new(),
            render_generation: 0,
            bootstrap_pending: false,
            deferred_reliable: std::collections::VecDeque::new(),
            deferred_clipboard: None,
        },
    );
    apply_terminal_notices(
        &mut state,
        pane_id,
        TerminalNoticeBatch {
            clipboard: Some("copied".into()),
            ..TerminalNoticeBatch::default()
        },
    );
    assert_eq!(events(&state).len(), 6);
    let Some(ClientWriteItem::Reliable(data)) = receiver.recv() else {
        panic!("clipboard broadcast must be a reliable frame");
    };
    assert_eq!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut data.as_slice()).unwrap(),
        ServerMessage::TerminalClipboard {
            pane_id,
            text: "copied".into(),
        }
    );
}

#[test]
fn terminal_frame_coalescing_keeps_the_latest_revision() {
    let (sender, updates) = mpsc::channel();
    sender.send(TerminalUpdate::View(2)).unwrap();
    sender.send(TerminalUpdate::View(3)).unwrap();

    assert_eq!(
        coalesce_terminal_update(TerminalUpdate::View(1), &updates, Instant::now()),
        TerminalUpdate::View(3)
    );
}

#[test]
fn terminal_exit_supersedes_queued_view_updates() {
    let (sender, updates) = mpsc::channel();
    sender.send(TerminalUpdate::View(2)).unwrap();
    sender.send(TerminalUpdate::Exited).unwrap();

    assert_eq!(
        coalesce_terminal_update(TerminalUpdate::View(1), &updates, Instant::now()),
        TerminalUpdate::Exited
    );
}

#[test]
fn client_terminal_baseline_advances_only_for_an_accepted_render() {
    let mut session = Session::new();
    session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let view = |revision| TerminalView {
        selection: None,
        revision,
        size: TerminalSize::new(1, 1),
        display_offset: 0,
        mouse_tracking: condr_core::TerminalMouseTracking::None,
        cells: Vec::new(),
        cursor: None,
    };
    let (writer, receiver) = ClientWriter::channel();
    let mut subscriber = ClientSubscriber {
        writer,
        terminal_baselines: std::collections::HashMap::new(),
        pending_terminals: std::collections::HashSet::new(),
        render_generation: 0,
        bootstrap_pending: false,
        deferred_reliable: std::collections::VecDeque::new(),
        deferred_clipboard: None,
    };

    subscriber
        .try_send_terminal_render(vec![vec![1]], vec![(pane_id, Arc::new(view(1)))])
        .unwrap();
    assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 1);
    assert!(matches!(
        subscriber.try_send_terminal_render(vec![vec![2]], vec![(pane_id, Arc::new(view(2)))]),
        Err(mpsc::TrySendError::Full(_))
    ));
    assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 1);

    assert_eq!(
        receiver.recv(),
        Some(ClientWriteItem::Render {
            data: vec![1],
            slot_drained: true,
        })
    );
    subscriber
        .try_send_terminal_render(vec![vec![3]], vec![(pane_id, Arc::new(view(3)))])
        .unwrap();
    assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 3);
}

#[test]
fn wire_projected_hyperlinks_are_committed_as_the_client_baseline() {
    let mut session = Session::new();
    session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let mut current = terminal_test_view(1, "x");
    current.cells[0].hyperlink = Some("x".repeat(8 * 1024 + 1).into());

    let prepared = prepare_terminal_render(TerminalRenderSnapshot {
        client_id: 7,
        generation: 1,
        server_id: ServerId(1),
        session_id: SessionId(1),
        panes: vec![TerminalRenderPaneSnapshot {
            pane_id,
            baseline: None,
            current: Arc::new(current.clone()),
            producer_frame: None,
        }],
    });
    let TerminalViewFrame::Full(projected) = &prepared.frames[0].frame else {
        panic!("a missing client baseline requires a full frame");
    };
    assert!(projected.cells[0].hyperlink.is_none());
    assert_eq!(prepared.baselines[0].1.as_ref(), projected);

    let (batches, baselines) = split_bootstrap_records(
        ServerId(1),
        SessionId(1),
        vec![BootstrapRecord::Terminal(PaneTerminalSnapshot {
            pane_id,
            view: current,
            exited: false,
            title: None,
            attention: false,
        })],
    )
    .unwrap();
    let payload = batches
        .into_iter()
        .flat_map(|batch| batch.payload)
        .collect::<Vec<_>>();
    let BootstrapRecord::Terminal(projected) =
        condr_core::protocol::decode_bootstrap_record(&payload).unwrap()
    else {
        panic!("the Bootstrap record must remain a Terminal");
    };
    assert!(projected.view.cells[0].hyperlink.is_none());
    assert_eq!(baselines[0].1.as_ref(), &projected.view);
}

#[test]
fn retained_terminal_hyperlinks_stay_bounded_across_deltas() {
    const URI_BYTES: usize = 8 * 1024;
    const LINK_COUNT: usize = 512;

    let mut session = Session::new();
    session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let linked_cell = |index: usize| {
        let mut cell = terminal_test_view(0, "x").cells.remove(0);
        cell.hyperlink = Some(format!("{index:04}:{}", "x".repeat(URI_BYTES - 5)).into());
        cell
    };
    let initial = terminal_test_view(0, &"x".repeat(LINK_COUNT + 1));
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    state.publish_terminal(pane_id, TerminalViewFrame::Full(initial));
    state.publish_terminal(
        pane_id,
        TerminalViewFrame::Delta(condr_core::TerminalViewDelta {
            selection: None,
            base_revision: 0,
            revision: 1,
            display_offset: 0,
            mouse_tracking: condr_core::TerminalMouseTracking::None,
            cursor: None,
            runs: vec![condr_core::TerminalCellRun {
                start: 1,
                cells: (1..=LINK_COUNT).map(linked_cell).collect(),
            }],
        }),
    );
    state.publish_terminal(
        pane_id,
        TerminalViewFrame::Delta(condr_core::TerminalViewDelta {
            selection: None,
            base_revision: 1,
            revision: 2,
            display_offset: 0,
            mouse_tracking: condr_core::TerminalMouseTracking::None,
            cursor: None,
            runs: vec![condr_core::TerminalCellRun {
                start: 0,
                cells: vec![linked_cell(0)],
            }],
        }),
    );

    let latest = &state.terminal_views[&pane_id];
    let hyperlinks = latest
        .view
        .cells
        .iter()
        .filter_map(|cell| cell.hyperlink.as_ref())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        hyperlinks.iter().map(|uri| uri.len()).sum::<usize>(),
        4 * 1024 * 1024
    );
    assert!(latest.view.cells[0].hyperlink.is_none());
    assert!(latest.view.cells[LINK_COUNT].hyperlink.is_some());
    assert!(matches!(
        &latest.producer_frame,
        Some(TerminalViewFrame::Delta(delta))
            if delta.runs.iter().any(|run| {
                run.start == 0
                    && run.cells.len() == 1
                    && run.cells[0].hyperlink.is_none()
            })
    ));

    state.publish_terminal(
        pane_id,
        TerminalViewFrame::Delta(condr_core::TerminalViewDelta {
            selection: None,
            base_revision: 2,
            revision: 3,
            display_offset: 0,
            mouse_tracking: condr_core::TerminalMouseTracking::None,
            cursor: None,
            runs: vec![
                condr_core::TerminalCellRun {
                    start: 0,
                    cells: vec![linked_cell(0)],
                },
                condr_core::TerminalCellRun {
                    start: LINK_COUNT as u32,
                    cells: vec![terminal_test_view(0, "x").cells.remove(0)],
                },
            ],
        }),
    );
    let latest = &state.terminal_views[&pane_id];
    assert!(latest.view.cells[0].hyperlink.is_some());
    assert!(latest.view.cells[LINK_COUNT].hyperlink.is_none());
}

#[test]
fn bootstrap_fence_keeps_an_unsent_clipboard_copy_and_lets_newer_copies_replace_it() {
    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let clipboard_text = |item| {
        let Some(ClientWriteItem::Reliable(frame)) = item else {
            panic!("clipboard must be a reliable frame");
        };
        match condr_core::protocol::read_message::<_, ServerMessage>(&mut frame.as_slice()).unwrap()
        {
            ServerMessage::TerminalClipboard { text, .. } => text,
            other => panic!("expected clipboard, got {other:?}"),
        }
    };
    let subscriber = |writer| ClientSubscriber {
        writer,
        terminal_baselines: std::collections::HashMap::new(),
        pending_terminals: std::collections::HashSet::new(),
        render_generation: 0,
        bootstrap_pending: false,
        deferred_reliable: std::collections::VecDeque::new(),
        deferred_clipboard: None,
    };

    // A copy queued before the fence, with no newer copy: it follows the Bootstrap intact.
    let (writer, receiver) = ClientWriter::channel();
    state.subscribers.insert(7, subscriber(writer));
    state.broadcast_clipboard(pane_id, "before".into());
    state.begin_bootstrap(7);
    let framed = frame_bootstrap_messages(state.capture_bootstrap().materialize()).unwrap();
    assert!(state.finish_bootstrap(7, framed).is_ok());
    assert!(matches!(
        receiver.recv(),
        Some(ClientWriteItem::ReliableBatch(_))
    ));
    assert_eq!(clipboard_text(receiver.recv()), "before");

    // A copy during the fence replaces the older unsent one; only the latest goes out.
    let (writer, receiver) = ClientWriter::channel();
    state.subscribers.insert(8, subscriber(writer));
    state.broadcast_clipboard(pane_id, "old".into());
    state.begin_bootstrap(8);
    state.broadcast_clipboard(pane_id, "newer".into());
    let framed = frame_bootstrap_messages(state.capture_bootstrap().materialize()).unwrap();
    assert!(state.finish_bootstrap(8, framed).is_ok());
    assert!(matches!(
        receiver.recv(),
        Some(ClientWriteItem::ReliableBatch(_))
    ));
    assert_eq!(clipboard_text(receiver.recv()), "newer");
    drop(state);
    assert!(receiver.recv().is_none());
}

#[test]
fn bootstrap_fence_queues_concurrent_events_after_the_complete_bootstrap() {
    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let capture = state.capture_bootstrap();
    let (writer, receiver) = ClientWriter::channel();
    state.subscribers.insert(
        7,
        ClientSubscriber {
            writer,
            terminal_baselines: std::collections::HashMap::new(),
            pending_terminals: std::collections::HashSet::new(),
            render_generation: 0,
            bootstrap_pending: false,
            deferred_reliable: std::collections::VecDeque::new(),
            deferred_clipboard: None,
        },
    );

    state.begin_bootstrap(7);
    let event = state.layout_changed_event();
    state.publish_background(event);
    state.broadcast_clipboard(pane_id, "during bootstrap".into());
    assert!(state.terminal_render_snapshot(7).is_none());
    assert_eq!(state.subscribers[&7].deferred_reliable.len(), 1);
    assert!(state.subscribers[&7].deferred_clipboard.is_some());

    let framed = frame_bootstrap_messages(capture.materialize()).unwrap();
    assert!(state.finish_bootstrap(7, framed).is_ok());
    let Some(ClientWriteItem::ReliableBatch(bootstrap_frames)) = receiver.recv() else {
        panic!("complete Bootstrap must be the first reliable item");
    };
    assert!(!bootstrap_frames.is_empty());
    let Some(ClientWriteItem::Reliable(event_frame)) = receiver.recv() else {
        panic!("event raised during Bootstrap must follow its batch");
    };
    let event = condr_core::protocol::read_message::<_, ServerMessage>(&mut event_frame.as_slice())
        .unwrap();
    assert!(matches!(
        event,
        ServerMessage::Event {
            event: SessionEvent::LayoutChanged { .. },
            ..
        }
    ));
    let Some(ClientWriteItem::Reliable(clipboard_frame)) = receiver.recv() else {
        panic!("live clipboard state must follow the replacement Bootstrap");
    };
    assert_eq!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut clipboard_frame.as_slice())
            .unwrap(),
        ServerMessage::TerminalClipboard {
            pane_id,
            text: "during bootstrap".into(),
        }
    );
    assert!(!state.subscribers[&7].bootstrap_pending);
}

#[test]
fn successful_resubscribe_preserves_the_committed_terminal_baseline() {
    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let baseline = Arc::new(terminal_test_view(7, "baseline"));
    let (writer, _receiver) = ClientWriter::channel();
    state.subscribers.insert(
        7,
        ClientSubscriber {
            writer,
            terminal_baselines: std::collections::HashMap::from([(pane_id, Arc::clone(&baseline))]),
            pending_terminals: std::collections::HashSet::new(),
            render_generation: 11,
            bootstrap_pending: false,
            deferred_reliable: std::collections::VecDeque::new(),
            deferred_clipboard: None,
        },
    );
    let (replacement_writer, _replacement_receiver) = ClientWriter::channel();

    assert!(!state.ensure_subscriber(7, replacement_writer));

    let subscriber = &state.subscribers[&7];
    assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 7);
    assert!(subscriber.pending_terminals.is_empty());
    assert_eq!(subscriber.render_generation, 11);
}

#[test]
fn full_render_slot_regenerates_the_latest_tail_after_drain() {
    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let initial = Arc::new(terminal_test_view(1, "old"));
    let latest = terminal_test_view(3, "oXd");
    state
        .terminal_views
        .insert(pane_id, latest_terminal_view(latest.clone()));
    let latest = Arc::clone(&state.terminal_views[&pane_id].view);
    let (writer, receiver) = ClientWriter::channel();
    writer.try_send_render(vec![vec![0]]).unwrap();
    state.subscribers.insert(
        7,
        ClientSubscriber {
            writer,
            terminal_baselines: std::collections::HashMap::from([(pane_id, Arc::clone(&initial))]),
            pending_terminals: std::collections::HashSet::from([pane_id]),
            render_generation: 1,
            bootstrap_pending: false,
            deferred_reliable: std::collections::VecDeque::new(),
            deferred_clipboard: None,
        },
    );
    let state = Arc::new(Mutex::new(state));

    flush_terminal_render(&state, 7);
    {
        let state = state.lock().unwrap();
        let subscriber = &state.subscribers[&7];
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 1);
        assert!(subscriber.pending_terminals.contains(&pane_id));
    }

    assert_eq!(
        receiver.recv(),
        Some(ClientWriteItem::Render {
            data: vec![0],
            slot_drained: true,
        })
    );
    flush_terminal_render(&state, 7);
    let Some(ClientWriteItem::Render { data, .. }) = receiver.recv() else {
        panic!("latest terminal tail was not regenerated");
    };
    let message: ServerMessage = condr_core::protocol::read_message(&mut data.as_slice()).unwrap();
    assert!(matches!(
        message,
        ServerMessage::TerminalFrame(TerminalFrameBatch { panes, .. })
            if panes.len() == 1
                && matches!(
                    &panes[0].frame,
                    TerminalViewFrame::Delta(delta)
                        if delta.base_revision == 1 && delta.revision == 3
                )
    ));
    let state = state.lock().unwrap();
    let subscriber = &state.subscribers[&7];
    assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 3);
    assert!(Arc::ptr_eq(
        &subscriber.terminal_baselines[&pane_id],
        &latest
    ));
    assert!(!subscriber.pending_terminals.contains(&pane_id));
}

#[test]
fn terminal_batches_are_split_before_the_protocol_limit() {
    let mut session = Session::new();
    session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let large_view = |revision| TerminalView {
        selection: None,
        revision,
        size: TerminalSize::new(1, 1),
        display_offset: 0,
        mouse_tracking: condr_core::TerminalMouseTracking::None,
        cells: vec![condr_core::TerminalCell {
            text: "x".repeat(MAX_FRAME_SIZE / 2 + 1024).into(),
            foreground: condr_core::TerminalColor::Named(0),
            background: condr_core::TerminalColor::Named(0),
            flags: 0,
            hyperlink: None,
        }],
        cursor: None,
    };
    let frames = frame_terminal_batches(
        ServerId(1),
        SessionId(1),
        vec![
            PaneTerminalFrame {
                pane_id,
                frame: TerminalViewFrame::Full(large_view(1)),
            },
            PaneTerminalFrame {
                pane_id,
                frame: TerminalViewFrame::Full(large_view(2)),
            },
        ],
    )
    .unwrap();
    assert_eq!(frames.len(), 2);
    for (revision, frame) in [1, 2].into_iter().zip(frames) {
        assert!(frame.len() <= MAX_FRAME_SIZE + size_of::<u32>());
        let mut frame = frame.as_slice();
        let message: ServerMessage = condr_core::protocol::read_message(&mut frame).unwrap();
        assert!(matches!(
            message,
            ServerMessage::TerminalFrame(TerminalFrameBatch { panes, .. })
                if panes.len() == 1
                    && matches!(
                        &panes[0].frame,
                        TerminalViewFrame::Full(view) if view.revision == revision
                    )
        ));
        assert!(frame.is_empty());
    }
}

#[test]
fn oversized_terminal_frame_is_transported_as_ordered_chunks() {
    let mut session = Session::new();
    session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let expected = PaneTerminalFrame {
        pane_id,
        frame: TerminalViewFrame::Full(TerminalView {
            selection: None,
            revision: 9,
            size: TerminalSize::new(1, 1),
            display_offset: 0,
            mouse_tracking: condr_core::TerminalMouseTracking::None,
            cells: vec![condr_core::TerminalCell {
                text: "x".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024).into(),
                foreground: condr_core::TerminalColor::Named(0),
                background: condr_core::TerminalColor::Named(0),
                flags: 0,
                hyperlink: None,
            }],
            cursor: None,
        }),
    };
    let frames = frame_terminal_batches(ServerId(1), SessionId(1), vec![expected.clone()])
        .expect("oversized terminal frame should be chunked");
    let mut payload = Vec::new();
    let mut chunks = 0;
    for frame in frames {
        assert!(frame.len() <= MAX_FRAME_SIZE + size_of::<u32>());
        let mut frame = frame.as_slice();
        let message: ServerMessage = condr_core::protocol::read_message(&mut frame).unwrap();
        let ServerMessage::TerminalFrameChunk(chunk) = message else {
            panic!("oversized terminal frame should use chunk messages");
        };
        assert_eq!(chunk.pane_id, pane_id);
        assert_eq!(chunk.revision, 9);
        assert_eq!(chunk.chunk_index, chunks);
        chunks += 1;
        assert_eq!(chunk.chunk_count, 2);
        payload.extend(chunk.payload);
        assert!(frame.is_empty());
    }
    assert_eq!(chunks, 2);
    assert_eq!(
        condr_core::protocol::decode_pane_terminal_frame(&payload).unwrap(),
        expected
    );
}

#[test]
fn bootstrap_dynamic_records_are_split_and_reassembled() {
    let mut session = Session::new();
    session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let first_pane = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let second_pane = session
        .split_pane(first_pane, condr_core::SplitDirection::Horizontal, 0.5)
        .unwrap();
    let large_view = |revision| TerminalView {
        selection: None,
        revision,
        size: TerminalSize::new(1, 1),
        display_offset: 0,
        mouse_tracking: condr_core::TerminalMouseTracking::None,
        cells: vec![condr_core::TerminalCell {
            text: "x".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024).into(),
            foreground: condr_core::TerminalColor::Named(0),
            background: condr_core::TerminalColor::Named(0),
            flags: 0,
            hyperlink: None,
        }],
        cursor: None,
    };
    let expected = SessionBootstrap {
        settings: Default::default(),
        server_id: ServerId(1),
        runtime_epoch: RuntimeEpoch(2),
        session_id: SessionId(1),
        sequence: 0,
        snapshot: session.snapshot(),
        terminals: vec![
            PaneTerminalSnapshot {
                pane_id: first_pane,
                view: large_view(1),
                exited: false,
                title: None,
                attention: false,
            },
            PaneTerminalSnapshot {
                pane_id: second_pane,
                view: large_view(2),
                exited: true,
                title: None,
                attention: false,
            },
        ],
        agents: Vec::new(),
        workspace_git: vec![WorkspaceGitSnapshot {
            workspace_id: session.active_workspace_id().unwrap(),
            branch: Some("b".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024)),
            linked_worktree: false,
            upstream: None,
            changes: Default::default(),
        }],
        zoomed_panes: vec![first_pane],
    };
    let frames = frame_bootstrap_messages(expected.clone()).unwrap().frames;

    assert!(frames.len() >= 7, "three large records should be chunked");
    assert!(frames.iter().all(|frame| frame.len() <= MAX_FRAME_SIZE + 4));
    let header: ServerMessage =
        condr_core::protocol::read_message(&mut frames[0].as_slice()).unwrap();
    let ServerMessage::Bootstrap(header) = header else {
        panic!("first frame should be a Bootstrap header");
    };
    assert_eq!(header.batch_count as usize, frames.len() - 1);
    assert_eq!(header.snapshot, expected.snapshot);
    let mut assembler = BootstrapAssembler::new(header).unwrap();
    for frame in &frames[1..] {
        let message: ServerMessage =
            condr_core::protocol::read_message(&mut frame.as_slice()).unwrap();
        let ServerMessage::BootstrapBatch(batch) = message else {
            panic!("Bootstrap payload should contain only batch frames");
        };
        assembler.push(batch).unwrap();
    }
    assert_eq!(assembler.finish().unwrap(), expected);
}

#[test]
fn untransportable_durable_mutations_leave_the_session_unchanged() {
    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    let workspace_id = state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let before = state.session.snapshot();

    let error = apply_layout_command(
        &mut state,
        LayoutCommand::RenameWorkspace {
            workspace_id,
            name: "x".repeat(MAX_PERSISTED_SNAPSHOT_BYTES),
        },
    )
    .err()
    .expect("oversized durable mutation should fail");
    assert!(error.contains("Server limit"));
    assert_eq!(state.session.snapshot(), before);

    assert!(!state.record_terminal_cwds([(
        pane_id,
        PathBuf::from("x".repeat(MAX_PERSISTED_SNAPSHOT_BYTES))
    )]));
    assert_eq!(state.session.snapshot(), before);
}

#[cfg(unix)]
#[test]
fn non_utf8_terminal_cwd_does_not_poison_the_durable_snapshot() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let before = state.session.snapshot();
    let cwd = PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));

    assert!(!state.record_terminal_cwds([(pane_id, cwd)]));
    assert_eq!(state.session.snapshot(), before);
    assert!(state.session.snapshot().to_bytes().is_ok());
}

#[cfg(unix)]
#[test]
fn invalid_terminal_cwd_does_not_discard_valid_sibling_observations() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    state
        .session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    let first_pane = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let second_pane = state
        .session
        .split_pane(first_pane, condr_core::SplitDirection::Horizontal, 0.5)
        .unwrap();
    let first_before = state
        .session
        .pane(first_pane)
        .and_then(|pane| pane.cwd())
        .map(PathBuf::from);
    let valid = std::env::temp_dir().join("condr-valid-cwd");
    let invalid = PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));

    assert!(state.record_terminal_cwds([(first_pane, invalid), (second_pane, valid.clone()),]));
    assert_eq!(
        state
            .session
            .pane(first_pane)
            .and_then(|pane| pane.cwd())
            .map(PathBuf::from),
        first_before
    );
    assert_eq!(
        state.session.pane(second_pane).and_then(|pane| pane.cwd()),
        Some(valid.as_path())
    );
    assert!(state.session.snapshot().to_bytes().is_ok());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stale_cwd_observation_cannot_update_a_replaced_terminal() {
    let directory = std::env::temp_dir().join(format!(
        "condr-server-stale-cwd-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let initial = directory.join("initial");
    let stale = directory.join("stale");
    std::fs::create_dir_all(&initial).unwrap();
    std::fs::create_dir_all(&stale).unwrap();

    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    state
        .session
        .create_workspace(initial.clone())
        .expect("Workspace capacity");
    let pane_id = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    let mut runtime =
        TerminalRuntime::spawn_shell(&initial, TerminalSize::new(24, 80), None, None).unwrap();
    let updates = runtime.take_updates().unwrap();
    let (_, stale_instance, ..) = state.install_terminal(pane_id, runtime, updates);
    state
        .terminal_instances
        .insert(pane_id, stale_instance.wrapping_add(1));

    assert!(!state.record_terminal_cwd_observations(vec![(pane_id, stale_instance, Some(stale),)]));
    assert_eq!(
        state.session.pane(pane_id).and_then(|pane| pane.cwd()),
        Some(initial.as_path())
    );

    state.terminal_instances.insert(pane_id, stale_instance);
    state.exited_terminals.insert(pane_id);
    assert!(!state.record_terminal_cwd_observations(vec![(
        pane_id,
        stale_instance,
        Some(directory.join("exited-stale")),
    )]));
    assert_eq!(
        state.session.pane(pane_id).and_then(|pane| pane.cwd()),
        Some(initial.as_path())
    );

    drop(state);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn load_shell_reads_the_nested_table() {
    let path = std::env::temp_dir().join(format!(
        "condr-load-shell-{}-{}.toml",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::write(
        &path,
        "# keep me\n[client]\nappearance = \"dark\"\n\n[server]\n\n[server.terminal]\nshell = \"nu\"\n",
    )
    .unwrap();
    assert_eq!(load_shell(Some(&path)), "nu");
    assert_eq!(load_shell(None), "");
    let _ = std::fs::remove_file(path);
}
