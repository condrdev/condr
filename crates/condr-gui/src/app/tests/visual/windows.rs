//! Detached windows (ADR 0043): each test opens a second window of the same Client.

use super::dock::create_workspace;
use super::*;
use crate::app::window_view::WindowScope;
use gpui_kit::AnyWindowHandle;
use relative_path::RelativePathBuf;

fn window_for(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
    scope: WindowScope,
) -> Option<AnyWindowHandle> {
    window.read(|app| view.read(app).window_for(scope))
}

fn draw(window: &mut VisualTestContext) {
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
}

#[test]
fn a_detached_tab_moves_to_its_own_window_and_comes_back() {
    let _serial_guard = acquire_visual_test_lock();
    let root = TestDirectory::new("detach-tab");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (workspace_id, tab_id, pane_id) = create_workspace(window, &view, &root);
    let scope = WindowScope::Tab(1, tab_id);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.files_view.changes_open.insert((1, workspace_id));
            this.open_in_new_window(scope, window, cx);
        })
    });
    window.run_until_parked();
    let handle = window_for(window, &view, scope).expect("the Tab opens in a window of its own");
    let mut detached = VisualTestContext::from_window(handle, window);
    draw(&mut detached);
    draw(window);

    assert!(
        detached.debug_bounds(terminal_selector(pane_id)).is_some(),
        "the new window draws the Tab"
    );
    assert!(
        detached.debug_bounds("condr-sidebar").is_none(),
        "a Tab window has no sidebar"
    );
    assert!(
        window.debug_bounds(terminal_selector(pane_id)).is_none(),
        "detaching moves, it does not mirror"
    );
    assert!(window.debug_bounds("shown-elsewhere").is_some());
    assert!(
        window.debug_bounds(tab_selector(tab_id)).is_some(),
        "the Tab strip keeps listing the Tab"
    );
    assert!(
        window.debug_bounds("condr-changes-column").is_some(),
        "the Workspace, and its right sidebar, stay in the main window"
    );

    let bring_back = detached
        .debug_bounds("bring-back-window")
        .expect("a detached window offers Bring Back");
    detached.simulate_click(bring_back.center(), Modifiers::default());
    draw(window);
    assert!(window_for(window, &view, scope).is_none());
    assert!(!window.windows().contains(&handle));
    assert!(window.debug_bounds(terminal_selector(pane_id)).is_some());
    assert!(window.debug_bounds("shown-elsewhere").is_none());
}

#[test]
fn a_detached_workspace_keeps_its_right_sidebar_and_lists_only_itself() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("detach-workspace-first");
    let second_root = TestDirectory::new("detach-workspace-second");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (first, _, first_pane) = create_workspace(window, &view, &first_root);
    let (second, _, second_pane) = create_workspace(window, &view, &second_root);
    let scope = WindowScope::Workspace(1, first);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.files_view.changes_open.insert((1, first));
            this.open_in_new_window(scope, window, cx);
        })
    });
    window.run_until_parked();
    let handle = window_for(window, &view, scope).expect("the Workspace opens in a window");
    let mut detached = VisualTestContext::from_window(handle, window);
    draw(&mut detached);
    draw(window);

    // The new window is the Workspace as the main window drew it, right sidebar
    // included, and its left sidebar lists only this Workspace.
    assert!(
        detached
            .debug_bounds(terminal_selector(first_pane))
            .is_some()
    );
    assert!(detached.debug_bounds("condr-changes-column").is_some());
    assert!(
        detached
            .debug_bounds(sidebar_workspace_selector(first))
            .is_some()
    );
    assert!(
        detached
            .debug_bounds(sidebar_workspace_selector(second))
            .is_none()
    );

    // The main window keeps it selected, shows where it went, and leaves its right
    // sidebar to the window that draws it; it still lists every Workspace.
    assert!(window.debug_bounds("shown-elsewhere").is_some());
    assert!(window.debug_bounds("condr-changes-column").is_none());
    assert!(
        window
            .debug_bounds(sidebar_workspace_selector(first))
            .is_some()
    );
    assert!(
        window
            .debug_bounds(sidebar_workspace_selector(second))
            .is_some()
    );

    // Selecting another Workspace there shows it as before.
    let row = window
        .debug_bounds(sidebar_workspace_selector(second))
        .unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    draw(window);
    assert!(
        window
            .debug_bounds(terminal_selector(second_pane))
            .is_some()
    );
    assert!(window.debug_bounds("shown-elsewhere").is_none());
    assert!(
        detached.read(|app| view.read(app).window_for(scope)) == Some(handle),
        "a selection in the main window leaves the detached window alone"
    );
    // Selecting the detached one again shows its page: the sidebar and the keys agree.
    let row = window.debug_bounds(sidebar_workspace_selector(first)).unwrap();
    window.simulate_click(row.center(), Modifiers::default());
    draw(window);
    assert!(window.debug_bounds("shown-elsewhere").is_some());

    // The OS close button brings it back, as the shown item.
    assert!(detached.simulate_close());
    draw(window);
    assert!(window_for(window, &view, scope).is_none());
    assert_eq!(
        super::dock::presented(window, &view).map(|(_, workspace, _)| workspace),
        Some(first)
    );
    assert!(window.debug_bounds(terminal_selector(first_pane)).is_some());
}

