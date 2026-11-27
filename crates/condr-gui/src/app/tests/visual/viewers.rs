use super::dock::create_workspace;
use super::*;
use gpui_kit::{Bounds, Pixels, Point};

/// A viewer opens in a Tab of its own; that Tab dragged from the strip onto a terminal's
/// right edge merges beside it, and the viewer's header dragged back onto the strip makes
/// it a Tab of its own again (ADR 0078).
#[test]
fn a_viewer_tab_merges_beside_a_terminal_and_its_header_leaves_for_a_tab() {
    let _serial_guard = acquire_visual_test_lock();
    let root = TestDirectory::new("viewer-panes");
    std::fs::write(root.0.join("notes.txt"), "alpha\n").unwrap();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (workspace_id, terminal_tab, terminal) = create_workspace(window, &view, &root);
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.show_file_on(1, workspace_id, "notes.txt".into(), window, cx)
        });
    });
    let preview = |window: &mut VisualTestContext| {
        window.read(|app| {
            let session = view.read(app).active_session()?;
            let (tab, pane) = session.viewer_pane(workspace_id, ViewerKind::File)?;
            Some((tab.id(), pane.id(), tab.panes().len()))
        })
    };
    assert!(wait_until(window, |window| preview(window).is_some()));
    let (preview_tab, preview_pane, _) = preview(window).unwrap();
    assert_ne!(
        preview_tab, terminal_tab,
        "a viewer opens in a Tab of its own"
    );

    let drag = |window: &mut VisualTestContext, from: &'static str, to: Point<Pixels>| {
        window.update(|window, cx| _ = window.draw(cx));
        let from = window.debug_bounds(from).expect("the drag source renders");
        window.simulate_mouse_down(from.center(), MouseButton::Left, Modifiers::default());
        window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        window.update(|window, cx| _ = window.draw(cx));
        window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        window.update(|window, cx| _ = window.draw(cx));
        window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    };

    // Back on the terminal's Tab, the Preview's Tab dropped on its right edge.
    let terminal_button = {
        window.update(|window, cx| _ = window.draw(cx));
        window
            .debug_bounds(leaked_selector(format!("tab-{}", terminal_tab.as_u64())))
            .unwrap()
    };
    window.simulate_click(terminal_button.center(), Modifiers::default());
    window.update(|window, cx| _ = window.draw(cx));
    let body = window
        .debug_bounds(leaked_selector(format!(
            "terminal-pane-{}",
            terminal.as_u64()
        )))
        .expect("the terminal renders");
    let right_edge = point(body.right() - px(10.), body.center().y);
    drag(
        window,
        leaked_selector(format!("tab-{}", preview_tab.as_u64())),
        right_edge,
    );
    assert!(
        wait_until(window, |window| preview(window).is_some_and(
            |(tab, _, panes)| { tab == terminal_tab && panes == 2 }
        )),
        "the Preview's Tab merged beside the terminal"
    );
    let rects = window.read(|app| {
        let session = view.read(app).active_session().unwrap();
        session.tab(terminal_tab).unwrap().pane_rects()
    });
    let rect = |id| *rects.iter().find(|rect| rect.id == id).unwrap();
    assert!(rect(terminal).right <= rect(preview_pane).left + 1e-6);

    // Its header dragged onto the strip, it leaves for a Tab of its own.
    let strip = window
        .debug_bounds(leaked_selector(format!("tab-{}", terminal_tab.as_u64())))
        .unwrap();
    drag(
        window,
        leaked_selector(format!("viewer-pane-header-{}", preview_pane.as_u64())),
        strip.center(),
    );
    assert!(
        wait_until(window, |window| preview(window).is_some_and(
            |(tab, _, panes)| { tab != terminal_tab && panes == 1 }
        )),
        "the Preview left for a Tab of its own"
    );
}

