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

    // Refresh asks the Server to rescan, and spins until its `WorkspaceFilesChanged`
    // arrives (ADR 0046).
    let refresh = window
        .debug_bounds("sidebar-refresh")
        .expect("the sidebar header should offer Refresh");
    // At the default width the three tabs and Refresh share the header without running
    // into each other; the summed `+N −N` sits on the comparison row instead.
    let history = window
        .debug_bounds("sidebar-view-history")
        .expect("a repository with a commit has History");
    assert!(
        history.right() <= refresh.left(),
        "History {history:?} runs under Refresh {refresh:?}"
    );
    window.simulate_click(refresh.center(), Modifiers::default());
    assert!(
        window.read(|app| !view.read(app).files_view.refreshing.is_empty()),
        "the click should start a refresh"
    );
    assert!(
        wait_until(window, |window| {
            window.read(|app| view.read(app).files_view.refreshing.is_empty())
        }),
        "the Server's rescan should end the refresh"
    );

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
        "the Server's diff should reach the Diff Tab"
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

    // "Show File" opens the Preview Tab at the new-file line last clicked in the diff:
    // the hunk is " alpha", "-beta", "+BETA", "+gamma", so a click on the old side's
    // "beta" lands on "BETA", the second line of the file.
    window.update(|_, cx| {
        view.update(cx, |view, _| {
            let viewer = view
                .files_view
                .diff_viewers
                .values_mut()
                .next()
                .expect("one Diff Tab viewer");
            viewer.clicked = Some(gpui_kit::component::diff::DiffLinePosition::new(
                crate::app::changes::PATCH_PATH,
                gpui_kit::component::diff::DiffSide::Original,
                2,
            ));
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
                }) && view.files_view.file_editors.values().any(|editor| {
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
    // And its diff replaces the first one's after that single click.
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.update(|_, cx| {
                view.read(cx)
                    .files_view
                    .diff_viewers
                    .values()
                    .any(|viewer| viewer.patch.contains("+new") && !viewer.patch.contains("BETA"))
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
                    .and_then(|connection| connection.model.workspace_git.values().next())
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
                    .files_view
                    .diff_viewers
                    .values()
                    .any(|viewer| viewer.patch.contains("+agent"))
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
                        .files_view
                        .diff_viewers
                        .values()
                        .all(|viewer| viewer.content == crate::app::changes::DiffContent::Unchanged)
                })
        }),
        "against HEAD the committed file is unchanged"
    );

    let _ = std::fs::remove_dir_all(&repository);
}

