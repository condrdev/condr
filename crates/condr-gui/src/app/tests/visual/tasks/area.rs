//! The interaction area: a card's terminal beside the cards, the keyboard, next-agent,
//! its Pane's header and the split.

use super::*;

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

    // Selected, the card still moves with its Agent's state, at once (ADR 0075).
    assert!(inside(window, card, "tasks-column-working"));
    agent_changed(
        window,
        &view,
        pane_id,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Blocked,
            detail: None,
        }),
    );
    assert!(inside(window, card, "tasks-column-needs-you"));
    assert!(window.debug_bounds("tasks-area").is_some());

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
    use crate::app::tasks::TasksScope;
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

/// The split between the cards and the area is Kit's resize handle: a press a few
/// pixels off its line still takes it, and the drag moves the split.
#[test]
fn the_area_split_takes_a_press_beside_its_line() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);
    click(
        window,
        leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64())),
    );
    let band = window
        .debug_bounds("tasks-split")
        .expect("the handle is drawn");
    assert!(
        band.size.height > px(4.),
        "a band, not a hairline: {band:?}"
    );
    let press = point(band.center().x, band.top() + px(1.));
    let to = point(press.x, press.y - px(120.));
    window.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
    for _ in 0..2 {
        window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        draw(window);
    }
    window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    let split = window.read(|app| view.read(app).main_view().tasks.split_board);
    assert!(
        split.is_some_and(|split| split < crate::app::tasks::DEFAULT_SPLIT_BOARD),
        "dragging up shrinks the cards: {split:?}"
    );
}
