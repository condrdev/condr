use condr_core::protocol::{
    BootstrapBatch, BootstrapHeader, BootstrapRecord, PaneTerminalFrame, PaneTerminalSnapshot,
    RuntimeEpoch, ServerId, ServerMessage, SessionBootstrap, SessionId, TerminalFrameBatch,
    TerminalFrameChunk, encode_bootstrap_record, encode_pane_terminal_frame,
};
use condr_core::{
    AgentDisplayState, Session, TerminalCell, TerminalCellRun, TerminalColor,
    TerminalMouseTracking, TerminalSize, TerminalView, TerminalViewDelta, TerminalViewFrame,
};
use condr_server::Endpoint;
use gpui::{AssetSource as _, KeyDownEvent, Keystroke, Task};

use super::{
    ClientIo, CondrAssets, ConnectionStatus, FocusLeft, NextTab, OpenSettings, PreviousTab,
    ServerConnection, SidebarGlyph, SidebarIconTone, SplitDown, SplitRight,
    TerminalClipboardShortcut, TerminalVisualSlot, accepted_text_input, agent_sidebar_status,
    apply_terminal_frame_batch, assemble_terminal_frame_chunk, clear_pending_sizes_for_bootstrap,
    connect_to_server_with, enforce_terminal_chunk_reliable_fence, fixed_shortcut,
    lock_exclusively, merge_terminal_deltas, read_bootstrap_batches, reorder_connection,
    should_defer_to_character_input, single_instance_lock_path, terminal_chunk_identity_matches,
    terminal_clipboard_shortcut,
};

fn terminal_cell(text: &str) -> TerminalCell {
    TerminalCell {
        text: text.into(),
        foreground: TerminalColor::Named(0),
        background: TerminalColor::Named(0),
        flags: 0,
    }
}

#[test]
fn server_path_input_preserves_valid_whitespace() {
    assert_eq!(
        accepted_text_input("/srv/worktree ".into(), false),
        Some("/srv/worktree ".into())
    );
    assert_eq!(
        accepted_text_input("  branch-name  ".into(), true),
        Some("branch-name".into())
    );
    assert_eq!(accepted_text_input(" \t ".into(), false), None);
}

#[test]
fn server_connection_preserves_start_error_and_final_concurrent_probe() {
    let local = condr_server::ServerConfig::default().endpoint;
    let (_, failed) = connect_to_server_with(
        local.clone(),
        || {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid endpoint marker",
            ))
        },
        |_| Err::<(), String>("named pipe was not found".into()),
    );
    assert_eq!(
        failed.unwrap_err(),
        "Failed to start or discover local condr-server: invalid endpoint marker"
    );

    let (_, concurrent) = connect_to_server_with(
        local,
        || Err(std::io::Error::other("concurrent launcher won")),
        |_| Ok("connected concurrently"),
    );
    assert_eq!(concurrent.unwrap(), "connected concurrently");

    let remote = Endpoint::tcp("127.0.0.1:4242".parse().unwrap());
    let (connected_endpoint, connected) = connect_to_server_with(
        remote.clone(),
        || panic!("remote endpoints must not start the local server"),
        |_| Ok(()),
    );
    assert_eq!(connected_endpoint, remote);
    connected.unwrap();
}

fn terminal_view(revision: u64, text: &str) -> TerminalView {
    let cells = text
        .chars()
        .map(|character| terminal_cell(&character.to_string()))
        .collect::<Vec<_>>();
    TerminalView {
        revision,
        size: TerminalSize::new(1, u16::try_from(cells.len()).unwrap()),
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cells,
        cursor: None,
    }
}

fn pane_id() -> condr_core::PaneId {
    let mut session = Session::new();
    session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id()
}

