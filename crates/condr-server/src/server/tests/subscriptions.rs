use super::*;

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
                name: None,
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
                    name: None,
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
