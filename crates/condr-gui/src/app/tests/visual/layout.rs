use super::*;

fn agent_changed(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
    pane_id: PaneId,
    agent: Option<AgentSnapshot>,
) {
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.connect_generation;
            let message = ServerMessage::Event {
                server_id: connection.server_id.unwrap(),
                session_id: connection.session_id.unwrap(),
                sequence: connection.sequence + 1,
                event: SessionEvent::AgentChanged { pane_id, agent },
            };
            this.handle_incoming(1, generation, Incoming::Message(message), cx);
            cx.notify();
        });
    });
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
}

#[test]
fn default_window_options_create_1280_by_720_window() {
    let _serial_guard = acquire_visual_test_lock();
    let app = TestAppContext::single();
    let handle = app.update(|cx| {
        cx.open_window(default_window_options(cx), |_, cx| {
            cx.new(|_| gpui_kit::Empty)
        })
        .unwrap()
    });
    let window = VisualTestContext::from_window(*handle.deref(), &app).into_mut();
    let bounds = window.update(|window, _| window.bounds());

    assert_eq!(bounds.size, DEFAULT_WINDOW_SIZE);
    window.quit();
}

#[test]
fn cold_split_workspace_uses_the_real_dock_size_before_first_paint() {
    let _serial_guard = acquire_visual_test_lock();
    let workspace_root = TestDirectory::new("cold-split-workspace");
    let snapshot_root = TestDirectory::new("cold-split-snapshot");
    let mut session = Session::new();
    let workspace_id = session
        .create_workspace(workspace_root.0.clone())
        .expect("test Workspace capacity");
    let workspace = session.workspace(workspace_id).unwrap();
    let tab_id = workspace.active_tab().id();
    let first_pane = workspace.active_tab().focused_pane().unwrap().id();
    let second_pane = session
        .split_pane(first_pane, SplitDirection::Horizontal, 0.3)
        .expect("test Pane capacity");
    let snapshot_path = snapshot_root.0.join("session.bin");
    std::fs::write(&snapshot_path, session.snapshot().to_bytes().unwrap()).unwrap();

    let endpoint = Endpoint::local(snapshot_root.0.join("server.sock"));
    let server = start_server_with_config(
        ServerConfig::at_socket(endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path),
    );
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    window.update(|window, cx| _ = window.draw(cx));

    let surface_key = DockSurfaceKey {
        connection_key: 1,
        tab_id,
    };
    let (layout_size, dock_bounds, rebuilds) = window.read(|app| {
        let condr = view.read(app);
        let surface = condr
            .dock_surfaces
            .get(&surface_key)
            .expect("cold Bootstrap should build its Dock surface");
        (
            surface
                .layout_size
                .expect("initial layout size is recorded"),
            surface.area.read(app).bounds(),
            condr.dock_rebuild_count,
        )
    });
    assert!((layout_size.width - dock_bounds.size.width).abs() <= px(1.));
    assert!((layout_size.height - dock_bounds.size.height).abs() <= px(1.));
    assert_eq!(rebuilds, 1, "cold startup must install the Dock once");

    let first = window
        .debug_bounds(terminal_selector(first_pane))
        .expect("first cold Pane should be visible");
    let second = window
        .debug_bounds(terminal_selector(second_pane))
        .expect("second cold Pane should be visible");
    let pane_width = first.size.width + second.size.width;
    assert!((first.size.width / pane_width - 0.3).abs() < 0.03);
    assert!(first.right() <= second.left());
    assert!(second.left() - first.right() <= px(8.));
    window.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        1,
        "the first paint must not trigger a corrective Dock replacement"
    );
}