#[test]
fn reordering_connections_moves_the_dragged_server_into_the_target_slot() {
    let endpoint = || Endpoint::Tcp("127.0.0.1:4242".parse().unwrap());
    let connection = |key: u64| ServerConnection::new(key, format!("server-{key}"), endpoint());
    let keys =
        |connections: &Vec<ServerConnection>| connections.iter().map(|c| c.key).collect::<Vec<_>>();

    let mut connections = vec![connection(1), connection(2), connection(3)];
    assert!(reorder_connection(&mut connections, 3, 1));
    assert_eq!(keys(&connections), [3, 1, 2]);

    assert!(reorder_connection(&mut connections, 3, 2));
    assert_eq!(keys(&connections), [1, 2, 3]);

    assert!(
        !reorder_connection(&mut connections, 2, 2),
        "dropping a server on itself should not change the order"
    );
    assert!(
        !reorder_connection(&mut connections, 9, 1),
        "an unknown dragged key should be ignored"
    );
    assert!(
        !reorder_connection(&mut connections, 1, 9),
        "an unknown target key should be ignored"
    );
    assert_eq!(keys(&connections), [1, 2, 3]);
}

#[test]
fn sidebar_status_visuals_follow_the_prototype_semantics() {
    let agent_cases = [
        (
            AgentDisplayState::Unknown,
            SidebarGlyph::Info,
            SidebarIconTone::Muted,
            "unknown",
        ),
        (
            AgentDisplayState::Idle,
            SidebarGlyph::Circle,
            SidebarIconTone::Muted,
            "idle",
        ),
        (
            AgentDisplayState::Working,
            SidebarGlyph::LoaderCircle,
            SidebarIconTone::Warning,
            "working",
        ),
        (
            AgentDisplayState::Blocked,
            SidebarGlyph::CircleAlert,
            SidebarIconTone::Danger,
            "blocked",
        ),
        (
            AgentDisplayState::Done,
            SidebarGlyph::CircleCheck,
            SidebarIconTone::Success,
            "done",
        ),
    ];
    for (state, glyph, tone, key) in agent_cases {
        let visual = agent_sidebar_status(state);
        assert_eq!((visual.glyph, visual.tone, visual.key), (glyph, tone, key));
    }
}

#[test]
fn condr_assets_include_the_prototype_agent_status_icons() {
    let assets = CondrAssets::new();
    for path in ["icons/circle.svg", "icons/circle-alert.svg"] {
        let bytes = assets
            .load(path)
            .unwrap()
            .expect("Condr status icon should be embedded");
        assert!(bytes.starts_with(b"<svg"));
    }
    let listed = assets.list("icons/circle").unwrap();
    assert!(
        listed
            .iter()
            .any(|path| path.as_ref() == "icons/circle.svg")
    );
    assert!(
        listed
            .iter()
            .any(|path| path.as_ref() == "icons/circle-alert.svg")
    );
}

#[test]
fn typed_subscription_rejection_requests_one_authoritative_bootstrap_then_resubscribes() {
    let mut connection = ServerConnection::new(
        1,
        "test".into(),
        Endpoint::tcp("127.0.0.1:9".parse().unwrap()),
    );
    connection.status = ConnectionStatus::Connected;
    connection.server_id = Some(ServerId(1));
    connection.runtime_epoch = Some(RuntimeEpoch(2));
    connection.session_id = Some(SessionId(3));
    connection.sequence = 7;
    connection.controlling = true;
    let (outgoing, outgoing_rx) = std::sync::mpsc::channel();
    connection.io = Some(ClientIo {
        outgoing,
        _incoming_task: Task::ready(()),
    });

    connection.subscribe();
    assert_eq!(
        outgoing_rx.recv().unwrap(),
        condr_core::protocol::ClientMessage::Subscribe {
            session_id: SessionId(3),
            after_sequence: 7,
        }
    );
    connection.bootstrap_resync_session_id = Some(SessionId(3));
    assert!(connection.recover_rejected_subscription(ServerId(1), SessionId(30)));
    assert_eq!(
        outgoing_rx.recv().unwrap(),
        condr_core::protocol::ClientMessage::SnapshotRequest {
            session_id: SessionId(30),
        }
    );
    assert!(!connection.recover_rejected_subscription(ServerId(1), SessionId(30)));
    assert!(matches!(
        outgoing_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));

    let application = connection.apply_bootstrap(SessionBootstrap {
        server_id: ServerId(1),
        runtime_epoch: RuntimeEpoch(2),
        session_id: SessionId(30),
        sequence: 11,
        snapshot: Session::new().snapshot(),
        terminals: Vec::new(),
        agents: Vec::new(),
        workspace_git: Vec::new(),
        zoomed_panes: Vec::new(),
    });
    assert!(application.resubscribe);
    assert!(application.reacquire_control);
    assert!(!connection.controlling);
    assert!(connection.bootstrap_resync_session_id.is_none());
    connection.send(condr_core::protocol::ClientMessage::AcquireControl {
        session_id: SessionId(30),
    });
    connection.subscribe();
    assert_eq!(
        outgoing_rx.recv().unwrap(),
        condr_core::protocol::ClientMessage::AcquireControl {
            session_id: SessionId(30),
        }
    );
    assert_eq!(
        outgoing_rx.recv().unwrap(),
        condr_core::protocol::ClientMessage::Subscribe {
            session_id: SessionId(30),
            after_sequence: 11,
        }
    );
}

