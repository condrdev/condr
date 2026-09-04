use super::*;

#[cfg(target_os = "linux")]
use condr_core::{
    TerminalModifiers, TerminalMouseButton, TerminalMouseEvent, TerminalMousePosition,
};

#[test]
fn compatible_client_gets_bootstrap_and_reconnect_sees_same_epoch() {
    let (handle, endpoint, thread) = start();
    let first = connect_and_bootstrap(&endpoint);
    let first_message: ServerMessage = {
        let mut stream = endpoint.connect().unwrap();
        condr_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Hello(Hello {
                version: PROTOCOL_VERSION,
                client_name: "first".into(),
            }),
        )
        .unwrap();
        condr_core::protocol::read_message(&mut stream).unwrap()
    };
    let (first_id, first_epoch) = match first_message {
        ServerMessage::Welcome {
            server_id,
            runtime_epoch,
            ..
        } => (server_id, runtime_epoch),
        other => panic!("unexpected message: {other:?}"),
    };
    drop(first);
    let mut second = endpoint.connect().unwrap();
    condr_core::protocol::write_message(
        &mut second,
        &ClientMessage::Hello(Hello {
            version: PROTOCOL_VERSION,
            client_name: "second".into(),
        }),
    )
    .unwrap();
    let welcome: ServerMessage = condr_core::protocol::read_message(&mut second).unwrap();
    assert!(matches!(
        welcome,
        ServerMessage::Welcome {
            server_id,
            runtime_epoch,
            error: None,
            ..
        } if server_id == first_id && runtime_epoch == first_epoch
    ));
    handle.stop();
    drop(second);
    thread.join().unwrap().unwrap();
}

#[test]
fn non_hello_first_frame_is_rejected_with_a_clear_error() {
    let (handle, endpoint, thread) = start();
    let mut stream = endpoint.connect().unwrap();
    condr_core::protocol::write_message(&mut stream, &ClientMessage::Detach).unwrap();

    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::Welcome {
            error: Some(message),
            ..
        } if message == "expected Hello as first message"
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn incompatible_client_is_rejected() {
    let (handle, endpoint, thread) = start();
    let mut stream = endpoint.connect().unwrap();
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Hello(Hello {
            version: PROTOCOL_VERSION + 1,
            client_name: "old".into(),
        }),
    )
    .unwrap();
    let response: ServerMessage = condr_core::protocol::read_message(&mut stream).unwrap();
    assert!(matches!(
        response,
        ServerMessage::Welcome { error: Some(_), .. }
    ));
    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn controller_can_acknowledge_attention_while_terminal_is_closing() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let (server_id, session_id, pane_id) = {
        let mut state = handle.state.lock().unwrap();
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
        state.closing_terminals.insert(pane_id);
        state.pending_terminal_bells.insert(pane_id);
        (state.server_id, state.session_id, pane_id)
    };
    acquire_control(&mut stream, session_id);

    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Focus(true),
    );
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Ping {
            server_id,
            nonce: 7,
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::Pong { nonce: 7, .. }
    ));
    {
        let state = handle.state.lock().unwrap();
        assert_eq!(state.focused_terminal, Some(pane_id));
        assert!(!state.pending_terminal_bells.contains(&pane_id));
    }

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

