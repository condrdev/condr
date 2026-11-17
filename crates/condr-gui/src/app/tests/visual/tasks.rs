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

/// The area's Pane keeps its header but offers no zoom or Pane menu, whose actions do
/// nothing in Tasks mode.
#[test]
fn the_area_pane_offers_no_zoom_or_pane_menu() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (_view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);
    let pane = pane_id.as_u64();
    click(window, leaked_selector(format!("tasks-card-1-{pane}")));
    assert!(
        window
            .debug_bounds(leaked_selector(format!("terminal-pane-header-{pane}")))
            .is_some()
    );
    for button in ["zoom", "menu"] {
        assert!(
            window
                .debug_bounds(leaked_selector(format!("terminal-pane-{button}-{pane}")))
                .is_none(),
            "{button}"
        );
    }
}

/// A Device the person disconnected from its menu needs no band saying so.
#[test]
fn a_device_the_person_disconnected_shows_no_band() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, _pane_id) = tasks_mode_with_an_agent(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.set_auto_connect(1, false, cx);
            this.disconnect_server(1);
            cx.notify();
        });
    });
    draw(window);
    assert!(window.debug_bounds("tasks-cards").is_some());
    assert!(window.debug_bounds("tasks-band-1").is_none());
}

// Tasks (ADR 0068): To do, the form, Start and the archive dialog.

use condr_core::{StartPlace, TaskCommand, TaskId, TaskResponse};

const SAVE: &str = if cfg!(target_os = "macos") {
    "cmd-s"
} else {
    "ctrl-s"
};

/// A GUI on a fresh Server that keeps its Tasks in memory, and another Client of it, as
/// the CLI would be.
fn tasks_gui(
    cx: &mut TestAppContext,
) -> (
    Entity<Condr>,
    &mut VisualTestContext,
    TestServer,
    ClientConnection,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (server, endpoint) = start_server();
    let cli = ClientConnection::connect_overview(&endpoint, "cli").unwrap();
    let (view, window, server) = connected_condr_with(cx, server, endpoint);
    (view, window, server, cli)
}

fn create_task(
    cli: &mut ClientConnection,
    title: &str,
    repository: Option<std::path::PathBuf>,
) -> TaskId {
    let Ok(Ok(TaskResponse::Task(task))) = cli.task(TaskCommand::Create {
        title: title.into(),
        description: format!("{title}, described"),
        repository,
    }) else {
        panic!("the Server keeps Tasks");
    };
    task.id
}

fn gui_tasks(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
) -> std::collections::BTreeMap<TaskId, condr_core::Task> {
    window.read(|app| {
        view.read(app)
            .connection(1)
            .and_then(|connection| connection.model.tasks.clone())
            .unwrap_or_default()
    })
}

fn click(window: &mut VisualTestContext, selector: &'static str) {
    let bounds = window
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    window.simulate_click(bounds.center(), Modifiers::default());
    window.run_until_parked();
    draw(window);
}

fn todo_card(task: TaskId) -> &'static str {
    leaked_selector(format!("tasks-todo-1-{}", task.as_u64()))
}

fn form_home(window: &mut VisualTestContext, view: &Entity<Condr>) -> Option<(u64, TaskId)> {
    window.read(|app| view.read(app).task_form.as_ref().and_then(|form| form.home))
}

/// A message from the Server, made by hand.
fn incoming(window: &mut VisualTestContext, view: &Entity<Condr>, message: ServerMessage) {
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let generation = this.connection(1).unwrap().connect_generation;
            this.handle_incoming(1, generation, Incoming::Message(message), cx);
            cx.notify();
        });
    });
    window.run_until_parked();
    draw(window);
}

/// A Session event made by hand, after the Client's last.
fn event(window: &mut VisualTestContext, view: &Entity<Condr>, event: SessionEvent) {
    let message = window.read(|app| {
        let connection = view.read(app).connection(1).unwrap();
        ServerMessage::Event {
            server_id: connection.model.server_id.unwrap(),
            session_id: connection.model.session_id.unwrap(),
            sequence: connection.model.sequence + 1,
            event,
        }
    });
    incoming(window, view, message);
}

/// What the GUI sends from here on, which then never reaches the Server.
fn capture(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
) -> std::sync::mpsc::Receiver<ClientMessage> {
    let (outgoing, sent) = std::sync::mpsc::channel();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().io = Some(ClientIo {
                outgoing,
                _incoming_task: Task::ready(()),
            });
        });
    });
    sent
}

/// Puts a Task in the GUI's model as an event would, without one.
fn put_task(window: &mut VisualTestContext, view: &Entity<Condr>, task: condr_core::Task) {
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1).unwrap().model.apply_task(task);
            cx.notify();
        });
    });
    draw(window);
}

