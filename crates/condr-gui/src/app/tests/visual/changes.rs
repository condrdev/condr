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
        window.debug_bounds("condr-right-sidebar").is_none(),
        "the Changes sidebar starts closed on a fresh config"
    );
    // Without a Workspace there is nothing to review: the title bar has no toggle.
    assert!(window.debug_bounds("toggle-changes").is_none());

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
                .is_some_and(|session| !session.workspaces().is_empty())
        })
    }));
    // A Workspace alone does not open the sidebar; the user does, and the column then
    // sits at the window's right edge.
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("condr-right-sidebar").is_none());
    let toggle = window.debug_bounds("toggle-changes").unwrap();
    window.simulate_click(toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("condr-right-sidebar").is_some());
    assert!(window.debug_bounds("condr-changes-column").is_some());

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
                view.read(app)
                    .presented()
                    .is_some_and(|(_, session, _, tab_id)| {
                        session.tab(tab_id).unwrap().diff().is_some_and(|diff| {
                            diff.path() == relative_path::RelativePath::new("notes.txt")
                        })
                    })
            })
        }),
        "the Diff Tab should become the shown Tab"
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
            .workspaces()
            .first()
            .unwrap()
            .id()
    });
    let tab_count = window.read(|app| {
        view.read(app)
            .active_session()
            .unwrap()
            .workspaces()
            .first()
            .unwrap()
            .tabs()
            .len()
    });
    assert_eq!(tab_count, 2, "one terminal Tab and one Diff Tab");

    // "Show File" opens the Preview Tab at the new-file line under the diff cursor: the
    // unified text is "@@ -1,2 +1,3 @@", " alpha", "-beta", "+BETA", "+gamma", so a cursor on
    // "-beta" lands on "BETA", the second line of the file.
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            let editor = view
                .diff_editors
                .values()
                .next()
                .expect("one Diff Tab Editor");
            editor.state.update(cx, |state, cx| {
                state.set_cursor_position(
                    gpui_kit::component::input::Position::new(2, 0),
                    window,
                    cx,
                );
            });
        });
    });
    let show_file = window
        .debug_bounds("diff-show-file")
        .expect("the Diff Tab header should offer Show File");
    window.simulate_click(show_file.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.update(|_, cx| {
                let view = view.read(cx);
                view.presented().is_some_and(|(_, session, _, tab_id)| {
                    session.tab(tab_id).unwrap().file().is_some_and(|file| {
                        file.path() == relative_path::RelativePath::new("notes.txt")
                    })
                }) && view.file_editors.values().any(|editor| {
                    let state = editor.state.read(cx);
                    state.value().contains("BETA") && state.cursor_position().line == 1
                })
            })
        }),
        "the Preview Tab should show notes.txt with its cursor on BETA"
    );
    let tab_count = window.read(|app| {
        view.read(app)
            .active_session()
            .unwrap()
            .workspace(workspace_id)
            .unwrap()
            .tabs()
            .len()
    });
    assert_eq!(tab_count, 3, "the Preview Tab sits beside the Diff Tab");
    // Back to the Diff Tab for the rest.
    let row = window.debug_bounds("change-notes.txt").unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .presented()
                .is_some_and(|(_, session, _, tab_id)| {
                    session.tab(tab_id).unwrap().diff().is_some()
                })
        })
    }));

    // A second file retargets the same Tab rather than adding one.
    let fresh = window.debug_bounds("change-fresh.txt").unwrap();
    window.simulate_click(fresh.center(), Modifiers::default());
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .diff_tab(workspace_id)
                    .and_then(|tab| tab.diff())
                    .is_some_and(|diff| {
                        diff.path() == relative_path::RelativePath::new("fresh.txt")
                    })
                    && session.workspace(workspace_id).unwrap().tabs().len() == 3
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
    assert!(window.debug_bounds("condr-right-sidebar").is_none());

    let _ = std::fs::remove_dir_all(&repository);
}

/// The Changes header switches a Workspace between `HEAD` and its base, and the Diff Tab
/// follows: a committed file is only in the base list (ADR 0034).
#[test]
fn the_changes_comparison_switches_between_head_and_the_base_branch() {
    let _serial_guard = acquire_visual_test_lock();
    let repository = std::env::temp_dir().join(format!(
        "condr-gui-changes-base-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&repository);
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, ["init", "-q", "-b", "main"]);
    run_git(&repository, ["config", "user.email", "condr@example.com"]);
    run_git(&repository, ["config", "user.name", "Condr"]);
    run_git(&repository, ["config", "commit.gpgsign", "false"]);
    std::fs::write(repository.join("notes.txt"), "alpha\n").unwrap();
    run_git(&repository, ["add", "notes.txt"]);
    run_git(&repository, ["commit", "-q", "-m", "notes"]);
    run_git(&repository, ["switch", "-q", "-c", "feature"]);
    std::fs::write(repository.join("committed.txt"), "agent\n").unwrap();
    run_git(&repository, ["add", "committed.txt"]);
    run_git(&repository, ["commit", "-q", "-m", "agent work"]);

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));
    let button = window.debug_bounds("open-project").unwrap();
    window.simulate_click(button.center(), Modifiers::default());
    let selected_root = repository.clone();
    window.simulate_path_prompt_response(move |_| Some(vec![selected_root]));
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| !session.workspaces().is_empty())
        })
    }));
    window.update(|window, cx| _ = window.draw(cx));
    let toggle = window.debug_bounds("toggle-changes").unwrap();
    window.simulate_click(toggle.center(), Modifiers::default());

    // Against HEAD the commit left nothing; the base the Server found is local `main`.
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.read(|app| {
                view.read(app)
                    .active_connection()
                    .and_then(|connection| connection.workspace_git.values().next())
                    .and_then(|git| git.base.as_ref())
                    .is_some_and(|base| base.branch == "main")
            })
        }),
        "the Server should report the base branch"
    );
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("changes-comparison").is_some());
    assert!(window.debug_bounds("change-committed.txt").is_none());

    // The menu item calls this; GPUI's test window cannot address a popup menu's rows.
    let (key, workspace_id) = window.read(|app| {
        let view = view.read(app);
        let session = view.active_session().unwrap();
        (
            view.active_connection().unwrap().key,
            session.workspaces()[0].id(),
        )
    });
    let compare_view = view.clone();
    let compare = move |window: &mut gpui_kit::Window, against, cx: &mut gpui_kit::App| {
        compare_view.update(cx, |view, cx| {
            view.compare_changes_against(key, workspace_id, against, window, cx);
        });
    };
    window.update(|window, cx| compare(window, condr_core::protocol::DiffBase::MergeBase, cx));
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("change-committed.txt").is_some()
        }),
        "the committed file should be listed against the base"
    );

    // The Diff Tab asks for the same side.
    let row = window.debug_bounds("change-committed.txt").unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.update(|_, cx| {
                view.read(cx)
                    .diff_editors
                    .values()
                    .any(|editor| editor.state.read(cx).value().contains("+agent"))
            })
        }),
        "the Diff Tab should show the committed file against the base"
    );
    assert!(window.debug_bounds("diff-comparison").is_some());

    // Back to HEAD: the file has no changes there.
    window.update(|window, cx| compare(window, condr_core::protocol::DiffBase::Head, cx));
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("change-committed.txt").is_none()
                && window.read(|app| {
                    view.read(app)
                        .diff_editors
                        .values()
                        .all(|editor| editor.content == crate::app::changes::DiffContent::Unchanged)
                })
        }),
        "against HEAD the committed file is unchanged"
    );

    let _ = std::fs::remove_dir_all(&repository);
}
