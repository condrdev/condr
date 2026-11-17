//! Entering and leaving Tasks mode, its columns and scope, and the sidebar beside it.

use super::*;

#[test]
fn tasks_mode_takes_the_window_sorts_agents_into_columns_and_leaves_the_view() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("tasks-mode");
    let mut session = Session::new();
    let quiet = session.create_workspace(directory.0.clone()).unwrap();
    let busy = session.create_workspace(directory.0.clone()).unwrap();
    session.create_tab(busy, None).unwrap();
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
        crate::app::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    let panes = |window: &mut VisualTestContext, workspace_id: WorkspaceId| {
        window.read(|app| {
            let session = view.read(app).connection(1)?.session()?;
            Some(
                session
                    .workspace(workspace_id)?
                    .tabs()
                    .iter()
                    .map(|tab| tab.panes()[0].id())
                    .collect::<Vec<_>>(),
            )
        })
    };
    assert!(wait_until(window, |window| panes(window, busy)
        .is_some_and(|panes| panes.len() == 2)));
    let working = panes(window, quiet).unwrap()[0];
    let [blocked, done] = panes(window, busy).unwrap()[..] else {
        unreachable!()
    };
    for (pane_id, state) in [
        (working, AgentState::Working),
        (done, AgentState::Done),
        (blocked, AgentState::Blocked),
    ] {
        agent_changed(
            window,
            &view,
            pane_id,
            Some(AgentSnapshot {
                session_id: None,
                kind: AgentKind::Codex,
                state,
                detail: (state == AgentState::Blocked).then(|| "Bash: cargo test".to_owned()),
            }),
        );
    }
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1)
                .unwrap()
                .model
                .agent_unseen
                .insert(done);
            cx.notify();
        });
    });
    draw(window);
    let before = presented(window, &view);
    assert!(before.is_some());
    assert!(
        window.debug_bounds("sidebar-tasks").is_some(),
        "the row tops the sidebar"
    );

    click(window, "sidebar-tasks");
    assert!(window.debug_bounds("tasks-mode").is_some());
    assert!(window.debug_bounds("tasks-title").is_some());
    // It covers the content area as the Ports page does: the sidebar stays beside it and
    // the Changes sidebar is under it.
    assert!(
        window.debug_bounds("condr-sidebar").is_some(),
        "the sidebar stays beside Tasks mode"
    );
    assert!(window.debug_bounds("condr-changes-column").is_none());
    assert!(
        window.read(|app| view.read(app).terminal_input.target.is_none()),
        "nothing targets the keyboard without a selection"
    );
    let card = |pane_id: PaneId| leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64()));
    assert!(inside(window, card(blocked), "tasks-column-needs-you"));
    assert!(inside(window, card(done), "tasks-column-done"));
    assert!(inside(window, card(working), "tasks-column-working"));
    assert!(
        window.read(|app| {
            view.read(app)
                .connection(1)
                .unwrap()
                .model
                .agent_unseen
                .contains(&done)
        }),
        "entering marks nothing seen"
    );

    // The scope and Layout sit in the toolbar over the cards, not in the title row.
    assert!(inside(window, "tasks-scope", "tasks-toolbar"));
    assert!(inside(window, "tasks-layout", "tasks-toolbar"));
    assert!(!inside(window, "tasks-layout", "tasks-title"));

    // Layout: compact cards, then the List, whose rows carry the same Agents.
    click(window, "tasks-layout");
    click(window, "tasks-compact");
    assert!(inside(window, card(blocked), "tasks-column-needs-you"));
    click(window, "tasks-layout-list");
    assert!(window.debug_bounds("tasks-list").is_some());
    assert!(window.debug_bounds("tasks-board").is_none());
    assert!(
        window.debug_bounds("tasks-compact").is_none(),
        "the List's rows are always compact, so it offers no switch"
    );
    assert!(
        window
            .debug_bounds(leaked_selector(format!("tasks-row-1-{}", blocked.as_u64())))
            .is_some()
    );
    assert_eq!(
        window.read(|app| view.read(app).collect_state(app).tasks),
        crate::app::tasks::TasksPrefs {
            layout: crate::app::tasks::TasksLayout::List,
            density: crate::app::tasks::TasksDensity::Compact,
            scope: Default::default(),
            folded: 0,
            split_board: None,
            split_list: None,
        },
        "the state file keeps the mode, the layout and the density"
    );

    click(window, "tasks-close");
    assert!(window.debug_bounds("tasks-mode").is_none());
    assert!(window.debug_bounds("condr-sidebar").is_some());
    assert_eq!(
        presented(window, &view),
        before,
        "leaving shows what was shown before"
    );
}