#[test]
fn readonly_dock_resize_restores_the_authoritative_projection() {
    let _serial_guard = acquire_visual_test_lock();
    let workspace_root = TestDirectory::new("readonly-dock-resize");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
                root_directory: workspace_root.0.clone(),
            });
        });
    });
    let mut initial = None;
    assert!(wait_until_event_driven(window, |window| {
        initial = window.read(|app| {
            let condr = view.read(app);
            let session = condr.active_session()?;
            let tab = session.active_workspace()?.active_tab();
            let surface_key = DockSurfaceKey {
                connection_key: 1,
                tab_id: tab.id(),
            };
            condr.dock_surfaces.contains_key(&surface_key).then_some((
                tab.id(),
                tab.focused_pane().unwrap().id(),
                surface_key,
            ))
        });
        initial.is_some()
    }));
    let (tab_id, pane_id, surface_key) = initial.unwrap();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                focus: true,
                pane_id,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            let session = condr.active_session().unwrap();
            session.tab(tab_id).is_some_and(|tab| {
                tab.panes().len() == 2
                    && condr.dock_surfaces[&surface_key].projection.as_ref()
                        == Some(tab.layout().unwrap())
            })
        })
    }));
    window.update(|window, cx| _ = window.draw(cx));
    window.run_until_parked();

    let (next_request_id, rebuilds) = window.read(|app| {
        let condr = view.read(app);
        (
            condr.connection(1).unwrap().next_layout_request_id,
            condr.dock_rebuild_count,
        )
    });
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let mut local = Session::restore(this.connection(1).unwrap().snapshot.clone()).unwrap();
            assert!(local.set_tab_split_ratios(tab_id, &[0.72]));
            let local_layout = local.tab(tab_id).unwrap().layout().unwrap().clone();
            let surface = this.dock_surfaces.get(&surface_key).unwrap();
            assert_eq!(surface.programmatic_layout_events, 0);
            let available_size = surface.area.read(cx).bounds().size;
            let area = surface.area.clone();
            let dock_layout = this.build_dock_layout(1, &local_layout, available_size, cx);
            this.connection_mut(1).unwrap().controlling = false;
            area.update(cx, |dock, cx| {
                dock.set_center(dock_layout, window, cx);
            });
        });
    });
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));

    // Agent metadata can advance the reliable cursor during a purely local resize.
    agent_changed(window, &view, pane_id, None);

    let (authoritative_layout, panes) = window.read(|app| {
        let condr = view.read(app);
        let session = condr.active_session().unwrap();
        let tab = session.tab(tab_id).unwrap();
        (
            tab.layout().unwrap().clone(),
            tab.panes().iter().map(|pane| pane.id()).collect::<Vec<_>>(),
        )
    });
    assert_eq!(
        window.read(|app| view.read(app).connection(1).unwrap().next_layout_request_id),
        next_request_id,
        "a read-only Dock resize must not send a Layout command"
    );
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds + 1,
        "the rejected local resize should be restored once"
    );
    assert!(window.read(|app| {
        let surface = &view.read(app).dock_surfaces[&surface_key];
        surface.pending_projection_request.is_none()
            && surface.pending_projection_applied_sequence.is_none()
            && surface.projection.as_ref() == Some(&authoritative_layout)
    }));
    let first = window.debug_bounds(terminal_selector(panes[0])).unwrap();
    let second = window.debug_bounds(terminal_selector(panes[1])).unwrap();
    let width = first.size.width + second.size.width;
    assert!((first.size.width / width - 0.5).abs() < 0.03);
}