#[test]
fn typed_snapshot_rejection_retargets_the_in_flight_resync() {
    let mut connection = ServerConnection::new(
        1,
        "test".into(),
        Endpoint::tcp("127.0.0.1:9".parse().unwrap()),
    );
    connection.status = ConnectionStatus::Connected;
    connection.server_id = Some(ServerId(1));
    connection.runtime_epoch = Some(RuntimeEpoch(2));
    connection.session_id = Some(SessionId(3));
    connection.sequence = 7;
    connection.controlling = true;
    connection.subscribed = true;
    connection.bootstrap_resync_session_id = Some(SessionId(3));
    let (outgoing, outgoing_rx) = std::sync::mpsc::channel();
    connection.io = Some(ClientIo {
        outgoing,
        _incoming_task: Task::ready(()),
    });

    assert!(connection.recover_rejected_snapshot(
        ServerId(1),
        SessionId(30),
        "unknown Session".into(),
    ));
    assert_eq!(
        outgoing_rx.recv().unwrap(),
        condr_core::protocol::ClientMessage::SnapshotRequest {
            session_id: SessionId(30),
        }
    );
    assert_eq!(connection.bootstrap_resync_session_id, Some(SessionId(30)));
    assert!(!connection.subscribed);
    assert!(!connection.subscription_pending);
    assert!(!connection.can_mutate());
    assert_eq!(connection.error.as_deref(), Some("unknown Session"));
}

#[test]
fn ordinary_runtime_bootstrap_keeps_the_existing_subscription_and_baseline() {
    let mut connection = ServerConnection::new(
        1,
        "test".into(),
        Endpoint::tcp("127.0.0.1:9".parse().unwrap()),
    );
    connection.status = ConnectionStatus::Connected;
    connection.server_id = Some(ServerId(1));
    connection.runtime_epoch = Some(RuntimeEpoch(2));
    connection.session_id = Some(SessionId(3));
    connection.sequence = 7;
    connection.controlling = true;
    connection.subscribed = true;
    connection.bootstrap_resync_session_id = Some(SessionId(3));

    let application = connection.apply_bootstrap(SessionBootstrap {
        server_id: ServerId(1),
        runtime_epoch: RuntimeEpoch(2),
        session_id: SessionId(3),
        sequence: 8,
        snapshot: Session::new().snapshot(),
        terminals: Vec::new(),
        agents: Vec::new(),
        workspace_git: Vec::new(),
        zoomed_panes: Vec::new(),
    });

    assert!(!application.resubscribe);
    assert!(!application.reacquire_control);
    assert!(connection.subscribed);
    assert!(connection.can_mutate());
}

#[test]
fn authoritative_bootstrap_releases_pending_resize_for_retry() {
    let pane_id = pane_id();
    let mut pending_sizes = std::collections::HashMap::from([
        ((1, pane_id), TerminalSize::new(40, 100)),
        ((2, pane_id), TerminalSize::new(20, 80)),
    ]);

    clear_pending_sizes_for_bootstrap(&mut pending_sizes, 1);

    assert!(!pending_sizes.contains_key(&(1, pane_id)));
    assert_eq!(
        pending_sizes.get(&(2, pane_id)),
        Some(&TerminalSize::new(20, 80))
    );
}

