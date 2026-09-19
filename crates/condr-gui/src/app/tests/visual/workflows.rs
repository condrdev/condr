use super::*;

#[test]
fn numbered_tab_shortcuts_follow_order_and_stay_out_of_terminal_input() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("numbered-tabs");
    let mut session = Session::new();
    session.create_workspace(std::env::temp_dir()).unwrap();
    let workspace_id = session.create_workspace(directory.0.clone()).unwrap();
    let first_tab = session.workspace(workspace_id).unwrap().tabs()[0].id();
    let second_tab = session.create_tab(workspace_id, None).unwrap();
    assert!(session.rename_tab(first_tab, "Named Tab"));
    let snapshot_path = directory.0.join("session.snapshot");
    std::fs::write(&snapshot_path, session.snapshot().to_bytes().unwrap()).unwrap();
    let endpoint = Endpoint::local(directory.0.join("server.sock"));
    let server = start_server_with_config(
        ServerConfig::ephemeral(endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path),
    );
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, server) = connected_condr_with(&mut cx, server, endpoint);
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "alt"
    };
    let selected = |window: &mut VisualTestContext, tab_id| {
        window.read(|app| {
            let condr = view.read(app);
            condr
                .active_dock_surface
                .is_some_and(|surface| surface.tab_id == tab_id)
                && condr
                    .presented()
                    .is_some_and(|(_, _, presented_workspace, presented_tab)| {
                        presented_workspace == workspace_id && presented_tab == tab_id
                    })
        })
    };
    // The view is this Client's own (ADR 0021): it opens on the first Workspace and Tab.
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_workspace(1, workspace_id, window, cx)
        });
    });
    assert!(wait_until(window, |window| selected(window, first_tab)));
    let structure = server.handle.snapshot();

    for (number, tab_id) in [(2, second_tab), (1, first_tab)] {
        window.simulate_keystrokes(&format!("{modifier}-{number}"));
        assert!(wait_until(window, |window| selected(window, tab_id)));
    }
    assert_eq!(
        server.handle.snapshot(),
        structure,
        "switching Tabs changes nothing on the Server"
    );

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::MoveTab {
                tab_id: second_tab,
                target_index: 0,
            });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .unwrap()
                .workspace(workspace_id)
                .unwrap()
                .tabs()[0]
                .id()
                == second_tab
        })
    }));
    for (number, tab_id) in [(2, first_tab), (1, second_tab)] {
        window.simulate_keystrokes(&format!("{modifier}-{number}"));
        assert!(wait_until(window, |window| selected(window, tab_id)));
    }
    let pane_id = window.read(|app| view.read(app).target_pane.unwrap().1);
    let terminal = window.debug_bounds(terminal_selector(pane_id)).unwrap();
    window.simulate_click(terminal.center(), Modifiers::default());
    window.run_until_parked();

    // Capture requests to prove that even missing numbers never become PTY input.
    let (outgoing, received) = std::sync::mpsc::channel();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().io = Some(ClientIo {
                outgoing,
                _incoming_task: Task::ready(()),
            });
        });
    });
    window.simulate_keystrokes(&format!("{modifier}-9"));
    window.run_until_parked();
    assert!(selected(window, second_tab));
    assert!(received.try_iter().next().is_none());

    window.update(|window, cx| {
        view.update(cx, |this, cx| this.prompt_rename_tab(window, cx));
    });
    window.run_until_parked();
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    window.simulate_keystrokes(&format!("{modifier}-2"));
    window.run_until_parked();
    assert!(selected(window, second_tab));
    assert!(received.try_iter().next().is_none());
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));

    window.simulate_keystrokes(&format!("{modifier}-2"));
    window.run_until_parked();
    assert!(selected(window, first_tab));
    let messages = received.try_iter().collect::<Vec<_>>();
    assert!(
        messages.is_empty(),
        "the shortcut switches the view locally and sends nothing: {messages:?}"
    );
}