/// Closing a Tab from the GUI races its focus bookkeeping: the Pane is gone on the
/// Server before the client's `Focus(false)` for it arrives. That is not an error worth
/// showing; typing into a missing Pane still is.
#[test]
fn focus_changes_for_a_missing_pane_are_ignored_but_input_is_rejected() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let (server_id, session_id) = {
        let state = handle.state.lock().unwrap();
        (state.server_id, state.session_id)
    };
    acquire_control(&mut stream, session_id);
    let missing = PaneId::from_u64(424_242);

    for focused in [false, true] {
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            missing,
            TerminalCommand::Focus(focused),
        );
    }
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Ping {
            server_id,
            nonce: 9,
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut stream),
        ServerMessage::Pong { nonce: 9, .. }
    ));

    send_terminal(
        &mut stream,
        server_id,
        session_id,
        missing,
        TerminalCommand::Text("ls".into()),
    );
    assert!(matches!(
        read_server(&mut stream),
        ServerMessage::Error { message } if message == "unknown Pane"
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn oversized_client_frame_is_rejected_with_a_clear_error() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let claimed = (condr_core::protocol::MAX_FRAME_SIZE as u32) + 1;
    std::io::Write::write_all(&mut stream, &claimed.to_le_bytes()).unwrap();

    let response: ServerMessage = condr_core::protocol::read_message(&mut stream).unwrap();
    assert!(matches!(
        response,
        ServerMessage::Error { message }
            if message.contains("exceeds maximum")
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn invalid_workspace_roots_preserve_authoritative_layout_focus_and_terminals() {
    let (handle, endpoint, thread) = start();
    let connection = ClientConnection::connect(&endpoint, "failed-workspace").unwrap();
    let bootstrap = connection.bootstrap().clone();
    let server_id = bootstrap.server_id;
    let session_id = bootstrap.session_id;
    let mut stream = connection.into_stream();
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    acquire_control(&mut stream, session_id);

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            },
        },
    )
    .unwrap();
    let message = wait_for_message(&mut stream, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event { sequence, .. } = message else {
        unreachable!("predicate only accepts LayoutChanged events");
    };
    assert_layout_applied(&mut stream, server_id, session_id, 1, sequence);

    let (snapshot_before, sequence_before, focus_before, terminal_instances_before) = {
        let state = handle.state.lock().unwrap();
        let workspace = state.session.active_workspace().unwrap();
        let tab = workspace.active_tab();
        (
            state.session.snapshot(),
            state.sequence,
            (workspace.id(), tab.id(), tab.focused_pane().id()),
            state.terminal_instances.clone(),
        )
    };
    assert_eq!(terminal_instances_before.len(), 1);

    let missing_root = std::env::temp_dir().join(format!(
        "condr-missing-workspace-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    assert!(!missing_root.exists());
    let regular_file = std::env::temp_dir().join(format!(
        "condr-file-workspace-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::write(&regular_file, b"not a directory").unwrap();
    let rejected_commands = [
        (
            42,
            LayoutCommand::CreateWorkspace {
                root_directory: missing_root,
            },
            "cannot access root directory",
        ),
        (
            43,
            LayoutCommand::CreateWorkspace {
                root_directory: PathBuf::from("relative-workspace-root"),
            },
            "must be an absolute path",
        ),
        (
            44,
            LayoutCommand::CreateWorkspace {
                root_directory: regular_file.clone(),
            },
            "is not a directory",
        ),
        (
            45,
            LayoutCommand::OpenWorktree {
                parent_workspace_id: focus_before.0,
                root_directory: PathBuf::from("relative-worktree-root"),
            },
            "must be an absolute path",
        ),
    ];

    for (request_id, command, expected_reason) in rejected_commands {
        condr_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Layout {
                server_id,
                session_id,
                request_id,
                command,
            },
        )
        .unwrap();

        let rejection = wait_for_message(&mut stream, |message| {
            matches!(
                message,
                ServerMessage::LayoutRejected {
                    request_id: rejected_request,
                    ..
                } if *rejected_request == request_id
            )
        });
        let ServerMessage::LayoutRejected {
            server_id: rejected_server,
            session_id: rejected_session,
            request_id: rejected_request,
            reason,
        } = rejection
        else {
            unreachable!("predicate only accepts LayoutRejected");
        };
        assert_eq!(rejected_server, server_id);
        assert_eq!(rejected_session, session_id);
        assert_eq!(rejected_request, request_id);
        assert!(
            reason.contains(expected_reason),
            "unexpected rejection reason: {reason}"
        );

        let state = handle.state.lock().unwrap();
        let workspace = state.session.active_workspace().unwrap();
        let tab = workspace.active_tab();
        assert_eq!(state.session.snapshot(), snapshot_before);
        assert!(
            state
                .events
                .iter()
                .filter(|event| event.sequence > sequence_before)
                .all(|event| matches!(
                    event.event,
                    SessionEvent::TerminalTitleChanged { .. }
                        | SessionEvent::TerminalAttentionChanged { .. }
                )),
            "a rejected layout command published a layout event"
        );
        assert_eq!(
            (workspace.id(), tab.id(), tab.focused_pane().id()),
            focus_before
        );
        assert_eq!(state.terminal_instances, terminal_instances_before);
        assert_eq!(state.terminals.len(), terminal_instances_before.len());
    }
    let _ = std::fs::remove_file(regular_file);

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn rejected_subscription_is_typed_and_removes_the_previous_subscriber() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let (server_id, session_id) = {
        let state = handle.state.lock().unwrap();
        (state.server_id, state.session_id)
    };

    subscribe(&mut stream, session_id, 0);
    assert_eq!(handle.state.lock().unwrap().subscribers.len(), 1);

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Subscribe {
            session_id,
            after_sequence: u64::MAX,
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::SubscriptionRejected {
            server_id: rejected_server,
            session_id: rejected_session,
            reason,
        } if rejected_server == server_id
            && rejected_session == session_id
            && reason == "event cursor is ahead of the server"
    ));
    assert!(handle.state.lock().unwrap().subscribers.is_empty());

    {
        let mut state = handle.state.lock().unwrap();
        for _ in 0..=EVENT_HISTORY_LIMIT {
            let event = state.layout_changed_event();
            state.publish_background(event);
        }
    }
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Subscribe {
            session_id: SessionId(session_id.0.wrapping_add(1)),
            after_sequence: 0,
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::SubscriptionRejected {
            server_id: rejected_server,
            session_id: authoritative_session,
            reason,
        } if rejected_server == server_id
            && authoritative_session == session_id
            && reason == "unknown Session"
    ));

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Subscribe {
            session_id,
            after_sequence: 0,
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::SubscriptionRejected {
            server_id: rejected_server,
            session_id: rejected_session,
            reason,
        } if rejected_server == server_id
            && rejected_session == session_id
            && reason == "event cursor expired"
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn lagged_subscriber_is_told_to_bootstrap_and_can_resubscribe() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let (server_id, session_id) = {
        let state = handle.state.lock().unwrap();
        (state.server_id, state.session_id)
    };
    subscribe(&mut stream, session_id, 0);
    let (client_id, subscriber_writer) = {
        let state = handle.state.lock().unwrap();
        let (&client_id, subscriber) = state.subscribers.iter().next().unwrap();
        (client_id, subscriber.writer.clone())
    };

    let blocking_frame = frame_message(&ServerMessage::Error {
        message: "x".repeat(MAX_FRAME_SIZE - 1024),
    })
    .unwrap();
    subscriber_writer.send_reliable(blocking_frame).unwrap();
    thread::sleep(Duration::from_millis(30));
    let queued_frame = frame_message(&ServerMessage::Pong {
        server_id,
        nonce: 1,
        sequence: 0,
    })
    .unwrap();
    let lagged = (0..600).any(|_| {
        subscriber_writer
            .send_reliable(queued_frame.clone())
            .is_err()
    });
    assert!(lagged, "test did not reach the reliable queue bound");
    handle.state.lock().unwrap().subscribers.remove(&client_id);

    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::Error { .. }
    ));
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::SubscriptionRejected {
            server_id: rejected_server,
            session_id: rejected_session,
            reason,
        } if rejected_server == server_id
            && rejected_session == session_id
            && reason == "client fell behind the reliable event stream"
    ));

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::SnapshotRequest { session_id },
    )
    .unwrap();
    let ServerMessage::Bootstrap(header) =
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap()
    else {
        panic!("expected recovery Bootstrap header");
    };
    let batch_count = header.batch_count;
    let mut assembler = BootstrapAssembler::new(header).unwrap();
    for _ in 0..batch_count {
        let ServerMessage::BootstrapBatch(batch) =
            condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap()
        else {
            panic!("expected recovery Bootstrap batch");
        };
        assembler.push(batch).unwrap();
    }
    let bootstrap = assembler.finish().unwrap();
    subscribe(&mut stream, session_id, bootstrap.sequence);
    assert_eq!(handle.state.lock().unwrap().subscribers.len(), 1);

    handle.stop();
    drop(stream);
    drop(subscriber_writer);
    thread.join().unwrap().unwrap();
}

