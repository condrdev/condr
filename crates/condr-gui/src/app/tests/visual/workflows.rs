use super::*;

#[test]
fn rename_dialogs_commit_server_workspace_and_tab_names() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.prompt_rename_server_on(1, "Local".into(), window, cx)
        });
    });
    submit_text_dialog(window, "Build Server");
    assert_eq!(
        window.read(|app| view.read(app).connection(1).unwrap().label.clone()),
        "Build Server"
    );

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| {
        window
            .read(|app| {
                let session = view.read(app).active_session()?;
                let workspace = session.active_workspace()?;
                Some((workspace.id(), workspace.active_tab().id()))
            })
            .is_some()
    }));
    let (workspace_id, tab_id) = window
        .read(|app| {
            let session = view.read(app).active_session()?;
            let workspace = session.active_workspace()?;
            Some((workspace.id(), workspace.active_tab().id()))
        })
        .unwrap();

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.prompt_rename_workspace_on(1, workspace_id, "Workspace 1".into(), window, cx)
        });
    });
    submit_text_dialog(window, "Build Workspace");
    let workspace_renamed = wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .workspace(workspace_id)
                    .is_some_and(|workspace| workspace.name() == "Build Workspace")
            })
        })
    });
    assert!(
        workspace_renamed,
        "Workspace rename did not reach the server"
    );

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.prompt_rename_tab_on(1, tab_id, "Tab 1".into(), window, cx)
        });
    });
    submit_text_dialog(window, "Build Tab");
    let tab_renamed = wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .tab(tab_id)
                    .is_some_and(|tab| tab.name() == "Build Tab")
            })
        })
    });
    assert!(tab_renamed, "Tab rename did not reach the server");
}