#[test]
fn to_do_heads_tasks_mode_newest_first_with_its_count_and_drags_to_reorder() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let first = create_task(&mut cli, "Fix the login redirect", None);
    let second = create_task(&mut cli, "Write the release notes", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 2));
    draw(window);
    assert!(
        window.debug_bounds("sidebar-tasks-count").is_some(),
        "the sidebar's row counts To do"
    );
    assert_eq!(window.read(|app| view.read(app).todo_count()), 2);

    // The Command Palette's New Task enters Tasks mode with the form; an empty one's
    // Cancel asks nothing.
    window.update(|window, cx| {
        // From the window's focus, as the palette runs it.
        let focus = view.read(cx).main_view().focus_handle.clone();
        focus.focus(window, cx);
        window.dispatch_action(Box::new(super::super::super::NewTask), cx)
    });
    window.run_until_parked();
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_some());
    assert!(window.debug_bounds("task-form").is_some());
    click(window, "task-form-cancel");
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));
    assert!(window.debug_bounds("task-form").is_none());
    assert!(inside(window, todo_card(first), "tasks-column-to-do"));
    assert!(inside(window, todo_card(second), "tasks-column-to-do"));
    let (top, below) = (
        window.debug_bounds(todo_card(second)).unwrap(),
        window.debug_bounds(todo_card(first)).unwrap(),
    );
    assert!(top.top() < below.top(), "a new Task heads To do");
    let to_do = window.debug_bounds("tasks-column-to-do").unwrap();
    let needs_you = window.debug_bounds("tasks-column-needs-you").unwrap();
    assert!(to_do.left() < needs_you.left(), "To do is the first column");

    // Dragging the lower card onto the upper one moves it up, on the Server.
    window.simulate_mouse_down(below.center(), MouseButton::Left, Modifiers::default());
    for _ in 0..2 {
        window.simulate_mouse_move(top.center(), MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        draw(window);
    }
    window.simulate_mouse_up(top.center(), MouseButton::Left, Modifiers::default());
    assert!(wait_until(window, |window| {
        let tasks = gui_tasks(window, &view);
        tasks[&first].order < tasks[&second].order
    }));

    // The List carries To do as its first group.
    click(window, "tasks-layout");
    click(window, "tasks-layout-list");
    assert!(window.debug_bounds(todo_card(first)).is_some());
    assert!(window.debug_bounds("tasks-new-task").is_some());
}

#[test]
fn a_device_whose_server_keeps_no_tasks_offers_no_to_do() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    // An older Server, or one built without Tasks, sends no Tasks record.
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1).unwrap().model.tasks = None;
            this.toggle_tasks(window, cx);
        })
    });
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_some());
    assert!(window.debug_bounds("tasks-column-to-do").is_none());
    assert!(window.debug_bounds("tasks-column-needs-you").is_some());
    assert_eq!(window.read(|app| view.read(app).todo_count()), 0);
    window.update(|window, cx| view.update(cx, |this, cx| this.new_task(window, cx)));
    window.run_until_parked();
    assert!(
        window.read(|app| view.read(app).task_form.is_none()),
        "no Device takes a new Task"
    );
    assert!(window.read(|app| view.read(app).last_error.is_some()));
}

#[test]
fn a_new_task_saves_with_the_shortcut_and_unsaved_changes_ask_before_they_go() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    // A Workspace to start in, so Start waits only for the changes.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| presented(window, &view).is_some()));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);

    // A new Task has no card and no Start until it is saved.
    click(window, "tasks-new-task");
    assert!(window.debug_bounds("task-form").is_some());
    assert!(window.debug_bounds("task-start-unsaved").is_some());
    assert!(window.debug_bounds("task-start").is_none());
    assert!(gui_tasks(window, &view).is_empty());
    window.simulate_input("Write the release notes");
    window.simulate_keystrokes(SAVE);
    assert!(wait_until(window, |window| form_home(window, &view).is_some()));
    draw(window);
    let (_, saved) = form_home(window, &view).unwrap();
    assert_eq!(
        gui_tasks(window, &view)[&saved].title,
        "Write the release notes"
    );
    assert!(inside(window, todo_card(saved), "tasks-column-to-do"));
    assert!(window.debug_bounds("task-start").is_some());
    assert!(window.debug_bounds("task-start-blocked").is_none());

    // A change disables Start until it is saved.
    click(window, "task-form-title");
    window.simulate_input(" now");
    draw(window);
    assert!(window.debug_bounds("task-start-blocked").is_some());

    // Leaving asks: Cancel stays, Discard goes and keeps the Task as it was.
    let other = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 2));
    draw(window);
    click(window, todo_card(other));
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    click(window, "file-choice-cancel");
    assert_eq!(form_home(window, &view), Some((1, saved)));
    click(window, todo_card(other));
    click(window, "file-choice-discard");
    assert_eq!(form_home(window, &view), Some((1, other)));
    assert_eq!(
        gui_tasks(window, &view)[&saved].title,
        "Write the release notes"
    );

    // The form fetched the description; Save from the dialog keeps a change on the way out.
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .and_then(|form| form.saved.as_ref())
            .is_some_and(|saved| saved.description.is_some())
    })));
    click(window, "task-form-title");
    window.simulate_input("!");
    click(window, "tasks-area-close");
    click(window, "file-choice-save");
    // The form goes once its save is in.
    assert!(wait_until(window, |window| window
        .read(|app| view.read(app).task_form.is_none())));
    assert!(wait_until(window, |window| gui_tasks(window, &view)
        [&other]
        .title
        .ends_with('!')));
    let Ok(Ok(TaskResponse::Task(task))) = cli.task(TaskCommand::Get { task: other }) else {
        panic!("the Task is there");
    };
    assert_eq!(
        task.description.as_deref(),
        Some("Fix the login redirect, described"),
        "saving a title leaves the description as it was"
    );

    // Archive asks nothing; Delete asks first.
    click(window, todo_card(saved));
    click(window, "task-form-delete");
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    window.update(|window, cx| window.dispatch_action(Box::new(Confirm { secondary: false }), cx));
    assert!(wait_until(window, |window| !gui_tasks(window, &view)
        .contains_key(&saved)));
    click(window, todo_card(other));
    click(window, "task-form-archive");
    assert!(wait_until(window, |window| gui_tasks(window, &view).is_empty()));
    assert!(window.read(|app| view.read(app).task_form.is_none()));

    // The Archived list unfolds with it, to restore.
    click(window, "tasks-archived");
    let restore = leaked_selector(format!("tasks-restore-1-{}", other.as_u64()));
    assert!(wait_until(window, |window| {
        draw(window);
        window.debug_bounds(restore).is_some()
    }));
    click(window, restore);
    assert!(wait_until(window, |window| gui_tasks(window, &view)
        .contains_key(&other)));
}

