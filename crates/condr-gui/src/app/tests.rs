use super::terminal_input::{
    clipboard_image_format, is_image_paste_gesture, next_hovered_link, terminal_key_for,
};
use crate::terminal_element::HoveredTerminalLink;
use condr_core::TerminalKey;
use condr_core::protocol::{
    BootstrapBatch, BootstrapHeader, BootstrapRecord, PaneTerminalFrame, PaneTerminalSnapshot,
    RuntimeEpoch, ServerId, ServerMessage, SessionBootstrap, SessionEvent, SessionId,
    TerminalFrameBatch, TerminalFrameChunk, encode_bootstrap_record, encode_pane_terminal_frame,
};
use condr_core::{
    AgentDisplayState, Session, TerminalCell, TerminalCellRun, TerminalColor,
    TerminalHyperlinkBudget, TerminalMouseTracking, TerminalSize, TerminalView, TerminalViewDelta,
    TerminalViewFrame,
};
use condr_server::{Endpoint, StaticKey, TcpEndpoint};
use gpui_kit::{AssetSource as _, KeyDownEvent, Keystroke, Task};

use super::ClientTerminal;
use super::{
    ClientIo, ClosePane, CondrAssets, ConnectionStatus, FocusLeft, NewTab, NextTab, OpenSettings,
    PreviousTab, ServerConnection, SidebarGlyph, SidebarIconTone, SplitDown, SplitRight,
    TerminalClipboardShortcut, TerminalVisualSlot, ToggleZoom, accepted_text_input,
    agent_sidebar_status, apply_terminal_frame_batch, assemble_terminal_frame_chunk,
    clear_pending_sizes_for_bootstrap, connect_to_server_with,
    enforce_terminal_chunk_reliable_fence, fixed_shortcut, lock_exclusively, merge_terminal_deltas,
    read_bootstrap_batches, reorder_connection, should_defer_to_character_input,
    single_instance_lock_path, terminal_chunk_identity_matches, terminal_clipboard_shortcut,
};

fn terminal_cell(text: &str) -> TerminalCell {
    TerminalCell {
        text: text.into(),
        foreground: TerminalColor::Named(0),
        background: TerminalColor::Named(0),
        flags: 0,
        hyperlink: None,
    }
}

fn terminal_hyperlink_budgets(
    terminals: &mut std::collections::HashMap<condr_core::PaneId, ClientTerminal>,
) -> std::collections::HashMap<condr_core::PaneId, TerminalHyperlinkBudget> {
    terminals
        .iter_mut()
        .map(|(&pane_id, terminal)| {
            (
                pane_id,
                TerminalHyperlinkBudget::new(std::sync::Arc::make_mut(&mut terminal.view)),
            )
        })
        .collect()
}

/// A TCP endpoint with fixed keys, so two calls with one address compare equal.
pub(super) fn tcp(address: &str) -> Endpoint {
    Endpoint::tcp(TcpEndpoint::at(
        address.parse().unwrap(),
        StaticKey::from_private([7; 32]).public(),
        StaticKey::from_private([9; 32]),
    ))
}

#[test]
fn edit_server_fields_only_accept_host_and_port_shaped_text() {
    use super::dialogs::{host_text_is_plausible, port_text_is_plausible};

    for host in [
        "",
        "jpdev",
        "lab.local",
        "10.0.0.5",
        "[fe80::1]",
        "my-box-2",
    ] {
        assert!(host_text_is_plausible(host), "{host:?}");
    }
    for host in ["lab local", "jpdev/", "h@st", "ab_c", &"x".repeat(254)] {
        assert!(!host_text_is_plausible(host), "{host:?}");
    }
    for port in ["", "1", "4242", "65535"] {
        assert!(port_text_is_plausible(port), "{port:?}");
    }
    for port in ["65536", "123456", "-1", "42a", " 1"] {
        assert!(!port_text_is_plausible(port), "{port:?}");
    }
}