/// Two terminal Tabs: the second dragged from the strip onto the first's right edge
/// becomes the right half of the first, and its Tab closes. A third, shown, dragged onto
/// its own Pane's right edge joins the Tab before it as IntelliJ's editor tabs do
/// (ADR 0078).
#[test]
fn a_terminal_tab_dragged_onto_another_tabs_pane_merges_beside_it() {
    let _serial_guard = acquire_visual_test_lock();
    let root = TestDirectory::new("tab-merge");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (workspace_id, first_tab, terminal) = create_workspace(window, &view, &root);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateTab {
                name: None,
                cwd_from: None,
                workspace_id,
            });
        });
    });
    let tabs = |window: &mut VisualTestContext| {
        window.read(|app| {
            let session = view.read(app).active_session()?;
            let workspace = session.workspace(workspace_id)?;
            Some(
                workspace
                    .tabs()
                    .iter()
                    .map(|tab| (tab.id(), tab.panes().len()))
                    .collect::<Vec<_>>(),
            )
        })
    };
    assert!(wait_until(window, |window| tabs(window)
        .is_some_and(|tabs| tabs.len() == 2)));
    let second_tab = tabs(window).unwrap()[1].0;

    window.update(|window, cx| _ = window.draw(cx));
    let first_button = window
        .debug_bounds(leaked_selector(format!("tab-{}", first_tab.as_u64())))
        .unwrap();
    window.simulate_click(first_button.center(), Modifiers::default());
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .main_view()
            .dock_surface
            .map(|surface| surface.tab_id)
            == Some(first_tab)
    })));
    window.update(|window, cx| _ = window.draw(cx));
    let body = window
        .debug_bounds(leaked_selector(format!(
            "terminal-pane-{}",
            terminal.as_u64()
        )))
        .expect("the terminal renders");
    let to = point(body.right() - px(10.), body.center().y);
    let from = window
        .debug_bounds(leaked_selector(format!("tab-{}", second_tab.as_u64())))
        .unwrap();
    let drag = |window: &mut VisualTestContext, from: Bounds<Pixels>, to: Point<Pixels>| {
        window.simulate_mouse_down(from.center(), MouseButton::Left, Modifiers::default());
        window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        window.update(|window, cx| _ = window.draw(cx));
        window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        window.update(|window, cx| _ = window.draw(cx));
        window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    };
    drag(window, from, to);
    assert!(
        wait_until(window, |window| tabs(window)
            .is_some_and(|tabs| tabs == [(first_tab, 2)])),
        "the second Tab merged into the first: {:?}",
        tabs(window)
    );

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateTab {
                name: None,
                cwd_from: None,
                workspace_id,
            });
        });
    });
    assert!(wait_until(window, |window| tabs(window)
        .is_some_and(|tabs| tabs.len() == 2)));
    let third_tab = tabs(window).unwrap()[1].0;
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .main_view()
            .dock_surface
            .map(|surface| surface.tab_id)
            == Some(third_tab)
    })));
    let third = window.read(|app| {
        let session = view.read(app).active_session().unwrap();
        session.tab(third_tab).unwrap().focused_pane().unwrap().id()
    });
    window.update(|window, cx| _ = window.draw(cx));
    let body = window
        .debug_bounds(leaked_selector(format!("terminal-pane-{}", third.as_u64())))
        .expect("the shown Tab's terminal renders");
    let from = window
        .debug_bounds(leaked_selector(format!("tab-{}", third_tab.as_u64())))
        .unwrap();
    drag(window, from, point(body.right() - px(10.), body.center().y));
    assert!(
        wait_until(window, |window| tabs(window)
            .is_some_and(|tabs| tabs == [(first_tab, 3)])),
        "the shown Tab joined the one before it: {:?}",
        tabs(window)
    );
    let rects = window.read(|app| {
        let session = view.read(app).active_session().unwrap();
        session.tab(first_tab).unwrap().pane_rects()
    });
    let rect = |id| *rects.iter().find(|rect| rect.id == id).unwrap();
    assert!(rect(terminal).right <= rect(third).left + 1e-6);
    assert_eq!((rect(third).top, rect(third).bottom), (0., 1.));
}