#[test]
fn a_form_closes_when_another_client_deletes_its_task_and_follows_one_that_starts() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| presented(window, &view).is_some()));
    let (workspace_id, _) = presented(window, &view).unwrap();
    let deleted = create_task(&mut cli, "Fix the login redirect", None);
    let started = create_task(&mut cli, "Write the release notes", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 2));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);

    click(window, todo_card(deleted));
    assert_eq!(form_home(window, &view), Some((1, deleted)));
    let Ok(Ok(_)) = cli.task(TaskCommand::Delete { task: deleted }) else {
        panic!("the other Client deletes it");
    };
    assert!(wait_until(window, |window| form_home(window, &view).is_none()));

    // Another Client starts it: the selection follows to its card, which reads
    // "Starting…" in Working until an Agent comes, and then "No agent running".
    click(window, todo_card(started));
    let mut task = gui_tasks(window, &view)[&started].clone();
    task.workspace = Some(workspace_id);
    event(window, &view, SessionEvent::TaskChanged(task));
    let card = leaked_selector(format!("tasks-task-1-{}", started.as_u64()));
    assert!(window.read(|app| view.read(app).task_form.is_none()));
    assert_eq!(
        window.read(|app| view.read(app).main_view().tasks_selection.unwrap().task),
        Some(started)
    );
    assert!(inside(window, card, "tasks-column-working"));
    assert!(window.debug_bounds("tasks-archive-task").is_some());
    assert!(window.debug_bounds("tasks-area-dock").is_some());

    window.update(|_, cx| view.update(cx, |this, cx| this.task_start_settled(1, started, cx)));
    draw(window);
    assert!(inside(window, card, "tasks-column-idle"));
    assert!(
        window.read(|app| view.read(app).last_error.is_none()),
        "only the Client that started it says the Agent did not come"
    );
}