#[test]
fn lagged_direct_requests_keep_the_connection_and_do_not_commit_control() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let (server_id, session_id) = {
        let state = handle.state.lock().unwrap();
        (state.server_id, state.session_id)
    };
    subscribe(&mut stream, session_id, 0);
    let (client_id, subscriber_writer) = {
        let state = handle.state.lock().unwrap();
        let (&client_id, subscriber) = state.subscribers.iter().next().unwrap();
        (client_id, subscriber.writer.clone())
    };

    // Fill the writer until it lags: the backlog collapses into one SubscriptionRejected.
    let blocking_frame = frame_message(&ServerMessage::Error {
        message: "x".repeat(MAX_FRAME_SIZE - 1024),
    })
    .unwrap();
    subscriber_writer.send_reliable(blocking_frame).unwrap();
    thread::sleep(Duration::from_millis(30));
    let queued_frame = frame_message(&ServerMessage::Pong {
        server_id,
        nonce: 1,
        sequence: 0,
    })
    .unwrap();
    let lagged = (0..600).any(|_| {
        subscriber_writer.send_reliable(queued_frame.clone()) == Err(ReliableSendError::Lagged)
    });
    assert!(lagged, "test did not reach the reliable queue bound");
    handle.state.lock().unwrap().subscribers.remove(&client_id);

    // A direct request while lagged is answered by nothing, but the connection survives and
    // the grant is not committed because its response never reached the client.
    condr_core::protocol::write_message(&mut stream, &ClientMessage::AcquireControl { session_id })
        .unwrap();
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Ping {
            server_id,
            nonce: 9,
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::Error { .. }
    ));
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::SubscriptionRejected { .. }
    ));

    // Recovery: Bootstrap reopens the queue, then subscribe and control work normally.
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::SnapshotRequest { session_id },
    )
    .unwrap();
    let ServerMessage::Bootstrap(header) =
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap()
    else {
        panic!("expected recovery Bootstrap header");
    };
    let batch_count = header.batch_count;
    let mut assembler = BootstrapAssembler::new(header).unwrap();
    for _ in 0..batch_count {
        let ServerMessage::BootstrapBatch(batch) =
            condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap()
        else {
            panic!("expected recovery Bootstrap batch");
        };
        assembler.push(batch).unwrap();
    }
    let bootstrap = assembler.finish().unwrap();
    // Inbound requests are handled in order on one connection, so once the Bootstrap has
    // arrived the earlier (lagged) AcquireControl was processed: it must not have committed.
    assert_eq!(handle.state.lock().unwrap().active_controller, None);
    subscribe(&mut stream, session_id, bootstrap.sequence);
    acquire_control(&mut stream, session_id);
    assert_eq!(
        handle.state.lock().unwrap().active_controller,
        Some(client_id)
    );

    handle.stop();
    drop(stream);
    drop(subscriber_writer);
    thread.join().unwrap().unwrap();
}

