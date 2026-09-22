use super::*;
use condr_core::protocol::Hello;

#[test]
fn overview_frames_fit_near_the_snapshot_and_terminal_metadata_limits() {
    let state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let mut overview = SessionOverview::from(&state.bootstrap());
    let mut session = Session::new();
    let workspace = session.create_workspace(std::env::temp_dir()).unwrap();
    session.rename_workspace(workspace, "x".repeat(MAX_PERSISTED_SNAPSHOT_BYTES - 4096));
    overview.snapshot = session.snapshot();
    validate_persistable_snapshot(&overview.snapshot).unwrap();
    overview.terminals = (1..=1024)
        .map(|id| PaneTerminalMetadata {
            pane_id: PaneId::from_u64(id),
            title: Some("\u{1f600}".repeat(256)),
            exited: false,
        })
        .collect();
    assert!(frame_message(&ServerMessage::Overview(overview.clone())).is_err());
    let frames = frame_overview_messages(overview.clone()).unwrap();
    assert_eq!(frames.len(), 2);
    let ServerMessage::Overview(mut decoded) =
        condr_core::protocol::read_message(&mut frames[0].as_slice()).unwrap()
    else {
        panic!("Overview header")
    };
    let ServerMessage::OverviewTerminals { terminals, .. } =
        condr_core::protocol::read_message(&mut frames[1].as_slice()).unwrap()
    else {
        panic!("Overview terminal metadata")
    };
    decoded.terminals = terminals;
    assert_eq!(decoded, overview);
}

