//! The Agents section (ADR 0067, ADR 0074, ADR 0075): its Workspace and state groups and
//! their order, a row sliding where its state moves it, the keys that step through it,
//! and folding it to its heading.

use super::layout::agent_changed;
use super::*;

const NEXT_AGENT: &str = if cfg!(target_os = "macos") {
    "cmd-]"
} else {
    "alt-]"
};
const PREVIOUS_AGENT: &str = if cfg!(target_os = "macos") {
    "cmd-["
} else {
    "alt-["
};

/// Draws the frame an animation in the cached sidebar asked for, `millis` later.
fn frame_after(window: &mut VisualTestContext, millis: u64) {
    window
        .executor()
        .advance_clock(Duration::from_millis(millis));
    window.update(|window, cx| {
        window.simulate_next_frame(cx);
        _ = window.draw(cx);
    });
}

#[test]
fn the_agents_section_lists_every_workspaces_agents_most_urgent_first() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("agents-section");
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
            let this = view.read(app);
            let session = this.connection(1)?.session()?;
            let workspace = session.workspace(workspace_id)?;
            Some(
                workspace
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
    window.update(|_, cx| {
        view.update(cx, |this, _| this.terminal_input.target = None);
    });
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds("agents-empty").is_some(),
        "with no Agent the section says so rather than vanishing"
    );

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
                detail: None,
            }),
        );
    }
    // The Server says a Done is unseen in an event of its own (ADR 0052).
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
    let row_selector =
        |pane_id: PaneId| leaked_selector(format!("agents-row-1-{}", pane_id.as_u64()));
    // Once the rows have slid to their places.
    let order = |window: &mut VisualTestContext| {
        window.update(|window, cx| _ = window.draw(cx));
        frame_after(window, 500);
        let mut rows = [working, done, blocked]
            .into_iter()
            .map(|pane_id| {
                let top = window
                    .debug_bounds(row_selector(pane_id))
                    .expect("the section lists every Agent")
                    .top();
                (top, pane_id)
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        rows.into_iter()
            .map(|(_, pane_id)| pane_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(order(window), [blocked, done, working]);
    let group = |window: &mut VisualTestContext, workspace_id: WorkspaceId| {
        window
            .debug_bounds(leaked_selector(format!(
                "agents-group-1-{}",
                workspace_id.as_u64()
            )))
            .expect("each Workspace with an Agent heads its group")
    };
    let row = |window: &mut VisualTestContext, pane_id: PaneId| {
        window.debug_bounds(row_selector(pane_id)).unwrap()
    };
    assert!(
        group(window, busy).bottom() <= row(window, blocked).top()
            && row(window, done).bottom() <= group(window, quiet).top()
            && group(window, quiet).bottom() <= row(window, working).top(),
        "the Workspace with the Blocked Agent comes first, each group under its name"
    );
    let heading = window.debug_bounds("agents-heading").unwrap();
    let footer_button = window.debug_bounds("open-settings").unwrap();
    assert!(
        heading.bottom() <= footer_button.top(),
        "the section sits above the sidebar's buttons"
    );

    let target =
        |window: &mut VisualTestContext| window.read(|app| view.read(app).terminal_input.target);
    let press = |window: &mut VisualTestContext, keys: &str| {
        window.update(|window, cx| {
            let focus = view.read(cx).main_view().focus_handle.clone();
            focus.focus(window, cx);
        });
        window.simulate_keystrokes(keys);
        window.run_until_parked();
    };
    press(window, NEXT_AGENT);
    assert_eq!(target(window), Some((1, blocked)), "next starts at the top");
    press(window, NEXT_AGENT);
    assert_eq!(target(window), Some((1, done)));
    assert_eq!(
        order(window),
        [blocked, done, working],
        "the Done just seen stays in its Workspace's group"
    );
    press(window, NEXT_AGENT);
    assert_eq!(
        target(window),
        Some((1, working)),
        "so the next one is the Agent after it, not one skipped"
    );
    press(window, NEXT_AGENT);
    assert_eq!(target(window), Some((1, blocked)), "next wraps to the top");
    press(window, PREVIOUS_AGENT);
    assert_eq!(
        target(window),
        Some((1, working)),
        "previous wraps to the end"
    );

    let done_row = row(window, done);
    window.simulate_click(done_row.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(
        target(window),
        Some((1, done)),
        "a click selects the Agent's Pane"
    );

    // The heading's button opens its menu rather than folding the section (ADR 0074).
    let button = window
        .debug_bounds("agents-order")
        .expect("open, the heading carries the button that groups and orders");
    window.simulate_click(button.center(), Modifiers::default());
    window.run_until_parked();
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    assert!(!window.read(|app| view.read(app).main_view().agents_collapsed));

    let set_order = |window: &mut VisualTestContext, by_state| {
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.main_view_mut().agents_order = crate::app::left_sidebar::AgentsOrder {
                    by_state,
                    tree_order: false,
                };
                cx.notify();
            });
        });
        // Regrouped, the rows slide too; this waits for their places.
        window.update(|window, cx| _ = window.draw(cx));
        frame_after(window, 500);
    };
    set_order(window, true);
    let state = |window: &mut VisualTestContext, key: &str| {
        window
            .debug_bounds(leaked_selector(format!("agents-state-{key}")))
            .expect("each state with an Agent heads its group")
    };
    assert!(
        window
            .debug_bounds(leaked_selector(format!("agents-group-1-{}", busy.as_u64())))
            .is_none(),
        "by state, no Workspace heads a group"
    );
    assert!(
        state(window, "blocked").bottom() <= row(window, blocked).top()
            && row(window, blocked).bottom() <= state(window, "working").top()
            && state(window, "working").bottom() <= row(window, working).top()
            && row(window, working).bottom() <= state(window, "idle").top()
            && state(window, "idle").bottom() <= row(window, done).top(),
        "each Agent under its state, the Done seen under Idle"
    );

    // The selected Agent moves with its state at once, sliding there (ADR 0075).
    let working_row = row(window, working);
    window.simulate_click(working_row.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(target(window), Some((1, working)));
    agent_changed(
        window,
        &view,
        working,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Blocked,
            detail: None,
        }),
    );
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(
        row(window, working).top(),
        working_row.top(),
        "it is drawn where it was first"
    );
    frame_after(window, 60);
    let sliding = row(window, working).top();
    assert_eq!(
        order(window),
        [working, blocked, done],
        "then it is under Blocked, ahead of the one later in the Session"
    );
    let placed = row(window, working).top();
    assert!(
        placed < sliding && sliding < working_row.top(),
        "having slid up there"
    );
    set_order(window, false);

    let open = window.debug_bounds("agents-section").unwrap();
    let heading = window.debug_bounds("agents-heading").unwrap();
    window.simulate_click(heading.center(), Modifiers::default());
    window.run_until_parked();
    // The frame the click asks for turns the spring; time then moves it.
    frame_after(window, 0);
    frame_after(window, 60);
    let sliding = window.debug_bounds("agents-section").unwrap();
    assert!(
        sliding.size.height < open.size.height && sliding.size.height > heading.size.height,
        "folding slides the section closed"
    );
    assert!(
        window.debug_bounds(row_selector(blocked)).is_some(),
        "the rows stay while the section closes over them"
    );
    frame_after(window, 500);
    assert!(window.debug_bounds(row_selector(blocked)).is_none());
    assert_eq!(
        window.debug_bounds("agents-section").unwrap().size.height,
        heading.size.height,
        "folded, the section is its heading"
    );
    assert!(
        window.debug_bounds("agents-heading").is_some(),
        "folded, the heading stays"
    );
    assert!(window.read(|app| view.read(app).main_view().agents_collapsed));
}

/// The section's top edge is Kit's resize handle: a press a few pixels off its line still
/// takes it, and dragging up makes the section taller.
#[test]
fn the_agents_handle_takes_a_press_beside_its_line() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));
    let before = window.debug_bounds("agents-section").unwrap().size.height;
    let band = window
        .debug_bounds("agents-resize")
        .expect("the handle is drawn");
    assert!(
        band.size.height > px(4.),
        "a band, not a hairline: {band:?}"
    );
    let press = point(band.center().x, band.top() + px(1.));
    let to = point(press.x, press.y - px(80.));
    window.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
    for _ in 0..2 {
        window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        window.update(|window, cx| _ = window.draw(cx));
    }
    window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    assert!(window.read(|app| view.read(app).main_view().agents_height.is_some()));
    assert!(
        window.debug_bounds("agents-section").unwrap().size.height > before,
        "dragging up makes the section taller"
    );
}