#[test]
fn start_takes_its_defaults_and_a_successful_start_selects_the_agent() {
    let _serial_guard = acquire_visual_test_lock();
    let repository = TestDirectory::new("tasks-start");
    run_git(&repository.0, ["init"]);
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let other_root = std::env::temp_dir().join("condr-tasks-start-other");
    std::fs::create_dir_all(&other_root).unwrap();
    for root in [repository.0.clone(), other_root] {
        window.update(|_, cx| {
            view.update(cx, |this, _| {
                this.send_layout(LayoutCommand::CreateWorkspace {
                    name: None,
                    root_directory: root.clone(),
                });
            });
        });
        let root = root.clone();
        assert!(wait_until(window, |window| window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspace_by_root(&root).is_some())
        })));
    }
    // A repository Workspace is one whose Git state the Server reported.
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .connection(1)
            .is_some_and(|connection| !connection.model.workspace_git.is_empty())
    })));
    let (repository_workspace, other_workspace) = window.read(|app| {
        let session = view.read(app).active_session().unwrap();
        (
            session.workspace_by_root(&repository.0).unwrap().id(),
            session.workspaces()[1].id(),
        )
    });
    let title = "Fix the login redirect";
    let task_id = create_task(&mut cli, title, Some(repository.0.clone()));
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    click(window, todo_card(task_id));
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .connection(1)
            .unwrap()
            .device_tasks
            .available
            .is_some()
    })));
    // Whatever this machine has on PATH, the Device offers one Agent here.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1).unwrap().device_tasks.available = Some(vec![AgentKind::Claude]);
            cx.notify();
        })
    });
    draw(window);
    let plan = |window: &mut VisualTestContext| {
        window.read(|app| {
            let view = view.read(app);
            let plan = view
                .start_plan(view.task_form.as_ref().unwrap(), app)
                .unwrap();
            (plan.place, plan.workspace, plan.blocked.is_some())
        })
    };
    use super::super::super::task_form::Where;
    assert_eq!(
        plan(window),
        (Where::Existing, Some(repository_workspace), false),
        "the repository's own Workspace, which no Task holds"
    );

    // Another Task holds it: greyed with that Task's title, and a new worktree on the
    // Server's branch is the default.
    let mut holder = gui_tasks(window, &view)[&task_id].clone();
    holder.id = TaskId::from_u64(99);
    holder.title = "Another task".into();
    holder.workspace = Some(repository_workspace);
    put_task(window, &view, holder.clone());
    assert_eq!(plan(window).0, Where::NewWorktree);
    assert!(window.debug_bounds("task-start-branch").is_some());
    let branch = window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .unwrap()
            .start
            .branch
            .read(app)
            .value()
            .to_string()
    });
    assert_eq!(branch, "worktree/fix-the-login-redirect");
    let held = window.read(|app| {
        let view = view.read(app);
        view.start_plan(view.task_form.as_ref().unwrap(), app)
            .unwrap()
            .options
            .workspaces
    });
    assert_eq!(held[0].holder.as_deref(), Some("Another task"));
    assert_eq!(held[1].holder, None);

    // A branch checked out elsewhere is refused under its field.
    let sent = capture(window, &view);
    click(window, "task-start-button");
    let request_id = sent
        .try_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command:
                    TaskCommand::Start {
                        task,
                        place: Some(StartPlace::NewWorktree { branch }),
                        agent: AgentKind::Claude,
                    },
                ..
            } if task == task_id
                && branch.as_deref() == Some("worktree/fix-the-login-redirect") =>
            {
                Some(request_id)
            }
            _ => None,
        })
        .expect("Start sends the place and the Agent");
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Err(condr_core::TaskError::new(
                condr_core::task_error::BRANCH_CHECKED_OUT,
                "worktree/fix-the-login-redirect is checked out elsewhere",
            )),
        },
    );
    assert!(inside(window, "field-error", "task-start"));
    assert!(window.read(|app| view.read(app).last_error.is_none()));

    // The other Workspace instead; with nothing free, Start says where to open one.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.choose_task_workspace(other_workspace, cx)
        })
    });
    assert_eq!(
        plan(window),
        (Where::Existing, Some(other_workspace), false)
    );
    let mut second = holder.clone();
    second.id = TaskId::from_u64(98);
    second.workspace = Some(other_workspace);
    put_task(window, &view, second.clone());
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let tasks = this
                .connection_mut(1)
                .unwrap()
                .model
                .tasks
                .as_mut()
                .unwrap();
            tasks.get_mut(&task_id).unwrap().repository = Some("/nowhere/cli".into());
            cx.notify();
        })
    });
    draw(window);
    assert!(window.debug_bounds("task-start-open-first").is_some());
    assert!(plan(window).2, "nothing to offer disables Start");
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let tasks = this
                .connection_mut(1)
                .unwrap()
                .model
                .tasks
                .as_mut()
                .unwrap();
            tasks.remove(&holder.id);
            tasks.remove(&second.id);
            tasks.get_mut(&task_id).unwrap().repository = Some(repository.0.clone());
            cx.notify();
        })
    });
    draw(window);

    // Start again, in the repository's Workspace: a Start under way reads "Starting…"
    // in To do.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.choose_task_workspace(repository_workspace, cx)
        })
    });
    draw(window);
    click(window, "task-start-button");
    let request_id = sent
        .try_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command:
                    TaskCommand::Start {
                        place: Some(StartPlace::Workspace(workspace)),
                        ..
                    },
                ..
            } if workspace == repository_workspace => Some(request_id),
            _ => None,
        })
        .expect("Start sends the Workspace chosen");
    let mut starting = gui_tasks(window, &view)[&task_id].clone();
    starting.starting = true;
    event(window, &view, SessionEvent::TaskChanged(starting.clone()));
    assert!(
        window
            .debug_bounds(leaked_selector(format!(
                "tasks-todo-starting-1-{}",
                task_id.as_u64()
            )))
            .is_some()
    );
    assert!(window.debug_bounds("task-start-blocked").is_some());

    // It started: the card is selected with the keyboard in the new Pane, and the
    // Agent's card carries the Task's title once it comes.
    let (tab_id, pane_id) = window.read(|app| {
        let tab = &view
            .read(app)
            .active_session()
            .unwrap()
            .workspace(repository_workspace)
            .unwrap()
            .tabs()[0];
        (tab.id(), tab.panes()[0].id())
    });
    let mut started = starting;
    started.starting = false;
    started.workspace = Some(repository_workspace);
    event(window, &view, SessionEvent::TaskChanged(started));
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Ok(TaskResponse::Started {
                workspace_id: repository_workspace,
                tab_id,
                pane_id,
            }),
        },
    );
    assert!(window.read(|app| view.read(app).task_form.is_none()));
    let task_card = leaked_selector(format!("tasks-task-1-{}", task_id.as_u64()));
    assert!(inside(window, task_card, "tasks-column-working"));
    assert_eq!(target(window, &view), Some((1, pane_id)));
    // No Agent within the wait: "No agent running", and one toast for this Start.
    assert!(window.read(|app| view.read(app).last_error.is_none()));
    window
        .executor()
        .advance_clock(super::super::super::task_form::AGENT_WAIT);
    window.run_until_parked();
    draw(window);
    assert!(inside(window, task_card, "tasks-column-idle"));
    assert!(window.read(|app| view.read(app).last_error.is_some()));
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
    let agent_card = leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64()));
    assert!(window.debug_bounds(task_card).is_none());
    assert!(inside(window, agent_card, "tasks-column-working"));
    assert_eq!(
        window.read(|app| view.read(app).main_view().tasks_selection.unwrap().pane_id),
        pane_id
    );
    assert!(window.debug_bounds("tasks-archive-task").is_some());
}