#[test]
fn lightweight_queries_and_admin_never_capture_terminal_views() {
    let (handle, endpoint, server_thread) = start();
    let mut client = ClientConnection::connect_overview(&endpoint, "inspect").unwrap();
    let LayoutResult::WorkspaceCreated { pane_id, .. } = client
        .layout(LayoutCommand::CreateWorkspace {
            root_directory: std::env::temp_dir(),
            name: None,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("Workspace result")
    };
    assert!(client.bootstrap().is_none());
    client.read_pane(pane_id, 5).unwrap();
    probe_server(&endpoint).unwrap();
    assert!(connected_devices(&endpoint).unwrap().is_empty());
    assert_eq!(
        revoke_devices(&endpoint, &DeviceKey::generate().unwrap().public()).unwrap(),
        0
    );
    let inspector = ClientConnection::connect_overview(&endpoint, "second inspect").unwrap();
    assert_eq!(inspector.overview().terminals.len(), 1);
    assert_eq!(
        handle
            .state
            .lock()
            .unwrap()
            .bootstrap_captures
            .load(Ordering::Relaxed),
        0
    );
    let gui = ClientConnection::connect(&endpoint, "gui").unwrap();
    assert_eq!(gui.bootstrap().unwrap().terminals.len(), 1);
    assert_eq!(
        handle
            .state
            .lock()
            .unwrap()
            .bootstrap_captures
            .load(Ordering::Relaxed),
        1
    );
    drop((client, inspector, gui));
    stop_server(&endpoint).unwrap();
    server_thread.join().unwrap().unwrap();
    assert_eq!(
        handle
            .state
            .lock()
            .unwrap()
            .bootstrap_captures
            .load(Ordering::Relaxed),
        1
    );
}

#[test]
fn tcp_peers_leave_the_table_when_initial_connections_are_abandoned() {
    let server =
        BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
            .unwrap();
    let endpoint = server.endpoint().clone();
    let handle = server.handle();
    let server_thread = thread::spawn(move || server.run());
    for index in 0..24 {
        let mut stream = endpoint.connect().unwrap();
        condr_core::protocol::write_message(
            &mut stream,
            &ClientHandshake::Hello(Hello::new("abandoned")),
        )
        .unwrap();
        if index % 2 == 0 {
            let welcome: Welcome = condr_core::protocol::read_message(&mut stream).unwrap();
            assert_eq!(welcome.refusal, None);
        }
        stream.shutdown().unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.state.lock().unwrap().tcp_peers.is_empty() {
        assert!(
            Instant::now() < deadline,
            "abandoned TCP peer registration leaked"
        );
        thread::sleep(Duration::from_millis(5));
    }
    handle.stop();
    server_thread.join().unwrap().unwrap();
    assert!(handle.state.lock().unwrap().tcp_peers.is_empty());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn invalid_workspace_roots_preserve_authoritative_layout_focus_and_terminals() {
    let (handle, endpoint, thread) = start();
    let connection = ClientConnection::connect(&endpoint, "failed-workspace").unwrap();
    let bootstrap = connection.bootstrap().unwrap().clone();
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
                name: None,
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
        let workspace = state.session.workspaces().first().unwrap();
        let tab = workspace.tabs().first().unwrap();
        (
            state.session.snapshot(),
            state.sequence,
            (workspace.id(), tab.id(), tab.focused_pane().unwrap().id()),
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
                name: None,
                root_directory: missing_root,
            },
            "cannot access root directory",
        ),
        (
            43,
            LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: PathBuf::from("relative-workspace-root"),
            },
            "must be an absolute path",
        ),
        (
            44,
            LayoutCommand::CreateWorkspace {
                name: None,
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
        let workspace = state.session.workspaces().first().unwrap();
        let tab = workspace.tabs().first().unwrap();
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
                        | SessionEvent::WorkspaceFilesChanged { .. }
                )),
            "a rejected layout command published a layout event"
        );
        assert_eq!(
            (workspace.id(), tab.id(), tab.focused_pane().unwrap().id()),
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

    let server =
        BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
            .unwrap();
    let handle = server.handle();
    let endpoint = server.endpoint().clone();
    let thread = thread::spawn(move || server.run());

    let connection = ClientConnection::connect(&endpoint, "tcp-controller").unwrap();
    let bootstrap = connection.bootstrap().unwrap().clone();
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
                name: None,
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
        let workspace = state.session.workspaces().first().unwrap();
        (
            workspace.id(),
            workspace
                .tabs()
                .first()
                .unwrap()
                .focused_pane()
                .unwrap()
                .id(),
        )
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
                        session_id: None,
                        kind: condr_core::AgentKind::Codex,
                        state: condr_core::AgentState::Unknown,
                        blocked_on: None,
                    }),
                },
                ..
            } if *event_pane == pane_id
        )
    });

    let reconnect = ClientConnection::connect(&endpoint, "tcp-reconnect").unwrap();
    assert!(reconnect.bootstrap().unwrap().agents.iter().any(|agent| {
        agent.pane_id == pane_id && agent.agent.kind == condr_core::AgentKind::Codex
    }));
    assert!(
        reconnect
            .bootstrap()
            .unwrap()
            .workspace_git
            .iter()
            .any(|git| {
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
    let server = BoundServer::bind(
        ServerConfig::ephemeral(endpoint.as_local_path().unwrap()).with_config_path(&config_path),
    )
    .unwrap();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);

    let initial = ClientConnection::connect(&endpoint, "test").unwrap();
    let server_id = initial.bootstrap().unwrap().server_id;
    let session_id = initial.bootstrap().unwrap().session_id;
    let sequence = initial.bootstrap().unwrap().sequence;
    assert_eq!(initial.bootstrap().unwrap().settings.shell, "");
    assert!(
        !initial
            .bootstrap()
            .unwrap()
            .settings
            .default_shell
            .is_empty(),
        "the Bootstrap names the system default shell"
    );
    drop(initial);

    let mut stream = connect_and_bootstrap(&endpoint);
    subscribe(&mut stream, session_id, sequence);
    let config_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("config.toml.lock"))
        .unwrap();
    config_lock.lock().unwrap();
    let settings_write = Arc::clone(&handle.state.lock().unwrap().settings_write);
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::SetServerSettings {
            server_id,
            shell: "  nu ".into(),
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while settings_write.try_lock().is_ok() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let waiting_for_config = settings_write.try_lock().is_err();
    let session_available = handle.state.try_lock().is_ok();
    drop(config_lock);
    assert!(
        waiting_for_config,
        "the setting should wait for the config transaction"
    );
    assert!(
        session_available,
        "disk I/O must not hold the global Session lock"
    );
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
    assert_eq!(reconnected.bootstrap().unwrap().settings.shell, "nu");
    drop(reconnected);
    drop(stream);
    handle.stop();
    thread.join().unwrap().unwrap();

    // A fresh Server reads the preference back from the file.
    let endpoint = test_endpoint();
    let server = BoundServer::bind(
        ServerConfig::ephemeral(endpoint.as_local_path().unwrap()).with_config_path(&config_path),
    )
    .unwrap();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);
    let restarted = ClientConnection::connect(&endpoint, "test").unwrap();
    assert_eq!(
        restarted.bootstrap().unwrap().settings.shell,
        "nu",
        "a restarted Server reads the file back: {}",
        std::fs::read_to_string(&config_path).unwrap()
    );
    drop(restarted);
    handle.stop();
    thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn revoking_a_device_drops_its_live_connections_and_refuses_its_return() {
    use crate::noise::{self, DeviceKey, ServerIdentity};

    let directory = std::env::temp_dir().join(format!(
        "condr-server-revoke-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let identity = ServerIdentity::load_or_create(&directory).unwrap();
    let server_key = identity.public_key();
    let server = BoundServer::bind(
        ServerConfig::ephemeral(directory.join("condr.sock"))
            .with_listen("127.0.0.1:0".parse().unwrap())
            .with_identity(identity),
    )
    .unwrap();
    let handle = server.handle();
    let address = server.local_addr().unwrap().unwrap();
    // Administration happens on the host, over the local socket.
    let host = server.endpoint().clone();
    let thread = thread::spawn(move || server.run());

    // A new device pairs with the invite, then reconnects on its key alone.
    let device_key = DeviceKey::generate().unwrap();
    let device = |invite| {
        let mut tcp = TcpEndpoint::at(address, server_key, device_key.clone());
        tcp.invite = invite;
        Endpoint::tcp(tcp)
    };
    let invite = noise::create_invite(&directory).unwrap();
    let mut first = connect_and_bootstrap(&device(Some(invite.secret.clone())));
    assert_eq!(noise::read_authorized(&directory).unwrap().len(), 1);
    // A Client that kept its invite past a successful pairing (it never saw the
    // Bootstrap) is served as a paired device instead of failing forever.
    drop(ClientConnection::connect(&device(Some(invite.secret)), "stale-invite").unwrap());
    let mut second = connect_and_bootstrap(&device(None));

    // A connection that finished its handshake but has not sent Hello is not in the peer
    // table yet; revoking while it waits must still refuse it once it speaks.
    let mut delayed = device(None).connect().unwrap();
    if let EndpointStream::Tcp(stream) = &mut delayed {
        stream.handshake().unwrap();
    }

    // A paired device is not the host and may not revoke anyone.
    let prefix = device_key.public().to_string()[..12].to_owned();
    condr_core::protocol::write_message(
        &mut second,
        &ClientMessage::RevokeDevice {
            key: device_key.public().to_string(),
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut second),
        ServerMessage::Error { message } if message.contains("only the Server host")
    ));

    // The host sees the device connected, and its last-seen time was written at pairing.
    let mut admin = connect_and_bootstrap(&host);
    condr_core::protocol::write_message(&mut admin, &ClientMessage::ConnectedDevices).unwrap();
    assert!(matches!(
        read_server(&mut admin),
        ServerMessage::ConnectedDevices { keys } if keys == vec![device_key.public().to_string()]
    ));
    let paired = noise::read_authorized(&directory).unwrap();
    assert!(paired[0].last_seen >= paired[0].paired_at);

    // The dropped stale-invite connection leaves the peer table only once the Server reads
    // its EOF; revoking before that would count it too.
    let peers_deadline = Instant::now() + Duration::from_secs(5);
    while handle.state.lock().unwrap().tcp_peers.len() != 2 {
        assert!(
            Instant::now() < peers_deadline,
            "expected the two live device connections in the peer table"
        );
        thread::sleep(Duration::from_millis(10));
    }
    // The host revokes: the file refuses the next handshake, the message drops both
    // live connections.
    assert_eq!(
        noise::revoke(&directory, &prefix).unwrap(),
        Some(device_key.public())
    );
    condr_core::protocol::write_message(
        &mut admin,
        &ClientMessage::RevokeDevice {
            key: device_key.public().to_string(),
        },
    )
    .unwrap();
    let revoked = read_server(&mut admin);
    assert!(
        matches!(revoked, ServerMessage::DevicesRevoked { disconnected: 2 }),
        "{revoked:?}"
    );
    for stream in [&mut first, &mut second] {
        stream
            .set_handshake_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        assert!(condr_core::protocol::read_message::<_, ServerMessage>(stream).is_err());
    }
    // The delayed device now sends Hello: the Server re-reads the list before serving it.
    delayed
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    condr_core::protocol::write_message(
        &mut delayed,
        &ClientHandshake::Hello(Hello::new("delayed")),
    )
    .unwrap();
    let welcome: Welcome = condr_core::protocol::read_message(&mut delayed).unwrap();
    assert_eq!(welcome.refusal, Some(Refusal::DeviceRevoked));
    drop(delayed);
    let error = match ClientConnection::connect(&device(None), "revoked") {
        Ok(_) => panic!("a revoked device reconnected"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

    handle.stop();
    drop((first, second, admin));
    thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(directory);
}