#[test]
fn rejected_snapshot_is_typed_and_keeps_the_connection_usable() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let (server_id, session_id) = {
        let state = handle.state.lock().unwrap();
        (state.server_id, state.session_id)
    };

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::SnapshotRequest {
            session_id: SessionId(session_id.0.wrapping_add(1)),
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut stream),
        ServerMessage::SnapshotRejected {
            server_id: rejected_server,
            session_id: authoritative_session,
            reason,
        } if rejected_server == server_id
            && authoritative_session == session_id
            && reason == "unknown Session"
    ));

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::SnapshotRequest { session_id },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut stream),
        ServerMessage::Bootstrap(BootstrapHeader {
            settings: _,
            server_id: bootstrap_server,
            session_id: bootstrap_session,
            ..
        }) if bootstrap_server == server_id && bootstrap_session == session_id
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn controller_is_exclusive_and_released_on_disconnect() {
    let (handle, endpoint, thread) = start();
    let mut first = connect_and_bootstrap(&endpoint);
    let mut second = connect_and_bootstrap(&endpoint);
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    acquire_control(&mut first, session_id);
    condr_core::protocol::write_message(&mut second, &ClientMessage::AcquireControl { session_id })
        .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut second).unwrap(),
        ServerMessage::ControlDenied { .. }
    ));
    condr_core::protocol::write_message(
        &mut second,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 73,
            command: LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            },
        },
    )
    .unwrap();
    // Control is exclusive, but layout is not gated on it: the denied client still
    // changes structure, as the CLI in a Pane does while the GUI holds control.
    // The response follows the events the change raised on this subscribed stream.
    let applied = std::iter::from_fn(|| {
        Some(condr_core::protocol::read_message::<_, ServerMessage>(&mut second).unwrap())
    })
    .find(|message| !matches!(message, ServerMessage::Event { .. }))
    .unwrap();
    assert!(
        matches!(
            applied,
            ServerMessage::LayoutApplied {
                server_id: applied_server,
                session_id: applied_session,
                request_id: 73,
                ..
            } if applied_server == server_id && applied_session == session_id
        ),
        "{applied:?}"
    );
    drop(first);
    thread::sleep(Duration::from_millis(20));
    acquire_control(&mut second, session_id);
    handle.stop();
    drop(second);
    thread.join().unwrap().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn releasing_control_releases_reported_mouse_before_focus() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    acquire_control(&mut stream, session_id);
    subscribe(&mut stream, session_id, 0);

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            },
        },
    )
    .unwrap();
    let message = wait_for_message(&mut stream, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event { sequence, .. } = message else {
        unreachable!("predicate only accepts LayoutChanged events");
    };
    assert_layout_applied(&mut stream, server_id, session_id, 1, sequence);
    let pane_id = handle
        .state
        .lock()
        .unwrap()
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    let mut views = std::collections::HashMap::new();
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Resize(TerminalSize::new(8, 160)),
    );
    wait_for_terminal(&mut stream, &mut views, pane_id, |view| {
        view.size.columns == 160
    });
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text(
            "stty raw -echo; printf 'controller-cleanup-ready\\r\\n\\033[?1002h\\033[?1006h\\033[?1004h'; bytes=$(dd bs=1 count=36 2>/dev/null | od -An -tx1 | tr -d ' \\n'); printf '\\033[?1002l\\033[?1006l\\033[?1004l'; stty sane; printf '\\r\\ncontroller-cleanup-bytes_%s\\r\\n' \"$bytes\"\r"
                .into(),
        ),
    );
    wait_for_terminal(&mut stream, &mut views, pane_id, |view| {
        view.mouse_tracking == condr_core::TerminalMouseTracking::Drag
            && view_text(view).contains("controller-cleanup-ready")
    });

    let press_position = TerminalMousePosition { row: 3, column: 7 };
    let motion_position = TerminalMousePosition { row: 4, column: 9 };
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Focus(true),
    );
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Mouse(TerminalMouseEvent::Button {
            button: TerminalMouseButton::Left,
            pressed: true,
            position: press_position,
            modifiers: TerminalModifiers::default(),
        }),
    );
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Mouse(TerminalMouseEvent::Motion {
            button: Some(TerminalMouseButton::Left),
            position: motion_position,
            modifiers: TerminalModifiers {
                alt: true,
                ..TerminalModifiers::default()
            },
        }),
    );
    condr_core::protocol::write_message(&mut stream, &ClientMessage::ReleaseControl { session_id })
        .unwrap();
    assert!(matches!(
        wait_for_message(&mut stream, |message| matches!(
            message,
            ServerMessage::ControlReleased { .. }
        )),
        ServerMessage::ControlReleased {
            server_id: released_server,
            session_id: released_session,
        } if released_server == server_id && released_session == session_id
    ));

    let expected = "controller-cleanup-bytes_1b5b491b5b3c303b383b344d1b5b3c34303b31303b354d1b5b3c383b31303b356d1b5b4f";
    wait_for_terminal_text(&mut stream, &mut views, pane_id, expected);

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn snapshot_change_is_replayed_after_the_bootstrap_cursor() {
    let (handle, endpoint, thread) = start();
    let mut first = connect_and_bootstrap(&endpoint);
    let mut second = connect_and_bootstrap(&endpoint);
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;

    acquire_control(&mut first, session_id);
    condr_core::protocol::write_message(
        &mut first,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            },
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut first).unwrap(),
        ServerMessage::Event {
            server_id: event_server,
            session_id: event_session,
            sequence: 1,
            event: SessionEvent::LayoutChanged { .. },
        } if event_server == server_id && event_session == session_id
    ));
    assert_layout_applied(&mut first, server_id, session_id, 1, 1);
    condr_core::protocol::write_message(&mut first, &ClientMessage::ReleaseControl { session_id })
        .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut first).unwrap(),
        ServerMessage::ControlReleased {
            server_id: released_server,
            session_id: released_session,
        } if released_server == server_id && released_session == session_id
    ));

    condr_core::protocol::write_message(
        &mut second,
        &ClientMessage::Subscribe {
            session_id,
            after_sequence: 0,
        },
    )
    .unwrap();
    let subscribed_sequence = loop {
        match condr_core::protocol::read_message::<_, ServerMessage>(&mut second).unwrap() {
            ServerMessage::Event {
                sequence: 1,
                event: SessionEvent::LayoutChanged { .. },
                ..
            } => {}
            ServerMessage::Event { .. } => {}
            ServerMessage::Subscribed {
                server_id: subscribed_server,
                session_id: subscribed_session,
                sequence,
            } => {
                assert_eq!(subscribed_server, server_id);
                assert_eq!(subscribed_session, session_id);
                break sequence;
            }
            other => panic!("unexpected replay response: {other:?}"),
        }
    };
    condr_core::protocol::write_message(
        &mut second,
        &ClientMessage::SnapshotRequest { session_id },
    )
    .unwrap();
    let bootstrap = loop {
        let message = condr_core::protocol::read_message(&mut second).unwrap();
        if matches!(message, ServerMessage::Bootstrap(_)) {
            break message;
        }
    };
    assert!(matches!(
        bootstrap,
        ServerMessage::Bootstrap(bootstrap)
            if bootstrap.sequence >= subscribed_sequence
                && bootstrap.snapshot != Session::new().snapshot()
    ));

    handle.stop();
    drop(first);
    drop(second);
    thread.join().unwrap().unwrap();
}

