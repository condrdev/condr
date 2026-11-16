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
