use super::*;

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn apply_for_test(
    state: &mut RuntimeState,
    updates: &mut Vec<mpsc::Receiver<TerminalUpdate>>,
    command: LayoutCommand,
) -> usize {
    let terminal_count_before = state.terminals.len();
    let plan = external_layout_plan(state, &command).unwrap();
    let effect = match plan {
        Some(plan) => {
            let mut prepared = prepare_external_layout(plan).unwrap();
            approve_external_layout(state, &mut prepared).unwrap();
            let prepared = finish_external_layout(prepared, &ShellLaunch::default())
                .unwrap_or_else(|failure| panic!("{}", failure.message));
            apply_prepared_external_layout(state, prepared).unwrap()
        }
        None => apply_layout_command(state, command).unwrap(),
    };
    for (_, _, receiver, _, _, _, _) in effect.started_terminals {
        updates.push(receiver);
    }
    let removed_count = terminal_count_before.saturating_sub(state.terminals.len());
    drop(effect.removed_terminals);
    removed_count
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn layout_commands_keep_structure_zoom_and_terminals_in_sync() {
    use condr_core::{PaneDirection, PaneLayout, SplitDirection};

    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let mut updates = Vec::new();
    let root = std::env::temp_dir();

    assert_eq!(
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
                root_directory: root.clone(),
            },
        ),
        0
    );
    let workspace_id = state.session.active_workspace_id().unwrap();
    let tab_one = state.session.active_workspace().unwrap().active_tab().id();
    let pane_one = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();

    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::CreateTab {
            workspace_id,
            name: None,
            focus: true,
        },
    );
    let tab_two = state.session.active_workspace().unwrap().active_tab().id();
    let pane_two = state
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::RenameWorkspace {
            workspace_id,
            name: "renamed workspace".into(),
        },
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::RenameTab {
            tab_id: tab_two,
            name: "renamed tab".into(),
        },
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::SplitPane {
            focus: true,
            pane_id: pane_two,
            direction: SplitDirection::Horizontal,
        },
    );
    let pane_three = state.session.tab(tab_two).unwrap().focused_pane().id();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::FocusPane { pane_id: pane_two },
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::FocusPaneDirection {
            pane_id: pane_two,
            direction: PaneDirection::Right,
        },
    );
    assert_eq!(
        state.session.tab(tab_two).unwrap().focused_pane().id(),
        pane_three
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::ResizePane {
            pane_id: pane_three,
            direction: PaneDirection::Left,
            amount: 0.05,
        },
    );

    let before_invalid_ratio = state.session.snapshot();
    assert!(
        apply_layout_command(
            &mut state,
            LayoutCommand::SetSplitRatios {
                tab_id: tab_two,
                ratios: Vec::new(),
            },
        )
        .is_err()
    );
    assert_eq!(state.session.snapshot(), before_invalid_ratio);
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::SetSplitRatios {
            tab_id: tab_two,
            ratios: vec![0.6],
        },
    );
    assert!(matches!(
        state.session.tab(tab_two).unwrap().layout(),
        PaneLayout::Split { ratio, .. } if (*ratio - 0.6).abs() < f32::EPSILON
    ));
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::SwapPane {
            pane_id: pane_three,
            direction: PaneDirection::Left,
        },
    );
    assert!(
        apply_layout_command(
            &mut state,
            LayoutCommand::MovePane {
                pane_id: pane_three,
                target_pane_id: pane_three,
                side: PaneDirection::Down,
            },
        )
        .is_err()
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::MovePane {
            pane_id: pane_three,
            target_pane_id: pane_two,
            side: PaneDirection::Up,
        },
    );
    assert!(matches!(
        state.session.tab(tab_two).unwrap().layout(),
        PaneLayout::Split { direction: SplitDirection::Vertical, first, .. }
            if **first == PaneLayout::Pane(pane_three)
    ));
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::TogglePaneZoom {
            pane_id: pane_three,
        },
    );
    assert_eq!(state.bootstrap().zoomed_panes, vec![pane_three]);

    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::CreateWorkspace {
            name: None,
            focus: true,
            root_directory: root,
        },
    );
    let workspace_two = state.session.active_workspace_id().unwrap();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::MoveWorkspace {
            workspace_id: workspace_two,
            target_index: 0,
        },
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::ActivateWorkspace { workspace_id },
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::MoveTab {
            tab_id: tab_two,
            target_index: 0,
        },
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::ActivateTab { tab_id: tab_two },
    );

    assert_eq!(state.terminals.len(), 4);
    assert_eq!(
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::ClosePane {
                pane_id: pane_three,
            },
        ),
        1
    );
    assert_eq!(
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CloseTab { tab_id: tab_two },
        ),
        1
    );
    assert_eq!(
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CloseWorkspace { workspace_id },
        ),
        1
    );
    assert!(state.session.pane(pane_one).is_none());
    assert_eq!(state.terminals.len(), 1);
    assert_eq!(
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CloseWorkspace {
                workspace_id: workspace_two,
            },
        ),
        1
    );
    assert!(state.session.is_empty());
    assert!(state.terminals.is_empty());
    assert!(state.bootstrap().zoomed_panes.is_empty());
    assert!(state.session.tab(tab_one).is_none());
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn layout_commands_create_and_remove_a_managed_worktree_without_deleting_its_branch() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-worktree-{}-{}",
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

    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    state.worktree_root = Some(temp.join("worktrees"));
    let mut updates = Vec::new();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::CreateWorkspace {
            name: None,
            focus: true,
            root_directory: repository.clone(),
        },
    );
    let parent_workspace_id = state.session.active_workspace_id().unwrap();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::CreateWorktree {
            parent_workspace_id,
            branch: "feature/server-flow".into(),
        },
    );

    let child_workspace_id = state.session.active_workspace_id().unwrap();
    let child = state.session.workspace(child_workspace_id).unwrap();
    let child_root = child.root_directory().to_path_buf();
    assert!(child.worktree().unwrap().is_managed());
    assert_eq!(
        state
            .workspace_git
            .get(&child_workspace_id)
            .and_then(GitRepository::branch),
        Some("feature/server-flow")
    );

    assert_eq!(
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::RemoveWorktree {
                workspace_id: child_workspace_id,
            },
        ),
        1
    );
    assert!(!child_root.exists());
    run_git(
        &repository,
        &["show-ref", "--verify", "refs/heads/feature/server-flow"],
    );

    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::CloseWorkspace {
            workspace_id: parent_workspace_id,
        },
    );
    drop(state);
    drop(updates);
    let _ = std::fs::remove_dir_all(temp);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn failed_managed_worktree_removal_restarts_its_live_terminals() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-worktree-recovery-{}-{}",
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

    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    state.worktree_root = Some(temp.join("worktrees"));
    let mut updates = Vec::new();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::CreateWorkspace {
            name: None,
            focus: true,
            root_directory: repository.clone(),
        },
    );
    let parent_workspace_id = state.session.active_workspace_id().unwrap();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::CreateWorktree {
            parent_workspace_id,
            branch: "feature/removal-recovery".into(),
        },
    );
    let child_workspace_id = state.session.active_workspace_id().unwrap();
    let child = state.session.workspace(child_workspace_id).unwrap();
    let child_root = child.root_directory().to_path_buf();
    let pane_id = child.active_tab().focused_pane().id();
    let previous_instance = state.terminal_instances[&pane_id];

    let command = LayoutCommand::RemoveWorktree {
        workspace_id: child_workspace_id,
    };
    let plan = external_layout_plan(&state, &command).unwrap().unwrap();
    let mut prepared = prepare_external_layout(plan).unwrap();
    std::fs::write(child_root.join("became-dirty.txt"), "dirty\n").unwrap();
    approve_external_layout(&mut state, &mut prepared).unwrap();
    assert!(!state.terminals.contains_key(&pane_id));
    assert!(!state.terminal_instances.contains_key(&pane_id));
    state.active_controller = Some(1);
    apply_terminal_notices(
        &mut state,
        pane_id,
        TerminalNoticeBatch {
            bells: 1,
            ..TerminalNoticeBatch::default()
        },
    );
    assert!(state.pending_terminal_bells.contains(&pane_id));

    let failure = match finish_external_layout(prepared, &ShellLaunch::default()) {
        Ok(_) => panic!("dirty worktree removal unexpectedly succeeded"),
        Err(failure) => failure,
    };
    assert!(failure.message.contains("modified or untracked files"));
    assert_eq!(failure.restarted.len(), 1);
    for (pane_id, runtime, receiver) in failure.restarted {
        let started = state.install_terminal(pane_id, runtime, receiver);
        updates.push(started.2);
    }
    assert!(state.terminals.contains_key(&pane_id));
    assert_ne!(state.terminal_instances[&pane_id], previous_instance);
    assert!(state.session.workspace(child_workspace_id).is_some());
    assert!(!state.pending_terminal_bells.contains(&pane_id));
    assert!(matches!(
        state.events.back().map(|event| &event.event),
        Some(SessionEvent::TerminalAttentionChanged {
            pane_id: event_pane_id,
            attention: false,
        }) if *event_pane_id == pane_id
    ));

    std::fs::remove_file(child_root.join("became-dirty.txt")).unwrap();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::RemoveWorktree {
            workspace_id: child_workspace_id,
        },
    );
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::CloseWorkspace {
            workspace_id: parent_workspace_id,
        },
    );
    drop(state);
    drop(updates);
    let _ = std::fs::remove_dir_all(temp);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn stopping_server_rolls_back_a_prepared_worktree() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-worktree-cancel-{}-{}",
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

    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    state.worktree_root = Some(temp.join("worktrees"));
    state.active_controller = Some(7);
    let parent_workspace_id = state
        .session
        .create_workspace(repository.clone())
        .expect("Workspace capacity");
    let command = LayoutCommand::CreateWorktree {
        parent_workspace_id,
        branch: "feature/cancelled".into(),
    };
    let plan = external_layout_plan(&state, &command).unwrap().unwrap();
    let prepared = prepare_external_layout(plan).unwrap();
    let child_root = match &prepared {
        PreparedExternalLayout::CreateWorktree { child, .. } => child.root().to_path_buf(),
        _ => panic!("expected a prepared worktree"),
    };
    assert!(child_root.exists());
    assert!(matches!(
        layout_authority_error(&state, 7, state.server_id, state.session_id, 17, true),
        Some(ServerMessage::LayoutRejected {
            request_id: 17,
            reason,
            ..
        }) if reason == "Server is stopping"
    ));

    cancel_prepared_external_layout(prepared).unwrap();
    assert!(!child_root.exists());
    run_git(
        &repository,
        &["show-ref", "--verify", "refs/heads/feature/cancelled"],
    );

    let _ = std::fs::remove_dir_all(temp);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn git_branch_refresh_accepts_activity_from_any_workspace_pane() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-branch-refresh-{}-{}",
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

    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let workspace_id = state
        .session
        .create_workspace(repository.clone())
        .expect("Workspace capacity");
    let first_pane = state
        .session
        .workspace(workspace_id)
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    let second_pane = state
        .session
        .split_pane(first_pane, condr_core::SplitDirection::Horizontal, 0.5)
        .unwrap();
    let git = discover_repository(&repository).unwrap();
    set_workspace_git(&mut state, workspace_id, git);

    run_git(&repository, &["checkout", "-b", "feature/second-pane"]);
    state.workspace_git_scanned_at.insert(
        workspace_id,
        Instant::now()
            .checked_sub(GIT_SCAN_INTERVAL)
            .expect("test Instant supports subtraction"),
    );
    let activity = Instant::now();
    let WorkspaceGitScan::Ready { workspace_id, root } =
        reserve_workspace_git_scan(&mut state, second_pane, activity, Instant::now())
    else {
        panic!("second Pane activity should reserve its Workspace Git scan");
    };
    let next = discover_repository(&root).unwrap();
    apply_workspace_git_refresh(&mut state, workspace_id, &root, next);
    assert_eq!(
        state
            .workspace_git
            .get(&workspace_id)
            .and_then(GitRepository::branch),
        Some("feature/second-pane")
    );
    assert!(matches!(
        reserve_workspace_git_scan(&mut state, first_pane, activity, Instant::now()),
        WorkspaceGitScan::Covered
    ));

    drop(state);
    let _ = std::fs::remove_dir_all(temp);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn opening_an_already_open_worktree_records_parent_membership() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-open-worktree-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let repository = temp.join("repository");
    let worktree = temp.join("worktree");
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
    run_git(
        &repository,
        &[
            "worktree",
            "add",
            "-b",
            "feature/existing",
            worktree.to_str().unwrap(),
        ],
    );

    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let parent_workspace_id = state
        .session
        .create_workspace(repository.clone())
        .expect("Workspace capacity");
    let child_workspace_id = state
        .session
        .create_workspace(worktree.clone())
        .expect("Workspace capacity");
    assert!(
        state
            .session
            .workspace(child_workspace_id)
            .unwrap()
            .worktree()
            .is_none()
    );

    let mut updates = Vec::new();
    apply_for_test(
        &mut state,
        &mut updates,
        LayoutCommand::OpenWorktree {
            parent_workspace_id,
            root_directory: worktree,
        },
    );
    let association = state
        .session
        .workspace(child_workspace_id)
        .unwrap()
        .worktree()
        .unwrap();
    assert_eq!(association.parent_workspace_id(), parent_workspace_id);
    assert_eq!(association.parent_root_directory(), repository);
    assert!(!association.is_managed());

    drop(state);
    let _ = std::fs::remove_dir_all(temp);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn pane_terminal_survives_disconnect_and_reconnects_with_live_state() {
    use condr_core::{TerminalPosition, TerminalScroll, TerminalSide};

    let (handle, endpoint, thread) = start();
    let first_connection = ClientConnection::connect(&endpoint, "first-terminal").unwrap();
    let first_bootstrap = first_connection.bootstrap().unwrap().clone();
    let server_id = first_bootstrap.server_id;
    let session_id = first_bootstrap.session_id;
    let mut first = first_connection.into_stream();
    first
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut first, session_id);
    subscribe(&mut first, session_id, first_bootstrap.sequence);
    let mut first_terminal_views = std::collections::HashMap::new();
    condr_core::protocol::write_message(
        &mut first,
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
    let message = wait_for_message(&mut first, |message| {
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
    assert_layout_applied(&mut first, server_id, session_id, 1, sequence);
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
    let pid_command = if cfg!(windows) {
        "Write-Output ('condr-' + 'pid=' + $PID)\r"
    } else {
        "printf 'condr-%s=%s\\n' pid $$\r"
    };
    send_terminal(
        &mut first,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text(pid_command.into()),
    );
    let first_view =
        wait_for_terminal_text(&mut first, &mut first_terminal_views, pane_id, "condr-pid=");
    let first_pid = marker_value(&view_text(&first_view), "condr-pid=");
    let snapshot_before_disconnect = handle.snapshot();
    drop(first);
    thread::sleep(Duration::from_millis(30));

    let second_connection = ClientConnection::connect(&endpoint, "second-terminal").unwrap();
    let second_bootstrap = second_connection.bootstrap().unwrap().clone();
    assert_eq!(second_bootstrap.snapshot, snapshot_before_disconnect);
    let terminal = second_bootstrap
        .terminals
        .iter()
        .find(|terminal| terminal.pane_id == pane_id)
        .expect("reconnect bootstrap contains the live Pane");
    assert!(!terminal.exited);
    assert!(view_text(&terminal.view).contains("condr-pid="));

    let mut second = second_connection.into_stream();
    second
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut second, session_id);
    condr_core::protocol::write_message(
        &mut second,
        &ClientMessage::Subscribe {
            session_id,
            after_sequence: second_bootstrap.sequence,
        },
    )
    .unwrap();
    wait_for_message(&mut second, |message| {
        matches!(message, ServerMessage::Subscribed { .. })
    });
    let mut second_terminal_views = second_bootstrap
        .terminals
        .iter()
        .map(|terminal| (terminal.pane_id, terminal.view.clone()))
        .collect();

    let reconnect_command = if cfg!(windows) {
        "Write-Output ('reconnect-' + 'pid=' + $PID)\r"
    } else {
        "printf 'reconnect-%s=%s\\n' pid $$\r"
    };
    send_terminal(
        &mut second,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text(reconnect_command.into()),
    );
    let reconnect_view = wait_for_terminal_text(
        &mut second,
        &mut second_terminal_views,
        pane_id,
        "reconnect-pid=",
    );
    assert_eq!(
        marker_value(&view_text(&reconnect_view), "reconnect-pid="),
        first_pid
    );

    if cfg!(windows) {
        handle.stop();
        drop(second);
        thread.join().unwrap().unwrap();
        return;
    }

    send_terminal(
        &mut second,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Paste("printf reconnect-ok\n".into()),
    );
    wait_for_terminal_text(
        &mut second,
        &mut second_terminal_views,
        pane_id,
        "reconnect-ok",
    );
    send_terminal(
        &mut second,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Resize(TerminalSize::new(10, 40)),
    );
    send_terminal(
        &mut second,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text("stty size\r".into()),
    );
    wait_for_terminal_text(&mut second, &mut second_terminal_views, pane_id, "10 40");
    send_terminal(
        &mut second,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text(
            "i=1; while [ $i -le 20 ]; do echo line-$i; i=$((i+1)); done\r".into(),
        ),
    );
    wait_for_terminal_text(&mut second, &mut second_terminal_views, pane_id, "line-20");
    thread::sleep(Duration::from_millis(50));
    send_terminal(
        &mut second,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Scroll(TerminalScroll::Top),
    );
    let scrolled = wait_for_terminal(&mut second, &mut second_terminal_views, pane_id, |view| {
        view.display_offset > 0
    });
    let expected = (0..4)
        .filter_map(|column| scrolled.cell(0, column))
        .map(|cell| cell.text.as_str())
        .collect::<String>();
    send_terminal(
        &mut second,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Copy {
            selection: Some(condr_core::TerminalSelection {
                start: TerminalPosition {
                    row: 0,
                    column: 0,
                    side: TerminalSide::Left,
                },
                end: TerminalPosition {
                    row: 0,
                    column: 3,
                    side: TerminalSide::Right,
                },
                display_offset: scrolled.display_offset,
            }),
        },
    );
    let copied = wait_for_message(
        &mut second,
        |message| matches!(message, ServerMessage::TerminalCopied { pane_id: copied_pane, .. } if *copied_pane == pane_id),
    );
    assert!(matches!(
        copied,
        ServerMessage::TerminalCopied { text: Some(text), .. } if text == expected
    ));

    handle.stop();
    drop(second);
    thread.join().unwrap().unwrap();
}