#[test]
fn worktree_actions_use_the_workspace_context_and_real_server() {
    let _serial_guard = acquire_visual_test_lock();
    let temp = TestDirectory::new("worktree-ui");
    let repository = temp.0.join("repository");
    let worktree = temp.0.join("existing-worktree");
    let created_branch = format!(
        "feature/ui-created-{}-{}",
        std::process::id(),
        NEXT_TEST_SERVER_ID.fetch_add(1, Ordering::Relaxed)
    );
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, ["init"]);
    run_git(&repository, ["config", "user.name", "Condr Tests"]);
    run_git(
        &repository,
        ["config", "user.email", "condr@example.invalid"],
    );
    std::fs::write(repository.join("README.md"), "condr\n").unwrap();
    run_git(&repository, ["add", "README.md"]);
    run_git(&repository, ["commit", "-m", "initial"]);
    run_git(
        &repository,
        [
            std::ffi::OsString::from("worktree"),
            std::ffi::OsString::from("add"),
            std::ffi::OsString::from("-b"),
            std::ffi::OsString::from("feature/ui"),
            worktree.as_os_str().to_os_string(),
        ],
    );

    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: repository.clone(),
            });
        });
    });
    let mut parent_workspace_id = None;
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            parent_workspace_id = condr
                .active_session()
                .and_then(|session| session.active_workspace_id());
            parent_workspace_id.is_some()
                && condr
                    .connection(1)
                    .is_some_and(|connection| !connection.workspace_git.is_empty())
        })
    }));
    window.update(|window, cx| _ = window.draw(cx));

    let workspace = window
        .debug_bounds(sidebar_workspace_selector(parent_workspace_id.unwrap()))
        .expect("Git Workspace should render in the sidebar");
    window.simulate_mouse_down(workspace.center(), MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.simulate_keystrokes("down down enter");
    window.run_until_parked();
    assert!(
        window.update(|window, cx| window.has_active_dialog(cx)),
        "Create Worktree should open its branch dialog"
    );
    submit_text_dialog(window, &created_branch);

    let mut managed_workspace = None;
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            managed_workspace = condr.active_session().and_then(|session| {
                session.workspaces().iter().find_map(|workspace| {
                    workspace
                        .worktree()
                        .is_some_and(|association| association.is_managed())
                        .then(|| (workspace.id(), workspace.root_directory().to_path_buf()))
                })
            });
            managed_workspace.as_ref().is_some_and(|(workspace_id, _)| {
                condr
                    .connection(1)
                    .and_then(|connection| connection.workspace_git.get(workspace_id))
                    .and_then(|git| git.branch.as_deref())
                    == Some(created_branch.as_str())
            })
        })
    }));
    let (managed_workspace_id, managed_root) = managed_workspace.unwrap();
    std::fs::write(managed_root.join("untracked.txt"), "keep me\n").unwrap();

    window.update(|window, cx| _ = window.draw(cx));
    let managed = window
        .debug_bounds(sidebar_workspace_selector(managed_workspace_id))
        .expect("managed worktree branch should render in the sidebar");
    window.simulate_mouse_down(managed.center(), MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.simulate_keystrokes("down down enter");
    window.run_until_parked();
    confirm_alert_dialog(window);
    let dirty_rejected = wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .and_then(|connection| connection.error.as_deref())
                .is_some_and(|error| error.contains("modified or untracked"))
        })
    });
    let (sequence, error) = window.read(|app| {
        let connection = view.read(app).connection(1).unwrap();
        (connection.sequence, connection.error.clone())
    });
    assert!(
        dirty_rejected,
        "dirty worktree removal did not reach the GUI; checkout exists: {}, client sequence: {sequence}, client error: {error:?}",
        managed_root.exists(),
    );
    assert!(managed_root.exists());

    std::fs::remove_file(managed_root.join("untracked.txt")).unwrap();
    let managed_name = window.read(|app| {
        view.read(app)
            .active_session()
            .and_then(|session| {
                session
                    .workspace(managed_workspace_id)
                    .map(|workspace| workspace.name().to_owned())
            })
            .unwrap()
    });
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.confirm_remove_worktree_on(
                1,
                managed_workspace_id,
                managed_name.clone(),
                window,
                cx,
            );
        });
    });
    window.run_until_parked();
    confirm_alert_dialog(window);
    let removed = wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspace(managed_workspace_id).is_none())
        })
    });
    let (client_has_workspace, sequence, error) = window.read(|app| {
        let condr = view.read(app);
        let connection = condr.connection(1).unwrap();
        (
            condr
                .active_session()
                .is_some_and(|session| session.workspace(managed_workspace_id).is_some()),
            connection.sequence,
            connection.error.clone(),
        )
    });
    let server_has_workspace = Session::restore(server.handle.snapshot())
        .unwrap()
        .workspace(managed_workspace_id)
        .is_some();
    assert!(
        removed,
        "clean worktree removal did not complete; checkout exists: {}, client Workspace exists: {client_has_workspace}, Server Workspace exists: {server_has_workspace}, client sequence: {sequence}, client error: {error:?}",
        managed_root.exists(),
    );
    assert!(!managed_root.exists());
    run_git(
        &repository,
        [
            std::ffi::OsString::from("show-ref"),
            std::ffi::OsString::from("--verify"),
            std::ffi::OsString::from(format!("refs/heads/{created_branch}")),
        ],
    );

    window.update(|window, cx| _ = window.draw(cx));
    let workspace = window
        .debug_bounds(sidebar_workspace_selector(parent_workspace_id.unwrap()))
        .unwrap();
    window.simulate_mouse_down(workspace.center(), MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.simulate_keystrokes("down down down enter");
    window.run_until_parked();
    assert!(window.did_prompt_for_paths());
    let selected_worktree = worktree.clone();
    window.simulate_path_prompt_response(move |options| {
        assert!(!options.files);
        assert!(options.directories);
        assert!(!options.multiple);
        Some(vec![selected_worktree])
    });

    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session.workspaces().iter().any(|workspace| {
                    workspace.root_directory() == worktree
                        && workspace
                            .worktree()
                            .is_some_and(|association| !association.is_managed())
                })
            })
        })
    }));
    assert!(window.read(|app| {
        view.read(app)
            .connection(1)
            .unwrap()
            .workspace_git
            .values()
            .any(|git| git.branch.as_deref() == Some("feature/ui"))
    }));
}

