use super::*;

#[test]
fn invalid_snapshot_inputs_yield_an_empty_session() {
    let directory = std::env::temp_dir().join(format!(
        "condr-server-invalid-snapshots-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&directory).unwrap();

    let valid_empty = Session::new().snapshot().to_bytes().unwrap();
    let mut unsupported = valid_empty.clone();
    unsupported[..std::mem::size_of::<u32>()].copy_from_slice(&3u32.to_le_bytes());
    let structurally_invalid = structurally_invalid_snapshot(directory.clone());
    let relative_root = {
        let mut session = Session::new();
        session
            .create_workspace(PathBuf::from("relative/root"))
            .expect("Workspace capacity");
        session.snapshot().to_bytes().unwrap()
    };
    let relative_parent_root = {
        let parent_root = directory.join("parent");
        let child_root = directory.join("child");
        let mut session = Session::new();
        let parent_id = session
            .create_workspace(parent_root)
            .expect("Workspace capacity");
        let child_id = session
            .create_workspace(child_root)
            .expect("Workspace capacity");
        assert!(session.associate_worktree(
            child_id,
            parent_id,
            PathBuf::from("relative/parent"),
            false,
        ));
        session.snapshot().to_bytes().unwrap()
    };
    let absolute_file_root = {
        let root = directory.join("regular-file-root");
        std::fs::write(&root, b"not a directory").unwrap();
        let mut session = Session::new();
        session.create_workspace(root).expect("Workspace capacity");
        session.snapshot().to_bytes().unwrap()
    };
    let transport_oversized = {
        let mut session = Session::new();
        let workspace_id = session
            .create_workspace(directory.clone())
            .expect("Workspace capacity");
        assert!(session.rename_workspace(workspace_id, "x".repeat(MAX_PERSISTED_SNAPSHOT_BYTES)));
        let bytes = session.snapshot().to_bytes().unwrap();
        assert!(bytes.len() > MAX_PERSISTED_SNAPSHOT_BYTES);
        bytes
    };
    assert!(
        Session::restore(condr_core::SessionSnapshot::from_bytes(&structurally_invalid).unwrap())
            .is_err()
    );
    let cases = [
        ("missing", None),
        ("empty", Some(Vec::new())),
        ("corrupt", Some(vec![0xff, 0x00, 0x7f])),
        (
            "oversized",
            Some(vec![
                0;
                (crate::persistence::MAX_SNAPSHOT_BYTES + 1) as usize
            ]),
        ),
        ("unsupported", Some(unsupported)),
        ("structurally-invalid", Some(structurally_invalid)),
        ("relative-root", Some(relative_root)),
        ("relative-parent-root", Some(relative_parent_root)),
        ("absolute-file-root", Some(absolute_file_root)),
        ("transport-oversized", Some(transport_oversized)),
        ("valid-empty", Some(valid_empty)),
    ];

    for (name, bytes) in cases {
        let path = directory.join(format!("{name}.snapshot"));
        if let Some(bytes) = bytes {
            std::fs::write(&path, bytes).unwrap();
        }
        let (state, startup_terminals) =
            RuntimeState::recover(&test_endpoint(), Some(path), None).unwrap();
        assert_eq!(
            state.session.snapshot(),
            Session::new().snapshot(),
            "{name}"
        );
        assert!(state.terminals.is_empty(), "{name}");
        assert!(startup_terminals.is_empty(), "{name}");
    }

    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn restart_falls_back_from_a_missing_pane_cwd_and_persists_the_repair() {
    let directory = std::env::temp_dir().join(format!(
        "condr-server-cwd-fallback-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let workspace_root = directory.join("workspace");
    let missing_cwd = workspace_root.join("missing");
    let snapshot_path = directory.join("session.snapshot");
    std::fs::create_dir_all(&workspace_root).unwrap();

    let mut session = Session::new();
    session
        .create_workspace(workspace_root.clone())
        .expect("Workspace capacity");
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    assert!(session.set_pane_cwd(pane_id, Some(missing_cwd)));
    let absent_cwd_pane = session
        .split_pane(pane_id, condr_core::SplitDirection::Horizontal, 0.5)
        .unwrap();
    assert!(session.set_pane_cwd(absent_cwd_pane, None));
    let relative_cwd_pane = session
        .split_pane(absent_cwd_pane, condr_core::SplitDirection::Vertical, 0.5)
        .unwrap();
    assert!(session.set_pane_cwd(relative_cwd_pane, Some(PathBuf::from("."))));
    write_snapshot_fixture(snapshot_path.clone(), session.snapshot());

    let endpoint = test_endpoint();
    let server = BoundServer::bind(
        ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    assert_eq!(server.startup_terminals.len(), 3);
    let recovered_before_runtime = Session::restore(server.handle().snapshot()).unwrap();
    assert_eq!(
        recovered_before_runtime
            .pane(absent_cwd_pane)
            .and_then(|pane| pane.cwd()),
        None
    );
    assert_eq!(
        recovered_before_runtime
            .pane(relative_cwd_pane)
            .and_then(|pane| pane.cwd()),
        Some(workspace_root.as_path())
    );
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);
    let connection = ClientConnection::connect(&endpoint, "cwd-fallback").unwrap();
    let repaired_session = Session::restore(connection.bootstrap().snapshot.clone()).unwrap();
    assert_eq!(
        repaired_session.pane(pane_id).and_then(|pane| pane.cwd()),
        Some(workspace_root.as_path())
    );
    assert_eq!(connection.bootstrap().terminals.len(), 3);
    let repaired_snapshot = connection.bootstrap().snapshot.clone();

    drop(connection);
    handle.stop();
    thread.join().unwrap().unwrap();
    let persisted =
        condr_core::SessionSnapshot::from_bytes(&std::fs::read(&snapshot_path).unwrap()).unwrap();
    assert_eq!(persisted, repaired_snapshot);
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn restart_prunes_only_failed_panes_and_persists_the_repair() {
    use condr_core::SplitDirection;

    let directory = std::env::temp_dir().join(format!(
        "condr-server-partial-restore-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let workspace_root = directory.join("missing-root");
    let valid_cwd = directory.join("valid");
    let missing_cwd = directory.join("missing-pane");
    let snapshot_path = directory.join("session.snapshot");
    std::fs::create_dir_all(&valid_cwd).unwrap();

    let mut session = Session::new();
    let workspace_id = session
        .create_workspace(workspace_root)
        .expect("Workspace capacity");
    assert!(session.rename_workspace(workspace_id, "Recovered Workspace"));
    let tab_id = session.active_workspace().unwrap().active_tab().id();
    assert!(session.rename_tab(tab_id, "Recovered Tab"));
    let surviving_pane = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    assert!(session.set_pane_cwd(surviving_pane, Some(valid_cwd.clone())));
    let failed_pane = session
        .split_pane(surviving_pane, SplitDirection::Vertical, 0.65)
        .unwrap();
    assert!(session.set_pane_cwd(failed_pane, Some(missing_cwd)));
    let persisted = session.snapshot();
    write_snapshot_fixture(snapshot_path.clone(), persisted);

    let endpoint = test_endpoint();
    let server = BoundServer::bind(
        ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    assert_eq!(server.startup_terminals.len(), 1);
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);
    let connection = ClientConnection::connect(&endpoint, "partial-restore").unwrap();
    let bootstrap = connection.bootstrap();
    assert_eq!(bootstrap.terminals.len(), 1);
    assert_eq!(bootstrap.terminals[0].pane_id, surviving_pane);
    assert!(bootstrap.agents.is_empty());
    let restored = Session::restore(bootstrap.snapshot.clone()).unwrap();
    assert_eq!(restored.active_workspace_id(), Some(workspace_id));
    assert_eq!(restored.workspaces().len(), 1);
    let workspace = restored.active_workspace().unwrap();
    assert_eq!(workspace.name(), "Recovered Workspace");
    assert_eq!(workspace.tabs().len(), 1);
    assert_eq!(workspace.active_tab().id(), tab_id);
    assert_eq!(workspace.active_tab().name(), "Recovered Tab");
    assert_eq!(workspace.active_tab().panes().len(), 1);
    assert_eq!(workspace.active_tab().focused_pane().id(), surviving_pane);
    assert_eq!(
        workspace.active_tab().layout(),
        &condr_core::PaneLayout::Pane(surviving_pane)
    );
    assert_eq!(
        workspace.active_tab().focused_pane().cwd(),
        Some(valid_cwd.as_path())
    );
    let repaired_snapshot = bootstrap.snapshot.clone();

    drop(connection);
    handle.stop();
    thread.join().unwrap().unwrap();
    let repaired =
        condr_core::SessionSnapshot::from_bytes(&std::fs::read(&snapshot_path).unwrap()).unwrap();
    assert_eq!(repaired, repaired_snapshot);
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn wholly_unrestorable_snapshot_persists_start_page_state() {
    let directory = std::env::temp_dir().join(format!(
        "condr-server-empty-restore-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let snapshot_path = directory.join("session.snapshot");
    std::fs::create_dir_all(&directory).unwrap();
    let mut session = Session::new();
    session
        .create_workspace(directory.join("missing"))
        .expect("Workspace capacity");
    write_snapshot_fixture(snapshot_path.clone(), session.snapshot());

    let endpoint = test_endpoint();
    let server = BoundServer::bind(
        ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    assert!(server.startup_terminals.is_empty());
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);
    let connection = ClientConnection::connect(&endpoint, "empty-restore").unwrap();
    assert_eq!(connection.bootstrap().snapshot, Session::new().snapshot());
    assert!(connection.bootstrap().terminals.is_empty());

    drop(connection);
    handle.stop();
    thread.join().unwrap().unwrap();
    let repaired =
        condr_core::SessionSnapshot::from_bytes(&std::fs::read(&snapshot_path).unwrap()).unwrap();
    assert_eq!(repaired, Session::new().snapshot());
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn restart_revalidates_worktree_authority_against_the_git_topology() {
    let directory = std::env::temp_dir().join(format!(
        "condr-server-worktree-restore-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let repository = directory.join("repository");
    let parent_workspace_root = repository.join("workspace-root");
    let child_root = directory.join("child");
    let snapshot_path = directory.join("session.snapshot");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init"]);
    run_git(&repository, &["config", "user.name", "Condr Tests"]);
    run_git(
        &repository,
        &["config", "user.email", "condr@example.invalid"],
    );
    std::fs::create_dir_all(&parent_workspace_root).unwrap();
    std::fs::write(parent_workspace_root.join("README.md"), "condr\n").unwrap();
    run_git(&repository, &["add", "workspace-root/README.md"]);
    run_git(&repository, &["commit", "-m", "initial"]);
    run_git(
        &repository,
        &[
            "worktree",
            "add",
            "-b",
            "feature/recovered",
            child_root.to_str().unwrap(),
        ],
    );

    let mut session = Session::new();
    let parent_workspace_id = session
        .create_workspace(parent_workspace_root.clone())
        .expect("Workspace capacity");
    let child_workspace_id = session
        .create_workspace(child_root.clone())
        .expect("Workspace capacity");
    assert!(session.associate_worktree(
        child_workspace_id,
        parent_workspace_id,
        parent_workspace_root,
        true,
    ));
    session
        .close_workspace(parent_workspace_id)
        .expect("historical parent Workspace exists");
    write_snapshot_fixture(snapshot_path.clone(), session.snapshot());

    let first_endpoint = test_endpoint();
    let first_server = BoundServer::bind(
        ServerConfig::new(first_endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    let first_handle = first_server.handle();
    let first_thread = thread::spawn(move || first_server.run());
    wait_for_connection(&first_endpoint);
    let first = ClientConnection::connect(&first_endpoint, "valid-worktree-restore").unwrap();
    let first_session = Session::restore(first.bootstrap().snapshot.clone()).unwrap();
    assert!(
        first_session
            .workspace(child_workspace_id)
            .and_then(|workspace| workspace.worktree())
            .is_some_and(|association| association.is_managed())
    );
    drop(first);
    first_handle.stop();
    first_thread.join().unwrap().unwrap();

    run_git(
        &repository,
        &["worktree", "remove", child_root.to_str().unwrap()],
    );
    std::fs::create_dir_all(&child_root).unwrap();

    let second_endpoint = test_endpoint();
    let second_server = BoundServer::bind(
        ServerConfig::new(second_endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    let second_handle = second_server.handle();
    let second_thread = thread::spawn(move || second_server.run());
    wait_for_connection(&second_endpoint);
    let second = ClientConnection::connect(&second_endpoint, "stale-worktree-restore").unwrap();
    let repaired = Session::restore(second.bootstrap().snapshot.clone()).unwrap();
    assert!(
        repaired
            .workspace(child_workspace_id)
            .expect("child Workspace survives")
            .worktree()
            .is_none()
    );
    drop(second);
    second_handle.stop();
    second_thread.join().unwrap().unwrap();

    let persisted = condr_core::SessionSnapshot::from_bytes(
        &std::fs::read(&snapshot_path).expect("repaired Snapshot exists"),
    )
    .unwrap();
    assert!(
        Session::restore(persisted)
            .unwrap()
            .workspace(child_workspace_id)
            .expect("child Workspace remains persisted")
            .worktree()
            .is_none()
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn server_restart_restores_structure_with_fresh_terminal_state() {
    use condr_core::SplitDirection;

    let directory = std::env::temp_dir().join(format!(
        "condr-server-restart-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let workspace_root = directory.join("workspace");
    let workspace_cwd = workspace_root.join("nested");
    let snapshot_path = directory.join("session.snapshot");
    std::fs::create_dir_all(&workspace_cwd).unwrap();
    run_git(&workspace_root, &["init"]);
    run_git(&workspace_root, &["config", "user.name", "Condr Tests"]);
    run_git(
        &workspace_root,
        &["config", "user.email", "condr@example.invalid"],
    );
    std::fs::write(workspace_root.join("README.md"), "condr\n").unwrap();
    run_git(&workspace_root, &["add", "README.md"]);
    run_git(&workspace_root, &["commit", "-m", "initial"]);
    run_git(&workspace_root, &["checkout", "-b", "ph6-restore"]);
    let endpoint = test_endpoint();

    let server = BoundServer::bind(
        ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    let first_handle = server.handle();
    let first_thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);
    let first = ClientConnection::connect(&endpoint, "restart-first").unwrap();
    let initial = first.bootstrap().clone();
    let first_server_id = initial.server_id;
    let first_epoch = initial.runtime_epoch;
    let session_id = initial.session_id;
    let mut stream = first.into_stream();
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut stream, session_id);
    subscribe(&mut stream, session_id, initial.sequence);

    let (restored_workspace_id, first_pane) = {
        let mut mutate = |command| {
            condr_core::protocol::write_message(
                &mut stream,
                &ClientMessage::Layout {
                    server_id: first_server_id,
                    session_id,
                    request_id: 1,
                    command,
                },
            )
            .unwrap();
            let message = wait_for_message(&mut stream, |message| {
                matches!(
                    message,
                    ServerMessage::Event {
                        event: SessionEvent::LayoutChanged,
                        ..
                    }
                )
            });
            let ServerMessage::Event { sequence, .. } = message else {
                unreachable!("predicate only accepts LayoutChanged events");
            };
            assert_layout_applied(&mut stream, first_server_id, session_id, 1, sequence);
        };
        mutate(LayoutCommand::CreateWorkspace {
            root_directory: workspace_root.clone(),
        });
        let (workspace_id, tab_id, first_pane) = {
            let state = first_handle.state.lock().unwrap();
            let workspace = state.session.active_workspace().unwrap();
            (
                workspace.id(),
                workspace.active_tab().id(),
                workspace.active_tab().focused_pane().id(),
            )
        };
        mutate(LayoutCommand::RenameWorkspace {
            workspace_id,
            name: "Persistent Workspace".into(),
        });
        mutate(LayoutCommand::RenameTab {
            tab_id,
            name: "Persistent Tab".into(),
        });
        mutate(LayoutCommand::SplitPane {
            pane_id: first_pane,
            direction: SplitDirection::Horizontal,
        });
        mutate(LayoutCommand::SetSplitRatios {
            tab_id,
            ratios: vec![0.7],
        });
        mutate(LayoutCommand::FocusPane {
            pane_id: first_pane,
        });
        (workspace_id, first_pane)
    };
    let sequence_after_layout = first_handle.state.lock().unwrap().sequence;
    let cwd_command = if cfg!(windows) {
        format!(
            "Set-Location -LiteralPath '{}'; Write-Output ('CONDR_' + 'CWD_CHANGED')\r",
            workspace_cwd.to_string_lossy().replace('\'', "''")
        )
    } else {
        format!(
            "cd '{}' && printf 'CONDR_%s\\n' CWD_CHANGED\r",
            workspace_cwd.to_string_lossy().replace('\'', "'\\''")
        )
    };
    send_terminal(
        &mut stream,
        first_server_id,
        session_id,
        first_pane,
        TerminalCommand::Text(cwd_command),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    let expected = loop {
        let state = first_handle.state.lock().unwrap();
        if state.session.pane(first_pane).and_then(|pane| pane.cwd())
            == Some(workspace_cwd.as_path())
        {
            // A cwd change is durable state, not an event; only the shell's own title
            // reports (and bells) may have been published meanwhile.
            assert!(
                state
                    .events
                    .iter()
                    .filter(|event| event.sequence > sequence_after_layout)
                    .all(|event| matches!(
                        event.event,
                        SessionEvent::TerminalTitleChanged { .. }
                            | SessionEvent::TerminalAttentionChanged { .. }
                    )),
                "cwd observation published a layout event"
            );
            break state.session.snapshot();
        }
        drop(state);
        assert!(
            Instant::now() < deadline,
            "authoritative Session cwd never became {}",
            workspace_cwd.display()
        );
        thread::sleep(Duration::from_millis(20));
    };

    send_terminal(
        &mut stream,
        first_server_id,
        session_id,
        first_pane,
        TerminalCommand::Text("echo CONDR_OLD_RUNTIME_MARKER\r".into()),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let contains_marker = {
            let state = first_handle.state.lock().unwrap();
            state.terminals.get(&first_pane).is_some_and(|runtime| {
                view_text(&runtime.view()).contains("CONDR_OLD_RUNTIME_MARKER")
            })
        };
        if contains_marker {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "old terminal runtime never displayed its marker"
        );
        thread::sleep(Duration::from_millis(20));
    }

    first_handle.stop();
    drop(stream);
    first_thread.join().unwrap().unwrap();
    assert_eq!(
        condr_core::SessionSnapshot::from_bytes(&std::fs::read(&snapshot_path).unwrap()).unwrap(),
        expected
    );

    // The first server's socket path and bind lock are released as its thread winds down;
    // under parallel test load that can trail the join briefly, so retry the address race.
    let replacement_config = ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path);
    let bind_deadline = Instant::now() + Duration::from_secs(5);
    let replacement = loop {
        match BoundServer::bind(replacement_config.clone()) {
            Ok(server) => break server,
            Err(error)
                if error.kind() == io::ErrorKind::AddrInUse && Instant::now() < bind_deadline =>
            {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("replacement server failed to bind: {error}"),
        }
    };
    let replacement_handle = replacement.handle();
    let replacement_thread = thread::spawn(move || replacement.run());
    wait_for_connection(&endpoint);
    let restored = ClientConnection::connect(&endpoint, "restart-second").unwrap();
    let bootstrap = restored.bootstrap().clone();
    assert_eq!(bootstrap.server_id, first_server_id);
    assert_ne!(bootstrap.runtime_epoch, first_epoch);
    // A fresh runtime starts its event log empty; the restarted shells may already have
    // reported their titles, which is the only kind of event allowed here.
    assert!(
        replacement_handle
            .state
            .lock()
            .unwrap()
            .events
            .iter()
            .all(|event| matches!(
                event.event,
                SessionEvent::TerminalTitleChanged { .. }
                    | SessionEvent::TerminalAttentionChanged { .. }
            ))
    );
    assert_eq!(bootstrap.snapshot, expected);
    assert_eq!(bootstrap.terminals.len(), 2);
    assert!(bootstrap.agents.is_empty());
    assert!(bootstrap.workspace_git.iter().any(|git| {
        git.workspace_id == restored_workspace_id
            && git.branch.as_deref() == Some("ph6-restore")
            && !git.linked_worktree
    }));
    assert!(
        bootstrap
            .terminals
            .iter()
            .all(|terminal| { !view_text(&terminal.view).contains("CONDR_OLD_RUNTIME_MARKER") })
    );
    let restored_session = Session::restore(bootstrap.snapshot.clone()).unwrap();
    assert_eq!(
        restored_session
            .pane(first_pane)
            .and_then(|pane| pane.cwd()),
        Some(workspace_cwd.as_path())
    );

    let mut restored_stream = restored.into_stream();
    restored_stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut restored_stream, session_id);
    let cwd_check = if cfg!(windows) {
        format!(
            "if ((Get-Location).Path -eq '{}') {{ Write-Output ('CONDR_RESTORED_' + 'CWD_OK') }} else {{ Write-Output ('CONDR_RESTORED_' + 'CWD_BAD') }}\r",
            workspace_cwd.to_string_lossy().replace('\'', "''")
        )
    } else {
        format!(
            "if [ \"$PWD\" = '{}' ]; then printf 'CONDR_RESTORED_%s\\n' CWD_OK; else printf 'CONDR_RESTORED_%s\\n' CWD_BAD; fi\r",
            workspace_cwd.to_string_lossy().replace('\'', "'\\''")
        )
    };
    send_terminal(
        &mut restored_stream,
        first_server_id,
        session_id,
        first_pane,
        TerminalCommand::Text(cwd_check),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let restored_text = loop {
        let text = {
            let state = replacement_handle.state.lock().unwrap();
            state
                .terminals
                .get(&first_pane)
                .map(|runtime| view_text(&runtime.view()))
                .unwrap()
        };
        if text.contains("CONDR_RESTORED_CWD_") {
            break text;
        }
        assert!(
            Instant::now() < deadline,
            "fresh shell did not report its working directory"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(
        restored_text.contains("CONDR_RESTORED_CWD_OK"),
        "fresh shell did not start in {}",
        workspace_cwd.display()
    );

    replacement_handle.stop();
    drop(restored_stream);
    replacement_thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn terminal_tail_cwd_survives_exit_and_shutdown() {
    #[cfg(target_os = "linux")]
    if std::path::Path::new(&condr_core::CommandBuilder::new_default_prog().get_shell())
        .file_name()
        .and_then(|name| name.to_str())
        != Some("bash")
    {
        return;
    }

    let directory = std::env::temp_dir().join(format!(
        "condr-server-exit-cwd-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let workspace_root = directory.join("workspace");
    let final_cwd = workspace_root.join("final");
    let snapshot_path = directory.join("session.snapshot");
    std::fs::create_dir_all(&final_cwd).unwrap();

    let mut session = Session::new();
    session
        .create_workspace(workspace_root)
        .expect("Workspace capacity");
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    write_snapshot_fixture(snapshot_path.clone(), session.snapshot());

    let endpoint = test_endpoint();
    let server = BoundServer::bind(
        ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for_connection(&endpoint);
    let connection = ClientConnection::connect(&endpoint, "exit-cwd").unwrap();
    let server_id = connection.bootstrap().server_id;
    let session_id = connection.bootstrap().session_id;
    let mut stream = connection.into_stream();
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut stream, session_id);

    #[cfg(target_os = "linux")]
    {
        let change_cwd_and_exit = format!(
            "cd '{}'; exit",
            final_cwd.to_string_lossy().replace('\'', "'\\''")
        );
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text(change_cwd_and_exit),
        );
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Key {
                key: condr_core::TerminalKey::Enter,
                modifiers: condr_core::TerminalModifiers::default(),
            },
        );
    }
    #[cfg(target_os = "windows")]
    {
        let change_cwd = format!(
            "Set-Location -LiteralPath '{}'",
            final_cwd.to_string_lossy().replace('\'', "''")
        );
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text(change_cwd),
        );
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Key {
                key: condr_core::TerminalKey::Enter,
                modifiers: condr_core::TerminalModifiers::default(),
            },
        );

        // Parallel Windows tests can heavily contend on ConPTY and process scans.
        let probe_deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let state = handle.state.lock().unwrap();
            let probed = state
                .terminals
                .get(&pane_id)
                .and_then(|runtime| runtime.cwd_probe().cwd());
            if probed.as_deref() == Some(final_cwd.as_path()) {
                assert_ne!(
                    state.session.pane(pane_id).and_then(|pane| pane.cwd()),
                    Some(final_cwd.as_path()),
                    "the low-frequency scan ran before the shutdown-path assertion"
                );
                break;
            }
            drop(state);
            assert!(
                Instant::now() < probe_deadline,
                "terminal cwd probe never observed {}",
                final_cwd.display()
            );
            thread::sleep(Duration::from_millis(5));
        }
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text("exit".into()),
        );
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Key {
                key: condr_core::TerminalKey::Enter,
                modifiers: condr_core::TerminalModifiers::default(),
            },
        );
    }

    #[cfg(target_os = "linux")]
    {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let state = handle.state.lock().unwrap();
            if state.exited_terminals.contains(&pane_id) {
                assert_eq!(
                    state.session.pane(pane_id).and_then(|pane| pane.cwd()),
                    Some(final_cwd.as_path())
                );
                break;
            }
            if Instant::now() >= deadline {
                let terminal = state
                    .terminals
                    .get(&pane_id)
                    .map(TerminalRuntime::visible_text)
                    .unwrap_or_default();
                panic!("terminal never reported exit: {terminal:?}");
            }
            drop(state);
            thread::sleep(Duration::from_millis(20));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = handle.state.lock().unwrap();
            let terminal = state
                .terminals
                .get(&pane_id)
                .map(TerminalRuntime::visible_text)
                .unwrap_or_default();
            if terminal.contains("> exit") {
                break;
            }
            drop(state);
            assert!(
                Instant::now() < deadline,
                "terminal never received exit: {terminal:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
    let persisted =
        condr_core::SessionSnapshot::from_bytes(&std::fs::read(snapshot_path).unwrap()).unwrap();
    assert_eq!(
        Session::restore(persisted)
            .unwrap()
            .pane(pane_id)
            .and_then(|pane| pane.cwd()),
        Some(final_cwd.as_path())
    );
    let _ = std::fs::remove_dir_all(directory);
}
