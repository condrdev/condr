use super::*;

/// ADR 0045: a rename moves the Preview Pane with its file, and a mutation refreshes Files
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
    let (_, pane_id) = state
        .session
        .show_viewer(
            workspace_id,
            condr_core::Viewer::File {
                path: RelativePathBuf::from("dir/a.md"),
            },
        )
        .unwrap();
    let state = Arc::new(Mutex::new(state));
    {
        let mut locked = state.lock().unwrap();
        let watcher = GitWatcher::without_filesystem(Arc::downgrade(&state));
        watcher.watch(workspace_id, root.clone(), Vec::new());
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
        assert_eq!(
            locked.session.pane(pane_id).unwrap().viewer(),
            Some(&condr_core::Viewer::File {
                path: RelativePathBuf::from("docs/a.md")
            })
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
        watcher.watch(workspace_id, root.clone(), Vec::new());
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
    // Git for Windows turns this on system-wide; the old side's line breaks are the test's.
    run_git(&root, &["config", "core.autocrlf", "false"]);
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
            .insert(workspace_id, vec![RepositoryGit::discovered(git)]);
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

/// ADR 0073: above several repositories, a diff's path picks the repository it lies in,
/// Git History names its repository, and Files dims by the right one's rules.
#[test]
fn the_repository_a_path_or_a_git_history_request_names_answers() {
    let root = std::env::temp_dir().join(format!(
        "condr-server-several-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    for name in ["api", "web"] {
        let repository = root.join(name);
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init", "-q", "-b", "main"]);
        run_git(&repository, &["config", "user.email", "condr@example.com"]);
        run_git(&repository, &["config", "user.name", "Condr"]);
        run_git(&repository, &["config", "commit.gpgsign", "false"]);
        run_git(&repository, &["config", "core.autocrlf", "false"]);
        std::fs::write(repository.join(".gitignore"), format!("{name}.log\n")).unwrap();
        std::fs::write(repository.join("notes.txt"), format!("{name}\n")).unwrap();
        run_git(&repository, &["add", "."]);
        run_git(
            &repository,
            &["commit", "-q", "-m", &format!("{name} notes")],
        );
        std::fs::write(repository.join("notes.txt"), "changed\n").unwrap();
        std::fs::write(repository.join("api.log"), "log\n").unwrap();
    }

    let (handle, endpoint, thread) = start();
    let connection = ClientConnection::connect(&endpoint, "several").unwrap();
    let bootstrap = connection.bootstrap().unwrap().clone();
    let mut stream = connection.into_stream();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let (workspace_id, api_head) = {
        let mut state = handle.state.lock().unwrap();
        let workspace_id = state.session.create_workspace(root.clone()).unwrap();
        let repositories: Vec<RepositoryGit> = workspace_repositories(&root)
            .unwrap()
            .into_iter()
            .map(|repository| RepositoryGit::from_repository(repository, None).unwrap())
            .collect();
        let api_head = repositories[0].repository.head().unwrap().to_owned();
        state.workspace_git.insert(workspace_id, repositories);
        (workspace_id, api_head)
    };
    let (server_id, session_id) = (bootstrap.server_id, bootstrap.session_id);
    let mut ask = |message: ClientMessage| {
        condr_core::protocol::write_message(&mut stream, &message).unwrap();
        wait_for_message(&mut stream, |reply| {
            matches!(
                reply,
                ServerMessage::GitOldFile { request_id: 1, .. }
                    | ServerMessage::GitLog { request_id: 2, .. }
                    | ServerMessage::GitCommit { request_id: 3, .. }
                    | ServerMessage::Directory { request_id: 4, .. }
                    | ServerMessage::GitCommitBranches { request_id: 5, .. }
            )
        })
    };
    let old_file = |ask: &mut dyn FnMut(ClientMessage) -> ServerMessage, path: &str| {
        let ServerMessage::GitOldFile { result, .. } = ask(ClientMessage::GitOldFile {
            server_id,
            session_id,
            request_id: 1,
            workspace_id,
            path: path.into(),
            against: DiffBase::Head,
        }) else {
            unreachable!("the predicate only accepts replies");
        };
        result
    };
    assert_eq!(
        old_file(&mut ask, "web/notes.txt"),
        Ok(condr_core::OldFile::Text("web\n".into()))
    );
    assert_eq!(
        old_file(&mut ask, "api/notes.txt"),
        Ok(condr_core::OldFile::Text("api\n".into()))
    );
    assert!(old_file(&mut ask, "notes.txt").is_err());

    let log = |ask: &mut dyn FnMut(ClientMessage) -> ServerMessage, repository: &str| {
        let ServerMessage::GitLog { result, .. } = ask(ClientMessage::GitLog {
            server_id,
            session_id,
            request_id: 2,
            workspace_id,
            from: Vec::new(),
            start: condr_core::LogStart::All,
            filter: condr_core::LogFilter::default(),
            skip: 0,
            limit: 10,
            repository: repository.into(),
        }) else {
            unreachable!("the predicate only accepts replies");
        };
        result
    };
    assert_eq!(
        log(&mut ask, "web").unwrap().commits[0].subject,
        "web notes"
    );
    assert!(log(&mut ask, "").is_err(), "the Root is in no repository");

    let ServerMessage::GitCommitBranches { result, .. } = ask(ClientMessage::GitCommitBranches {
        server_id,
        session_id,
        request_id: 5,
        workspace_id,
        commit: api_head.clone(),
        repository: "api".into(),
    }) else {
        unreachable!("the predicate only accepts replies");
    };
    assert_eq!(result.unwrap(), ["refs/heads/main"]);

    let ServerMessage::GitCommit { result, .. } = ask(ClientMessage::GitCommit {
        server_id,
        session_id,
        request_id: 3,
        workspace_id,
        commit: api_head,
        repository: "api".into(),
    }) else {
        unreachable!("the predicate only accepts replies");
    };
    let paths: Vec<String> = result
        .unwrap()
        .changes
        .entries
        .iter()
        .map(|entry| entry.path.to_string())
        .collect();
    assert_eq!(paths, ["api/.gitignore", "api/notes.txt"]);

    let mut api_log_ignored = |directory: &str| {
        let ServerMessage::Directory { result, .. } = ask(ClientMessage::ListDirectory {
            server_id,
            session_id,
            request_id: 4,
            workspace_id,
            path: directory.into(),
        }) else {
            unreachable!("the predicate only accepts replies");
        };
        let listing = result.unwrap();
        let entry = listing.entries.iter().find(|entry| entry.name == "api.log");
        entry.unwrap().ignored
    };
    assert!(api_log_ignored("api"));
    assert!(!api_log_ignored("web"), "web ignores only web.log");

    handle.stop();
    thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(root);
}