fn connection_with_io() -> ServerConnection {
    let mut connection = ServerConnection::new(1, "test".into(), tcp("127.0.0.1:9"));
    connection.status = ConnectionStatus::Connected;
    connection.server_id = Some(ServerId(1));
    connection.runtime_epoch = Some(RuntimeEpoch(2));
    connection.session_id = Some(SessionId(3));
    connection.sequence = 7;
    let (outgoing, outgoing_rx) = std::sync::mpsc::channel();
    std::mem::forget(outgoing_rx);
    connection.io = Some(ClientIo {
        outgoing,
        _incoming_task: Task::ready(()),
    });
    connection
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
    let local = condr_server::ServerConfig::default().local_endpoint();
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
        "Failed to start or discover the local Server: invalid endpoint marker"
    );

    let (_, concurrent) = connect_to_server_with(
        local,
        || Err(std::io::Error::other("concurrent launcher won")),
        |_| Ok("connected concurrently"),
    );
    assert_eq!(concurrent.unwrap(), "connected concurrently");

    let remote = tcp("127.0.0.1:4242");
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
        selection: None,
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
    let endpoint = || tcp("127.0.0.1:4242");
    let connection = |key: u64| ServerConnection::new(key, format!("server-{key}"), endpoint());
    let keys =
        |connections: &Vec<ServerConnection>| connections.iter().map(|c| c.key).collect::<Vec<_>>();

    let mut connections = vec![connection(1), connection(2), connection(3)];
    assert!(reorder_connection(&mut connections, 3, 1, false));
    assert_eq!(keys(&connections), [3, 1, 2]);

    assert!(reorder_connection(&mut connections, 3, 2, true));
    assert_eq!(keys(&connections), [1, 2, 3]);

    assert!(
        !reorder_connection(&mut connections, 2, 2, false),
        "dropping a server on itself should not change the order"
    );
    assert!(
        !reorder_connection(&mut connections, 1, 2, false),
        "dropping just before the next item is where it already is"
    );
    assert!(
        !reorder_connection(&mut connections, 9, 1, false),
        "an unknown dragged key should be ignored"
    );
    assert!(
        !reorder_connection(&mut connections, 1, 9, true),
        "an unknown target key should be ignored"
    );
    assert_eq!(keys(&connections), [1, 2, 3]);
}

