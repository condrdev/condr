use super::*;

#[test]
fn edit_server_fields_only_accept_host_and_port_shaped_text() {
    use crate::app::dialogs::{host_text_is_plausible, port_text_is_plausible};

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

#[test]
fn hooks_actions_go_to_the_server_as_agent_commands() {
    let mut connection = ServerConnection::new(1, "test".into(), tcp("127.0.0.1:9"));
    connection.status = ConnectionStatus::Connected;
    connection.server_id = Some(ServerId(1));
    connection.session_id = Some(SessionId(3));
    let (outgoing, outgoing_rx) = std::sync::mpsc::channel();
    connection.io = Some(ClientIo {
        outgoing,
        _incoming_task: Task::ready(()),
    });

    connection.send_agent_hooks(
        condr_core::AgentKind::Codex,
        condr_core::agent_hooks::HooksAction::Install,
    );
    assert_eq!(
        outgoing_rx.recv().unwrap(),
        ClientMessage::Agent {
            server_id: ServerId(1),
            session_id: SessionId(3),
            command: AgentCommand::Hooks {
                agent: condr_core::AgentKind::Codex,
                action: condr_core::agent_hooks::HooksAction::Install,
            },
        }
    );
    // No Session yet: nothing to ask, and no disconnect either.
    connection.session_id = None;
    connection.send_agent_hooks(
        condr_core::AgentKind::Claude,
        condr_core::agent_hooks::HooksAction::Status,
    );
    assert!(matches!(
        outgoing_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(matches!(connection.status, ConnectionStatus::Connected));
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
        .unwrap()
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
