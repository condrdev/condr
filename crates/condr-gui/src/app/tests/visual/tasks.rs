//! Tasks mode (ADR 0068): entering and leaving it, its columns, its two layouts and
//! densities, and the layout actions it leaves inert.

use super::layout::agent_changed;
use super::*;

const TOGGLE_TASKS: &str = if cfg!(target_os = "macos") {
    "cmd-0"
} else {
    "alt-0"
};
const SPLIT_RIGHT: &str = if cfg!(target_os = "macos") {
    "cmd-d"
} else {
    "alt-shift-="
};

fn presented(window: &mut VisualTestContext, view: &Entity<Condr>) -> Option<(WorkspaceId, TabId)> {
    window.read(|app| {
        view.read(app)
            .presented()
            .map(|(_, _, workspace_id, tab_id)| (workspace_id, tab_id))
    })
}

fn draw(window: &mut VisualTestContext) {
    window.update(|window, cx| _ = window.draw(cx));
}

fn inside(window: &mut VisualTestContext, inner: &'static str, outer: &'static str) -> bool {
    let (Some(inner), Some(outer)) = (window.debug_bounds(inner), window.debug_bounds(outer))
    else {
        return false;
    };
    outer.contains(&inner.center())
}

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
        super::super::super::startup::bind_keys(cx);
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

    window.simulate_keystrokes(TOGGLE_TASKS);
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_some());
    assert!(window.debug_bounds("tasks-title").is_some());
    assert!(
        window.debug_bounds("condr-sidebar").is_none(),
        "Tasks mode hides the sidebar"
    );
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

    // Compact, then the List, whose rows carry the same Agents and no density switch.
    let compact = window.debug_bounds("tasks-density-compact").unwrap();
    window.simulate_click(compact.center(), Modifiers::default());
    draw(window);
    assert!(inside(window, card(blocked), "tasks-column-needs-you"));
    let list = window.debug_bounds("tasks-layout-list").unwrap();
    window.simulate_click(list.center(), Modifiers::default());
    draw(window);
    assert!(window.debug_bounds("tasks-list").is_some());
    assert!(window.debug_bounds("tasks-board").is_none());
    assert!(window.debug_bounds("tasks-density-cards").is_none());
    assert!(
        window
            .debug_bounds(leaked_selector(format!("tasks-row-1-{}", blocked.as_u64())))
            .is_some()
    );
    assert_eq!(
        window.read(|app| view.read(app).collect_state(app).tasks),
        super::super::super::tasks::TasksPrefs {
            on: true,
            layout: super::super::super::tasks::TasksLayout::List,
            density: super::super::super::tasks::TasksDensity::Compact,
            scope: Default::default(),
            folded: 0,
            split_board: None,
            split_list: None,
        },
        "the state file keeps the mode, the layout and the density"
    );

    let back = window.debug_bounds("tasks-back").unwrap();
    window.simulate_click(back.center(), Modifiers::default());
    draw(window);
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
        super::super::super::startup::bind_keys(cx);
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
    window.simulate_keystrokes(TOGGLE_TASKS);
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
    use super::super::super::tasks::TasksScope;
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

const NEXT_AGENT: &str = if cfg!(target_os = "macos") {
    "cmd-]"
} else {
    "alt-]"
};

/// A Workspace with one Pane whose Agent works, in Tasks mode.
fn tasks_mode_with_an_agent(
    cx: &mut TestAppContext,
) -> (Entity<Condr>, &mut VisualTestContext, TestServer, PaneId) {
    tasks_mode_with_workspaces(cx, 1)
}