#[test]
fn terminal_frame_chunks_are_exposed_only_after_complete_reassembly() {
    let pane_id = pane_id();
    let expected = PaneTerminalFrame {
        pane_id,
        frame: TerminalViewFrame::Full(terminal_view(7, "chunked")),
    };
    let payload = encode_pane_terminal_frame(&expected).unwrap();
    let midpoint = payload.len() / 2;
    let chunks = [&payload[..midpoint], &payload[midpoint..]];
    let mut assembly = None;

    for (chunk_index, payload) in chunks.into_iter().enumerate() {
        let assembled = assemble_terminal_frame_chunk(
            &mut assembly,
            TerminalFrameChunk {
                server_id: ServerId(1),
                session_id: SessionId(1),
                pane_id,
                revision: 7,
                chunk_index: chunk_index as u32,
                chunk_count: 2,
                payload: payload.to_vec(),
            },
        )
        .unwrap();
        if chunk_index == 0 {
            assert!(assembled.is_none());
        } else {
            assert_eq!(assembled, Some(expected.clone()));
        }
    }
    assert!(assembly.is_none());
}

#[test]
fn terminal_chunk_identity_mismatch_aborts_the_in_progress_record() {
    let pane_id = pane_id();
    let mut assembly = None;
    assemble_terminal_frame_chunk(
        &mut assembly,
        TerminalFrameChunk {
            server_id: ServerId(1),
            session_id: SessionId(2),
            pane_id,
            revision: 7,
            chunk_index: 0,
            chunk_count: 2,
            payload: vec![1],
        },
    )
    .unwrap();

    let mismatched = TerminalFrameChunk {
        server_id: ServerId(9),
        session_id: SessionId(2),
        pane_id,
        revision: 7,
        chunk_index: 1,
        chunk_count: 2,
        payload: vec![2],
    };
    assert!(
        terminal_chunk_identity_matches(&mut assembly, &mismatched, ServerId(1), SessionId(2))
            .is_err()
    );
    assert!(assembly.is_none());
}

#[test]
fn reliable_message_is_a_terminal_chunk_protocol_fence() {
    let pane_id = pane_id();
    let mut assembly = None;
    assemble_terminal_frame_chunk(
        &mut assembly,
        TerminalFrameChunk {
            server_id: ServerId(1),
            session_id: SessionId(2),
            pane_id,
            revision: 7,
            chunk_index: 0,
            chunk_count: 2,
            payload: vec![1],
        },
    )
    .unwrap();

    let reliable = ServerMessage::ControlGranted {
        server_id: ServerId(1),
        session_id: SessionId(2),
    };
    assert!(enforce_terminal_chunk_reliable_fence(&mut assembly, &reliable).is_err());
    assert!(assembly.is_none());
}

#[test]
fn terminal_frame_batch_is_atomic_when_a_later_pane_has_a_gap() {
    let first_pane = pane_id();
    let second_pane = pane_id();
    let first_view = terminal_view(1, "a");
    let second_view = terminal_view(1, "b");
    let mut terminals = std::collections::HashMap::from([
        (
            first_pane,
            PaneTerminalSnapshot {
                pane_id: first_pane,
                view: first_view.clone(),
                exited: false,
            },
        ),
        (
            second_pane,
            PaneTerminalSnapshot {
                pane_id: second_pane,
                view: second_view.clone(),
                exited: false,
            },
        ),
    ]);
    let batch = vec![
        PaneTerminalFrame {
            pane_id: first_pane,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                base_revision: 1,
                revision: 2,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 0,
                    cells: vec![terminal_cell("x")],
                }],
            }),
        },
        PaneTerminalFrame {
            pane_id: second_pane,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                base_revision: 99,
                revision: 100,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 0,
                    cells: vec![terminal_cell("y")],
                }],
            }),
        },
    ];

    assert!(apply_terminal_frame_batch(&mut terminals, batch).is_err());
    assert_eq!(terminals[&first_pane].view, first_view);
    assert_eq!(terminals[&second_pane].view, second_view);
}

