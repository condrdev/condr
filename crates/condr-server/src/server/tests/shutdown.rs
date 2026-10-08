use super::*;

#[test]
fn stop_message_ends_server_when_the_requesting_client_is_not_reading() {
    let (handle, endpoint, server_thread) = start();
    let connection = ClientConnection::connect(&endpoint, "blocked-stop-writer").unwrap();
    let session_id = connection.bootstrap().unwrap().session_id;
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
    let bootstrap = connection.bootstrap().unwrap().clone();
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
                name: None,
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
    let pane_id = handle.state.lock().unwrap().session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
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
struct InflightWorktree {
    temp: PathBuf,
    release: PathBuf,
    handle: ServerHandle,
    endpoint: Endpoint,
    server_thread: Option<thread::JoinHandle<io::Result<()>>>,
    controller: Option<EndpointStream>,
    server_id: ServerId,
    session_id: SessionId,
    parent_workspace_id: WorkspaceId,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl Drop for InflightWorktree {
    fn drop(&mut self) {
        // Release the child even after an assertion fails, so a regression cannot
        // leave the smudge process and Server running indefinitely in the test suite.
        let _ = std::fs::write(&self.release, "release\n");
        self.handle.stop();
        if let Some(thread) = self.server_thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn inflight_worktree() -> InflightWorktree {
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
    // gix runs no hooks, but it does run the smudge filter of a checked-out file: that is
    // the point where the worktree creation can be held in flight.
    let shell_path = |path: &std::path::Path| path.to_string_lossy().replace('\\', "/");
    let smudge = temp.join("slow-smudge.sh");
    std::fs::write(
        &smudge,
        format!(
            "#!/bin/sh\nprintf started > '{}'\nwhile [ ! -f '{}' ]; do sleep 0.05; done\ncat\n",
            shell_path(&marker),
            shell_path(&release)
        ),
    )
    .unwrap();
    std::fs::write(repository.join(".gitattributes"), "README.md filter=slow\n").unwrap();
    std::fs::write(repository.join("README.md"), "condr\n").unwrap();
    run_git(&repository, &["add", ".gitattributes", "README.md"]);
    run_git(&repository, &["commit", "-m", "initial"]);
    run_git(
        &repository,
        &[
            "config",
            "filter.slow.smudge",
            &format!("sh '{}'", shell_path(&smudge)),
        ],
    );

    let (handle, endpoint, thread) = start();
    handle.state.lock().unwrap().worktree_root = Some(temp.join("worktrees"));
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    let mut controller = connect_and_bootstrap(&endpoint);
    controller
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut controller, session_id);
    condr_core::protocol::write_message(
        &mut controller,
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
    let parent_workspace_id = handle.state.lock().unwrap().session.workspaces()[0].id();
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
    if !checkout_started {
        std::fs::write(&release, "release\n").unwrap();
        handle.stop();
        thread.join().unwrap().unwrap();
        panic!("the smudge filter did not start");
    }

    InflightWorktree {
        temp,
        release,
        handle,
        endpoint,
        server_thread: Some(thread),
        controller: Some(controller),
        server_id,
        session_id,
        parent_workspace_id,
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stop_message_waits_for_an_inflight_worktree_to_roll_back() {
    let mut test = inflight_worktree();
    // Stop on the same connection that owns the checkout: it must not sit behind Git.
    condr_core::protocol::write_message(
        test.controller.as_mut().unwrap(),
        &ClientMessage::StopServer {
            server_id: test.server_id,
        },
    )
    .unwrap();
    assert_eq!(
        read_server(test.controller.as_mut().unwrap()),
        ServerMessage::ServerStopping
    );
    let stop_signalled = (0..100).any(|_| {
        if test.handle.stop.load(Ordering::Acquire) {
            true
        } else {
            thread::sleep(Duration::from_millis(5));
            false
        }
    });
    assert!(stop_signalled);
    thread::sleep(Duration::from_millis(50));
    assert!(!test.server_thread.as_ref().unwrap().is_finished());
    std::fs::write(&test.release, "release\n").unwrap();
    test.server_thread.take().unwrap().join().unwrap().unwrap();

    let child_root = test
        .temp
        .join("worktrees")
        .join("repository")
        .join("feature-stopping");
    assert!(!child_root.exists());
    assert_eq!(
        test.handle.state.lock().unwrap().session.workspaces().len(),
        1
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn inflight_worktree_keeps_input_and_ping_responsive_and_rejects_more_layout() {
    let mut test = inflight_worktree();
    condr_core::protocol::write_message(
        test.controller.as_mut().unwrap(),
        &ClientMessage::Ping {
            server_id: test.server_id,
            nonce: 41,
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(test.controller.as_mut().unwrap()),
        ServerMessage::Pong { nonce: 41, .. }
    ));

    let (pane_id, sequence) = {
        let state = test.handle.state.lock().unwrap();
        (
            state.session.workspaces()[0].tabs()[0]
                .focused_pane()
                .unwrap()
                .id(),
            state.sequence,
        )
    };
    subscribe(test.controller.as_mut().unwrap(), test.session_id, sequence);
    let mut views = std::collections::HashMap::new();
    // Start consuming the Full baseline before any test helper skips visual messages.
    send_terminal(
        test.controller.as_mut().unwrap(),
        test.server_id,
        test.session_id,
        pane_id,
        TerminalCommand::Text("echo condr-checkout-responsive\r".into()),
    );
    wait_for_terminal_text(
        test.controller.as_mut().unwrap(),
        &mut views,
        pane_id,
        "condr-checkout-responsive",
    );

    for (request_id, command) in [
        (
            3,
            LayoutCommand::CloseWorkspace {
                workspace_id: test.parent_workspace_id,
            },
        ),
        (
            4,
            LayoutCommand::CreateWorktree {
                parent_workspace_id: test.parent_workspace_id,
                branch: "feature/rejected".into(),
            },
        ),
    ] {
        condr_core::protocol::write_message(
            test.controller.as_mut().unwrap(),
            &ClientMessage::Layout {
                server_id: test.server_id,
                session_id: test.session_id,
                request_id,
                command,
            },
        )
        .unwrap();
        let response = wait_for_message(test.controller.as_mut().unwrap(), |message| {
            matches!(message, ServerMessage::LayoutRejected { .. })
        });
        assert!(
            matches!(response, ServerMessage::LayoutRejected { request_id: actual, reason, .. }
            if actual == request_id && reason.contains("in progress"))
        );
    }
    assert!(!test.release.exists());
    assert_eq!(
        test.handle.state.lock().unwrap().session.workspaces().len(),
        1
    );

    std::fs::write(&test.release, "release\n").unwrap();
    let event = wait_for_message(test.controller.as_mut().unwrap(), |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event { sequence, .. } = event else {
        unreachable!()
    };
    assert_layout_applied(
        test.controller.as_mut().unwrap(),
        test.server_id,
        test.session_id,
        2,
        sequence,
    );
    // Receiving success releases the connection's slot; no retry or extra Ping needed.
    condr_core::protocol::write_message(
        test.controller.as_mut().unwrap(),
        &ClientMessage::Layout {
            server_id: test.server_id,
            session_id: test.session_id,
            request_id: 5,
            command: LayoutCommand::RenameWorkspace {
                workspace_id: test.parent_workspace_id,
                name: "after-checkout".into(),
            },
        },
    )
    .unwrap();
    let event = wait_for_message(test.controller.as_mut().unwrap(), |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event { sequence, .. } = event else {
        unreachable!()
    };
    assert_layout_applied(
        test.controller.as_mut().unwrap(),
        test.server_id,
        test.session_id,
        5,
        sequence,
    );
    assert_eq!(
        test.handle.state.lock().unwrap().session.workspaces().len(),
        2
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn failed_worktree_terminal_start_rolls_back_checkout_and_registration() {
    let mut test = inflight_worktree();
    test.handle.state.lock().unwrap().settings.shell = test
        .temp
        .join("missing-shell")
        .to_string_lossy()
        .into_owned();
    std::fs::write(&test.release, "release\n").unwrap();
    let response = read_layout_response(test.controller.as_mut().unwrap());
    assert!(matches!(
        response,
        ServerMessage::LayoutRejected { request_id: 2, reason, .. }
            if reason.contains("failed to start terminal")
    ));
    let state = test.handle.state.lock().unwrap();
    assert_eq!(state.session.workspaces().len(), 1);
    assert_eq!(state.terminals.len(), 1);
    drop(state);
    assert!(
        !test
            .temp
            .join("worktrees/repository/feature-stopping")
            .exists()
    );
    let registrations = test.temp.join("repository/.git/worktrees");
    if registrations.exists() {
        assert_eq!(std::fs::read_dir(registrations).unwrap().count(), 0);
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn disconnect_during_worktree_creation_releases_control_and_starts_its_terminal() {
    let mut test = inflight_worktree();
    // Local sockets close on drop; shutdown() only closes TCP/SSH/Peer-to-peer.
    drop(test.controller.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    while test
        .handle
        .state
        .lock()
        .unwrap()
        .active_controller
        .is_some()
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        test.handle
            .state
            .lock()
            .unwrap()
            .active_controller
            .is_none()
    );
    assert!(!test.release.exists());
    std::fs::write(&test.release, "release\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while test.handle.lifecycle.state.lock().unwrap().operations != 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(test.handle.lifecycle.state.lock().unwrap().operations, 0);
    let pane_id = {
        let state = test.handle.state.lock().unwrap();
        assert_eq!(state.session.workspaces().len(), 2);
        state.session.workspaces()[1].tabs()[0]
            .focused_pane()
            .unwrap()
            .id()
    };
    let connection = ClientConnection::connect(&test.endpoint, "checkout-observer").unwrap();
    let bootstrap = connection.bootstrap().unwrap();
    let sequence = bootstrap.sequence;
    let mut views = bootstrap
        .terminals
        .iter()
        .map(|terminal| (terminal.pane_id, terminal.view.clone()))
        .collect();
    let mut observer = connection.into_stream();
    observer
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut observer, test.session_id);
    subscribe(&mut observer, test.session_id, sequence);
    send_terminal(
        &mut observer,
        test.server_id,
        test.session_id,
        pane_id,
        TerminalCommand::Text("echo condr-disconnected-checkout\r".into()),
    );
    wait_for_terminal_text(
        &mut observer,
        &mut views,
        pane_id,
        "condr-disconnected-checkout",
    );
    test.handle.stop();
    test.server_thread.take().unwrap().join().unwrap().unwrap();
    assert_eq!(test.handle.lifecycle.state.lock().unwrap().operations, 0);
}
