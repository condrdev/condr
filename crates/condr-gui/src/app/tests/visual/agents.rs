//! The Agents section (ADR 0067): its order across Workspaces, the keys that step through
//! it, and folding it to its heading.

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
    let order = |window: &mut VisualTestContext| {
        window.update(|window, cx| _ = window.draw(cx));
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
        "the Done just seen keeps its place while it is selected"
    );
    press(window, NEXT_AGENT);
    assert_eq!(
        target(window),
        Some((1, working)),
        "so the next one is the Agent after it, not one skipped"
    );
    assert_eq!(
        order(window),
        [blocked, working, done],
        "once left, the seen Done ranks as the Idle it now is"
    );
    press(window, NEXT_AGENT);
    assert_eq!(target(window), Some((1, done)));
    press(window, NEXT_AGENT);
    assert_eq!(target(window), Some((1, blocked)), "next wraps to the top");
    press(window, PREVIOUS_AGENT);
    assert_eq!(target(window), Some((1, done)), "previous wraps to the end");

    let row = window.debug_bounds(row_selector(working)).unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(
        target(window),
        Some((1, working)),
        "a click selects the Agent's Pane"
    );

    let heading = window.debug_bounds("agents-heading").unwrap();
    window.simulate_click(heading.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds(row_selector(blocked)).is_none());
    assert!(
        window.debug_bounds("agents-heading").is_some(),
        "folded, the heading stays"
    );
    assert!(window.read(|app| view.read(app).main_view().agents_collapsed));
}
