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