#[test]
fn focus_and_activation_follow_the_window_last_activated() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("window-focus-first");
    let second_root = TestDirectory::new("window-focus-second");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (server, endpoint) = start_server();
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint.clone());
    let (first, _, first_pane) = create_workspace(window, &view, &first_root);
    let (second, _, second_pane) = create_workspace(window, &view, &second_root);
    let scope = WindowScope::Workspace(1, first);
    window.update(|window, cx| {
        view.update(cx, |this, cx| this.open_in_new_window(scope, window, cx))
    });
    window.run_until_parked();
    let handle = window_for(window, &view, scope).unwrap();
    let mut detached = VisualTestContext::from_window(handle, window);
    draw(&mut detached);
    let reported = |window: &mut VisualTestContext| {
        window.read(|app| view.read(app).terminal_input.reported_focus)
    };

    // The main window shows the page for the first Workspace; select the second.
    window.update(|window, cx| {
        view.update(cx, |this, cx| this.select_workspace(1, second, window, cx))
    });
    draw(window);

    detached.update(|window, _| window.activate_window());
    draw(&mut detached);
    assert!(
        wait_until(&mut detached, |window| reported(window)
            == Some((1, first_pane))),
        "the detached window's Pane reports focus once it is active"
    );
    window.update(|window, _| window.activate_window());
    draw(window);
    assert!(
        wait_until(window, |window| reported(window) == Some((1, second_pane))),
        "returning to the main window reports its Pane instead"
    );

    // An agent asking every viewer to look at the first Workspace brings the window
    // drawing it forward, and leaves the main window where it was.
    let mut cli = ClientConnection::connect_overview(&endpoint, "activate-detached").unwrap();
    cli.layout(LayoutCommand::ActivateWorkspace {
        workspace_id: first,
    })
    .unwrap()
    .unwrap();
    assert!(wait_until(window, |window| {
        window.read(|app| app.active_window()) == Some(handle)
    }));
    assert!(wait_until(window, |window| reported(window) == Some((1, first_pane))));
    assert_eq!(
        super::dock::presented(window, &view).map(|(_, workspace, _)| workspace),
        Some(first),
        "the actions follow the window now active"
    );
    window.update(|window, _| window.activate_window());
    draw(window);
    assert_ne!(
        super::dock::presented(window, &view).map(|(_, workspace, _)| workspace),
        Some(first),
        "the main window still shows the second Workspace"
    );
}

#[test]
fn a_detached_window_closes_with_its_tab() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("window-closes-first");
    let second_root = TestDirectory::new("window-closes-second");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (server, endpoint) = start_server();
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint.clone());
    create_workspace(window, &view, &first_root);
    let (_, second_tab, _) = create_workspace(window, &view, &second_root);
    let scope = WindowScope::Tab(1, second_tab);
    window.update(|window, cx| {
        view.update(cx, |this, cx| this.open_in_new_window(scope, window, cx))
    });
    window.run_until_parked();
    let handle = window_for(window, &view, scope).unwrap();

    let mut cli = ClientConnection::connect_overview(&endpoint, "close-detached").unwrap();
    cli.layout(LayoutCommand::CloseTab { tab_id: second_tab })
        .unwrap()
        .unwrap();
    assert!(wait_until(window, |window| {
        window_for(window, &view, scope).is_none() && !window.windows().contains(&handle)
    }));
    draw(window);
    assert!(window.debug_bounds("shown-elsewhere").is_none());
}