#[test]
fn sidebar_header_and_tree_controls_match_the_prototype() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));

    let heading_row = window
        .debug_bounds("server-heading-1-row")
        .expect("the Server heading row should render");
    let heading = window
        .debug_bounds("server-heading-1")
        .expect("the Server heading should render");
    let new_workspace = window
        .debug_bounds("new-workspace-server-1")
        .expect("New Workspace should render in the Server heading");
    assert!(
        (heading.center().y - new_workspace.center().y).abs() <= px(1.),
        "the Server name and New Workspace should share a row"
    );
    assert!(
        heading.right() <= new_workspace.left(),
        "New Workspace should sit to the right of the Server name"
    );
    assert!(
        (heading_row.right() - new_workspace.right()).abs() <= px(1.),
        "New Workspace should align with the heading's right edge"
    );
    window
        .debug_bounds("add-server")
        .expect("Connect Remote Device should render in the sidebar header");

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
                root_directory: std::env::temp_dir(),
            });
        });
    });

    let mut tree_ids = None;
    assert!(wait_until(window, |window| {
        tree_ids = window.read(|app| {
            let session = view.read(app).active_session()?;
            let workspace = session.active_workspace()?;
            Some((
                workspace.id(),
                workspace.active_tab().focused_pane().unwrap().id(),
            ))
        });
        tree_ids.is_some()
    }));
    let (workspace_id, pane_id) = tree_ids.unwrap();
    window.update(|window, cx| _ = window.draw(cx));

    let workspace_toggle_selector =
        leaked_selector(format!("workspace-toggle-1-{}", workspace_id.as_u64()));
    let workspace_icon_selector =
        leaked_selector(format!("workspace-icon-1-{}", workspace_id.as_u64()));
    let workspace_label_selector =
        leaked_selector(format!("workspace-label-1-{}", workspace_id.as_u64()));
    let workspace_selector = sidebar_workspace_selector(workspace_id);
    assert!(
        window.debug_bounds(workspace_selector).is_some(),
        "a Server group should always show its Workspaces"
    );

    let empty_workspace_toggle = window
        .debug_bounds(workspace_toggle_selector)
        .expect("a Workspace without Agents should retain its disclosure control");
    let empty_workspace_icon = window.debug_bounds(workspace_icon_selector).unwrap();
    assert!(
        empty_workspace_toggle.contains(&empty_workspace_icon.center()),
        "the disclosure control shares the icon's slot and shows on hover"
    );
    window.simulate_click(empty_workspace_toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));

    window.update(|window, cx| {
        let focus = view.read(cx).panels[&(1, pane_id)]
            .read(cx)
            .focus_handle
            .clone();
        focus.focus(window, cx);
    });
    let focus_before = window.update(|window, cx| window.focused(cx));
    agent_changed(
        window,
        &view,
        pane_id,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Idle,
        }),
    );
    assert_eq!(window.update(|window, cx| window.focused(cx)), focus_before);
    let agent_selector = leaked_selector(format!("agent-1-{}", pane_id.as_u64()));
    assert!(
        window.debug_bounds(agent_selector).is_some(),
        "a newly detected Agent should expand its collapsed Workspace"
    );

    let workspace_toggle = window.debug_bounds(workspace_toggle_selector).unwrap();
    let workspace_icon = window.debug_bounds(workspace_icon_selector).unwrap();
    let workspace_label = window.debug_bounds(workspace_label_selector).unwrap();
    assert!(workspace_toggle.contains(&workspace_icon.center()));
    assert!(workspace_icon.right() <= workspace_label.left());
    assert!(
        (workspace_icon.left() - empty_workspace_icon.left()).abs() <= px(1.),
        "the Workspace icon must not move when the first Agent appears"
    );

    let agent_status_selector =
        leaked_selector(format!("agent-status-1-{}-idle", pane_id.as_u64()));
    let agent_label_selector = leaked_selector(format!("agent-label-1-{}", pane_id.as_u64()));
    let agent_status = window.debug_bounds(agent_status_selector).unwrap();
    let agent_label = window.debug_bounds(agent_label_selector).unwrap();
    assert!(agent_status.right() <= agent_label.left());

    let selection_before = window.read(|app| {
        let condr = view.read(app);
        (
            condr.active_connection,
            condr
                .active_session()
                .and_then(|session| session.active_workspace_id()),
            condr.target_pane,
        )
    });
    window.simulate_click(workspace_toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds(workspace_selector).is_some());
    assert!(
        window.debug_bounds(agent_selector).is_none(),
        "collapsing a Workspace should hide its Agents"
    );
    assert_eq!(
        window.read(|app| {
            let condr = view.read(app);
            (
                condr.active_connection,
                condr
                    .active_session()
                    .and_then(|session| session.active_workspace_id()),
                condr.target_pane,
            )
        }),
        selection_before,
        "the Workspace disclosure button must not change selection"
    );

    for state in [AgentState::Working, AgentState::Idle] {
        agent_changed(
            window,
            &view,
            pane_id,
            Some(AgentSnapshot {
                session_id: None,
                kind: AgentKind::Codex,
                state,
            }),
        );
        assert!(
            window.debug_bounds(agent_selector).is_none(),
            "status changes must respect a manual collapse"
        );
    }
    // A replacement process may be detected without a separate removal event.
    agent_changed(
        window,
        &view,
        pane_id,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Unknown,
        }),
    );
    assert!(window.debug_bounds(agent_selector).is_some());
    let toggle_center = window
        .debug_bounds(workspace_toggle_selector)
        .unwrap()
        .center();
    window.simulate_click(toggle_center, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds(agent_selector).is_none());
    agent_changed(window, &view, pane_id, None);
    agent_changed(
        window,
        &view,
        pane_id,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Unknown,
        }),
    );
    assert!(window.debug_bounds(agent_selector).is_some());
}