#[cfg(target_os = "linux")]
#[test]
fn detected_agent_sidebar_item_activates_its_real_pty_pane() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            });
        });
    });
    let mut agent_pane = None;
    assert!(wait_until(window, |window| {
        agent_pane = window.read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        });
        agent_pane.is_some()
    }));
    let agent_pane = agent_pane.unwrap();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                pane_id: agent_pane,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    let mut other_pane = None;
    assert!(wait_until(window, |window| {
        other_pane = window.read(|app| {
            let session = view.read(app).active_session()?;
            let tab = session.active_workspace()?.active_tab();
            (tab.panes().len() == 2).then(|| tab.focused_pane().id())
        });
        other_pane.is_some_and(|pane_id| pane_id != agent_pane)
    }));

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.terminal_command(
                1,
                agent_pane,
                TerminalCommand::Text(
                    "exec -a codex /bin/bash -c \"echo '◦ Working (1s - esc to interrupt)'; sleep 1; printf '\\033[2J\\033[H›\\n'; sleep 30 & wait\"\r"
                        .into(),
                ),
            );
        });
    });
    let working = wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .and_then(|connection| connection.agents.get(&agent_pane))
                .is_some_and(|agent| agent.state == condr_core::AgentState::Working)
        })
    });
    assert!(
        working,
        "working Agent was not detected; terminal={:?}; error={:?}",
        window.read(|app| {
            view.read(app).terminal(1, agent_pane).map(|terminal| {
                terminal
                    .view
                    .cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
            })
        }),
        window.read(|app| view.read(app).connection(1).unwrap().error.clone()),
    );
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .and_then(|connection| connection.agent_trackers.get(&agent_pane))
                .is_some_and(|tracker| tracker.display_state().label() == "done")
        })
    }));
    window.update(|window, cx| _ = window.draw(cx));
    let done_status = leaked_selector(format!("agent-status-1-{}-done", agent_pane.as_u64()));
    assert!(
        window.debug_bounds(done_status).is_some(),
        "an unseen completion should render the done status icon"
    );
    let agent = window
        .debug_bounds(sidebar_agent_selector(agent_pane))
        .expect("detected Agent should render below its Workspace");
    window.simulate_click(agent.center(), Modifiers::default());

    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session.active_workspace().is_some_and(|workspace| {
                    workspace.active_tab().focused_pane().id() == agent_pane
                })
            })
        })
    }));
    assert_eq!(
        window.read(|app| {
            view.read(app)
                .connection(1)
                .unwrap()
                .agent_trackers
                .get(&agent_pane)
                .unwrap()
                .display_state()
                .label()
        }),
        "idle"
    );
    window.update(|window, cx| _ = window.draw(cx));
    let idle_status = leaked_selector(format!("agent-status-1-{}-idle", agent_pane.as_u64()));
    assert!(window.debug_bounds(done_status).is_none());
    assert!(window.debug_bounds(idle_status).is_some());
}

#[test]
fn tcp_paths_use_server_side_text_dialogs() {
    let _serial_guard = acquire_visual_test_lock();
    let temp = TestDirectory::new("tcp-path-dialogs");
    let repository = temp.0.join("repository");
    let worktree = temp.0.join("existing-worktree");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, ["init"]);
    run_git(&repository, ["config", "user.name", "Condr Tests"]);
    run_git(
        &repository,
        ["config", "user.email", "condr@example.invalid"],
    );
    std::fs::write(repository.join("README.md"), "condr\n").unwrap();
    run_git(&repository, ["add", "README.md"]);
    run_git(&repository, ["commit", "-m", "initial"]);
    run_git(
        &repository,
        [
            std::ffi::OsString::from("worktree"),
            std::ffi::OsString::from("add"),
            std::ffi::OsString::from("-b"),
            std::ffi::OsString::from("feature/tcp-path"),
            worktree.as_os_str().to_os_string(),
        ],
    );

    let (server, endpoint) = start_tcp_server();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    window.update(|window, cx| _ = window.draw(cx));
    let new_workspace = window
        .debug_bounds("new-terminal-workspace")
        .expect("TCP Start Page should offer New Workspace");
    window.simulate_click(new_workspace.center(), Modifiers::default());
    window.run_until_parked();
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
        !window.did_prompt_for_paths(),
        "a TCP Server path must not use the client filesystem picker"
    );
    submit_text_dialog(window, &repository.to_string_lossy());

    let mut parent_workspace_id = None;
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            parent_workspace_id = condr
                .active_session()
                .and_then(|session| session.active_workspace_id());
            parent_workspace_id.is_some()
                && condr
                    .connection(1)
                    .is_some_and(|connection| !connection.workspace_git.is_empty())
        })
    }));

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.choose_worktree_directory_on(
                1,
                parent_workspace_id.unwrap(),
                "repository".into(),
                window,
                cx,
            )
        });
    });
    window.run_until_parked();
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
        !window.did_prompt_for_paths(),
        "a TCP worktree path must not use the client filesystem picker"
    );
    submit_text_dialog(window, &worktree.to_string_lossy());

    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session.workspaces().iter().any(|workspace| {
                    workspace.root_directory() == worktree
                        && workspace
                            .worktree()
                            .is_some_and(|association| !association.is_managed())
                })
            })
        })
    }));
}