/// A directory with a tracked change and an untracked file is a row in two sections.
/// Each row folds on its own.
#[test]
fn a_directory_listed_in_two_sections_folds_from_either_row() {
    let _serial_guard = acquire_visual_test_lock();
    let repository = std::env::temp_dir().join(format!(
        "condr-gui-changes-dup-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&repository);
    std::fs::create_dir_all(repository.join("src/app")).unwrap();
    run_git(&repository, ["init", "-q", "-b", "main"]);
    run_git(&repository, ["config", "user.email", "condr@example.com"]);
    run_git(&repository, ["config", "user.name", "Condr"]);
    run_git(&repository, ["config", "commit.gpgsign", "false"]);
    std::fs::write(repository.join("src/app/a.txt"), "alpha\n").unwrap();
    run_git(&repository, ["add", "src/app/a.txt"]);
    run_git(&repository, ["commit", "-q", "-m", "a"]);
    std::fs::write(repository.join("src/app/a.txt"), "ALPHA\n").unwrap();
    std::fs::write(repository.join("src/app/b.txt"), "new\n").unwrap();

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
    window.run_until_parked();
    assert!(wait_until(window, |window| {
        window.update(|window, cx| _ = window.draw(cx));
        window.debug_bounds("change-src/app/a.txt").is_some()
            && window.debug_bounds("change-src/app/b.txt").is_some()
    }));

    let row = window
        .debug_bounds("changes-untracked-dir-src/app")
        .unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds("change-src/app/b.txt").is_none(),
        "clicking the Untracked src/app row should fold it"
    );
    assert!(
        window.debug_bounds("change-src/app/a.txt").is_some(),
        "the Tracked src/app row stays open"
    );

    let _ = std::fs::remove_dir_all(&repository);
}

/// History lists the real Server's commits, a commit unfolds into its files, a file opens
/// in the Diff Tab as that commit changed it, and a new commit appears on its own (ADR 0046).
#[test]
fn history_lists_commits_and_shows_one_commits_file_in_the_diff_tab() {
    let _serial_guard = acquire_visual_test_lock();
    let repository = std::env::temp_dir().join(format!(
        "condr-gui-history-{}-{:?}",
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
    run_git(&repository, ["commit", "-q", "-m", "first"]);
    std::fs::write(repository.join("notes.txt"), "alpha\nBETA\n").unwrap();
    run_git(
        &repository,
        ["commit", "-q", "-am", "second\n\nUppercase the beta."],
    );
    let short_head = |repository: &std::path::Path| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(["rev-parse", "--short=7", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let second = short_head(&repository);
    // Uncommitted, so the Diff Tab first shows notes.txt from the working tree.
    std::fs::write(repository.join("notes.txt"), "alpha\nBETA\ngamma\n").unwrap();

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
    assert!(wait_until(window, |window| {
        window.update(|window, cx| _ = window.draw(cx));
        window.debug_bounds("change-notes.txt").is_some()
    }));
    let change = window.debug_bounds("change-notes.txt").unwrap();
    window.simulate_click(change.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.read(|app| {
                view.read(app)
                    .files_view
                    .diff_viewers
                    .values()
                    .any(|viewer| viewer.patch.contains("+gamma"))
            })
        }),
        "the Diff Tab first shows the working tree"
    );

    // The tab appears once the Server's Git snapshot names `HEAD`.
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("sidebar-view-history").is_some()
        }),
        "a Server that sends HEAD offers History"
    );
    let tab = window.debug_bounds("sidebar-view-history").unwrap();
    window.simulate_click(tab.center(), Modifiers::default());
    // `debug_bounds` takes a static name.
    let commit_row: &'static str = format!("commit-{second}").leak();
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds(commit_row).is_some()
        }),
        "History lists the latest commit"
    );

    // Selecting asks for the commit's files, listed below; one of them opens in the Diff Tab.
    let row = window.debug_bounds(commit_row).unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    let file_row: &'static str = format!("commit-file-{second}-notes.txt").leak();
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds(file_row).is_some()
        }),
        "a selected commit lists its files"
    );
    let file = window.debug_bounds(file_row).unwrap();
    window.simulate_click(file.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("diff-commit-header").is_some()
                && window.read(|app| {
                    view.read(app)
                        .files_view
                        .diff_viewers
                        .values()
                        .any(|viewer| {
                            viewer.patch.contains("-beta") && viewer.patch.contains("+BETA")
                        })
                })
        }),
        "the same path's Diff Tab switches to the file as the commit changed it"
    );
    assert!(window.read(|app| {
        view.read(app)
            .files_view
            .diff_viewers
            .values()
            .all(|viewer| !viewer.patch.contains("gamma"))
    }));
    assert!(window.debug_bounds("diff-copy-commit").is_some());
    let message = window.debug_bounds("diff-commit-message").unwrap();
    window.simulate_click(message.center(), Modifiers::default());
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("diff-commit-body").is_some());

    // Another click on the commit lets it go, and its files go with it.
    let row = window.debug_bounds(commit_row).unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds(file_row).is_none(),
        "a second click hides the commit's files"
    );

    // A commit made elsewhere moves HEAD, and History lists it without a refresh.
    run_git(&repository, ["commit", "-q", "-am", "third"]);
    let third: &'static str = format!("commit-{}", short_head(&repository)).leak();
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds(third).is_some() && window.debug_bounds(commit_row).is_some()
        }),
        "a new commit should appear above the others"
    );

    let _ = std::fs::remove_dir_all(&repository);
}
