//! A New Directory and a Clone Repository on the Server (ADR 0059).

use super::*;

#[test]
fn a_workspace_directory_is_one_new_level_and_goes_again_when_the_workspace_does_not() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-new-directory-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(temp.join("taken")).unwrap();
    let state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let plan = |name: &str, url: Option<&str>| {
        external_layout_plan(
            &state,
            &LayoutCommand::CreateWorkspaceDirectory {
                parent_directory: temp.clone(),
                directory_name: name.into(),
                name: None,
                clone_url: url.map(str::to_owned),
            },
        )
        .unwrap()
        .unwrap()
    };
    for refused in [
        "taken", "", ".", "..", "a/b", r"a\b", "C:foo", "nul", "end.",
    ] {
        assert!(
            prepare_external_layout(plan(refused, None)).is_err(),
            "{refused:?}"
        );
    }
    // A URL is checked before anything is made.
    assert!(prepare_external_layout(plan("fresh", Some("https://h/x';id"))).is_err());
    assert!(!temp.join("fresh").exists());

    let prepared = prepare_external_layout(plan("fresh", Some(" https://h/r.git\n"))).unwrap();
    match &prepared {
        PreparedExternalLayout::CreateWorkspace {
            root,
            made_directory,
            clone_url,
            ..
        } => {
            assert_eq!(root, &temp.join("fresh"));
            assert!(*made_directory && root.is_dir());
            assert_eq!(clone_url.as_deref(), Some("https://h/r.git"));
        }
        _ => panic!("expected a prepared Workspace"),
    }
    cancel_prepared_external_layout(prepared).unwrap();
    assert!(!temp.join("fresh").exists());

    // A directory something wrote into meanwhile stays.
    let prepared = prepare_external_layout(plan("fresh", None)).unwrap();
    std::fs::write(temp.join("fresh/note"), "kept").unwrap();
    cancel_prepared_external_layout(prepared).unwrap();
    assert!(temp.join("fresh/note").is_file());
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
fn a_pending_clone_gives_way_to_keys_typed_first() {
    let temp = std::env::temp_dir().join(format!(
        "condr-server-clone-typed-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let command = LayoutCommand::CreateWorkspaceDirectory {
        parent_directory: temp.clone(),
        directory_name: "repo".into(),
        name: None,
        clone_url: Some("/srv/none".into()),
    };
    let plan = external_layout_plan(&state, &command).unwrap().unwrap();
    let mut prepared = prepare_external_layout(plan).unwrap();
    approve_external_layout(&mut state, &mut prepared).unwrap();
    let prepared = finish_external_layout(prepared, &ShellLaunch::default())
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    let effect = apply_prepared_external_layout(&mut state, prepared).unwrap();
    let pane_id = *state.pending_clones.keys().next().expect("a pending clone");
    let instance_id = state.terminal_instances[&pane_id];
    state.terminals[&pane_id]
        .execute(TerminalCommand::Text("ls".into()))
        .unwrap();
    let state = Arc::new(Mutex::new(state));
    assert!(!crate::server::clones::type_pending_clone(
        &state,
        pane_id,
        instance_id
    ));
    let state = state.lock().unwrap();
    assert!(state.pending_clones.is_empty());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !state.terminals[&pane_id]
        .visible_text()
        .contains("Could not start the clone")
    {
        assert!(Instant::now() < deadline, "no notice");
        thread::sleep(Duration::from_millis(20));
    }
    // The printed command takes double quotes on Windows, whose cmd has no single ones.
    let quote = if cfg!(windows) { '"' } else { '\'' };
    let text = state.terminals[&pane_id]
        .visible_text()
        .replace(['\r', '\n'], "");
    assert!(
        text.contains(&format!("git clone -- {quote}/srv/none{quote}")),
        "{text}"
    );
    drop(state);
    drop(effect.started_terminals);
    let _ = std::fs::remove_dir_all(temp);
}