#[test]
fn archiving_a_started_task_closes_a_workspace_its_start_made_and_keeps_a_borrowed_one() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("tasks-archive");
    let mut session = Session::new();
    let repository = session.create_workspace(directory.0.clone()).unwrap();
    let made = session.create_workspace(directory.0.clone()).unwrap();
    let borrowed = session.create_workspace(directory.0.clone()).unwrap();
    let reopened = session.create_workspace(directory.0.clone()).unwrap();
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
    assert!(wait_until(window, |window| pane(window, reopened).is_some()));
    let in_made = pane(window, made).unwrap();
    let sent = capture(window, &view);
    // The test Server's directory is no Git worktree, so the Client is told two are
    // Managed Worktrees of the repository.
    let snapshot = window.read(|app| {
        let mut session = view
            .read(app)
            .connection(1)
            .unwrap()
            .session()
            .unwrap()
            .clone();
        for worktree in [made, borrowed] {
            assert!(session.associate_worktree(
                worktree,
                repository,
                directory.0.clone(),
                true,
                None
            ));
        }
        session.snapshot()
    });
    event(
        window,
        &view,
        SessionEvent::LayoutChanged {
            snapshot,
            zoomed_panes: Vec::new(),
        },
    );
    // What the Server recorded at each Start's commit: a new worktree and a plain folder
    // opened again are the Start's own; an open Workspace, Managed Worktree or not, is
    // borrowed.
    let task = |id: u64, workspace_id, made_workspace| condr_core::Task {
        id: TaskId::from_u64(id),
        title: format!("task {id}"),
        description: None,
        repository: Some(directory.0.clone()),
        order: 0,
        archived: false,
        workspace: Some(workspace_id),
        made_workspace,
        starting: false,
        last: Some(condr_core::LastPlace {
            path: directory.0.clone(),
            branch: Some("worktree/fix".into()),
            base_branch: None,
            managed: true,
            repository: Some(directory.0.clone()),
        }),
    };
    put_task(window, &view, task(1, made, true));
    put_task(window, &view, task(2, borrowed, false));
    put_task(window, &view, task(3, reopened, true));
    put_task(window, &view, task(4, repository, false));
    agent_changed(
        window,
        &view,
        in_made,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Blocked,
            detail: Some("Bash: cargo test".into()),
        }),
    );
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);

    // The Agent carries its Task's title; the others, with no Agent, are cards of their
    // own.
    let facts = window.read(|app| {
        view.read(app)
            .archive_facts(1, TaskId::from_u64(1))
            .unwrap()
    });
    assert!(facts.created && facts.managed);
    assert_eq!(facts.agents, 1);
    assert_eq!(
        facts.busy,
        Some((AgentKind::Codex, condr_core::AgentDisplayState::Blocked))
    );
    let archived = |sent: &std::sync::mpsc::Receiver<ClientMessage>| {
        sent.try_iter().find_map(|message| match message {
            ClientMessage::Task {
                command:
                    TaskCommand::Archive {
                        task,
                        close_workspace,
                        remove_worktree,
                    },
                ..
            } => Some((task.as_u64(), close_workspace, remove_worktree)),
            _ => None,
        })
    };
    // Archives a Task from its card's area with the dialog's defaults; says whether the
    // dialog offered to remove the worktree.
    let archive = |window: &mut VisualTestContext, card: &'static str| {
        click(window, card);
        click(window, "tasks-archive-task");
        assert!(window.update(|window, cx| window.has_active_dialog(cx)));
        draw(window);
        let removable = window.debug_bounds("task-archive-remove").is_some();
        window.update(|window, cx| {
            window.dispatch_action(Box::new(Confirm { secondary: false }), cx)
        });
        window.run_until_parked();
        draw(window);
        removable
    };
    for (card, removable, expected, why) in [
        (
            leaked_selector(format!("tasks-card-1-{}", in_made.as_u64())),
            true,
            (1, true, false),
            "a new worktree its Start made closes by default; the worktree stays",
        ),
        (
            "tasks-task-1-2",
            true,
            (2, false, false),
            "a borrowed Managed Worktree stays open by default",
        ),
        (
            "tasks-task-1-3",
            false,
            (3, true, false),
            "a plain folder its Start opened again closes by default",
        ),
        (
            "tasks-task-1-4",
            false,
            (4, false, false),
            "the repository's own Workspace, borrowed, stays open",
        ),
    ] {
        assert_eq!(
            archive(window, card),
            removable,
            "only a Managed Worktree can be removed: {why}"
        );
        assert_eq!(archived(&sent), Some(expected), "{why}");
    }
}

#[test]
fn a_new_device_for_a_to_do_task_creates_it_there_and_deletes_it_here() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let task_id = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    // A second Device that keeps Tasks, whose messages the test answers.
    let (outgoing, box_sent) = std::sync::mpsc::channel();
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let mut device =
                ServerConnection::new(2, "box".into(), crate::app::tests::tcp("127.0.0.1:9"));
            device.status = ConnectionStatus::Connected;
            device.model.server_id = Some(condr_core::protocol::ServerId(77));
            device.model.session_id = Some(condr_core::protocol::SessionId(1));
            device.model.tasks = Some(Default::default());
            device.io = Some(ClientIo {
                outgoing,
                _incoming_task: Task::ready(()),
            });
            this.connections.push(device);
            this.toggle_tasks(window, cx);
        })
    });
    draw(window);
    click(window, todo_card(task_id));
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .and_then(|form| form.saved.as_ref())
            .is_some_and(|saved| saved.description.is_some())
    })));
    window.update(|_, cx| view.update(cx, |this, cx| this.choose_task_device(2, cx)));
    draw(window);
    assert!(
        window.debug_bounds("task-start-blocked").is_some(),
        "a move is a change to save before Start"
    );
    click(window, "task-form-save");
    let to_box: Vec<ClientMessage> = box_sent.try_iter().collect();
    assert!(
        to_box.iter().any(|message| matches!(
            message,
            ClientMessage::Agent {
                command: condr_core::protocol::AgentCommand::Available,
                ..
            }
        )),
        "the Device the Task moves to is asked for its Agents"
    );
    let (request_id, title, description) = to_box
        .into_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command:
                    TaskCommand::Create {
                        title,
                        description,
                        repository: None,
                    },
                ..
            } => Some((request_id, title, description)),
            _ => None,
        })
        .expect("the new Device is asked to create it");
    assert_eq!(description, "Fix the login redirect, described");
    let moved = condr_core::Task {
        id: TaskId::from_u64(5),
        title,
        description: Some(description),
        repository: None,
        order: 0,
        archived: false,
        workspace: None,
        made_workspace: false,
        starting: false,
        last: None,
    };
    let answer = |window: &mut VisualTestContext, request_id, result| {
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.handle_incoming(
                    2,
                    0,
                    Incoming::Message(ServerMessage::Task { request_id, result }),
                    cx,
                );
            })
        });
        window.run_until_parked();
    };
    // Its event comes before its answer, as the Server sends them.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(2)
                .unwrap()
                .model
                .apply_task(moved.clone());
        })
    });
    answer(window, request_id, Ok(TaskResponse::Task(moved)));
    assert_eq!(form_home(window, &view), Some((2, TaskId::from_u64(5))));
    assert!(
        wait_until(window, |window| gui_tasks(window, &view).is_empty()),
        "then the old Device deletes its copy"
    );

    // From a Device whose delete fails: both copies stay, and a toast says so.
    let stay = |window: &mut VisualTestContext, id: u64| {
        let mut task = moved_task(id);
        task.description = None;
        window.update(|_, cx| {
            view.update(cx, |this, _| {
                this.connection_mut(2).unwrap().model.apply_task(task);
            })
        });
        window.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.open_task_form(2, TaskId::from_u64(id), window, cx)
            })
        });
        let get = box_sent
            .try_iter()
            .find_map(|message| match message {
                ClientMessage::Task {
                    request_id,
                    command: TaskCommand::Get { .. },
                    ..
                } => Some(request_id),
                _ => None,
            })
            .expect("the form fetches its description");
        answer(window, get, Ok(TaskResponse::Task(moved_task(id))));
        window.update(|_, cx| view.update(cx, |this, cx| this.choose_task_device(1, cx)));
    };
    stay(window, 7);
    click(window, "task-form-save");
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    let delete = box_sent
        .try_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command: TaskCommand::Delete { task },
                ..
            } if task == TaskId::from_u64(7) => Some(request_id),
            _ => None,
        })
        .expect("the old Device is asked to delete its copy");
    answer(
        window,
        delete,
        Err(condr_core::TaskError::new(
            condr_core::task_error::TASK_STARTED,
            "Task 7 is started; archive it first",
        )),
    );
    assert!(window.read(|app| view.read(app).last_error.is_some()));
    let both = window.read(|app| {
        view.read(app)
            .connection(2)
            .unwrap()
            .model
            .tasks
            .as_ref()
            .unwrap()
            .contains_key(&TaskId::from_u64(7))
    });
    assert!(both, "the old copy stays beside the new one");

    // An old Device gone before the new copy exists: nothing to delete it on, said so.
    window.update(|_, cx| view.update(cx, |this, _| this.last_error = None));
    stay(window, 8);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(2).unwrap().status = ConnectionStatus::Disconnected;
        })
    });
    click(window, "task-form-save");
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 2));
    window.run_until_parked();
    assert!(window.read(|app| view.read(app).last_error.is_some()));
}