#[test]
fn drop_index_inserts_before_or_after_the_target_in_the_list_without_the_source() {
    use super::sidebar::drop_index;
    // Moving right: the removal shifts the target left by one.
    assert_eq!(drop_index(0, 2, false), Some(1));
    assert_eq!(drop_index(0, 2, true), Some(2));
    // Moving left: indexes are unaffected by the removal.
    assert_eq!(drop_index(2, 0, false), Some(0));
    assert_eq!(drop_index(2, 0, true), Some(1));
    // Both sides of the source's own slot are no-ops.
    assert_eq!(drop_index(1, 1, false), None);
    assert_eq!(drop_index(1, 1, true), None);
    assert_eq!(drop_index(1, 0, true), None);
    assert_eq!(drop_index(1, 2, false), None);
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
            SidebarGlyph::CircleFilled,
            SidebarIconTone::Warning,
            "working",
        ),
        (
            AgentDisplayState::Blocked,
            SidebarGlyph::CircleFilled,
            SidebarIconTone::Danger,
            "blocked",
        ),
        (
            AgentDisplayState::Done,
            SidebarGlyph::CircleFilled,
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
fn condr_assets_include_custom_and_kit_icons() {
    let assets = CondrAssets::new();
    let listed = assets.list("icons/").unwrap();
    for path in [
        "icons/circle.svg",
        "icons/circle-filled.svg",
        "icons/circle-alert.svg",
        "icons/server-plus.svg",
        "icons/info.svg",
        "icons/settings.svg",
    ] {
        let bytes = assets
            .load(path)
            .unwrap()
            .expect("Condr and GPUI Kit icons should be embedded");
        assert!(bytes.starts_with(b"<svg"), "invalid SVG asset: {path}");
        assert!(listed.iter().any(|listed| listed.as_ref() == path));
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
    let mut connection = ServerConnection::new(1, "test".into(), tcp("127.0.0.1:9"));
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
    // A repeated rejection is still accepted but does not request a second snapshot.
    assert!(connection.recover_rejected_subscription(ServerId(1), SessionId(30)));
    assert!(matches!(
        outgoing_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(!connection.recover_rejected_subscription(ServerId(9), SessionId(30)));

    // Same authority: a rejection-recovery Bootstrap re-acquires control without discarding
    // the connection's GUI state, and a plain visual resync Bootstrap touches neither.
    {
        let bootstrap = |sequence| SessionBootstrap {
            settings: Default::default(),
            server_id: ServerId(1),
            runtime_epoch: RuntimeEpoch(2),
            session_id: SessionId(3),
            sequence,
            snapshot: Session::new().snapshot(),
            terminals: Vec::new(),
            agents: Vec::new(),
            workspace_git: Vec::new(),
            zoomed_panes: Vec::new(),
        };
        let mut same_authority = connection_with_io();
        same_authority.subscribed = true;
        same_authority.controlling = true;
        same_authority.bootstrap_resync_session_id = Some(SessionId(3));
        assert!(same_authority.recover_rejected_subscription(ServerId(1), SessionId(3)));
        let recovered = same_authority.apply_bootstrap(bootstrap(8));
        assert!(recovered.reacquire_control);
        assert!(recovered.resubscribe);
        assert!(!recovered.authority_changed);
        assert!(same_authority.controlling);

        same_authority.subscribed = true;
        assert!(same_authority.request_snapshot());
        let resynced = same_authority.apply_bootstrap(bootstrap(9));
        assert!(!resynced.reacquire_control);
        assert!(!resynced.resubscribe);
        assert!(!resynced.authority_changed);
    }

    let application = connection.apply_bootstrap(SessionBootstrap {
        settings: Default::default(),
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
    let mut connection = ServerConnection::new(1, "test".into(), tcp("127.0.0.1:9"));
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
fn a_dropped_active_subscription_requests_an_authoritative_bootstrap() {
    let mut connection = ServerConnection::new(1, "test".into(), tcp("127.0.0.1:9"));
    connection.status = ConnectionStatus::Connected;
    connection.server_id = Some(ServerId(1));
    connection.session_id = Some(SessionId(3));
    connection.subscribed = true;
    let (outgoing, outgoing_rx) = std::sync::mpsc::channel();
    connection.io = Some(ClientIo {
        outgoing,
        _incoming_task: Task::ready(()),
    });

    assert!(connection.recover_rejected_subscription(ServerId(1), SessionId(3)));
    assert_eq!(
        outgoing_rx.recv().unwrap(),
        condr_core::protocol::ClientMessage::SnapshotRequest {
            session_id: SessionId(3),
        }
    );
    assert!(!connection.subscribed);
}

#[test]
fn ordinary_runtime_bootstrap_keeps_subscription_baseline_and_attention() {
    let mut connection = ServerConnection::new(1, "test".into(), tcp("127.0.0.1:9"));
    connection.status = ConnectionStatus::Connected;
    connection.server_id = Some(ServerId(1));
    connection.runtime_epoch = Some(RuntimeEpoch(2));
    connection.session_id = Some(SessionId(3));
    connection.sequence = 7;
    connection.controlling = true;
    connection.subscribed = true;
    connection.bootstrap_resync_session_id = Some(SessionId(3));
    let pane_id = pane_id();

    let application = connection.apply_bootstrap(SessionBootstrap {
        settings: Default::default(),
        server_id: ServerId(1),
        runtime_epoch: RuntimeEpoch(2),
        session_id: SessionId(3),
        sequence: 8,
        snapshot: Session::new().snapshot(),
        terminals: vec![PaneTerminalSnapshot {
            pane_id,
            view: terminal_view(1, "bell"),
            exited: false,
            title: None,
            attention: true,
        }],
        agents: Vec::new(),
        workspace_git: Vec::new(),
        zoomed_panes: Vec::new(),
    });

    assert!(!application.resubscribe);
    assert!(!application.reacquire_control);
    assert!(connection.subscribed);
    assert!(connection.can_mutate());
    assert!(connection.attention.contains(&pane_id));
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
fn terminal_frame_chunks_survive_metadata_interleaving_until_complete() {
    let pane_id = pane_id();
    let expected = PaneTerminalFrame {
        pane_id,
        frame: TerminalViewFrame::Full(terminal_view(7, "chunked")),
    };
    let payload = encode_pane_terminal_frame(&expected).unwrap();
    let midpoint = payload.len() / 2;
    let chunks = [&payload[..midpoint], &payload[midpoint..]];
    for interleaved in [
        ServerMessage::TerminalClipboard {
            pane_id,
            text: "copied between chunks".into(),
        },
        ServerMessage::Event {
            server_id: ServerId(1),
            session_id: SessionId(1),
            sequence: 1,
            event: SessionEvent::TerminalTitleChanged {
                pane_id,
                title: Some("title between chunks".into()),
            },
        },
    ] {
        let mut assembly = None;
        for (chunk_index, payload) in chunks.iter().enumerate() {
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
                assert!(enforce_terminal_chunk_reliable_fence(&mut assembly, &interleaved).is_ok());
                assert!(assembly.is_some());
            } else {
                assert_eq!(assembled, Some(expected.clone()));
            }
        }
        assert!(assembly.is_none());
    }
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
            ClientTerminal::from(PaneTerminalSnapshot {
                pane_id: first_pane,
                view: first_view.clone(),
                exited: false,
                title: None,
                attention: false,
            }),
        ),
        (
            second_pane,
            ClientTerminal::from(PaneTerminalSnapshot {
                pane_id: second_pane,
                view: second_view.clone(),
                exited: false,
                title: None,
                attention: false,
            }),
        ),
    ]);
    let batch = vec![
        PaneTerminalFrame {
            pane_id: first_pane,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                selection: None,
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
                selection: None,
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
    let mut terminal_hyperlinks = terminal_hyperlink_budgets(&mut terminals);

    assert!(apply_terminal_frame_batch(&mut terminals, &mut terminal_hyperlinks, batch).is_err());
    assert_eq!(*terminals[&first_pane].view, first_view);
    assert_eq!(*terminals[&second_pane].view, second_view);
}

#[test]
fn terminal_frame_batch_bounds_hyperlinks_across_retained_deltas() {
    let pane_id = pane_id();
    let mut terminals = std::collections::HashMap::from([(
        pane_id,
        ClientTerminal::from(PaneTerminalSnapshot {
            pane_id,
            view: terminal_view(1, "x"),
            exited: false,
            title: None,
            attention: false,
        }),
    )]);
    let mut terminal_hyperlinks = terminal_hyperlink_budgets(&mut terminals);
    let mut linked = terminal_cell("x");
    linked.hyperlink = Some("x".repeat(8 * 1024 + 1).into());

    apply_terminal_frame_batch(
        &mut terminals,
        &mut terminal_hyperlinks,
        vec![PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                selection: None,
                base_revision: 1,
                revision: 2,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 0,
                    cells: vec![linked],
                }],
            }),
        }],
    )
    .unwrap();

    assert!(terminals[&pane_id].view.cells[0].hyperlink.is_none());
}

#[test]
fn terminal_frame_batch_canonicalizes_links_across_retained_deltas() {
    let pane_id = pane_id();
    let uri = "https://example.com/a-long-target-shared-across-deltas";
    let mut view = terminal_view(1, "ab");
    view.cells[0].hyperlink = Some(uri.into());
    let mut terminals = std::collections::HashMap::from([(
        pane_id,
        ClientTerminal::from(PaneTerminalSnapshot {
            pane_id,
            view,
            exited: false,
            title: None,
            attention: false,
        }),
    )]);
    let mut terminal_hyperlinks = terminal_hyperlink_budgets(&mut terminals);
    let mut linked = terminal_cell("b");
    linked.hyperlink = Some(uri.into());

    apply_terminal_frame_batch(
        &mut terminals,
        &mut terminal_hyperlinks,
        vec![PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                selection: None,
                base_revision: 1,
                revision: 2,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 1,
                    cells: vec![linked],
                }],
            }),
        }],
    )
    .unwrap();

    let view = &terminals[&pane_id].view;
    assert_eq!(
        view.cells[0].hyperlink.as_ref().unwrap().as_ptr(),
        view.cells[1].hyperlink.as_ref().unwrap().as_ptr()
    );
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
        title: None,
        attention: false,
    };
    let payload = encode_bootstrap_record(&BootstrapRecord::Terminal(terminal.clone())).unwrap();
    let midpoint = payload.len() / 2;
    let header = BootstrapHeader {
        settings: Default::default(),
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
    if cfg!(target_os = "macos") {
        // Cmd is the application modifier; Ctrl and Option belong to the shell.
        assert!(action("cmd-t").unwrap().as_any().is::<NewTab>());
        assert!(action("cmd-w").unwrap().as_any().is::<ClosePane>());
        assert!(action("cmd-d").unwrap().as_any().is::<SplitRight>());
        assert!(action("cmd-shift-d").unwrap().as_any().is::<SplitDown>());
        assert!(action("cmd-}").unwrap().as_any().is::<NextTab>());
        assert!(action("cmd-{").unwrap().as_any().is::<PreviousTab>());
        assert!(action("cmd-alt-left").unwrap().as_any().is::<FocusLeft>());
        assert!(action("cmd-ctrl-left").is_some());
        assert!(
            action("cmd-shift-enter")
                .unwrap()
                .as_any()
                .is::<ToggleZoom>()
        );
        assert!(action("alt-left").is_none(), "Option+Left moves by word");
        assert!(action("alt-shift-=").is_none(), "Option+Shift+= types ±");
        assert!(
            action("ctrl-shift-t").is_none(),
            "Ctrl chords reach the PTY"
        );
        assert!(action("cmd-shift-t").is_none());
        assert!(action("cmd-ctrl-alt-left").is_none());
    } else {
        assert!(action("alt-shift-=").unwrap().as_any().is::<SplitRight>());
        assert!(action("alt-shift--").unwrap().as_any().is::<SplitDown>());
        assert!(action("alt-+").unwrap().as_any().is::<SplitRight>());
        assert!(action("alt-_").unwrap().as_any().is::<SplitDown>());
        assert!(action("alt-left").unwrap().as_any().is::<FocusLeft>());
        assert!(
            action("alt-shift-enter")
                .unwrap()
                .as_any()
                .is::<ToggleZoom>()
        );
        assert!(action("cmd-t").is_none());
    }
    assert!(action("alt-enter").is_none());
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

    // Ctrl+Shift+C/V: clipboard on Linux/Windows; on macOS every Ctrl chord is PTY input.
    let control_shift_copy =
        (!cfg!(target_os = "macos")).then_some(TerminalClipboardShortcut::Copy);
    let control_shift_paste =
        (!cfg!(target_os = "macos")).then_some(TerminalClipboardShortcut::Paste);
    assert_eq!(shortcut("ctrl-shift-c", false), control_shift_copy);
    assert_eq!(shortcut("ctrl-shift-v", false), control_shift_paste);
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
fn terminal_keys_follow_platform_keystroke_semantics() {
    let key = |keys: &str| terminal_key_for(&Keystroke::parse(keys).unwrap());
    let character = |text: &str| Some(TerminalKey::Character(text.into()));

    // Plain text and shifted text arrive through the input handler, not as keys.
    assert_eq!(key("s"), None);
    assert_eq!(key("shift-s->S"), None);
    assert_eq!(key("ctrl-c"), character("c"));
    assert_eq!(key("ctrl-alt-c"), character("c"));
    // Ctrl/Alt+Space are NUL and ESC SP, not the letters of "space".
    assert_eq!(key("ctrl-space"), character(" "));
    assert_eq!(key("alt-space"), character(" "));
    // Windows leaves key_char empty under Alt: the shifted letter is still uppercase.
    assert_eq!(key("alt-shift-s"), character("S"));
    assert_eq!(key("alt-shift-s->S"), character("S"));
    // Keys a terminal cannot encode are not turned into their names.
    assert_eq!(key("ctrl-pause"), None);
    assert_eq!(key("shift-tab"), Some(TerminalKey::BackTab));
    assert_eq!(key("ctrl-f13"), Some(TerminalKey::Function(13)));

    // macOS Option produces a character (ß), which is typed rather than sent as ESC ß;
    // elsewhere Alt is Meta.
    let option_s = key("alt-s->ß");
    if cfg!(target_os = "macos") {
        assert_eq!(option_s, None);
        assert_eq!(key("ctrl-alt-s"), character("s"));
    } else {
        assert_eq!(option_s, character("ß"));
        assert_eq!(key("alt-s->s"), character("s"));
    }
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
    let uri = "https://example.com/a-long-target-shared-across-pending-deltas";
    let mut first_cell = terminal_cell("X");
    first_cell.hyperlink = Some(uri.into());
    let mut second_cell = terminal_cell("Y");
    second_cell.hyperlink = Some(uri.into());
    let first = TerminalViewDelta {
        selection: None,
        base_revision: 1,
        revision: 2,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: vec![first_cell],
        }],
    };
    let second = TerminalViewDelta {
        selection: None,
        base_revision: 2,
        revision: 3,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 2,
            cells: vec![second_cell],
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
    assert_eq!(
        view.cells[1].hyperlink.as_ref().unwrap().as_ptr(),
        view.cells[2].hyperlink.as_ref().unwrap().as_ptr()
    );
}

