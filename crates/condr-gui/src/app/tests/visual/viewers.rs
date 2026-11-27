use super::dock::create_workspace;
use super::*;
use gpui_kit::{Pixels, Point};

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