/// `count` Workspaces, the first's Pane running an Agent that works, in Tasks mode. The
/// Workspaces come first: a hand-made Agent event moves the Client's sequence past the
/// Server's next one.
fn tasks_mode_with_workspaces(
    cx: &mut TestAppContext,
    count: usize,
) -> (Entity<Condr>, &mut VisualTestContext, TestServer, PaneId) {
    cx.update(gpui_kit::init);
    let (view, window, server) = connected_condr(cx);
    for index in 0..count {
        // One directory each: a second Workspace on the same root waits for the first.
        let root_directory = std::env::temp_dir().join(format!("condr-tasks-{index}"));
        std::fs::create_dir_all(&root_directory).unwrap();
        window.update(|_, cx| {
            view.update(cx, |this, _| {
                this.send_layout(LayoutCommand::CreateWorkspace {
                    name: None,
                    root_directory,
                });
            });
        });
        assert!(wait_until(window, |window| window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspaces().len() > index)
        })));
    }
    let mut pane_id = None;
    assert!(wait_until(window, |window| {
        pane_id = window.read(|app| {
            let workspaces = view.read(app).active_session()?.workspaces();
            if workspaces.len() < count {
                return None;
            }
            workspaces
                .first()?
                .tabs()
                .first()?
                .focused_pane()
                .map(|pane| pane.id())
        });
        pane_id.is_some_and(|pane_id| {
            // The first Workspace's terminal has its view, drawn or not.
            window.read(|app| view.read(app).terminal(1, pane_id).is_some())
        })
    }));
    let pane_id = pane_id.unwrap();
    agent_changed(
        window,
        &view,
        pane_id,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Claude,
            state: AgentState::Working,
            detail: None,
        }),
    );
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    (view, window, server, pane_id)
}

fn target(window: &mut VisualTestContext, view: &Entity<Condr>) -> Option<(u64, PaneId)> {
    window.read(|app| view.read(app).terminal_input.target)
}

#[test]
fn a_card_opens_its_terminal_beside_the_cards_without_changing_the_view_or_the_layout() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);
    let before = presented(window, &view);
    let card = leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64()));
    assert!(
        window.debug_bounds("tasks-area").is_none(),
        "the area waits for a click"
    );

    let bounds = window.debug_bounds(card).unwrap();
    window.simulate_click(bounds.center(), Modifiers::default());
    window.run_until_parked();
    draw(window);
    assert!(window.debug_bounds("tasks-area").is_some());
    assert!(window.debug_bounds("tasks-area-dock").is_some());
    assert!(inside(window, terminal_selector(pane_id), "tasks-area"));
    assert_eq!(
        target(window, &view),
        Some((1, pane_id)),
        "the terminal has the keyboard"
    );
    assert_eq!(
        presented(window, &view),
        before,
        "selecting changes no View"
    );

    // A real drag-selection in the area's terminal, which sends no Layout command.
    let (outgoing, sent) = std::sync::mpsc::channel();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().io = Some(ClientIo {
                outgoing,
                _incoming_task: Task::ready(()),
            });
        });
    });
    let terminal = window.debug_bounds(terminal_selector(pane_id)).unwrap();
    let start = point(
        terminal.left() + terminal.size.width * 0.25,
        terminal.top() + terminal.size.height * 0.35,
    );
    let end = point(
        terminal.left() + terminal.size.width * 0.75,
        terminal.top() + terminal.size.height * 0.65,
    );
    window.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    let dragging = window.read(|app| view.read(app).terminal_input.selection.unwrap());
    assert!(dragging.dragging);
    assert_eq!((dragging.connection_key, dragging.pane_id), (1, pane_id));
    window.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    assert!(
        sent.try_iter()
            .all(|message| !matches!(message, ClientMessage::Layout { .. })),
        "the interaction area's Dock sends no Layout command"
    );
    assert_eq!(
        presented(window, &view),
        before,
        "a click in the terminal changes no View"
    );

    // The same card closes the area, and nothing keeps the keyboard.
    let bounds = window.debug_bounds(card).unwrap();
    window.simulate_click(bounds.center(), Modifiers::default());
    window.run_until_parked();
    draw(window);
    assert!(window.debug_bounds("tasks-area").is_none());
    assert_eq!(target(window, &view), None);
}

