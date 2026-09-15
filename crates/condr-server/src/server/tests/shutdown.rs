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
                focus: true,
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
    acquire_control(&mut controller, session_id);
    condr_core::protocol::write_message(
        &mut controller,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
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
    assert!(checkout_started, "the smudge filter did not start");

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
