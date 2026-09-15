use super::*;

/// The Changes sidebar lists the real Server's status for a repository Workspace, a click
/// opens the Diff Tab through the Server, and the Diff Tab shows the file's hunks once the
/// Server answers (ADR 0017).
#[test]
fn changes_sidebar_lists_the_repository_and_opens_one_diff_tab() {
    let _serial_guard = acquire_visual_test_lock();
    let repository = std::env::temp_dir().join(format!(
        "condr-gui-changes-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&repository);
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, ["init", "-q", "-b", "main"]);
    run_git(&repository, ["config", "user.email", "condr@example.com"]);
    run_git(&repository, ["config", "user.name", "Condr"]);
    run_git(&repository, ["config", "commit.gpgsign", "false"]);
    std::fs::write(repository.join("notes.txt"), "alpha\nbeta\n").unwrap();
    run_git(&repository, ["add", "notes.txt"]);
    run_git(&repository, ["commit", "-q", "-m", "notes"]);
    std::fs::write(repository.join("notes.txt"), "alpha\nBETA\ngamma\n").unwrap();
    std::fs::write(repository.join("fresh.txt"), "new\n").unwrap();

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds("condr-changes").is_some(),
        "the Changes sidebar is open by default"
    );
    assert!(
        window.debug_bounds("condr-changes-column").is_some(),
        "the column sits at the window's right edge"
    );

    let button = window
        .debug_bounds("open-project")
        .expect("new workspace button should be rendered");
    window.simulate_click(button.center(), Modifiers::default());
    assert!(window.did_prompt_for_paths());
    let selected_root = repository.clone();
    window.simulate_path_prompt_response(move |_| Some(vec![selected_root]));
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.active_workspace().is_some())
        })
    }));

    // The Server scans the repository off its own lock and publishes the list.
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("change-notes.txt").is_some()
                && window.debug_bounds("change-fresh.txt").is_some()
        }),
        "both the modified and the untracked file should be listed"
    );
    assert!(window.debug_bounds("changes-tracked").is_some());
    assert!(window.debug_bounds("changes-untracked").is_some());
    let changes = window.debug_bounds("condr-changes-column").unwrap();
    let row = window.debug_bounds("change-notes.txt").unwrap();
    assert!(
        row.left() >= changes.left() && row.right() <= changes.right() + px(1.),
        "rows live inside the sidebar column: {row:?} vs {changes:?}"
    );

    // Clicking a file opens the Workspace's Diff Tab on the Server and presents it.
    window.simulate_click(row.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.read(|app| {
                view.read(app).active_session().is_some_and(|session| {
                    session.active_workspace().is_some_and(|workspace| {
                        workspace
                            .active_tab()
                            .diff()
                            .is_some_and(|diff| diff.path() == std::path::Path::new("notes.txt"))
                    })
                })
            })
        }),
        "the Diff Tab should become the active Tab"
    );
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("diff-body").is_some()
        }),
        "the Server's diff should reach the Editor"
    );
    let workspace_id = window.read(|app| {
        view.read(app)
            .active_session()
            .unwrap()
            .active_workspace()
            .unwrap()
            .id()
    });
    let tab_count = window.read(|app| {
        view.read(app)
            .active_session()
            .unwrap()
            .active_workspace()
            .unwrap()
            .tabs()
            .len()
    });
    assert_eq!(tab_count, 2, "one terminal Tab and one Diff Tab");

    // A second file retargets the same Tab rather than adding one.
    let fresh = window.debug_bounds("change-fresh.txt").unwrap();
    window.simulate_click(fresh.center(), Modifiers::default());
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .diff_tab(workspace_id)
                    .and_then(|tab| tab.diff())
                    .is_some_and(|diff| diff.path() == std::path::Path::new("fresh.txt"))
                    && session.workspace(workspace_id).unwrap().tabs().len() == 2
            })
        })
    }));
    // And its diff replaces the first one's in the Editor after that single click.
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.update(|_, cx| {
                view.read(cx).diff_editors.values().any(|editor| {
                    let text = editor.state.read(cx).value();
                    text.contains("+new") && !text.contains("BETA")
                })
            })
        }),
        "the retargeted Diff Tab should show the second file's hunks"
    );

    // The title bar toggle hides the sidebar.
    let toggle = window
        .debug_bounds("toggle-changes")
        .expect("the title bar should expose the Changes toggle");
    window.simulate_click(toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("condr-changes").is_none());

    let _ = std::fs::remove_dir_all(&repository);
}