#[test]
fn incomplete_bootstrap_batches_never_produce_a_partial_snapshot() {
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
    let terminal = PaneTerminalSnapshot {
        pane_id,
        view: terminal_view(3, "restore"),
        exited: false,
    };
    let payload = encode_bootstrap_record(&BootstrapRecord::Terminal(terminal.clone())).unwrap();
    let midpoint = payload.len() / 2;
    let header = BootstrapHeader {
        server_id: ServerId(1),
        runtime_epoch: RuntimeEpoch(2),
        session_id: SessionId(1),
        sequence: 4,
        snapshot: session.snapshot(),
        batch_count: 2,
    };
    let batches = [
        BootstrapBatch {
            server_id: ServerId(1),
            session_id: SessionId(1),
            batch_index: 0,
            record_index: 0,
            chunk_index: 0,
            chunk_count: 2,
            payload: payload[..midpoint].to_vec(),
        },
        BootstrapBatch {
            server_id: ServerId(1),
            session_id: SessionId(1),
            batch_index: 1,
            record_index: 0,
            chunk_index: 1,
            chunk_count: 2,
            payload: payload[midpoint..].to_vec(),
        },
    ];
    let mut complete = Vec::new();
    for batch in &batches {
        condr_core::protocol::write_message(
            &mut complete,
            &ServerMessage::BootstrapBatch(batch.clone()),
        )
        .unwrap();
    }
    let assembled = read_bootstrap_batches(&mut complete.as_slice(), header.clone()).unwrap();
    assert_eq!(assembled.terminals, vec![terminal]);

    let mut incomplete = Vec::new();
    condr_core::protocol::write_message(
        &mut incomplete,
        &ServerMessage::BootstrapBatch(batches[0].clone()),
    )
    .unwrap();
    assert!(read_bootstrap_batches(&mut incomplete.as_slice(), header).is_err());
}

#[test]
fn terminal_shortcut_fallback_maps_only_fixed_chords() {
    let action = |keys: &str| fixed_shortcut(&Keystroke::parse(keys).unwrap());

    assert!(action("ctrl-tab").unwrap().as_any().is::<NextTab>());
    assert!(
        action("ctrl-shift-tab")
            .unwrap()
            .as_any()
            .is::<PreviousTab>()
    );
    assert!(action("alt-shift-=").unwrap().as_any().is::<SplitRight>());
    assert!(action("alt-shift--").unwrap().as_any().is::<SplitDown>());
    assert!(action("alt-+").unwrap().as_any().is::<SplitRight>());
    assert!(action("alt-_").unwrap().as_any().is::<SplitDown>());
    assert!(action("alt-left").unwrap().as_any().is::<FocusLeft>());
    // Settings must open even while a terminal owns the keystroke, on the platform's
    // own chord: Cmd+, on macOS, Ctrl+, elsewhere.
    assert!(action("secondary-,").unwrap().as_any().is::<OpenSettings>());
    if cfg!(target_os = "macos") {
        assert!(action("ctrl-,").is_none());
    } else {
        assert!(action("cmd-,").is_none());
    }
    assert!(action("ctrl-p").is_none());
}

#[test]
fn terminal_clipboard_shortcuts_preserve_terminal_control_keys() {
    let shortcut = |keys: &str, has_selection| {
        terminal_clipboard_shortcut(&Keystroke::parse(keys).unwrap(), has_selection)
    };

    assert_eq!(
        shortcut("ctrl-shift-c", false),
        Some(TerminalClipboardShortcut::Copy)
    );
    assert_eq!(
        shortcut("ctrl-shift-v", false),
        Some(TerminalClipboardShortcut::Paste)
    );
    assert_eq!(
        shortcut("ctrl-insert", false),
        Some(TerminalClipboardShortcut::Copy)
    );
    assert_eq!(
        shortcut("shift-insert", false),
        Some(TerminalClipboardShortcut::Paste)
    );
    assert_eq!(shortcut("ctrl-c", false), None);
    #[cfg(windows)]
    assert_eq!(
        shortcut("ctrl-c", true),
        Some(TerminalClipboardShortcut::Copy)
    );
    #[cfg(not(windows))]
    assert_eq!(shortcut("ctrl-c", true), None);
    #[cfg(target_os = "macos")]
    assert_eq!(
        shortcut("cmd-c", true),
        Some(TerminalClipboardShortcut::Copy)
    );
    #[cfg(not(target_os = "macos"))]
    assert_eq!(shortcut("cmd-c", true), None);
    assert_eq!(shortcut("ctrl-alt-v", false), None);

    #[cfg(windows)]
    assert_eq!(
        shortcut("ctrl-v", false),
        Some(TerminalClipboardShortcut::Paste)
    );
    #[cfg(not(windows))]
    assert_eq!(shortcut("ctrl-v", false), None);
}