#[test]
fn the_sidebar_enters_tasks_mode_where_layout_actions_do_nothing() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::app::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| presented(window, &view).is_some()));
    draw(window);

    let row = window.debug_bounds("sidebar-tasks").unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_some());

    let (outgoing, sent) = std::sync::mpsc::channel();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().io = Some(ClientIo {
                outgoing,
                _incoming_task: Task::ready(()),
            });
        });
    });
    window.simulate_keystrokes(SPLIT_RIGHT);
    window.run_until_parked();
    assert!(
        sent.try_iter()
            .all(|message| !matches!(message, ClientMessage::Layout { .. })),
        "splitting a Tab Tasks mode does not show sends nothing"
    );

    // Back in normal mode, the collapsed sidebar's icon enters it too.
    click(window, "tasks-close");
    window.update(|window, cx| {
        view.update(cx, |this, cx| this.toggle_sidebar(window, cx));
    });
    draw(window);
    let icon = window.debug_bounds("collapsed-tasks").unwrap();
    window.simulate_click(icon.center(), Modifiers::default());
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_some());
}

#[test]
fn columns_fold_needs_you_unfolds_for_an_arrival_and_the_scope_takes_in_worktrees() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("tasks-scope");
    let mut session = Session::new();
    let repository = session.create_workspace(directory.0.clone()).unwrap();
    let worktree = session.create_workspace(directory.0.clone()).unwrap();
    let other = session.create_workspace(directory.0.clone()).unwrap();
    let snapshot_path = directory.0.join("session.snapshot");
    std::fs::write(&snapshot_path, session.snapshot().to_bytes().unwrap()).unwrap();
    let endpoint = Endpoint::local(directory.0.join("server.sock"));
    let server = start_server_with_config(
        ServerConfig::ephemeral(endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path),
    );
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    let pane = |window: &mut VisualTestContext, workspace_id: WorkspaceId| {
        window.read(|app| {
            let session = view.read(app).connection(1)?.session()?;
            Some(session.workspace(workspace_id)?.tabs()[0].panes()[0].id())
        })
    };
    assert!(wait_until(window, |window| pane(window, other).is_some()));
    let [in_repository, in_worktree, in_other] =
        [repository, worktree, other].map(|workspace_id| pane(window, workspace_id).unwrap());
    // The test Server's directory is no Git worktree, so the Client is told it is one.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let mut session = connection.session().unwrap().clone();
            assert!(session.associate_worktree(
                worktree,
                repository,
                directory.0.clone(),
                true,
                None
            ));
            let message = ServerMessage::Event {
                server_id: connection.model.server_id.unwrap(),
                session_id: connection.model.session_id.unwrap(),
                sequence: connection.model.sequence + 1,
                event: SessionEvent::LayoutChanged {
                    snapshot: session.snapshot(),
                    zoomed_panes: Vec::new(),
                },
            };
            let generation = connection.connect_generation;
            this.handle_incoming(1, generation, Incoming::Message(message), cx);
        });
    });
    for (pane_id, state) in [
        (in_repository, AgentState::Idle),
        (in_worktree, AgentState::Working),
        (in_other, AgentState::Working),
    ] {
        agent_changed(
            window,
            &view,
            pane_id,
            Some(AgentSnapshot {
                session_id: None,
                kind: AgentKind::Claude,
                state,
                detail: None,
            }),
        );
    }
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    let card = |pane_id: PaneId| leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64()));

    // A folded column is a strip that a click unfolds.
    let heading = window.debug_bounds("tasks-heading-working").unwrap();
    window.simulate_click(heading.center(), Modifiers::default());
    draw(window);
    assert!(window.debug_bounds("tasks-strip-working").is_some());
    assert!(window.debug_bounds(card(in_worktree)).is_none());
    let strip = window.debug_bounds("tasks-strip-working").unwrap();
    window.simulate_click(strip.center(), Modifiers::default());
    draw(window);
    assert!(inside(window, card(in_worktree), "tasks-column-working"));

    // Folded, Needs you unfolds when an Agent arrives in it.
    let heading = window.debug_bounds("tasks-heading-needs-you").unwrap();
    window.simulate_click(heading.center(), Modifiers::default());
    draw(window);
    assert!(window.debug_bounds("tasks-strip-needs-you").is_some());
    agent_changed(
        window,
        &view,
        in_other,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Claude,
            state: AgentState::Blocked,
            detail: Some("Bash: cargo test".into()),
        }),
    );
    draw(window);
    assert!(window.debug_bounds("tasks-strip-needs-you").is_none());
    assert!(inside(window, card(in_other), "tasks-column-needs-you"));

    // A repository's scope takes in its worktree; the worktree's is just itself.
    let server = window.read(|app| {
        view.read(app)
            .connection(1)
            .unwrap()
            .model
            .server_id
            .unwrap()
    });
    let scope = |window: &mut VisualTestContext, scope| {
        window.update(|_, cx| view.update(cx, |this, cx| this.set_tasks_scope(scope, cx)));
        draw(window);
    };
    use crate::app::tasks::TasksScope;
    scope(window, TasksScope::Workspace(server, repository));
    assert!(window.debug_bounds(card(in_repository)).is_some());
    assert!(window.debug_bounds(card(in_worktree)).is_some());
    assert!(window.debug_bounds(card(in_other)).is_none());
    scope(window, TasksScope::Workspace(server, worktree));
    assert!(window.debug_bounds(card(in_worktree)).is_some());
    assert!(window.debug_bounds(card(in_repository)).is_none());
    scope(window, TasksScope::Device(server));
    for pane_id in [in_repository, in_worktree, in_other] {
        assert!(window.debug_bounds(card(pane_id)).is_some());
    }
    assert_eq!(
        window.read(|app| view.read(app).collect_state(app).tasks.scope),
        TasksScope::Device(server)
    );
    // A Workspace the Device no longer has falls back to All.
    scope(
        window,
        TasksScope::Workspace(server, WorkspaceId::from_u64(999)),
    );
    assert!(window.debug_bounds(card(in_other)).is_some());
}