#[test]
fn context_menus_reorder_the_target_items_without_changing_focus() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("reorder-first");
    let second_root = TestDirectory::new("reorder-second");
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, server) = connected_condr(&mut cx);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: first_root.0.clone(),
            });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspaces().len() == 1)
        })
    }));
    let first_workspace =
        window.read(|app| view.read(app).active_session().unwrap().workspaces()[0].id());

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: second_root.0.clone(),
            });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspaces().len() == 2)
        })
    }));
    let second_workspace = window.read(|app| {
        view.read(app)
            .active_session()
            .unwrap()
            .active_workspace_id()
            .unwrap()
    });

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateTab {
                workspace_id: second_workspace,
            });
        });
    });
    let mut tab_state = None;
    assert!(wait_until(window, |window| {
        tab_state = window.read(|app| {
            let session = view.read(app).active_session()?;
            let workspace = session.workspace(second_workspace)?;
            (workspace.tabs().len() == 2).then(|| {
                (
                    workspace.tabs()[0].id(),
                    workspace.active_tab().id(),
                    workspace.active_tab().focused_pane().id(),
                )
            })
        });
        tab_state.is_some()
    }));
    let (first_tab, active_tab, focused_pane) = tab_state.unwrap();

    window.update(|window, cx| _ = window.draw(cx));
    let workspace = window
        .debug_bounds(sidebar_workspace_selector(first_workspace))
        .expect("the non-active Workspace should render in the sidebar");
    window.simulate_mouse_down(workspace.center(), MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    window.simulate_keystrokes("down down enter");
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session.workspaces()[0].id() == second_workspace
                    && session.workspaces()[1].id() == first_workspace
            })
        })
    }));

    window.update(|window, cx| _ = window.draw(cx));
    let tab = window
        .debug_bounds(tab_selector(first_tab))
        .expect("the non-active Tab should render");
    window.simulate_mouse_down(tab.center(), MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    window.simulate_keystrokes("down down enter");
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                let workspace = session.workspace(second_workspace).unwrap();
                workspace.tabs()[0].id() == active_tab && workspace.tabs()[1].id() == first_tab
            })
        })
    }));

    let assert_identity = |session: &Session| {
        assert_eq!(session.active_workspace_id(), Some(second_workspace));
        let workspace = session.workspace(second_workspace).unwrap();
        assert_eq!(workspace.active_tab().id(), active_tab);
        assert_eq!(workspace.active_tab().focused_pane().id(), focused_pane);
    };
    window.read(|app| assert_identity(&view.read(app).active_session().unwrap()));
    let authoritative = Session::restore(server.handle.snapshot()).unwrap();
    assert_eq!(authoritative.workspaces()[0].id(), second_workspace);
    assert_eq!(authoritative.workspaces()[1].id(), first_workspace);
    assert_eq!(
        authoritative.workspace(second_workspace).unwrap().tabs()[0].id(),
        active_tab
    );
    assert_eq!(
        authoritative.workspace(second_workspace).unwrap().tabs()[1].id(),
        first_tab
    );
    assert_identity(&authoritative);
}