#[test]
fn subscribed_client_receives_future_events_in_sequence_order() {
    let (handle, endpoint, thread) = start();
    let mut controller = connect_and_bootstrap(&endpoint);
    let mut subscriber = connect_and_bootstrap(&endpoint);
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;

    condr_core::protocol::write_message(
        &mut subscriber,
        &ClientMessage::Subscribe {
            session_id,
            after_sequence: 0,
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut subscriber).unwrap(),
        ServerMessage::Subscribed { sequence: 0, .. }
    ));

    acquire_control(&mut controller, session_id);

    let mut snapshot_sequences = Vec::new();
    for request_id in 1..=2 {
        condr_core::protocol::write_message(
            &mut controller,
            &ClientMessage::Layout {
                server_id,
                session_id,
                request_id,
                command: LayoutCommand::CreateWorkspace {
                    root_directory: std::env::temp_dir(),
                },
            },
        )
        .unwrap();
        loop {
            match condr_core::protocol::read_message::<_, ServerMessage>(&mut controller).unwrap() {
                ServerMessage::Event {
                    sequence,
                    event: SessionEvent::LayoutChanged { .. },
                    ..
                } => {
                    snapshot_sequences.push(sequence);
                    assert_layout_applied(
                        &mut controller,
                        server_id,
                        session_id,
                        request_id,
                        sequence,
                    );
                    break;
                }
                ServerMessage::TerminalFrame(_) => {}
                other => panic!("unexpected mutation response: {other:?}"),
            }
        }
    }

    let mut previous_sequence = 0;
    let mut replayed_snapshots = Vec::new();
    while replayed_snapshots.len() < snapshot_sequences.len() {
        match condr_core::protocol::read_message::<_, ServerMessage>(&mut subscriber).unwrap() {
            ServerMessage::Event {
                sequence, event, ..
            } => {
                assert_eq!(sequence, previous_sequence + 1);
                previous_sequence = sequence;
                if matches!(event, SessionEvent::LayoutChanged { .. }) {
                    replayed_snapshots.push(sequence);
                }
            }
            ServerMessage::TerminalFrame(_) => {}
            other => panic!("unexpected subscriber response: {other:?}"),
        }
    }
    assert_eq!(replayed_snapshots, snapshot_sequences);

    handle.stop();
    drop(controller);
    drop(subscriber);
    thread.join().unwrap().unwrap();
}

