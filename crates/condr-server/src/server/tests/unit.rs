use super::*;

#[test]
fn snapshot_paths_preserve_the_complete_local_endpoint_name() {
    let socket = snapshot_path_for_endpoint(&Endpoint::local("condr.sock"));
    let pipe = snapshot_path_for_endpoint(&Endpoint::local("condr.pipe"));

    assert_eq!(socket, PathBuf::from("condr.sock.snapshot"));
    assert_eq!(pipe, PathBuf::from("condr.pipe.snapshot"));
    assert_ne!(socket, pipe);
}

#[test]
fn an_os_assigned_tcp_port_is_ephemeral_without_an_explicit_snapshot() {
    let endpoint = Endpoint::tcp("127.0.0.1:0".parse().unwrap());

    assert!(
        ServerConfig::new(endpoint.clone())
            .snapshot_path()
            .is_none()
    );
    assert_eq!(
        ServerConfig::new(endpoint)
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
        .id();

    assert!(layout_command_needs_cwd_observation(
        &LayoutCommand::CreateTab { workspace_id }
    ));
    assert!(layout_command_needs_cwd_observation(
        &LayoutCommand::SplitPane {
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
        })
        .collect::<Vec<_>>();
    TerminalView {
        revision,
        size: TerminalSize::new(1, u16::try_from(cells.len()).unwrap()),
        display_offset: 0,
        mouse_tracking: condr_core::TerminalMouseTracking::None,
        cells,
        cursor: None,
    }
}

#[test]
fn bootstrap_retains_a_closing_terminal_without_inventing_an_exit() {
    let mut state = RuntimeState::new(&test_endpoint());
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
        .id();
    state.terminal_views.insert(
        pane_id,
        LatestTerminalView {
            view: Arc::new(terminal_test_view(1, "tail")),
            producer_frame: None,
        },
    );
    state.closing_terminals.insert(pane_id);

    let bootstrap = state.bootstrap();
    assert_eq!(bootstrap.terminals.len(), 1);
    assert!(!bootstrap.terminals[0].exited);
    assert_eq!(view_text(&bootstrap.terminals[0].view), "tail");

    state.exited_terminals.insert(pane_id);
    assert!(state.bootstrap().terminals[0].exited);
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
        .id();
    let view = |revision| TerminalView {
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
fn bootstrap_fence_queues_concurrent_events_after_the_complete_bootstrap() {
    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(&endpoint);
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
        },
    );

    state.begin_bootstrap(7);
    state.publish_background(SessionEvent::LayoutChanged);
    assert!(state.terminal_render_snapshot(7).is_none());
    assert_eq!(state.subscribers[&7].deferred_reliable.len(), 1);

    let framed = frame_bootstrap_messages(capture.materialize()).unwrap();
    assert!(!state.finish_bootstrap(7, framed));
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
            event: SessionEvent::LayoutChanged,
            ..
        }
    ));
    assert!(!state.subscribers[&7].bootstrap_pending);
}

#[test]
fn successful_resubscribe_preserves_the_committed_terminal_baseline() {
    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(&endpoint);
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
    let mut state = RuntimeState::new(&endpoint);
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
        .id();
    let initial = Arc::new(terminal_test_view(1, "old"));
    let latest = Arc::new(terminal_test_view(3, "oXd"));
    state.terminal_views.insert(
        pane_id,
        LatestTerminalView {
            view: Arc::clone(&latest),
            producer_frame: None,
        },
    );
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
        .id();
    let large_view = |revision| TerminalView {
        revision,
        size: TerminalSize::new(1, 1),
        display_offset: 0,
        mouse_tracking: condr_core::TerminalMouseTracking::None,
        cells: vec![condr_core::TerminalCell {
            text: "x".repeat(MAX_FRAME_SIZE / 2 + 1024).into(),
            foreground: condr_core::TerminalColor::Named(0),
            background: condr_core::TerminalColor::Named(0),
            flags: 0,
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
        .id();
    let expected = PaneTerminalFrame {
        pane_id,
        frame: TerminalViewFrame::Full(TerminalView {
            revision: 9,
            size: TerminalSize::new(1, 1),
            display_offset: 0,
            mouse_tracking: condr_core::TerminalMouseTracking::None,
            cells: vec![condr_core::TerminalCell {
                text: "x".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024).into(),
                foreground: condr_core::TerminalColor::Named(0),
                background: condr_core::TerminalColor::Named(0),
                flags: 0,
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
        .id();
    let second_pane = session
        .split_pane(first_pane, condr_core::SplitDirection::Horizontal, 0.5)
        .unwrap();
    let large_view = |revision| TerminalView {
        revision,
        size: TerminalSize::new(1, 1),
        display_offset: 0,
        mouse_tracking: condr_core::TerminalMouseTracking::None,
        cells: vec![condr_core::TerminalCell {
            text: "x".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024).into(),
            foreground: condr_core::TerminalColor::Named(0),
            background: condr_core::TerminalColor::Named(0),
            flags: 0,
        }],
        cursor: None,
    };
    let expected = SessionBootstrap {
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
            },
            PaneTerminalSnapshot {
                pane_id: second_pane,
                view: large_view(2),
                exited: true,
            },
        ],
        agents: Vec::new(),
        workspace_git: vec![WorkspaceGitSnapshot {
            workspace_id: session.active_workspace_id().unwrap(),
            branch: Some("b".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024)),
            linked_worktree: false,
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
    let mut state = RuntimeState::new(&endpoint);
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
    let mut state = RuntimeState::new(&endpoint);
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
    let mut state = RuntimeState::new(&endpoint);
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
    let mut state = RuntimeState::new(&endpoint);
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
        .id();
    let mut runtime = TerminalRuntime::spawn_shell(&initial, TerminalSize::new(24, 80)).unwrap();
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
