use super::*;

fn rejects_invalid_model(as_bootstrap: bool) {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection_mut(1).unwrap();
            let mut session = connection.session().unwrap().clone();
            session.create_workspace(std::env::temp_dir()).unwrap();
            connection
                .apply_layout(session.snapshot(), Vec::new())
                .unwrap();
            let snapshot = session.snapshot();
            let generation = connection.connect_generation;
            let server_id = connection.server_id.unwrap();
            let runtime_epoch = connection.runtime_epoch.unwrap();
            let session_id = connection.session_id.unwrap();
            let sequence = connection.sequence;
            connection.reconnect_deadline = Some(Instant::now() + TEST_TIMEOUT);
            let invalid = crate::app::tests::session_model::duplicate_workspaces(&snapshot);
            let incoming = if as_bootstrap {
                Incoming::Bootstrap(SessionBootstrap {
                    settings: Default::default(),
                    server_id,
                    runtime_epoch,
                    session_id,
                    sequence: sequence + 1,
                    snapshot: invalid,
                    terminals: Vec::new(),
                    agents: Vec::new(),
                    workspace_git: Vec::new(),
                    zoomed_panes: Vec::new(),
                })
            } else {
                Incoming::Message(ServerMessage::Event {
                    server_id,
                    session_id,
                    sequence: sequence + 1,
                    event: SessionEvent::LayoutChanged {
                        snapshot: invalid,
                        zoomed_panes: Vec::new(),
                    },
                })
            };
            this.sync_sidebar_workspace_open(cx);
            this.schedule_reconnect(1, cx);
            let effect = this.handle_incoming(1, generation, incoming, cx);
            let connection = this.connection(1).unwrap();
            assert!(effect.notify);
            assert!(connection.status == ConnectionStatus::Disconnected);
            assert!(
                connection
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("invalid Session snapshot")
            );
            assert_eq!(connection.session().unwrap().snapshot(), snapshot);
            assert_eq!(connection.sequence, sequence);
            assert!(connection.io.is_none());
            assert!(connection.reconnect_deadline.is_none());
            assert!(connection.bootstrap_resync_session_id.is_none());
        });
    });
    window.run_until_parked();
    window
        .executor()
        .advance_clock(crate::app::RESTART_RECONNECT_DELAY);
    window.run_until_parked();
    window.read(|app| {
        let connection = view.read(app).connection(1).unwrap();
        assert!(connection.status == ConnectionStatus::Disconnected);
        assert!(connection.reconnect_deadline.is_none());
    });
}

#[test]
fn invalid_layout_disconnects_without_replacing_the_last_valid_model() {
    rejects_invalid_model(false);
}

#[test]
fn invalid_bootstrap_disconnects_without_replacing_the_last_valid_model() {
    rejects_invalid_model(true);
}

/// Uses a real local handshake to exercise the initial Bootstrap, before ClientIo exists.
#[cfg(unix)]
#[test]
fn invalid_initial_bootstrap_stops_pending_reconnects_but_transport_failure_retries() {
    use condr_core::protocol::{
        BootstrapHeader, ClientHandshake, RuntimeEpoch, ServerId, SessionId, Welcome, read_message,
        write_message,
    };
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let directory = TestDirectory::new("invalid-model");
    let path = directory.0.join("invalid.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let endpoint = Endpoint::local(&path);
    let mut session = Session::new();
    session.create_workspace(std::env::temp_dir()).unwrap();
    let invalid = crate::app::tests::session_model::duplicate_workspaces(&session.snapshot());
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _: ClientHandshake = read_message(&mut stream).unwrap();
        write_message(&mut stream, &Welcome::new(ServerId(91), SessionId(1), None)).unwrap();
        let request: ClientMessage = read_message(&mut stream).unwrap();
        assert!(matches!(request, ClientMessage::SnapshotRequest { .. }));
        write_message(
            &mut stream,
            &ServerMessage::Bootstrap(BootstrapHeader {
                server_id: ServerId(91),
                runtime_epoch: RuntimeEpoch(92),
                session_id: SessionId(1),
                sequence: 7,
                snapshot: invalid,
                settings: Default::default(),
                batch_count: 0,
            }),
        )
        .unwrap();
        stream
    });
    let client = ClientConnection::connect(&endpoint, "invalid-model-test").unwrap();
    let _peer = peer.join().unwrap();
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.disconnect_server(1);
            let connection = this.connection_mut(1).unwrap();
            let expected = connection.session().unwrap().snapshot();
            connection.status = ConnectionStatus::Connecting;
            let deadline = Instant::now() + TEST_TIMEOUT;
            connection.reconnect_deadline = Some(deadline);
            let generation = connection.connect_generation;
            this.handle_connection_result(
                ConnectionResult {
                    key: 1,
                    generation,
                    endpoint: endpoint.clone(),
                    result: Err("temporary transport failure".into()),
                    refusal: None,
                },
                window,
                cx,
            );
            let connection = this.connection_mut(1).unwrap();
            assert_eq!(connection.reconnect_deadline, Some(deadline));
            connection.status = ConnectionStatus::Connecting;
            this.handle_connection_result(
                ConnectionResult {
                    key: 1,
                    generation,
                    endpoint,
                    result: Ok(client),
                    refusal: None,
                },
                window,
                cx,
            );
            let connection = this.connection(1).unwrap();
            assert!(connection.status == ConnectionStatus::Disconnected);
            assert!(connection.reconnect_deadline.is_none());
            assert!(
                connection
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("invalid Session snapshot")
            );
            assert_eq!(connection.session().unwrap().snapshot(), expected);
        });
    });
    window.run_until_parked();
    window
        .executor()
        .advance_clock(crate::app::RESTART_RECONNECT_DELAY);
    window.run_until_parked();
    window.read(|app| {
        let connection = view.read(app).connection(1).unwrap();
        assert!(connection.status == ConnectionStatus::Disconnected);
        assert!(
            connection
                .error
                .as_deref()
                .unwrap()
                .contains("invalid Session snapshot")
        );
    });
}