#[test]
fn rename_dialogs_commit_server_workspace_and_tab_names() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.prompt_text(
                "Rename Server",
                "Save",
                "Local".into(),
                |this, name, _, _| {
                    this.connection_mut(1).unwrap().label = name;
                    Ok(())
                },
                window,
                cx,
            )
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
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| {
        window
            .read(|app| {
                let session = view.read(app).active_session()?;
                let workspace = session.workspaces().first()?;
                Some((workspace.id(), workspace.tabs().first().unwrap().id()))
            })
            .is_some()
    }));
    let (workspace_id, tab_id) = window
        .read(|app| {
            let session = view.read(app).active_session()?;
            let workspace = session.workspaces().first()?;
            Some((workspace.id(), workspace.tabs().first().unwrap().id()))
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
        view.update(cx, |this, cx| this.prompt_rename_tab(window, cx));
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
    cx.update(gpui_kit::init);
    let (view, window, server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: repository.clone(),
            });
        });
    });
    let mut parent_workspace_id = None;
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            parent_workspace_id = condr
                .presented()
                .map(|(_, _, workspace_id, _)| workspace_id);
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
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("modified or untracked"))
        })
    });
    let (sequence, error) = window.read(|app| {
        let condr = view.read(app);
        (
            condr.connection(1).unwrap().sequence,
            condr.last_error.clone(),
        )
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
            condr.last_error.clone(),
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
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    let mut agent_pane = None;
    assert!(wait_until(window, |window| {
        agent_pane = window.read(|app| {
            view.read(app)
                .active_session()?
                .workspaces()
                .first()
                .map(|workspace| {
                    workspace
                        .tabs()
                        .first()
                        .unwrap()
                        .focused_pane()
                        .unwrap()
                        .id()
                })
        });
        agent_pane.is_some()
    }));
    let agent_pane = agent_pane.unwrap();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                focus: true,
                pane_id: agent_pane,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    let mut other_pane = None;
    assert!(wait_until(window, |window| {
        other_pane = window.read(|app| {
            let session = view.read(app).active_session()?;
            let tab = session.workspaces().first()?.tabs().first().unwrap();
            (tab.panes().len() == 2).then(|| tab.focused_pane().unwrap().id())
        });
        other_pane.is_some_and(|pane_id| pane_id != agent_pane)
    }));

    let event = |kind| {
        condr_core::AgentEvent::new(AgentKind::Codex, kind, None, None)
            .encode()
            .iter()
            .map(|byte| format!("\\{byte:03o}"))
            .collect::<String>()
    };
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.terminal_command(
                1,
                agent_pane,
                // ADR 0014: state comes from hooks, while the process supplies identity.
                TerminalCommand::Text(format!(
                    "exec -a codex /bin/bash -c \"printf '{}'; sleep 2; printf '{}'; sleep 30 & wait\"\r",
                    event(condr_core::AgentEventKind::PromptSubmit),
                    event(condr_core::AgentEventKind::Stop),
                )),
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
        window.read(|app| view.read(app).last_error.clone()),
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
                session.workspaces().first().is_some_and(|workspace| {
                    workspace
                        .tabs()
                        .first()
                        .unwrap()
                        .focused_pane()
                        .unwrap()
                        .id()
                        == agent_pane
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
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    window.update(|window, cx| _ = window.draw(cx));
    let new_workspace = window
        .debug_bounds("open-project")
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
                .presented()
                .map(|(_, _, workspace_id, _)| workspace_id);
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
fn dragging_workspaces_and_tabs_reorders_them_without_changing_focus() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("reorder-first");
    let second_root = TestDirectory::new("reorder-second");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, server) = connected_condr(&mut cx);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
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
                name: None,
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
            .presented()
            .map(|(_, _, workspace_id, _)| workspace_id)
            .unwrap()
    });
    assert_ne!(
        second_workspace, first_workspace,
        "the created Workspace is shown"
    );

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateTab {
                name: None,
                cwd_from: None,
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
                    workspace.tabs()[1].id(),
                    workspace.tabs()[1].focused_pane().unwrap().id(),
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
    let drop_target = window
        .debug_bounds(sidebar_workspace_selector(second_workspace))
        .expect("the drop target Workspace should render in the sidebar");
    window.simulate_mouse_down(workspace.center(), MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(
        drop_target.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.simulate_mouse_move(
        drop_target.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    window.simulate_mouse_up(
        drop_target.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
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
    let tab_target = window
        .debug_bounds(tab_selector(active_tab))
        .expect("the drop target Tab should render");
    window.simulate_mouse_down(tab.center(), MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(tab_target.center(), MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.simulate_mouse_move(tab_target.center(), MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(tab_target.center(), MouseButton::Left, Modifiers::default());
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                let workspace = session.workspace(second_workspace).unwrap();
                workspace.tabs()[0].id() == active_tab && workspace.tabs()[1].id() == first_tab
            })
        })
    }));

    let assert_identity = |session: &Session| {
        assert_eq!(
            session
                .tab(active_tab)
                .unwrap()
                .focused_pane()
                .unwrap()
                .id(),
            focused_pane
        );
    };
    window.read(|app| {
        let condr = view.read(app);
        assert_eq!(
            condr
                .presented()
                .map(|(_, _, workspace_id, tab_id)| (workspace_id, tab_id)),
            Some((second_workspace, active_tab)),
            "reordering leaves the view where it was"
        );
        assert_identity(&condr.active_session().unwrap());
    });
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
    cx.update(gpui_kit::init);
    let (view, window, server) = connected_condr(&mut cx);
    let button = window
        .debug_bounds("open-project")
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
                .is_some_and(|session| !session.workspaces().is_empty())
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
        window.read(|app| view.read(app).last_error.clone()),
    );
    assert_eq!(
        window.read(|app| {
            view.read(app)
                .active_session()
                .unwrap()
                .workspaces()
                .first()
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
                    .workspaces()
                    .first()
                    .is_some_and(|workspace| workspace.tabs().len() == 2)
            })
        })
    });
    assert!(
        tab_created,
        "GUI did not render the tab created by the real server"
    );

    // The Client that created the Tab shows it (ADR 0021).
    let new_pane = window
        .read(|app| {
            let (_, session, _, tab_id) = view.read(app).presented()?;
            let workspace = session.workspaces().first()?;
            assert_eq!(tab_id, workspace.tabs()[1].id(), "the new Tab is shown");
            Some(session.tab(tab_id)?.focused_pane()?.id())
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
        .read(|app| view.read(app).presented().map(|(_, _, _, tab_id)| tab_id))
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
            let (_, session, _, tab_id) = view.read(app).presented()?;
            Some(session.tab(tab_id)?.focused_pane()?.id())
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
            view.read(app)
                .presented()
                .is_some_and(|(_, session, _, tab_id)| {
                    session.tab(tab_id).unwrap().panes().len() == 2
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
        let (_, session, _, tab_id) = condr.presented().unwrap();
        let tab = session.tab(tab_id).unwrap();
        let focused = tab.focused_pane().unwrap().id();
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
            view.read(app)
                .presented()
                .is_some_and(|(_, session, _, tab_id)| {
                    session.tab(tab_id).unwrap().focused_pane().unwrap().id() == pane_to_focus
                })
        })
    });
    assert!(pane_focused, "clicking a Pane did not focus it");
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds_before_focus,
        "focus-only updates must not rebuild Dock"
    );

    let split_down_shortcut = if cfg!(target_os = "macos") {
        "cmd-shift-d"
    } else {
        "alt-shift--"
    };
    window.simulate_keystrokes(split_down_shortcut);
    let split_down = wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .presented()
                .is_some_and(|(_, session, _, tab_id)| {
                    session.tab(tab_id).unwrap().panes().len() == 3
                })
        })
    });
    assert!(split_down, "Alt+Shift+- did not split down");

    // The new Pane's shell must show its prompt before anything is typed: input that
    // arrives while bash is still setting up the terminal is discarded, which is what
    // happens on a slow CI runner.
    let new_pane = window
        .read(|app| {
            view.read(app)
                .active_session()?
                .workspaces()
                .first()
                .map(|workspace| {
                    workspace
                        .tabs()
                        .first()
                        .unwrap()
                        .focused_pane()
                        .unwrap()
                        .id()
                })
        })
        .unwrap();
    let prompt_ready = wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .and_then(|connection| connection.terminals.get(&new_pane))
                .is_some_and(|terminal| {
                    terminal
                        .view
                        .cells
                        .iter()
                        .any(|cell| !cell.text.trim().is_empty())
                })
        })
    });
    assert!(prompt_ready, "the split Pane's shell did not show a prompt");

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

    let active_tab = window.read(|app| view.read(app).presented().map(|(_, _, _, tab_id)| tab_id));
    window.simulate_keystrokes("ctrl-tab");
    let tab_switched = wait_until(window, |window| {
        let next_tab =
            window.read(|app| view.read(app).presented().map(|(_, _, _, tab_id)| tab_id));
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
    cx.update(gpui_kit::init);
    let (_, window, _server) = connected_condr(&mut cx);
    window.simulate_resize(size(px(1280.), px(720.)));
    let bounds = window
        .debug_bounds("condr-sidebar")
        .expect("sidebar should remain rendered");
    assert!(bounds.size.width > px(0.));
    assert!(bounds.size.height > px(0.));
}