#[test]
fn arrows_preview_without_the_keyboard_and_enter_hands_it_over() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);

    // A press on the card area's background gives it the keyboard.
    let cards = window.debug_bounds("tasks-cards").unwrap();
    let corner = point(cards.right() - px(4.), cards.bottom() - px(4.));
    window.simulate_click(corner, Modifiers::default());
    window.simulate_keystrokes("down");
    window.run_until_parked();
    draw(window);
    assert!(
        window.debug_bounds("tasks-area").is_some(),
        "the arrow previews the Agent"
    );
    assert_eq!(
        target(window, &view),
        None,
        "the keyboard stays with the cards"
    );
    assert!(
        window.read(|app| view.read(app).tasks_previews(1, pane_id)),
        "a preview sends the terminal no size"
    );

    window.simulate_keystrokes("enter");
    window.run_until_parked();
    draw(window);
    assert_eq!(target(window, &view), Some((1, pane_id)));
    assert!(!window.read(|app| view.read(app).tasks_previews(1, pane_id)));
}

#[test]
fn next_agent_selects_a_card_and_an_exited_agent_keeps_its_pane_until_open_in_workspace() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);

    window.simulate_keystrokes(NEXT_AGENT);
    window.run_until_parked();
    draw(window);
    assert!(window.debug_bounds("tasks-area").is_some());
    assert_eq!(target(window, &view), Some((1, pane_id)));

    // The Agent exits; its Pane stays in the area, marked.
    agent_changed(window, &view, pane_id, None);
    draw(window);
    assert!(window.debug_bounds("tasks-area").is_some());
    assert!(window.debug_bounds("tasks-agent-exited").is_some());

    let open = window.debug_bounds("tasks-open-in-workspace").unwrap();
    window.simulate_click(open.center(), Modifiers::default());
    window.run_until_parked();
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_none());
    assert!(window.debug_bounds(terminal_selector(pane_id)).is_some());
    assert_eq!(target(window, &view), Some((1, pane_id)));
}

#[test]
fn going_to_an_agent_outside_the_scope_widens_it_and_selects_its_card() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_workspaces(&mut cx, 2);
    let other = window.read(|app| {
        view.read(app)
            .active_session()
            .map(|session| session.workspaces()[1].id())
    });
    let server = window.read(|app| {
        view.read(app)
            .connection(1)
            .unwrap()
            .model
            .server_id
            .unwrap()
    });
    use super::super::super::tasks::TasksScope;
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.set_tasks_scope(TasksScope::Workspace(server, other.unwrap()), cx)
        })
    });
    draw(window);
    let card = leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64()));
    assert!(
        window.debug_bounds(card).is_none(),
        "the other Workspace's scope hides it"
    );
    // As a notification's click does.
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_pane(1, pane_id, window, cx);
        })
    });
    window.run_until_parked();
    draw(window);
    assert_eq!(
        window.read(|app| view.read(app).main_view().tasks.scope),
        TasksScope::All
    );
    assert!(
        window.debug_bounds("tasks-mode").is_some(),
        "the person stays in Tasks mode"
    );
    assert!(window.debug_bounds(card).is_some());
    assert!(window.debug_bounds("tasks-area").is_some());
}

#[test]
fn a_disconnected_device_shows_a_band_and_its_open_pane_a_veil() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);
    let card = leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64()));
    let bounds = window.debug_bounds(card).unwrap();
    window.simulate_click(bounds.center(), Modifiers::default());
    window.run_until_parked();
    draw(window);
    assert!(
        window.debug_bounds("tasks-band-1").is_none(),
        "no band while connected"
    );

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
    assert!(window.debug_bounds("tasks-band-1").is_some());
    assert!(window.debug_bounds("tasks-band-reconnect-1").is_some());
    assert!(inside(window, "reconnect-veil-1", "tasks-area"));
}
