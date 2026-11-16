use super::*;

/// Opens a Workspace on the test Server and waits until its first Pane draws.
fn workspace_with_a_pane(window: &mut VisualTestContext, view: &Entity<Condr>) -> PaneId {
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    let mut pane_id = None;
    assert!(wait_until(window, |window| {
        pane_id = window.read(|app| {
            view.read(app)
                .active_session()?
                .workspaces()
                .first()
                .and_then(|workspace| workspace.tabs().first()?.focused_pane())
                .map(|pane| pane.id())
        });
        pane_id.is_some_and(|pane_id| {
            window.debug_bounds(terminal_selector(pane_id)).is_some()
                && window.read(|app| view.read(app).terminal(1, pane_id).is_some())
        })
    }));
    pane_id.unwrap()
}

/// Hands the Client a full frame of the Pane's current view, one revision on.
fn deliver_frame(window: &mut VisualTestContext, view: &Entity<Condr>, pane_id: PaneId) {
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let (generation, server_id, session_id, mut terminal_view) = {
                let connection = this.connection(1).unwrap();
                (
                    connection.connect_generation,
                    connection.model.server_id.unwrap(),
                    connection.model.session_id.unwrap(),
                    connection.model.terminals[&pane_id].view.as_ref().clone(),
                )
            };
            terminal_view.revision += 1;
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::TerminalFrame(TerminalFrameBatch {
                    server_id,
                    session_id,
                    panes: vec![PaneTerminalFrame {
                        pane_id,
                        frame: TerminalViewFrame::Full(terminal_view),
                    }],
                })),
                cx,
            );
        });
    });
}

fn sidebar_renders(window: &mut VisualTestContext, view: &Entity<Condr>) -> usize {
    window.read(|app| view.read(app).sidebar_render_count.get())
}

/// ADR 0068: a terminal frame redraws its Panel and never the sidebar, which is
/// rendered again only when `Condr` itself changes.
#[test]
fn terminal_frames_leave_the_sidebar_alone() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let pane_id = workspace_with_a_pane(window, &view);
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));

    let before = sidebar_renders(window, &view);
    for _ in 0..5 {
        deliver_frame(window, &view, pane_id);
        window.update(|window, cx| _ = window.draw(cx));
    }
    assert_eq!(
        sidebar_renders(window, &view),
        before,
        "terminal frames rebuilt the sidebar"
    );

    window.update(|_, cx| view.update(cx, |_, cx| cx.notify()));
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(
        sidebar_renders(window, &view),
        before + 1,
        "a change to Condr renders the sidebar once"
    );
}

fn task_card_renders(window: &mut VisualTestContext, view: &Entity<Condr>) -> usize {
    window.read(|app| view.read(app).tasks_board_render_count.get())
}

/// ADR 0068: in Tasks mode, frames of the Agent the interaction area shows redraw its
/// Panel and never the card area, which renders again once per change to `Condr`, the
/// first one after entering included.
#[test]
fn terminal_frames_leave_the_task_cards_alone() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let pane_id = workspace_with_a_pane(window, &view);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let message = ServerMessage::Event {
                server_id: connection.model.server_id.unwrap(),
                session_id: connection.model.session_id.unwrap(),
                sequence: connection.model.sequence + 1,
                event: SessionEvent::AgentChanged {
                    pane_id,
                    agent: Some(AgentSnapshot {
                        session_id: None,
                        kind: AgentKind::Codex,
                        state: AgentState::Working,
                        detail: None,
                    }),
                },
            };
            let generation = connection.connect_generation;
            this.handle_incoming(1, generation, Incoming::Message(message), cx);
        });
    });
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    window.update(|window, cx| _ = window.draw(cx));

    // The first change after entering reaches the cards.
    let entered = task_card_renders(window, &view);
    window.update(|_, cx| view.update(cx, |_, cx| cx.notify()));
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(task_card_renders(window, &view), entered + 1);

    // With the Agent's terminal open beside them, its frames leave the cards alone.
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_task_card(1, pane_id, true, window, cx)
        })
    });
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    let before = task_card_renders(window, &view);
    for _ in 0..5 {
        deliver_frame(window, &view, pane_id);
        window.update(|window, cx| _ = window.draw(cx));
    }
    assert_eq!(
        task_card_renders(window, &view),
        before,
        "terminal frames rebuilt the task cards"
    );
    window.update(|_, cx| view.update(cx, |_, cx| cx.notify()));
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(task_card_renders(window, &view), before + 1);
}