/// A Task on the test's own second Device.
fn moved_task(id: u64) -> condr_core::Task {
    condr_core::Task {
        id: TaskId::from_u64(id),
        title: format!("Moved {id}"),
        description: Some("Fix the login redirect, described".into()),
        repository: None,
        order: 0,
        archived: false,
        workspace: None,
        made_workspace: false,
        starting: false,
        last: None,
    }
}

/// Opens a To do Task's form and waits for its description.
fn open_form(window: &mut VisualTestContext, view: &Entity<Condr>, task: TaskId) {
    click(window, todo_card(task));
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .is_some_and(|form| form.description_loaded())
    })));
    draw(window);
}

fn form_text(window: &mut VisualTestContext, view: &Entity<Condr>) -> (String, String) {
    window.read(|app| {
        let form = view.read(app).task_form.as_ref().unwrap();
        (
            form.title.read(app).value().to_string(),
            form.description.read(app).value().to_string(),
        )
    })
}

fn set_title(window: &mut VisualTestContext, view: &Entity<Condr>, title: &str) {
    let input = window.read(|app| view.read(app).task_form.as_ref().unwrap().title.clone());
    let title = title.to_owned();
    window.update(|window, cx| input.update(cx, |input, cx| input.set_value(title, window, cx)));
    draw(window);
}

fn form_error(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
) -> Option<super::super::super::task_form::FormField> {
    window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .and_then(|form| form.error.as_ref())
            .map(|(field, _)| *field)
    })
}