#[test]
fn lag_notice_during_a_visual_gap_resync_still_reacquires_control() {
    let mut connection = connection_with_io();
    // A revision gap already dropped the subscription and requested a snapshot.
    connection.subscribed = false;
    connection.subscription_pending = false;
    assert!(connection.request_snapshot());
    assert!(connection.recover_rejected_subscription(ServerId(1), SessionId(3)));
    let application = connection.apply_bootstrap(SessionBootstrap {
        settings: Default::default(),
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
    assert!(application.reacquire_control);
    assert!(application.resubscribe);
    assert!(!application.authority_changed);
}

#[test]
fn bootstrap_attention_survives_arriving_before_control_is_granted() {
    let mut connection = connection_with_io();
    connection.controlling = false;
    let pane = pane_id();
    connection.apply_bootstrap(SessionBootstrap {
        settings: Default::default(),
        server_id: ServerId(1),
        runtime_epoch: RuntimeEpoch(2),
        session_id: SessionId(3),
        sequence: 8,
        snapshot: Session::new().snapshot(),
        terminals: vec![PaneTerminalSnapshot {
            pane_id: pane,
            view: terminal_view(1, "x"),
            exited: false,
            title: None,
            attention: true,
        }],
        agents: Vec::new(),
        workspace_git: Vec::new(),
        zoomed_panes: Vec::new(),
    });
    assert!(connection.attention.contains(&pane));
}

#[test]
fn hover_clears_only_from_the_pane_that_owns_it() {
    let owner = pane_id();
    let other = pane_id();
    let link = HoveredTerminalLink {
        range: 0..4,
        uri: "https://example.com".into(),
        position: condr_core::TerminalMousePosition { row: 0, column: 1 },
    };
    let current = (1, owner, link.clone());

    assert_eq!(
        next_hovered_link(None, 1, owner, Some(link.clone())),
        Some(Some(current.clone()))
    );
    assert_eq!(
        next_hovered_link(Some(&current), 1, owner, Some(link.clone())),
        None
    );
    // Another pane (or another connection) reporting no link leaves the owner's hover alone.
    assert_eq!(next_hovered_link(Some(&current), 1, other, None), None);
    assert_eq!(next_hovered_link(Some(&current), 2, owner, None), None);
    // The owner reporting no link clears it; a new link elsewhere replaces it.
    assert_eq!(
        next_hovered_link(Some(&current), 1, owner, None),
        Some(None)
    );
    assert_eq!(
        next_hovered_link(Some(&current), 1, other, Some(link.clone())),
        Some(Some((1, other, link)))
    );
}

#[test]
fn visual_slot_publish_is_atomic_across_panes() {
    let first_pane = pane_id();
    let second_pane = pane_id();
    let slot = TerminalVisualSlot::default();
    let batch = |panes| TerminalFrameBatch {
        server_id: ServerId(1),
        session_id: SessionId(2),
        panes,
    };
    let delta = |base_revision, text| {
        TerminalViewFrame::Delta(TerminalViewDelta {
            selection: None,
            base_revision,
            revision: base_revision + 1,
            display_offset: 0,
            mouse_tracking: TerminalMouseTracking::None,
            cursor: None,
            runs: vec![TerminalCellRun {
                start: 0,
                cells: vec![terminal_cell(text)],
            }],
        })
    };

    assert_eq!(
        slot.publish(batch(vec![
            PaneTerminalFrame {
                pane_id: first_pane,
                frame: TerminalViewFrame::Full(terminal_view(1, "a")),
            },
            PaneTerminalFrame {
                pane_id: second_pane,
                frame: TerminalViewFrame::Full(terminal_view(1, "b")),
            },
        ]))
        .unwrap(),
        Some(0)
    );

    // The first pane merges cleanly; the second pane's delta does not match its base.
    assert!(
        slot.publish(batch(vec![
            PaneTerminalFrame {
                pane_id: first_pane,
                frame: delta(1, "x"),
            },
            PaneTerminalFrame {
                pane_id: second_pane,
                frame: delta(5, "y"),
            },
        ]))
        .is_err()
    );
    {
        let state = slot.state.lock().unwrap();
        assert_eq!(state.generation, 0);
        assert!(state.signaled);
        let pending = state.pending.as_ref().unwrap();
        assert_eq!(pending.panes.len(), 2);
        assert!(matches!(
            &pending.panes[&first_pane],
            TerminalViewFrame::Full(view) if view.revision == 1 && view.cells[0].text == "a"
        ));
        assert!(matches!(
            &pending.panes[&second_pane],
            TerminalViewFrame::Full(view) if view.revision == 1 && view.cells[0].text == "b"
        ));
    }
    let taken = slot.take(0).unwrap();
    assert_eq!(taken.panes.len(), 2);
    assert!(taken.panes.iter().all(|pane| matches!(
        &pane.frame,
        TerminalViewFrame::Full(view) if view.revision == 1
    )));
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
        selection: None,
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
        selection: None,
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
    let path = directory.join("condr.lock");

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
        condr_core::runtime_directory().map(|root| root.join("condr.lock"))
    );
}

#[test]
fn alt_v_alone_is_the_image_paste_gesture_and_maps_only_supported_formats() {
    assert!(is_image_paste_gesture(&Keystroke::parse("alt-v").unwrap()));
    let mut function_chord = Keystroke::parse("alt-v").unwrap();
    function_chord.modifiers.function = true;
    assert!(!is_image_paste_gesture(&function_chord));
    for keys in ["v", "ctrl-v", "alt-shift-v", "ctrl-alt-v", "cmd-v", "alt-c"] {
        assert!(
            !is_image_paste_gesture(&Keystroke::parse(keys).unwrap()),
            "{keys}"
        );
    }
    // Alt+V is not a text-paste shortcut either: on a local Server it reaches the PTY.
    assert!(terminal_clipboard_shortcut(&Keystroke::parse("alt-v").unwrap(), false).is_none());
    use condr_core::protocol::ClipboardImageFormat as Wire;
    assert_eq!(
        clipboard_image_format(gpui_kit::ImageFormat::Png),
        Some(Wire::Png)
    );
    assert_eq!(
        clipboard_image_format(gpui_kit::ImageFormat::Jpeg),
        Some(Wire::Jpeg)
    );
    assert_eq!(clipboard_image_format(gpui_kit::ImageFormat::Svg), None);
    assert_eq!(clipboard_image_format(gpui_kit::ImageFormat::Tiff), None);
}
