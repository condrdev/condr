//! The Command Palette (ADR 0064): its keys, its pages, and what choosing a row does.

use super::layout::agent_changed;
use super::*;

const SECONDARY: &str = if cfg!(target_os = "macos") {
    "cmd"
} else {
    "ctrl"
};

fn press_shift_twice(window: &mut VisualTestContext) {
    for _ in 0..2 {
        window.simulate_modifiers_change(Modifiers::shift());
        window.simulate_modifiers_change(Modifiers::default());
    }
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
}

fn palette_open(window: &mut VisualTestContext) -> bool {
    window.update(|window, cx| _ = window.draw(cx));
    window.debug_bounds("command-palette").is_some()
}

/// Puts the keyboard in the main window, where the window's keys are bound.
fn focus_window(window: &mut VisualTestContext, view: &Entity<Condr>) {
    window.update(|window, cx| {
        let focus = view.read(cx).main_view().focus_handle.clone();
        focus.focus(window, cx);
    });
    window.run_until_parked();
}

fn type_and_draw(window: &mut VisualTestContext, text: &str) {
    window.simulate_input(text);
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
}

#[test]
fn double_shift_opens_the_palette_and_enter_runs_the_chosen_action() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);
    focus_window(window, &view);

    // One Shift is half a sequence, and typing after it is typing.
    window.simulate_modifiers_change(Modifiers::shift());
    window.simulate_modifiers_change(Modifiers::default());
    window.simulate_keystrokes("a");
    window.simulate_modifiers_change(Modifiers::shift());
    window.simulate_modifiers_change(Modifiers::default());
    window.run_until_parked();
    assert!(
        !palette_open(window),
        "Shift, a key, Shift is no double Shift"
    );

    press_shift_twice(window);
    assert!(palette_open(window), "Shift twice opens the palette");
    // All lists the most urgent Agents and the recent actions; there are neither yet.
    assert!(
        window
            .debug_bounds("command-palette-action-split-right")
            .is_none()
    );

    type_and_draw(window, "toggle sidebar");
    assert!(
        window
            .debug_bounds("command-palette-action-toggle-sidebar")
            .is_some()
    );
    assert!(
        window
            .debug_bounds("command-palette-action-go-to-tab-1")
            .is_none(),
        "Go to tab N is left to Go to"
    );
    window.simulate_keystrokes("enter");
    assert!(wait_until(window, |window| {
        window.read(|app| view.read(app).main_view().sidebar_collapsed)
    }));
    assert!(!palette_open(window), "choosing closes the palette");
    let toggle_sidebar = crate::app::keymap::all()
        .position(|shortcut| shortcut.name == "toggle-sidebar")
        .unwrap();
    assert_eq!(
        window.read(|app| view.read(app).recent_actions.clone()),
        [toggle_sidebar]
    );

    // Its own key opens it on Actions, the action just chosen first, and closes it again.
    focus_window(window, &view);
    window.simulate_keystrokes(&format!("{SECONDARY}-shift-p"));
    window.run_until_parked();
    assert!(palette_open(window));
    let recent = window
        .debug_bounds("command-palette-action-toggle-sidebar")
        .expect("the recent action is listed");
    let settings = window
        .debug_bounds("command-palette-action-open-settings")
        .expect("Actions lists the actions by group");
    assert!(recent.top() < settings.top(), "the recent action leads");
    window.simulate_keystrokes(&format!("{SECONDARY}-shift-p"));
    window.run_until_parked();
    assert!(!palette_open(window), "the palette's key closes it");

    // Tab turns the page: All shows the recent action alone, Actions every one.
    press_shift_twice(window);
    let shown = |window: &mut VisualTestContext, selector| window.debug_bounds(selector).is_some();
    assert!(shown(window, "command-palette-action-toggle-sidebar"));
    assert!(!shown(window, "command-palette-action-open-settings"));
    window.simulate_keystrokes("tab");
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(shown(window, "command-palette-action-open-settings"));
    press_shift_twice(window);
    assert!(!palette_open(window), "Shift twice closes it too");
}

#[test]
fn go_to_reaches_a_tab_and_lists_agents_most_urgent_first() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("command-palette-go-to");
    let mut session = Session::new();
    let workspace_id = session.create_workspace(directory.0.clone()).unwrap();
    let first_tab = session.workspace(workspace_id).unwrap().tabs()[0].id();
    let second_tab = session.create_tab(workspace_id, None).unwrap();
    assert!(session.rename_tab(second_tab, "Palette Target"));
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
    let presented_tab = |window: &mut VisualTestContext| {
        window.read(|app| view.read(app).presented().map(|(_, _, _, tab_id)| tab_id))
    };
    assert!(wait_until(window, |window| presented_tab(window) == Some(first_tab)));
    let pane_of = |window: &mut VisualTestContext, tab_id: TabId| {
        window.read(|app| {
            let session = view
                .read(app)
                .connection(1)
                .unwrap()
                .session()
                .unwrap()
                .clone();
            session.tab(tab_id).unwrap().panes()[0].id()
        })
    };
    let (working, blocked) = (pane_of(window, first_tab), pane_of(window, second_tab));
    for (pane_id, state) in [
        (working, AgentState::Working),
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

    focus_window(window, &view);
    let go_to = if cfg!(target_os = "macos") {
        "cmd-p"
    } else {
        "ctrl-shift-k"
    };
    window.simulate_keystrokes(go_to);
    window.run_until_parked();
    assert!(palette_open(window));
    let row = |window: &mut VisualTestContext, pane_id: PaneId| {
        window
            .debug_bounds(leaked_selector(format!(
                "command-palette-agent-1-{}",
                pane_id.as_u64()
            )))
            .expect("Go to lists every Agent")
    };
    assert!(
        row(window, blocked).top() < row(window, working).top(),
        "the Blocked Agent comes before the Working one"
    );
    assert!(
        window.debug_bounds("command-palette-device-1").is_some(),
        "and the Device is a place too"
    );

    type_and_draw(window, "palette target");
    let target = leaked_selector(format!("command-palette-tab-1-{}", second_tab.as_u64()));
    assert!(window.debug_bounds(target).is_some());
    window.simulate_keystrokes("enter");
    assert!(wait_until(window, |window| presented_tab(window) == Some(second_tab)));
    assert!(!palette_open(window));
}