#[test]
fn a_start_answer_never_takes_a_form_that_would_lose_text() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    // Two Workspaces, one for each Start.
    for index in 0..2 {
        let root_directory = std::env::temp_dir().join(format!("condr-tasks-answer-{index}"));
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
    let workspaces = window.read(|app| {
        view.read(app)
            .active_session()
            .unwrap()
            .workspaces()
            .iter()
            .map(|workspace| workspace.id())
            .collect::<Vec<_>>()
    });
    let started = create_task(&mut cli, "Fix the login redirect", None);
    let other = create_task(&mut cli, "Write the release notes", None);
    let own = create_task(&mut cli, "Update the changelog", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 3));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    let sent = capture(window, &view);
    // The form's description, answered by hand since the Server no longer hears it.
    let open = |window: &mut VisualTestContext, task: TaskId| {
        click(window, todo_card(task));
        let request_id = sent
            .try_iter()
            .find_map(|message| match message {
                ClientMessage::Task {
                    request_id,
                    command: TaskCommand::Get { .. },
                    ..
                } => Some(request_id),
                _ => None,
            })
            .expect("the form fetches its description");
        let mut full = gui_tasks(window, &view)[&task].clone();
        full.description = Some(String::new());
        incoming(
            window,
            &view,
            ServerMessage::Task {
                request_id,
                result: Ok(TaskResponse::Task(full)),
            },
        );
    };
    let start = |window: &mut VisualTestContext, task: TaskId| {
        open(window, task);
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.connection_mut(1).unwrap().device_tasks.available =
                    Some(vec![AgentKind::Claude]);
                cx.notify();
            })
        });
        draw(window);
        click(window, "task-start-button");
        sent.try_iter()
            .find_map(|message| match message {
                ClientMessage::Task {
                    request_id,
                    command: TaskCommand::Start { .. },
                    ..
                } => Some(request_id),
                _ => None,
            })
            .expect("Start is sent")
    };
    let started_answer = |window: &mut VisualTestContext,
                          task: TaskId,
                          workspace_id: WorkspaceId,
                          request_id: u64| {
        let mut record = gui_tasks(window, &view)[&task].clone();
        record.workspace = Some(workspace_id);
        event(window, &view, SessionEvent::TaskChanged(record));
        let (tab_id, pane_id) = window.read(|app| {
            let tab = &view
                .read(app)
                .active_session()
                .unwrap()
                .workspace(workspace_id)
                .unwrap()
                .tabs()[0];
            (tab.id(), tab.panes()[0].id())
        });
        incoming(
            window,
            &view,
            ServerMessage::Task {
                request_id,
                result: Ok(TaskResponse::Started {
                    workspace_id,
                    tab_id,
                    pane_id,
                }),
            },
        );
    };

    // Another Task's form, with text typed, outlives the Start's answer.
    let request = start(window, started);
    open(window, other);
    set_title(window, &view, "Write the release notes today");
    started_answer(window, started, workspaces[0], request);
    assert_eq!(form_home(window, &view), Some((1, other)));
    assert_eq!(form_text(window, &view).0, "Write the release notes today");
    assert!(window.read(|app| view.read(app).main_view().tasks_selection.is_none()));
    assert!(window.read(|app| view.read(app).task_starting(1, started)));

    // As does the started Task's own form with a change: it stays to be saved, and
    // Start says it is under way.
    set_title(window, &view, "Write the release notes");
    let request = start(window, own);
    set_title(window, &view, "Update the changelog for 0.3");
    started_answer(window, own, workspaces[1], request);
    assert_eq!(form_home(window, &view), Some((1, own)));
    assert_eq!(form_text(window, &view).0, "Update the changelog for 0.3");
    assert!(window.debug_bounds("task-start-blocked").is_some());
}

#[test]
fn the_leave_dialogs_save_keeps_the_form_until_its_save_is_in() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let task = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    open_form(window, &view, task);
    let sent = capture(window, &view);
    use super::super::super::task_form::FormField;

    // Saved on the way out and refused: the form stays, with the refusal, where it was.
    set_title(window, &view, "Fix the login redirect!");
    click(window, "tasks-area-close");
    click(window, "file-choice-save");
    assert_eq!(
        form_home(window, &view),
        Some((1, task)),
        "waiting for the answer"
    );
    let request_id = sent
        .try_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command: TaskCommand::Edit { .. },
                ..
            } => Some(request_id),
            _ => None,
        })
        .expect("the save is sent");
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Err(condr_core::TaskError::new(
                condr_core::task_error::INVALID_TASK,
                "no repository Workspace is open at /src/api",
            )),
        },
    );
    assert_eq!(form_home(window, &view), Some((1, task)));
    assert_eq!(form_text(window, &view).0, "Fix the login redirect!");
    assert_eq!(form_error(window, &view), Some(FormField::Form));
    assert!(inside(window, "field-error", "task-form"));
    assert!(window.read(|app| view.read(app).last_error.is_none()));

    // What the Server would refuse in a title shows under it, and nothing is sent.
    set_title(window, &view, "Fix the\u{7}login redirect");
    click(window, "task-form-save");
    assert_eq!(form_error(window, &view), Some(FormField::Title));
    assert!(sent.try_iter().next().is_none());

    // A Device that is not connected keeps the form too.
    set_title(window, &view, "Fix the login redirect!");
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().status = ConnectionStatus::Disconnected;
        })
    });
    click(window, "task-form-save");
    assert_eq!(form_error(window, &view), Some(FormField::Form));
    assert_eq!(form_home(window, &view), Some((1, task)));

    // With the form gone, a refusal is a toast that names the Task.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.task_form = None;
            this.connection_mut(1).unwrap().device_tasks.pending.insert(
                900,
                super::super::super::task_form::TaskRequest::Edit {
                    title: "Fix the login redirect".into(),
                },
            );
            cx.notify();
        })
    });
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id: 900,
            result: Err(condr_core::TaskError::new(
                condr_core::task_error::TASK_NOT_FOUND,
                "no Task 1",
            )),
        },
    );
    assert!(
        window
            .read(|app| view.read(app).last_error.clone())
            .is_some_and(|error| error.contains("Fix the login redirect"))
    );
}