#[test]
fn server_workspace_button_and_only_tab_close_round_trip() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);

    let add_server = window
        .debug_bounds("add-server")
        .expect("Connect Remote Device button should be rendered");
    window.simulate_click(add_server.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));
    window.simulate_keystrokes("down enter");
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("dialog-primary-action").is_some());
    window.update(|window, cx| window.close_dialog(cx));
    window.run_until_parked();
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));

    let welcome_connect = window
        .debug_bounds("connect-remote-device")
        .expect("the welcome page should expose Connect Remote Device");
    window.simulate_click(welcome_connect.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));
    window.simulate_keystrokes("down enter");
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("dialog-primary-action").is_some());
    window.update(|window, cx| window.close_dialog(cx));
    window.run_until_parked();
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));

    let new_workspace = window
        .debug_bounds("new-workspace-server-1")
        .expect("Local server should expose New Workspace");
    window.simulate_click(new_workspace.center(), Modifiers::default());
    assert!(window.did_prompt_for_paths());
    window.simulate_path_prompt_response(|options| {
        assert!(!options.files);
        assert!(options.directories);
        assert!(!options.multiple);
        assert_eq!(options.prompt.as_deref(), Some("New Workspace on Local"));
        Some(vec![std::env::temp_dir()])
    });

    let mut tab_id = None;
    assert!(
        wait_until(window, |window| {
            tab_id = window.read(|app| {
                view.read(app)
                    .active_session()
                    .and_then(|session| Some(session.active_workspace()?.active_tab().id()))
            });
            tab_id.is_some()
        }),
        "server-scoped button should create a Workspace"
    );
    let tab_id = tab_id.unwrap();

    let tab = window
        .debug_bounds(tab_selector(tab_id))
        .expect("the only Tab should be rendered");
    window.simulate_mouse_down(tab.center(), MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| {
        _ = window.draw(cx);
    });
    window.simulate_keystrokes("down enter");
    window.run_until_parked();
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    window.update(|window, cx| window.close_dialog(cx));
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));

    // Hovering the Tab reveals its close button; closing the only Tab asks first, since
    // it takes the Workspace with it.
    let close = window
        .debug_bounds(leaked_selector(format!("close-tab-{}", tab_id.as_u64())))
        .expect("the Tab row should carry a hover close button");
    assert!(
        close.right() <= tab.right() + px(2.) && close.left() >= tab.left(),
        "the close button overlays the Tab's trailing end: {close:?} vs {tab:?}"
    );
    window.simulate_mouse_move(tab.center(), MouseButton::Left, Modifiers::default());
    window.simulate_click(close.center(), Modifiers::default());
    confirm_alert_dialog(window);

    let empty = wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspaces().is_empty())
        })
    });
    assert!(empty, "closing the only Tab should close its Workspace");
}