/// ADR 0020 in Tasks mode: the sidebar beside it marks the Device, and the open Pane of a
/// Device that is retrying wears the veil.
#[test]
fn a_disconnected_device_veils_its_open_pane_and_keeps_its_sidebar_mark() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);
    let card = leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64()));
    let bounds = window.debug_bounds(card).unwrap();
    window.simulate_click(bounds.center(), Modifiers::default());
    window.run_until_parked();
    draw(window);

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let generation = this.connection(1).unwrap().connect_generation;
            this.handle_incoming(
                1,
                generation,
                Incoming::Disconnected("the device closed the connection".into()),
                cx,
            );
            // Past the grace a blip that heals on its own is never shown for.
            this.connection_mut(1).unwrap().disconnected_at =
                Some(Instant::now() - Duration::from_secs(3));
            cx.notify();
        });
    });
    draw(window);
    assert!(inside(window, "device-status-1", "condr-sidebar"));
    assert!(inside(window, "reconnect-veil-1", "tasks-area"));
}

/// Tasks mode covers the content area as the Ports page does: toggle-sidebar works
/// beside it, and a Workspace chosen in the sidebar, or the Ports page, leaves it.
#[test]
fn the_sidebar_beside_tasks_mode_leaves_it_for_a_workspace() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, _pane_id) = tasks_mode_with_workspaces(&mut cx, 2);
    assert!(window.debug_bounds("tasks-mode").is_some());
    let collapsed = |window: &mut VisualTestContext| {
        window.read(|app| view.read(app).main_view().sidebar_collapsed)
    };
    let before = collapsed(window);
    window.update(|window, cx| window.dispatch_action(Box::new(crate::app::ToggleSidebar), cx));
    draw(window);
    assert_ne!(
        collapsed(window),
        before,
        "toggle-sidebar works beside Tasks mode"
    );
    window.update(|window, cx| window.dispatch_action(Box::new(crate::app::ToggleSidebar), cx));
    draw(window);

    // The Workspace shown before is not lit beside Tasks mode, so its row leaves too
    // rather than folding.
    let shown = presented(window, &view).unwrap().0;
    click(window, sidebar_workspace_selector(shown));
    assert!(window.debug_bounds("tasks-mode").is_none());
    assert_eq!(
        presented(window, &view).map(|(workspace, _)| workspace),
        Some(shown)
    );
    click(window, "sidebar-tasks");
    let second = window.read(|app| view.read(app).active_session().unwrap().workspaces()[1].id());
    click(window, sidebar_workspace_selector(second));
    assert!(window.debug_bounds("tasks-mode").is_none());
    assert_eq!(
        presented(window, &view).map(|(workspace, _)| workspace),
        Some(second)
    );

    click(window, "sidebar-tasks");
    assert!(window.debug_bounds("tasks-mode").is_some());
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_ports(crate::app::ports::PortsScope::All, window, cx)
        })
    });
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_none());
    assert!(
        window.read(|app| { matches!(view.read(app).main_view().page, ContentPage::Ports(_)) })
    );
}