#[test]
fn a_form_loads_its_description_before_typing_and_follows_the_server_where_untouched() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let task = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);

    // A fetch that fails says so and can be asked again; till then nothing is typed.
    let sent = capture(window, &view);
    click(window, todo_card(task));
    let get = |sent: &std::sync::mpsc::Receiver<ClientMessage>| {
        sent.try_iter()
            .find_map(|message| match message {
                ClientMessage::Task {
                    request_id,
                    command: TaskCommand::Get { .. },
                    ..
                } => Some(request_id),
                _ => None,
            })
            .expect("the form fetches its description")
    };
    let request_id = get(&sent);
    let loaded = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .task_form
                .as_ref()
                .unwrap()
                .description_loaded()
        })
    };
    assert!(!loaded(window), "the description waits disabled");
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Err(condr_core::TaskError::new(
                condr_core::task_error::INTERNAL_ERROR,
                "the store is busy",
            )),
        },
    );
    assert!(window.debug_bounds("task-form-description-error").is_some());
    click(window, "task-form-retry");
    let request_id = get(&sent);
    let mut full = gui_tasks(window, &view)[&task].clone();
    full.description = Some("Users land on /home.".into());
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Ok(TaskResponse::Task(full.clone())),
        },
    );
    assert!(loaded(window));
    assert!(window.debug_bounds("task-form-description-error").is_none());
    assert_eq!(form_text(window, &view).1, "Users land on /home.");

    // Another Client's change follows into the untouched description, and a title
    // saved after it leaves that change alone.
    full.description = Some("Users land on /home after signing in.".into());
    event(window, &view, SessionEvent::TaskChanged(full.clone()));
    let request_id = get(&sent);
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Ok(TaskResponse::Task(full.clone())),
        },
    );
    assert_eq!(
        form_text(window, &view).1,
        "Users land on /home after signing in."
    );
    set_title(window, &view, "Fix the login redirect on Safari");
    click(window, "task-form-save");
    let edit = sent.try_iter().find_map(|message| match message {
        ClientMessage::Task {
            command: TaskCommand::Edit {
                title, description, ..
            },
            ..
        } => Some((title, description)),
        _ => None,
    });
    assert_eq!(
        edit,
        Some((Some("Fix the login redirect on Safari".into()), None)),
        "only the title is sent"
    );

    // A repository chosen on one Device comes back with it.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.choose_task_repository(Some("/src/api".into()), cx);
            this.choose_task_device(99, cx);
        })
    });
    let repository = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .task_form
                .as_ref()
                .unwrap()
                .repository
                .clone()
        })
    };
    assert_eq!(repository(window), None);
    window.update(|_, cx| view.update(cx, |this, cx| this.choose_task_device(1, cx)));
    assert_eq!(repository(window), Some("/src/api".into()));
}

#[test]
fn a_task_started_again_waits_afresh_for_its_agent() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| presented(window, &view).is_some()));
    let (workspace_id, _) = presented(window, &view).unwrap();
    let task = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    let _sent = capture(window, &view);
    let mut record = gui_tasks(window, &view)[&task].clone();
    let waiting = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .unwrap()
                .device_tasks
                .waiting
                .contains_key(&task)
        })
    };
    record.workspace = Some(workspace_id);
    event(window, &view, SessionEvent::TaskChanged(record.clone()));
    assert!(waiting(window));
    // Its Workspace closed: back in To do, and the wait is over.
    record.workspace = None;
    event(window, &view, SessionEvent::TaskChanged(record.clone()));
    assert!(!waiting(window));
    window.executor().advance_clock(Duration::from_secs(20));
    record.workspace = Some(workspace_id);
    event(window, &view, SessionEvent::TaskChanged(record));
    // The first Start's thirty seconds pass; the second's have not.
    window.executor().advance_clock(Duration::from_secs(15));
    window.run_until_parked();
    assert!(
        waiting(window),
        "the first Start's timer leaves the second's wait"
    );
    window.executor().advance_clock(Duration::from_secs(20));
    window.run_until_parked();
    assert!(!waiting(window));
}

#[test]
fn a_recovery_bootstrap_forgets_answers_it_dropped_and_asks_again() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let task = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    open_form(window, &view, task);
    set_title(window, &view, "Fix the login redirect!");
    // A save whose answer, and an Agent request whose answer, writer lag dropped.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            connection.device_tasks.pending.insert(
                900,
                super::super::super::task_form::TaskRequest::Edit {
                    title: "Fix the login redirect!".into(),
                },
            );
            connection
                .device_tasks
                .agent_requests
                .push_front(super::super::super::task_form::AgentRequest::Hooks);
            connection.recover_after_bootstrap = true;
            connection.request_snapshot();
            this.task_form.as_mut().unwrap().saving = Some((1, 900));
        })
    });
    assert!(wait_until(window, |window| window.read(|app| {
        !view
            .read(app)
            .connection(1)
            .unwrap()
            .device_tasks
            .pending
            .contains_key(&900)
    })));
    assert!(window.read(|app| {
        let view = view.read(app);
        view.task_form.as_ref().unwrap().saving.is_none()
            && !view
                .connection(1)
                .unwrap()
                .device_tasks
                .agent_requests
                .contains(&super::super::super::task_form::AgentRequest::Hooks)
    }));
    // Save works again, and the Agents are asked for again.
    click(window, "task-form-save");
    assert!(wait_until(window, |window| gui_tasks(window, &view)[&task]
        .title
        .ends_with('!')));
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .connection(1)
            .unwrap()
            .device_tasks
            .available
            .is_some()
    })));
}

#[test]
fn agent_answers_land_where_they_were_asked() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, _cli) = tasks_gui(&mut cx);
    let _sent = capture(window, &view);
    use super::super::super::task_form::AgentRequest;
    let answer = |window: &mut VisualTestContext, request: AgentRequest| {
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                let connection = this.connection_mut(1).unwrap();
                connection.device_tasks.agent_requests.clear();
                connection.device_tasks.agent_requests.push_back(request);
                connection.hooks_error = None;
                this.last_error = None;
                cx.notify();
            })
        });
        incoming(
            window,
            &view,
            ServerMessage::AgentResult {
                result: Err(condr_core::protocol::AgentError {
                    code: "io".into(),
                    message: "permission denied".into(),
                }),
            },
        );
        window.read(|app| {
            let view = view.read(app);
            (
                view.last_error.is_some(),
                view.connection(1).unwrap().hooks_error.is_some(),
            )
        })
    };
    assert_eq!(
        answer(window, AgentRequest::Available),
        (true, false),
        "the Agents on PATH: a toast"
    );
    assert_eq!(
        answer(window, AgentRequest::Hooks),
        (false, true),
        "Settings' hooks: its page"
    );
    assert_eq!(
        answer(window, AgentRequest::InstallHooks),
        (true, false),
        "Start's Install: a toast"
    );
}
