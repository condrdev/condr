use super::*;

/// ADR 0045: a rename moves the Preview Tab with its file, and a mutation refreshes Files
/// for every Client itself, since the watcher skips ignored paths or may not run at all.
#[test]
fn a_rename_moves_the_preview_tab_and_refreshes_files_without_notify() {
    let root = std::env::temp_dir().join(format!(
        "condr-server-files-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(root.join("dir")).unwrap();
    std::fs::write(root.join("dir/a.md"), "a").unwrap();
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let workspace_id = state.session.create_workspace(root.clone()).unwrap();
    let tab_id = state
        .session
        .show_file(workspace_id, RelativePathBuf::from("dir/a.md"))
        .unwrap();
    let state = Arc::new(Mutex::new(state));
    {
        let mut locked = state.lock().unwrap();
        let watcher = GitWatcher::without_filesystem(Arc::downgrade(&state));
        watcher.watch(workspace_id, root.clone(), None);
        locked.git_watcher = Some(watcher);
    }

    let rename = FileMutation::Rename {
        from: "dir".into(),
        to: "docs".into(),
    };
    let result = condr_core::mutate_file(&root, &rename);
    assert_eq!(result, FileMutationResult::Done { version: None });
    files_mutated(&state, workspace_id, &rename, &result);
    {
        let locked = state.lock().unwrap();
        let workspace = locked.session.workspace(workspace_id).unwrap();
        assert_eq!(
            workspace.tab(tab_id).unwrap().file().unwrap().path(),
            relative_path::RelativePath::new("docs/a.md")
        );
        assert!(
            locked
                .events
                .iter()
                .any(|event| matches!(event.event, SessionEvent::LayoutChanged { .. }))
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !state
        .lock()
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(event.event, SessionEvent::WorkspaceFilesChanged { .. }))
    {
        assert!(
            Instant::now() < deadline,
            "a mutation must refresh Files without notify"
        );
        thread::sleep(Duration::from_millis(5));
    }

    let watcher = state.lock().unwrap().git_watcher.take().unwrap();
    watcher.shutdown();
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

/// ADR 0046: the sidebar's refresh button asks for the same forced scan, which publishes
/// `WorkspaceFilesChanged` even when nothing moved and the Workspace is no repository.
#[test]
fn a_refresh_rescans_the_workspace_and_refuses_an_unknown_one() {
    let root = std::env::temp_dir().join(format!(
        "condr-server-refresh-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let workspace_id = state.session.create_workspace(root.clone()).unwrap();
    let (server_id, session_id) = (state.server_id, state.session_id);
    let state = Arc::new(Mutex::new(state));
    {
        let mut locked = state.lock().unwrap();
        let watcher = GitWatcher::without_filesystem(Arc::downgrade(&state));
        watcher.watch(workspace_id, root.clone(), None);
        locked.git_watcher = Some(watcher);
    }
    let files_changed = |state: &Arc<Mutex<RuntimeState>>| {
        state
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| matches!(event.event, SessionEvent::WorkspaceFilesChanged { .. }))
            .count()
    };
    let before = files_changed(&state);
    let deadline = Instant::now() + Duration::from_secs(5);
    refresh_workspace(&state, server_id, session_id, workspace_id).unwrap();
    while files_changed(&state) == before {
        assert!(
            Instant::now() < deadline,
            "a refresh must publish WorkspaceFilesChanged"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(refresh_workspace(&state, server_id, session_id, WorkspaceId::from_u64(999)).is_err());

    let watcher = state.lock().unwrap().git_watcher.take().unwrap();
    watcher.shutdown();
    drop(state);
    let _ = std::fs::remove_dir_all(root);
}

/// ADR 0050: `GitOldFile` answers with the side `GitDiff` compares with, and refuses what
/// `GitDiff` refuses.
#[test]
fn the_old_side_is_heads_text_and_refuses_an_unknown_workspace_or_a_missing_base() {
    let root = std::env::temp_dir().join(format!(
        "condr-server-old-file-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "-q", "-b", "main"]);
    run_git(&root, &["config", "user.email", "condr@example.com"]);
    run_git(&root, &["config", "user.name", "Condr"]);
    run_git(&root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("notes.txt"), "one\n").unwrap();
    run_git(&root, &["add", "notes.txt"]);
    run_git(&root, &["commit", "-q", "-m", "notes"]);
    std::fs::write(root.join("notes.txt"), "two\n").unwrap();

    let (handle, endpoint, thread) = start();
    let connection = ClientConnection::connect(&endpoint, "old-file").unwrap();
    let bootstrap = connection.bootstrap().unwrap().clone();
    let mut stream = connection.into_stream();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let workspace_id = {
        let mut state = handle.state.lock().unwrap();
        let workspace_id = state.session.create_workspace(root.clone()).unwrap();
        let git = discover_repository(&root).unwrap().unwrap();
        state
            .workspace_git
            .insert(workspace_id, WorkspaceGit::discovered(git));
        workspace_id
    };
    let mut old_file = |request_id, workspace_id, against| {
        condr_core::protocol::write_message(
            &mut stream,
            &ClientMessage::GitOldFile {
                server_id: bootstrap.server_id,
                session_id: bootstrap.session_id,
                request_id,
                workspace_id,
                path: "notes.txt".into(),
                against,
            },
        )
        .unwrap();
        let reply = wait_for_message(
            &mut stream,
            |message| matches!(message, ServerMessage::GitOldFile { request_id: id, .. } if *id == request_id),
        );
        let ServerMessage::GitOldFile { result, .. } = reply else {
            unreachable!("the predicate only accepts GitOldFile");
        };
        result
    };

    assert_eq!(
        old_file(1, workspace_id, DiffBase::Head),
        Ok(condr_core::OldFile::Text("one\n".into()))
    );
    assert_eq!(
        old_file(2, WorkspaceId::from_u64(999), DiffBase::Head),
        Err("unknown Workspace".into())
    );
    assert_eq!(
        old_file(3, workspace_id, DiffBase::MergeBase),
        Err("the Workspace has no base branch".into())
    );

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(root);
}