#[test]
fn a_changes_click_retargets_a_detached_diff_tab_without_moving_either_window() {
    let _serial_guard = acquire_visual_test_lock();
    let repository = TestDirectory::new("detach-diff");
    run_git(&repository.0, ["init", "-q", "-b", "main"]);
    run_git(&repository.0, ["config", "user.email", "condr@example.com"]);
    run_git(&repository.0, ["config", "user.name", "Condr"]);
    run_git(&repository.0, ["config", "commit.gpgsign", "false"]);
    std::fs::write(repository.0.join("notes.txt"), "alpha\n").unwrap();
    std::fs::write(repository.0.join("other.txt"), "beta\n").unwrap();
    run_git(&repository.0, ["add", "."]);
    run_git(&repository.0, ["commit", "-q", "-m", "files"]);
    std::fs::write(repository.0.join("notes.txt"), "ALPHA\n").unwrap();
    std::fs::write(repository.0.join("other.txt"), "BETA\n").unwrap();

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (workspace_id, terminal_tab, _) = create_workspace(window, &view, &repository);
    let shown_diff = |window: &mut VisualTestContext| {
        window.read(|app| {
            let session = view.read(app).connection(1)?.session()?;
            let tab = session.diff_tab(workspace_id)?;
            Some((tab.id(), tab.diff()?.path().to_string()))
        })
    };
    let show_diff = |window: &mut VisualTestContext, path: &str| {
        let path = RelativePathBuf::from(path);
        window.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.show_diff_on(1, workspace_id, path, window, cx)
            })
        });
    };

    show_diff(window, "notes.txt");
    assert!(wait_until(window, |window| {
        shown_diff(window).is_some_and(|(_, path)| path == "notes.txt")
    }));
    let diff_tab = shown_diff(window).unwrap().0;
    let scope = WindowScope::Tab(1, diff_tab);
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_in_new_window(scope, window, cx);
            this.activate_tab_on(1, terminal_tab, window, cx);
        })
    });
    window.run_until_parked();
    let handle = window_for(window, &view, scope).unwrap();
    window.update(|window, _| window.activate_window());
    draw(window);
    let main = window.update(|window, _| window.window_handle());

    show_diff(window, "other.txt");
    assert!(wait_until(window, |window| {
        shown_diff(window).is_some_and(|(_, path)| path == "other.txt")
    }));
    draw(window);
    assert_eq!(
        super::dock::presented(window, &view).map(|(_, _, tab)| tab),
        Some(terminal_tab),
        "the main window keeps its terminal while the Diff Tab elsewhere follows"
    );
    assert!(window.debug_bounds("shown-elsewhere").is_none());
    assert_eq!(window.read(|app| app.active_window()), Some(main));
    assert_eq!(window_for(window, &view, scope), Some(handle));
}

#[test]
fn a_detached_device_leaves_its_page_whose_bring_back_returns_it() {
    let _serial_guard = acquire_visual_test_lock();
    let root = TestDirectory::new("detach-device");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (workspace_id, _, pane_id) = create_workspace(window, &view, &root);
    let scope = WindowScope::Device(1);
    window.update(|window, cx| {
        view.update(cx, |this, cx| this.open_in_new_window(scope, window, cx))
    });
    window.run_until_parked();
    let handle = window_for(window, &view, scope).unwrap();
    let mut detached = VisualTestContext::from_window(handle, window);
    draw(&mut detached);
    draw(window);
    assert!(detached.debug_bounds(terminal_selector(pane_id)).is_some());
    assert!(
        detached
            .debug_bounds(sidebar_workspace_selector(workspace_id))
            .is_some()
    );
    assert!(window.debug_bounds(terminal_selector(pane_id)).is_none());
    assert!(
        window
            .debug_bounds(sidebar_workspace_selector(workspace_id))
            .is_some()
    );

    let bring_back = window
        .debug_bounds("bring-back")
        .expect("the page offers Bring Back");
    window.simulate_click(bring_back.center(), Modifiers::default());
    draw(window);
    assert!(window_for(window, &view, scope).is_none());
    assert!(!window.windows().contains(&handle));
    assert!(window.debug_bounds(terminal_selector(pane_id)).is_some());
}
