use super::*;
use gpui_kit::{Bounds, Focusable as _, Pixels};

fn double_click(window: &mut VisualTestContext, bounds: Bounds<Pixels>) {
    let position = bounds.center();
    for click_count in [1, 2] {
        window.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        window.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count,
        });
    }
}

/// Git Graph opens from Git History's header as a Tab of its own, lists every branch's
/// commits, filters by a branch double-clicked in its tree and by text on Enter, steps to a
/// parent with Right, and shows a selected commit's files and details, a double-clicked file
/// in the Diff Pane beside it (ADR 0078, ADR 0079).
#[test]
fn git_graph_lists_every_branch_filters_and_opens_a_commits_file() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("gui-git-graph");
    let repository = directory.0.clone();
    run_git(&repository, ["init", "-q", "-b", "main"]);
    run_git(&repository, ["config", "user.email", "condr@example.com"]);
    run_git(&repository, ["config", "user.name", "Condr"]);
    run_git(&repository, ["config", "commit.gpgsign", "false"]);
    std::fs::write(repository.join("notes.txt"), "alpha\n").unwrap();
    run_git(&repository, ["add", "notes.txt"]);
    run_git(&repository, ["commit", "-q", "-m", "first"]);
    run_git(&repository, ["checkout", "-q", "-b", "feature"]);
    std::fs::write(repository.join("feature.txt"), "work\n").unwrap();
    run_git(&repository, ["add", "feature.txt"]);
    run_git(&repository, ["commit", "-q", "-m", "feature work"]);
    run_git(&repository, ["checkout", "-q", "main"]);
    std::fs::write(repository.join("notes.txt"), "alpha\nbeta\n").unwrap();
    run_git(&repository, ["commit", "-q", "-am", "second"]);
    run_git(
        &repository,
        [
            "merge",
            "-q",
            "--no-ff",
            "--no-edit",
            "-m",
            "merge",
            "feature",
        ],
    );
    run_git(&repository, ["tag", "v1"]);

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    cx.update(crate::app::startup::bind_keys);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));
    let button = window.debug_bounds("open-project").unwrap();
    window.simulate_click(button.center(), Modifiers::default());
    let selected_root = repository.clone();
    window.simulate_path_prompt_response(move |_| Some(vec![selected_root]));
    assert!(wait_until(window, |window| {
        window.update(|window, cx| _ = window.draw(cx));
        window.debug_bounds("right-sidebar-git-history").is_some()
    }));
    let history = window.debug_bounds("right-sidebar-git-history").unwrap();
    window.simulate_click(history.center(), Modifiers::default());
    window.update(|window, cx| _ = window.draw(cx));
    let open = window
        .debug_bounds("right-sidebar-git-graph")
        .expect("Git History's header opens Git Graph");
    window.simulate_click(open.center(), Modifiers::default());

    // The Server makes the Pane, in a Tab of its own, and this window shows it.
    let graph = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app).active_session().and_then(|session| {
                let workspace_id = session.workspaces()[0].id();
                let (tab, pane) = session.viewer_pane(workspace_id, ViewerKind::GitGraph)?;
                Some((workspace_id, tab.id(), pane.id()))
            })
        })
    };
    assert!(wait_until(window, |window| graph(window).is_some()));
    let (workspace_id, graph_tab, pane_id) = graph(window).unwrap();
    let presented_tab = |window: &mut VisualTestContext| {
        window.read(|app| view.read(app).presented().map(|(_, _, _, tab_id)| tab_id))
    };
    assert_eq!(presented_tab(window), Some(graph_tab));
    assert!(
        window.read(|app| view.read(app).right_sidebar.open.is_empty()),
        "Git Graph closes the right sidebar it was opened from"
    );
    let subjects = |window: &mut VisualTestContext| {
        window.update(|window, cx| _ = window.draw(cx));
        window.read(|app| view.read(app).git_graph_subjects(1, pane_id, app))
    };
    assert!(
        wait_until(window, |window| subjects(window).len() == 4),
        "every branch's commits: {:?}",
        subjects(window)
    );
    assert_eq!(
        subjects(window),
        ["merge", "second", "feature work", "first"]
    );
    assert!(window.debug_bounds("git-graph").is_some());

    // A double-click in the tree filters by that branch; the clear button shows them all.
    let feature = window
        .debug_bounds("git-graph-ref-refs/heads/feature")
        .expect("the tree lists the local branch");
    double_click(window, feature);
    assert!(wait_until(window, |window| {
        subjects(window) == ["feature work", "first"]
    }));
    assert!(window.debug_bounds("git-graph-branch-filter").is_some());
    let clear = window.debug_bounds("git-graph-clear-branch").unwrap();
    window.simulate_click(clear.center(), Modifiers::default());
    assert!(wait_until(window, |window| subjects(window).len() == 4));

    // Right goes from the merge to its first parent.
    let table = window
        .read(|app| view.read(app).git_graph_table(1, pane_id))
        .unwrap();
    window.update(|window, cx| {
        table.update(cx, |table, cx| table.set_selected_row(0, cx));
        window.focus(&table.read(cx).focus_handle(cx), cx);
    });
    window.simulate_keystrokes("right");
    assert_eq!(
        window.read(|app| table.read(app).selected_row()),
        Some(1),
        "Right selects the merge's first parent"
    );

    // A double-clicked file opens the Diff in a Tab of its own the first time.
    let open_file = |window: &mut VisualTestContext, row: usize, file: &'static str| {
        window.update(|_, cx| table.update(cx, |table, cx| table.set_selected_row(row, cx)));
        assert!(wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds(file).is_some()
        }));
        let bounds = window.debug_bounds(file).unwrap();
        double_click(window, bounds);
    };
    let diff_shows = |window: &mut VisualTestContext, line: &'static str| {
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("diff-commit-header").is_some()
                && window.read(|app| {
                    view.read(app)
                        .diff_pane
                        .viewers
                        .values()
                        .any(|viewer| viewer.patch.contains(line))
                })
        })
    };
    open_file(window, 1, "git-graph-file-notes.txt");
    assert!(
        diff_shows(window, "+beta"),
        "the Diff shows the commit's change"
    );
    let diff_tab = window
        .read(|app| {
            let session = view.read(app).active_session()?;
            Some(session.viewer_pane(workspace_id, ViewerKind::Diff)?.0.id())
        })
        .unwrap();
    assert_ne!(diff_tab, graph_tab);
    assert_eq!(presented_tab(window), Some(diff_tab));

    // Merged beside Git Graph once, the Diff is retargeted there without leaving its Tab.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::MoveTabInto {
                tab_id: diff_tab,
                target_pane_id: pane_id,
                side: condr_core::PaneDirection::Right,
            });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session.tab(diff_tab).is_none()
                    && session
                        .tab(graph_tab)
                        .is_some_and(|tab| tab.panes().len() == 2)
            })
        })
    }));
    let graph_tab_button = {
        window.update(|window, cx| _ = window.draw(cx));
        window
            .debug_bounds(leaked_selector(format!("tab-{}", graph_tab.as_u64())))
            .unwrap()
    };
    window.simulate_click(graph_tab_button.center(), Modifiers::default());
    assert!(wait_until(window, |window| presented_tab(window) == Some(graph_tab)));
    open_file(window, 2, "git-graph-file-feature.txt");
    assert!(
        diff_shows(window, "+work"),
        "the Diff beside Git Graph follows it"
    );
    assert_eq!(
        presented_tab(window),
        Some(graph_tab),
        "nothing left the Tab"
    );

    // The text filter applies on Enter, and matches the message; its key works while Git
    // Graph has the keyboard.
    window.update(|window, cx| window.focus(&table.read(cx).focus_handle(cx), cx));
    let find = if cfg!(target_os = "macos") {
        "cmd-l"
    } else {
        "ctrl-l"
    };
    window.simulate_keystrokes(find);
    window.simulate_input("feature");
    window.simulate_keystrokes("enter");
    assert!(wait_until(window, |window| subjects(window) == ["feature work"]));

    // The selected commit lists its files, details and the branches it is on.
    window.update(|_, cx| table.update(cx, |table, cx| table.set_selected_row(0, cx)));
    assert!(wait_until(window, |window| {
        window.update(|window, cx| _ = window.draw(cx));
        window.debug_bounds("git-graph-file-feature.txt").is_some()
            && window.debug_bounds("git-graph-details").is_some()
    }));
    let feature_work = window.read(|app| view.read(app).git_graph_ids(1, pane_id, app)[0].clone());
    assert!(
        wait_until(window, |window| {
            window.read(|app| {
                let resources = &view.read(app).connection(1).unwrap().resources;
                resources
                    .commit_branches
                    .keys()
                    .find(|(_, id)| *id == feature_work)
                    .and_then(|slot| resources.commit_branches.get(slot))
                    .is_some_and(|branches| {
                        branches.as_ref().is_ok_and(|names| {
                            names.contains(&"refs/heads/feature".to_owned())
                                && names.contains(&"refs/heads/main".to_owned())
                        })
                    })
            })
        }),
        "the details ask which branches the commit is on"
    );
}