#[test]
fn stop_message_ends_server_and_preserves_session_handle() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::StopServer {
            server_id: handle.server_id(),
        },
    )
    .unwrap();
    assert_eq!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::ServerStopping
    );
    drop(stream);
    thread.join().unwrap().unwrap();
    assert_eq!(handle.snapshot(), Session::new().snapshot());
}

#[test]
fn stop_message_ends_server_when_the_requesting_client_is_not_reading() {
    let (handle, endpoint, server_thread) = start();
    let connection = ClientConnection::connect(&endpoint, "blocked-stop-writer").unwrap();
    let session_id = connection.bootstrap().session_id;
    let mut stream = connection.into_stream();
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    subscribe(&mut stream, session_id, 0);

    let subscriber_writer = {
        let state = handle.state.lock().unwrap();
        assert_eq!(state.subscribers.len(), 1);
        state.subscribers.values().next().unwrap().writer.clone()
    };
    subscriber_writer
        .send_reliable(vec![0; 32 * 1024 * 1024])
        .unwrap();
    subscriber_writer.send_reliable(vec![0]).unwrap();
    thread::sleep(Duration::from_millis(30));

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::StopServer {
            server_id: handle.server_id(),
        },
    )
    .unwrap();

    let (finished_tx, finished_rx) = mpsc::sync_channel(1);
    let joiner = thread::spawn(move || {
        let _ = finished_tx.send(server_thread.join().unwrap());
    });
    let result = finished_rx
        .recv_timeout(STOP_ACK_TIMEOUT + Duration::from_secs(2))
        .expect("Server stop waited indefinitely for a non-reading client");
    result.unwrap();

    drop(stream);
    drop(subscriber_writer);
    joiner.join().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn stop_server_cancels_resize_queued_behind_pty_backpressure() {
    let (handle, endpoint, server_thread) = start();
    let connection = ClientConnection::connect(&endpoint, "blocked-resize").unwrap();
    let bootstrap = connection.bootstrap().clone();
    let server_id = bootstrap.server_id;
    let session_id = bootstrap.session_id;
    let mut controller = connection.into_stream();
    controller
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    acquire_control(&mut controller, session_id);
    subscribe(&mut controller, session_id, bootstrap.sequence);
    condr_core::protocol::write_message(
        &mut controller,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            },
        },
    )
    .unwrap();
    let message = wait_for_message(&mut controller, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event { sequence, .. } = message else {
        unreachable!("predicate only accepts LayoutChanged events");
    };
    assert_layout_applied(&mut controller, server_id, session_id, 1, sequence);
    let pane_id = handle
        .state
        .lock()
        .unwrap()
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();

    let stopper_connection = ClientConnection::connect(&endpoint, "blocked-resize-stop").unwrap();
    let mut stopper = stopper_connection.into_stream();
    stopper
        .set_handshake_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    send_terminal(
        &mut controller,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text(
            "stty raw -echo; printf 'condr-writer-blocked\\r\\n'; sleep 30\r".into(),
        ),
    );
    let mut views = std::collections::HashMap::new();
    wait_for_terminal_text(&mut controller, &mut views, pane_id, "condr-writer-blocked");
    send_terminal(
        &mut controller,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text("x".repeat(1024 * 1024)),
    );
    send_terminal(
        &mut controller,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Resize(TerminalSize::new(10, 40)),
    );
    thread::sleep(Duration::from_millis(100));

    condr_core::protocol::write_message(&mut stopper, &ClientMessage::StopServer { server_id })
        .unwrap();
    assert_eq!(read_server(&mut stopper), ServerMessage::ServerStopping);
    drop(controller);
    drop(stopper);

    let deadline = Instant::now() + Duration::from_secs(2);
    while !server_thread.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        server_thread.is_finished(),
        "Server stop waited for a blocked Terminal resize"
    );
    server_thread.join().unwrap().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stop_message_waits_for_an_inflight_worktree_to_roll_back() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-stop-worktree-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let repository = temp.join("repository");
    let marker = temp.join("checkout-started");
    let release = temp.join("release-checkout");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init"]);
    run_git(&repository, &["config", "user.name", "Condr Tests"]);
    run_git(
        &repository,
        &["config", "user.email", "condr@example.invalid"],
    );
    std::fs::write(repository.join("README.md"), "condr\n").unwrap();
    run_git(&repository, &["add", "README.md"]);
    run_git(&repository, &["commit", "-m", "initial"]);

    let shell_path = |path: &std::path::Path| path.to_string_lossy().replace('\\', "/");
    let hook = repository.join(".git").join("hooks").join("post-checkout");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\nprintf started > '{}'\nwhile [ ! -f '{}' ]; do sleep 0.05; done\n",
            shell_path(&marker),
            shell_path(&release)
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook, permissions).unwrap();
    }

    let (handle, endpoint, thread) = start();
    handle.state.lock().unwrap().worktree_root = Some(temp.join("worktrees"));
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    let mut controller = connect_and_bootstrap(&endpoint);
    acquire_control(&mut controller, session_id);
    condr_core::protocol::write_message(
        &mut controller,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                root_directory: repository.clone(),
            },
        },
    )
    .unwrap();
    // The applied sequence is the LayoutChanged event's own; a Git or agent probe may
    // already have moved the Server's sequence on by the time this thread looks.
    let message = wait_for_message(&mut controller, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event { sequence, .. } = message else {
        unreachable!("predicate only accepts LayoutChanged events");
    };
    assert_layout_applied(&mut controller, server_id, session_id, 1, sequence);
    let parent_workspace_id = handle
        .state
        .lock()
        .unwrap()
        .session
        .active_workspace_id()
        .unwrap();
    condr_core::protocol::write_message(
        &mut controller,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 2,
            command: LayoutCommand::CreateWorktree {
                parent_workspace_id,
                branch: "feature/stopping".into(),
            },
        },
    )
    .unwrap();

    let checkout_deadline = Instant::now() + Duration::from_secs(10);
    let checkout_started = loop {
        if marker.exists() {
            break true;
        }
        if Instant::now() >= checkout_deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert!(checkout_started, "Git checkout hook did not start");

    let mut stopper = connect_and_bootstrap(&endpoint);
    condr_core::protocol::write_message(&mut stopper, &ClientMessage::StopServer { server_id })
        .unwrap();
    assert_eq!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stopper).unwrap(),
        ServerMessage::ServerStopping
    );
    let stop_signalled = (0..100).any(|_| {
        if handle.stop.load(Ordering::Acquire) {
            true
        } else {
            thread::sleep(Duration::from_millis(5));
            false
        }
    });
    assert!(stop_signalled);
    thread::sleep(Duration::from_millis(50));
    assert!(!thread.is_finished());
    std::fs::write(&release, "release\n").unwrap();
    drop(controller);
    drop(stopper);
    thread.join().unwrap().unwrap();

    let child_root = temp
        .join("worktrees")
        .join("repository")
        .join("feature-stopping");
    assert!(!child_root.exists());
    assert_eq!(handle.state.lock().unwrap().session.workspaces().len(), 1);
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
fn tcp_endpoint_uses_the_same_handshake_and_bootstrap() {
    let server = BoundServer::bind(ServerConfig::ephemeral(Endpoint::tcp(
        "127.0.0.1:0".parse().unwrap(),
    )))
    .unwrap();
    let handle = server.handle();
    let address = server.local_addr().unwrap().unwrap();
    let endpoint = Endpoint::tcp(address);
    let thread = thread::spawn(move || server.run());
    let mut stream = connect_and_bootstrap(&endpoint);
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::StopServer {
            server_id: handle.server_id(),
        },
    )
    .unwrap();
    assert_eq!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::ServerStopping
    );
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn tcp_reconnect_bootstraps_authoritative_agent_and_git_state() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-tcp-state-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let repository = temp.join("repository");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init"]);
    run_git(&repository, &["config", "user.name", "Condr Tests"]);
    run_git(
        &repository,
        &["config", "user.email", "condr@example.invalid"],
    );
    std::fs::write(repository.join("README.md"), "condr\n").unwrap();
    run_git(&repository, &["add", "README.md"]);
    run_git(&repository, &["commit", "-m", "initial"]);
    run_git(&repository, &["branch", "-M", "main"]);

    let server = BoundServer::bind(ServerConfig::ephemeral(Endpoint::tcp(
        "127.0.0.1:0".parse().unwrap(),
    )))
    .unwrap();
    let handle = server.handle();
    let endpoint = Endpoint::tcp(server.local_addr().unwrap().unwrap());
    let thread = thread::spawn(move || server.run());

    let connection = ClientConnection::connect(&endpoint, "tcp-controller").unwrap();
    let bootstrap = connection.bootstrap().clone();
    let server_id = bootstrap.server_id;
    let session_id = bootstrap.session_id;
    let mut stream = connection.into_stream();
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut stream, session_id);
    subscribe(&mut stream, session_id, bootstrap.sequence);
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                root_directory: repository.clone(),
            },
        },
    )
    .unwrap();
    let message = wait_for_message(&mut stream, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event { sequence, .. } = message else {
        unreachable!("predicate only accepts LayoutChanged events");
    };
    assert_layout_applied(&mut stream, server_id, session_id, 1, sequence);
    let (workspace_id, pane_id) = {
        let state = handle.state.lock().unwrap();
        let workspace = state.session.active_workspace().unwrap();
        (workspace.id(), workspace.active_tab().focused_pane().id())
    };
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text(
            "exec -a codex /bin/bash -c \"echo '◦ Working (1s - esc to interrupt)'; sleep 30 & wait\"\r"
                .into(),
        ),
    );
    wait_for_message(&mut stream, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::AgentChanged {
                    pane_id: event_pane,
                    agent: Some(AgentSnapshot {
                        kind: condr_core::AgentKind::Codex,
                        state: condr_core::AgentState::Working,
                    }),
                },
                ..
            } if *event_pane == pane_id
        )
    });

    let reconnect = ClientConnection::connect(&endpoint, "tcp-reconnect").unwrap();
    assert!(reconnect.bootstrap().agents.iter().any(|agent| {
        agent.pane_id == pane_id && agent.agent.kind == condr_core::AgentKind::Codex
    }));
    assert!(
        reconnect.bootstrap().workspace_git.iter().any(|git| {
            git.workspace_id == workspace_id && git.branch.as_deref() == Some("main")
        })
    );

    handle.stop();
    drop(reconnect);
    drop(stream);
    thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(temp);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn server_settings_are_stored_published_and_reloaded() {
    let directory = std::env::temp_dir().join(format!(
        "condr-server-settings-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let config_path = directory.join("config.toml");
    std::fs::write(&config_path, "# keep me\n[client]\nappearance = \"dark\"\n").unwrap();
    let endpoint = test_endpoint();
    let server =
        BoundServer::bind(ServerConfig::ephemeral(endpoint.clone()).with_config_path(&config_path))
            .unwrap();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);

    let initial = ClientConnection::connect(&endpoint, "test").unwrap();
    let server_id = initial.bootstrap().server_id;
    let session_id = initial.bootstrap().session_id;
    let sequence = initial.bootstrap().sequence;
    assert_eq!(initial.bootstrap().settings.shell, "");
    assert!(
        !initial.bootstrap().settings.default_shell.is_empty(),
        "the Bootstrap names the system default shell"
    );
    drop(initial);

    let mut stream = connect_and_bootstrap(&endpoint);
    subscribe(&mut stream, session_id, sequence);
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::SetServerSettings {
            server_id,
            shell: "  nu ".into(),
        },
    )
    .unwrap();
    let message = wait_for_message(&mut stream, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::ServerSettingsChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event {
        event: SessionEvent::ServerSettingsChanged { settings },
        ..
    } = message
    else {
        unreachable!()
    };
    assert_eq!(settings.shell, "nu", "the Server stores the trimmed value");

    let text = std::fs::read_to_string(&config_path).unwrap();
    assert!(text.starts_with("# keep me\n"), "comments survive: {text}");
    assert!(
        text.contains("appearance = \"dark\""),
        "other keys survive: {text}"
    );
    assert!(
        text.contains("shell = \"nu\""),
        "the shell is written: {text}"
    );

    let reconnected = ClientConnection::connect(&endpoint, "test").unwrap();
    assert_eq!(reconnected.bootstrap().settings.shell, "nu");
    drop(reconnected);
    drop(stream);
    handle.stop();
    thread.join().unwrap().unwrap();

    // A fresh Server reads the preference back from the file.
    let endpoint = test_endpoint();
    let server =
        BoundServer::bind(ServerConfig::ephemeral(endpoint.clone()).with_config_path(&config_path))
            .unwrap();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);
    let restarted = ClientConnection::connect(&endpoint, "test").unwrap();
    assert_eq!(
        restarted.bootstrap().settings.shell,
        "nu",
        "a restarted Server reads the file back: {}",
        std::fs::read_to_string(&config_path).unwrap()
    );
    drop(restarted);
    handle.stop();
    thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(directory);
}