#[test]
fn printable_character_input_preference_bypasses_terminal_key_encoding() {
    let event = KeyDownEvent {
        keystroke: Keystroke::parse("ctrl-alt-q->@").unwrap(),
        is_held: false,
        prefer_character_input: true,
    };
    assert!(should_defer_to_character_input(&event));

    let mut ordinary_modified_key = event.clone();
    ordinary_modified_key.prefer_character_input = false;
    assert!(!should_defer_to_character_input(&ordinary_modified_key));

    let mut non_printable = event;
    non_printable.keystroke.key_char = Some("\t".into());
    assert!(!should_defer_to_character_input(&non_printable));
}

#[test]
fn gui_visual_slot_composes_pending_deltas_into_one_signal() {
    let pane_id = pane_id();
    let slot = TerminalVisualSlot::default();
    let batch = |frame| TerminalFrameBatch {
        server_id: ServerId(1),
        session_id: SessionId(2),
        panes: vec![PaneTerminalFrame { pane_id, frame }],
    };
    let first = TerminalViewDelta {
        base_revision: 1,
        revision: 2,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: vec![terminal_cell("X")],
        }],
    };
    let second = TerminalViewDelta {
        base_revision: 2,
        revision: 3,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 2,
            cells: vec![terminal_cell("Y")],
        }],
    };

    assert_eq!(
        slot.publish(batch(TerminalViewFrame::Delta(first)))
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        slot.publish(batch(TerminalViewFrame::Delta(second)))
            .unwrap(),
        None
    );
    let mut view = terminal_view(1, "abcd");
    let pending = slot.take(0).unwrap();
    assert_eq!(pending.panes.len(), 1);
    view.apply_frame(pending.panes.into_iter().next().unwrap().frame)
        .unwrap();
    assert_eq!(view.revision, 3);
    assert_eq!(view.cells[1].text.as_str(), "X");
    assert_eq!(view.cells[2].text.as_str(), "Y");
}

#[test]
fn bootstrap_generation_discards_an_old_visual_signal() {
    let pane_id = pane_id();
    let slot = TerminalVisualSlot::default();
    let batch = |revision, text| TerminalFrameBatch {
        server_id: ServerId(1),
        session_id: SessionId(2),
        panes: vec![PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Full(terminal_view(revision, text)),
        }],
    };

    assert_eq!(slot.publish(batch(1, "old")).unwrap(), Some(0));
    slot.advance();
    assert_eq!(slot.publish(batch(2, "new")).unwrap(), Some(1));
    assert!(slot.take(0).is_none());
    let current = slot.take(1).unwrap();
    assert!(matches!(
        &current.panes[0].frame,
        TerminalViewFrame::Full(view) if view.revision == 2
    ));
}

#[test]
fn delta_composition_keeps_later_cell_values_and_latest_metadata() {
    let previous = TerminalViewDelta {
        base_revision: 4,
        revision: 5,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: vec![terminal_cell("A"), terminal_cell("B")],
        }],
    };
    let next = TerminalViewDelta {
        base_revision: 5,
        revision: 6,
        display_offset: 3,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 2,
            cells: vec![terminal_cell("C")],
        }],
    };

    let merged = merge_terminal_deltas(previous, next).unwrap();
    assert_eq!((merged.base_revision, merged.revision), (4, 6));
    assert_eq!(merged.display_offset, 3);
    assert_eq!(merged.runs.len(), 1);
    assert_eq!(merged.runs[0].start, 1);
    assert_eq!(merged.runs[0].cells[0].text.as_str(), "A");
    assert_eq!(merged.runs[0].cells[1].text.as_str(), "C");
}

#[cfg(feature = "test-support")]
mod visual;

#[test]
fn single_instance_lock_admits_one_holder_at_a_time() {
    let directory = std::env::temp_dir().join(format!(
        "condr-instance-lock-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let path = directory.join("condr-gui.lock");

    let held = lock_exclusively(&path).expect("the first GUI takes the lock");
    let refused = lock_exclusively(&path).expect_err("a second GUI must be refused");
    assert_eq!(refused.kind(), std::io::ErrorKind::WouldBlock);

    drop(held);
    lock_exclusively(&path).expect("the lock is free once the first GUI exits");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn single_instance_lock_lives_in_the_platform_runtime_directory() {
    assert_eq!(
        single_instance_lock_path(),
        condr_core::runtime_directory().map(|root| root.join("condr-gui.lock"))
    );
}