#[test]
fn the_sidebar_keeps_its_dragged_width_across_window_resizes() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));

    let initial = window.read(|app| view.read(app).sidebar_width);
    let sidebar = window.debug_bounds("condr-sidebar").unwrap();
    assert_eq!(sidebar.size.width, initial);

    let toggle = window
        .debug_bounds("toggle-sidebar")
        .expect("the title bar should expose the sidebar toggle");
    window.simulate_click(toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(
        window.debug_bounds("condr-sidebar").unwrap().size.width,
        px(48.),
        "collapsing keeps the workspace avatar rail"
    );
    // The collapsed rail is narrower than the macOS traffic-light inset; the toggle must
    // still sit inside its title segment rather than run under the Tab strip.
    let toggle = window.debug_bounds("toggle-sidebar").unwrap();
    let segment = window.debug_bounds("title-sidebar").unwrap();
    assert!(
        toggle.left() >= segment.left() && toggle.right() <= segment.right() + px(1.),
        "the sidebar toggle overflows its title segment: {toggle:?} vs {segment:?}"
    );
    window.simulate_click(toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(
        window.debug_bounds("condr-sidebar").unwrap().size.width,
        initial,
        "expanding restores the previous sidebar width"
    );

    // Drag the handle 80px to the right.
    let handle = window
        .debug_bounds("condr-sidebar-resize")
        .expect("the sidebar renders its resize handle");
    let start = handle.center();
    window.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(
        point(start.x + px(40.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    window.simulate_mouse_move(
        point(start.x + px(80.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    window.simulate_mouse_up(
        point(start.x + px(80.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    let dragged = window.read(|app| view.read(app).sidebar_width);
    assert_eq!(
        dragged,
        initial + px(80.),
        "dragging the handle resizes the sidebar"
    );
    assert_eq!(
        window.debug_bounds("condr-sidebar").unwrap().size.width,
        dragged
    );

    // A much wider window must not scale the sidebar along with it.
    window.simulate_resize(size(px(1920.), px(1080.)));
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(
        window.debug_bounds("condr-sidebar").unwrap().size.width,
        dragged,
        "the sidebar width is absolute, not a share of the window"
    );
}

#[test]
fn the_sidebar_coffee_button_toggles_keep_awake() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));
    assert!(!window.read(|app| view.read(app).keep_awake));
    let toggle = window
        .debug_bounds("toggle-keep-awake")
        .expect("the sidebar footer shows the keep-awake toggle");
    window.simulate_click(toggle.center(), Modifiers::default());
    // A headless test box has no session bus to inhibit, so the request may fail; the
    // switch then reports the failure instead of claiming a hold that is not there.
    let held = window.read(|app| {
        let this = view.read(app);
        assert_eq!(this.keep_awake, this._keep_awake.is_some());
        assert!(
            this.keep_awake
                || this
                    .app_error
                    .as_deref()
                    .is_some_and(|e| e.contains("awake"))
        );
        this.keep_awake
    });
    if held {
        window.simulate_click(toggle.center(), Modifiers::default());
        assert!(!window.read(|app| {
            let this = view.read(app);
            this.keep_awake || this._keep_awake.is_some()
        }));
    }
}

/// The title bar's "Open in" split button: hidden without a Workspace, placed after the
/// Tab strip, failing loudly when the program is gone, and remembering the editor a
/// launch succeeded with.
#[test]
fn the_title_bar_open_in_button_launches_and_remembers_the_editor() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds("open-in").is_none(),
        "without a Workspace there is nothing to open"
    );

    let button = window
        .debug_bounds("open-project")
        .expect("new workspace button should be rendered");
    window.simulate_click(button.center(), Modifiers::default());
    assert!(window.did_prompt_for_paths());
    let workspace_root = std::env::temp_dir();
    let selected_root = workspace_root.clone();
    window.simulate_path_prompt_response(move |_| Some(vec![selected_root]));
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.active_workspace().is_some())
        })
    }));

    // Stand in for the machine's scan: one program that does not exist, one that exits
    // at once, so the test opens no real editor.
    let (program, args) = if cfg!(windows) {
        ("cmd.exe", vec!["/c".into(), "exit".into()])
    } else {
        ("true", Vec::new())
    };
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.open_targets = Some(vec![
                OpenTarget {
                    id: "missing".into(),
                    label: "Missing".into(),
                    icon: OpenTargetIcon::Custom,
                    program: "condr-no-such-editor".into(),
                    args: Vec::new(),
                },
                OpenTarget {
                    id: "ok".into(),
                    label: "OK".into(),
                    icon: OpenTargetIcon::Custom,
                    program: program.into(),
                    args,
                },
            ]);
            cx.notify();
        });
    });
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));

    let open_in = window
        .debug_bounds("open-in")
        .expect("the title bar should show Open in once the Workspace is presented");
    let tabs = window
        .debug_bounds("workspace-tabs")
        .expect("the Tab strip should be rendered");
    assert!(
        open_in.left() >= tabs.right(),
        "Open in sits after the Tab strip: {open_in:?} vs {tabs:?}"
    );
    assert!(
        open_in.bottom() <= tabs.bottom() + px(1.),
        "Open in stays inside the title bar: {open_in:?} vs {tabs:?}"
    );

    // The primary half opens with the first target, whose program is missing.
    window.simulate_click(open_in.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| !window.notifications(cx).is_empty())
        }),
        "a failed launch should show a notification"
    );
    assert_eq!(
        window.read(|app| view.read(app).default_editor.clone()),
        None,
        "a failed launch must not become the default"
    );

    // The caret half lists every target; the second one launches and is remembered.
    let control = window.debug_bounds("open-in-control").unwrap();
    window.simulate_click(
        point(control.right() - px(12.), control.center().y),
        Modifiers::default(),
    );
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.simulate_keystrokes("down down enter");
    assert!(
        wait_until(window, |window| {
            window.read(|app| view.read(app).default_editor.as_deref() == Some("ok"))
        }),
        "a successful launch becomes the last-used editor"
    );
    assert_eq!(
        window.read(|app| view
            .read(app)
            .workspace_editors
            .get(&workspace_root)
            .cloned()),
        Some("ok".to_owned()),
        "and the project's own choice"
    );
}