#[test]
fn new_workspace_round_trip_updates_gui_from_real_server() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, server) = connected_condr(&mut cx);
    let button = window
        .debug_bounds("new-terminal-workspace")
        .expect("new workspace button should be rendered");
    window.simulate_click(button.center(), Modifiers::default());
    assert!(window.did_prompt_for_paths());
    let workspace_root = std::env::temp_dir();
    let selected_root = workspace_root.clone();
    window.simulate_path_prompt_response(move |_| Some(vec![selected_root]));

    let received = wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.active_workspace().is_some())
        })
    });
    assert!(
        received,
        "GUI did not render the workspace created by the real server; server workspaces: {}, client workspaces: {}, client sequence: {}, connection error: {:?}",
        condr_core::Session::restore(server.handle.snapshot())
            .unwrap()
            .workspaces()
            .len(),
        window.read(|app| condr_core::Session::restore(
            view.read(app).connection(1).unwrap().snapshot.clone()
        )
        .unwrap()
        .workspaces()
        .len()),
        window.read(|app| view.read(app).connection(1).unwrap().sequence),
        window.read(|app| view.read(app).connection(1).unwrap().error.clone()),
    );
    assert_eq!(
        window.read(|app| {
            view.read(app)
                .active_session()
                .unwrap()
                .active_workspace()
                .unwrap()
                .root_directory()
                .to_path_buf()
        }),
        workspace_root
    );

    let new_tab = window
        .debug_bounds("new-tab")
        .expect("new tab button should be rendered after workspace creation");
    window.simulate_click(new_tab.center(), Modifiers::default());
    let tab_created = wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .active_workspace()
                    .is_some_and(|workspace| workspace.tabs().len() == 2)
            })
        })
    });
    assert!(
        tab_created,
        "GUI did not render the tab created by the real server"
    );

    let new_pane = window
        .read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        })
        .unwrap();
    let new_tab_focused = wait_until(window, |window| {
        let (terminal_ready, focus) = window.read(|app| {
            let condr = view.read(app);
            (
                condr
                    .connection(1)
                    .and_then(|connection| connection.terminals.get(&new_pane))
                    .is_some_and(|terminal| !terminal.exited),
                condr
                    .panels
                    .get(&(1, new_pane))
                    .map(|panel| panel.read(app).focus_handle.clone()),
            )
        });
        terminal_ready
            && focus.is_some_and(|focus| window.update(|window, _| focus.is_focused(window)))
    });
    assert!(
        new_tab_focused,
        "the terminal in a newly created Tab did not receive keyboard focus"
    );
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds(terminal_selector(new_pane)).is_some(),
        "the terminal in a newly created Tab was not painted before text input"
    );
    window.simulate_input("CONDR_NEW_TAB_FOCUS");
    let new_tab_received_input = wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .and_then(|connection| connection.terminals.get(&new_pane))
                .is_some_and(|terminal| {
                    terminal
                        .view
                        .cells
                        .iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                        .contains("CONDR_NEW_TAB_FOCUS")
                })
        })
    });
    assert!(
        new_tab_received_input,
        "the terminal in a newly created Tab did not receive text input"
    );

    let active_tab = window
        .read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().id())
        })
        .unwrap();
    let tab = window
        .debug_bounds(tab_selector(active_tab))
        .expect("active Tab should be rendered");
    window.simulate_mouse_down(tab.center(), MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| {
        _ = window.draw(cx);
    });
    window.simulate_keystrokes("down enter");
    window.run_until_parked();
    assert!(
        window.update(|window, cx| window.has_active_dialog(cx)),
        "Rename Tab should open from its context menu"
    );
    window.update(|window, cx| window.close_dialog(cx));
    window.run_until_parked();

    let initial_pane = window
        .read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        })
        .unwrap();
    let initial_terminal = terminal_selector(initial_pane);
    let terminal_ready = wait_until(window, |window| {
        window.read(|app| {
            view.read(app).connection(1).is_some_and(|connection| {
                connection
                    .terminals
                    .values()
                    .any(|terminal| !terminal.exited)
            })
        }) && window.debug_bounds(initial_terminal).is_some()
    });
    assert!(terminal_ready, "GUI did not render the server PTY");

    let terminal = window.debug_bounds(initial_terminal).unwrap();
    window.simulate_mouse_down(terminal.center(), MouseButton::Right, Modifiers::default());
    window.simulate_mouse_up(terminal.center(), MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| {
        _ = window.draw(cx);
    });
    let terminal_focus = window.read(|app| {
        view.read(app).panels[&(1, initial_pane)]
            .read(app)
            .focus_handle
            .clone()
    });
    assert!(
        window.update(|window, _| terminal_focus.is_focused(window)),
        "right-clicking a Pane must keep focus in the terminal"
    );
    let pane_menu = window
        .debug_bounds(leaked_selector(format!(
            "terminal-pane-menu-{}",
            initial_pane.as_u64()
        )))
        .expect("Pane actions button should be rendered");
    window.simulate_mouse_down(pane_menu.center(), MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(pane_menu.center(), MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| {
        _ = window.draw(cx);
    });
    window.simulate_keystrokes("down enter");
    let split_right = wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .active_workspace()
                    .is_some_and(|workspace| workspace.active_tab().panes().len() == 2)
            })
        })
    });
    assert!(split_right, "Pane actions menu did not split right");
    assert!(
        wait_until_event_driven(window, |window| {
            window.read(|app| {
                view.read(app)
                    .connection(1)
                    .is_some_and(ServerConnection::can_mutate)
            })
        }),
        "GUI did not finish synchronizing the split"
    );

    let (pane_to_focus, rebuilds_before_focus) = window.read(|app| {
        let condr = view.read(app);
        let workspace = condr.active_session().unwrap();
        let tab = workspace.active_workspace().unwrap().active_tab();
        let focused = tab.focused_pane().id();
        let other = tab
            .panes()
            .iter()
            .find(|pane| pane.id() != focused)
            .unwrap()
            .id();
        (other, condr.dock_rebuild_count)
    });
    let other_terminal = window
        .debug_bounds(terminal_selector(pane_to_focus))
        .expect("the other Pane should be rendered");
    window.simulate_click(other_terminal.center(), Modifiers::default());
    let pane_focused = wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session.active_workspace().is_some_and(|workspace| {
                    workspace.active_tab().focused_pane().id() == pane_to_focus
                })
            })
        })
    });
    assert!(pane_focused, "clicking a Pane did not focus it");
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds_before_focus,
        "focus-only updates must not rebuild Dock"
    );

    window.simulate_keystrokes("alt-shift--");
    let split_down = wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .active_workspace()
                    .is_some_and(|workspace| workspace.active_tab().panes().len() == 3)
            })
        })
    });
    assert!(split_down, "Alt+Shift+- did not split down");

    window.simulate_input("printf CONDR_E2E");
    window.simulate_keystrokes("enter");

    let output_received = wait_until(window, |window| {
        window.read(|app| {
            view.read(app).connection(1).is_some_and(|connection| {
                connection.terminals.values().any(|terminal| {
                    terminal
                        .view
                        .cells
                        .iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                        .contains("CONDR_E2E")
                })
            })
        })
    });
    assert!(
        output_received,
        "GUI did not receive output from the server PTY"
    );

    let active_tab = window.read(|app| {
        view.read(app).active_session().and_then(|session| {
            session
                .active_workspace()
                .map(|workspace| workspace.active_tab().id())
        })
    });
    window.simulate_keystrokes("ctrl-tab");
    let tab_switched = wait_until(window, |window| {
        let next_tab = window.read(|app| {
            view.read(app).active_session().and_then(|session| {
                session
                    .active_workspace()
                    .map(|workspace| workspace.active_tab().id())
            })
        });
        next_tab.is_some() && next_tab != active_tab
    });
    assert!(
        tab_switched,
        "Ctrl+Tab did not switch tabs through the real server"
    );
}

#[test]
fn visual_context_can_resize_condr_window() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (_, window, _server) = connected_condr(&mut cx);
    window.simulate_resize(size(px(1280.), px(720.)));
    let bounds = window
        .debug_bounds("condr-sidebar")
        .expect("sidebar should remain rendered");
    assert!(bounds.size.width > px(0.));
    assert!(bounds.size.height > px(0.));
    window.quit();
}
