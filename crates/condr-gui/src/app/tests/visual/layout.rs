use super::*;

#[test]
fn default_window_options_create_1280_by_720_window() {
    let _serial_guard = acquire_visual_test_lock();
    let app = TestAppContext::single();
    let handle = app.update(|cx| {
        cx.open_window(default_window_options(cx), |_, cx| cx.new(|_| gpui::Empty))
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
    let first_pane = workspace.active_tab().focused_pane().id();
    let second_pane = session
        .split_pane(first_pane, SplitDirection::Horizontal, 0.3)
        .expect("test Pane capacity");
    let snapshot_path = snapshot_root.0.join("session.bin");
    std::fs::write(&snapshot_path, session.snapshot().to_bytes().unwrap()).unwrap();

    let endpoint = Endpoint::local(snapshot_root.0.join("server.sock"));
    let server = start_server_with_config(
        ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path),
    );
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
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
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
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
                tab.focused_pane().id(),
                surface_key,
            ))
        });
        initial.is_some()
    }));
    let (tab_id, pane_id, surface_key) = initial.unwrap();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
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
                    && condr.dock_surfaces[&surface_key].projection.as_ref() == Some(tab.layout())
            })
        })
    }));
    window.update(|window, cx| _ = window.draw(cx));
    window.run_until_parked();

    let (sequence, rebuilds) = window.read(|app| {
        let condr = view.read(app);
        (
            condr.connection(1).unwrap().sequence,
            condr.dock_rebuild_count,
        )
    });
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let mut local = Session::restore(this.connection(1).unwrap().snapshot.clone()).unwrap();
            assert!(local.set_tab_split_ratios(tab_id, &[0.72]));
            let local_layout = local.tab(tab_id).unwrap().layout().clone();
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

    let (authoritative_layout, panes) = window.read(|app| {
        let condr = view.read(app);
        let session = condr.active_session().unwrap();
        let tab = session.tab(tab_id).unwrap();
        (
            tab.layout().clone(),
            tab.panes().iter().map(|pane| pane.id()).collect::<Vec<_>>(),
        )
    });
    assert_eq!(
        window.read(|app| view.read(app).connection(1).unwrap().sequence),
        sequence,
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
    cx.update(gpui_component::init);
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
        .expect("Add Server should render in the sidebar header");

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            });
        });
    });

    let mut tree_ids = None;
    assert!(wait_until(window, |window| {
        tree_ids = window.read(|app| {
            let session = view.read(app).active_session()?;
            let workspace = session.active_workspace()?;
            Some((workspace.id(), workspace.active_tab().focused_pane().id()))
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
    assert!(empty_workspace_toggle.right() <= empty_workspace_icon.left());
    window.simulate_click(empty_workspace_toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection_mut(1).unwrap();
            connection.agents.insert(
                pane_id,
                AgentSnapshot {
                    kind: AgentKind::Codex,
                    state: AgentState::Idle,
                },
            );
            connection
                .agent_trackers
                .insert(pane_id, AgentTracker::new(AgentState::Idle));
            cx.notify();
        });
    });
    window.update(|window, cx| _ = window.draw(cx));
    let agent_selector = leaked_selector(format!("agent-1-{}", pane_id.as_u64()));
    assert!(
        window.debug_bounds(agent_selector).is_none(),
        "a Workspace collapsed while empty should stay collapsed when its first Agent appears"
    );
    let workspace_toggle = window.debug_bounds(workspace_toggle_selector).unwrap();
    window.simulate_click(workspace_toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));

    let workspace_toggle = window.debug_bounds(workspace_toggle_selector).unwrap();
    let workspace_icon = window.debug_bounds(workspace_icon_selector).unwrap();
    let workspace_label = window.debug_bounds(workspace_label_selector).unwrap();
    assert!(workspace_toggle.right() <= workspace_icon.left());
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

    let workspace_toggle = window.debug_bounds(workspace_toggle_selector).unwrap();
    window.simulate_click(workspace_toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds(agent_selector).is_some());
}

#[test]
fn server_workspace_button_and_only_tab_close_round_trip() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);

    let add_server = window
        .debug_bounds("add-server")
        .expect("Add Server button should be rendered");
    window.simulate_click(add_server.center(), Modifiers::default());
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
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
    assert!(
        window.debug_bounds("close-tab").is_none(),
        "Tab row should not render a close button"
    );

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
    window.update(|window, cx| {
        window.close_dialog(cx);
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CloseTab { tab_id });
        });
    });

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
fn cached_dock_navigation_avoids_visible_rebuilds_and_background_layout() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("cached-dock-first");
    let second_root = TestDirectory::new("cached-dock-second");
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: first_root.0.clone(),
            });
        });
    });
    let mut first = None;
    assert!(wait_until(window, |window| {
        first = window.read(|app| {
            let condr = view.read(app);
            let session = condr.active_session()?;
            let workspace = session.active_workspace()?;
            let tab = workspace.active_tab();
            let surface = DockSurfaceKey {
                connection_key: 1,
                tab_id: tab.id(),
            };
            (condr.active_dock_surface == Some(surface)
                && condr.dock_surfaces.contains_key(&surface))
            .then(|| (workspace.id(), tab.id(), tab.focused_pane().id(), surface))
        });
        first.is_some()
    }));
    let (first_workspace, first_tab, first_pane, first_surface) = first.unwrap();

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                pane_id: first_pane,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            let Some(session) = condr.active_session() else {
                return false;
            };
            session.tab(first_tab).is_some_and(|tab| {
                tab.panes().len() == 2
                    && condr
                        .dock_surfaces
                        .get(&first_surface)
                        .and_then(|surface| surface.projection.as_ref())
                        == Some(tab.layout())
            })
        })
    }));

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: second_root.0.clone(),
            });
        });
    });
    let mut second = None;
    assert!(wait_until(window, |window| {
        second = window.read(|app| {
            let condr = view.read(app);
            let session = condr.active_session()?;
            let workspace = session.active_workspace()?;
            (workspace.id() != first_workspace).then(|| {
                let tab = workspace.active_tab();
                let surface = DockSurfaceKey {
                    connection_key: 1,
                    tab_id: tab.id(),
                };
                (workspace.id(), tab.id(), tab.focused_pane().id(), surface)
            })
        });
        second.is_some_and(|(_, _, _, surface)| {
            window.read(|app| view.read(app).dock_surfaces.contains_key(&surface))
        })
    }));
    let (second_workspace, second_tab, second_pane, second_surface) = second.unwrap();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                pane_id: second_pane,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SetSplitRatios {
                tab_id: second_tab,
                ratios: vec![0.35],
            });
        });
    });
    let mut second_panes = Vec::new();
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            let Some(session) = condr.active_session() else {
                return false;
            };
            let Some(tab) = session.tab(second_tab) else {
                return false;
            };
            second_panes = tab.panes().iter().map(|pane| pane.id()).collect();
            second_panes.len() == 2
                && matches!(
                    tab.layout(),
                    PaneLayout::Split { ratio, .. } if (*ratio - 0.35).abs() < 0.001
                )
                && condr
                    .dock_surfaces
                    .get(&second_surface)
                    .and_then(|surface| surface.projection.as_ref())
                    == Some(tab.layout())
        })
    }));

    let (focused_before, same_surface_target) = window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let focused_before = this
                .active_session()
                .unwrap()
                .tab(second_tab)
                .unwrap()
                .focused_pane()
                .id();
            let same_surface_target = *second_panes
                .iter()
                .find(|pane_id| **pane_id != focused_before)
                .unwrap();
            assert!(this.select_pane(1, same_surface_target, window, cx));
            let first_request = this.pending_workspace_selection_for(1).unwrap().request_id;
            assert_eq!(this.target_pane, Some((1, same_surface_target)));
            assert_eq!(this.active_dock_surface, Some(second_surface));

            assert!(this.select_pane(1, focused_before, window, cx));
            let corrective = this.pending_workspace_selection_for(1).unwrap();
            assert_eq!(corrective.pane_id, Some(focused_before));
            assert_ne!(corrective.request_id, first_request);
            assert_eq!(this.target_pane, Some((1, focused_before)));
            (focused_before, same_surface_target)
        })
    });
    window.run_until_parked();
    let focused_handle = window.read(|app| {
        view.read(app).panels[&(1, focused_before)]
            .read(app)
            .focus_handle
            .clone()
    });
    assert!(window.update(|window, _| focused_handle.is_focused(window)));
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            condr.pending_workspace_selection_for(1).is_none()
                && condr.active_session().is_some_and(|session| {
                    session
                        .tab(second_tab)
                        .is_some_and(|tab| tab.focused_pane().id() == focused_before)
                })
        })
    }));
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            assert!(this.select_pane(1, same_surface_target, window, cx));
            assert_eq!(this.target_pane, Some((1, same_surface_target)));
        });
    });
    window.run_until_parked();
    let target_handle = window.read(|app| {
        view.read(app).panels[&(1, same_surface_target)]
            .read(app)
            .focus_handle
            .clone()
    });
    assert!(window.update(|window, _| target_handle.is_focused(window)));
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            condr.pending_workspace_selection_for(1).is_none()
                && condr.target_pane == Some((1, same_surface_target))
                && condr.active_session().is_some_and(|session| {
                    session
                        .tab(second_tab)
                        .is_some_and(|tab| tab.focused_pane().id() == same_surface_target)
                })
        })
    }));

    let rebuilds = window.read(|app| view.read(app).dock_rebuild_count);
    let sequence = window.read(|app| view.read(app).connection(1).unwrap().sequence);
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_workspace(1, second_workspace, window, cx);
        });
    });
    window.run_until_parked();
    assert!(window.read(|app| { view.read(app).pending_workspace_selection_for(1).is_none() }));
    assert_eq!(
        window.read(|app| view.read(app).connection(1).unwrap().sequence),
        sequence,
        "reselecting the visible Workspace must not send a Layout command"
    );
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds,
        "reselecting the visible Workspace must not replace the Dock tree"
    );
    window.update(|window, cx| {
        view.update(cx, |this, cx| this.select_server(1, window, cx));
    });
    window.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds,
        "reselecting the active Server must not replace the Dock tree"
    );

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_workspace(1, first_workspace, window, cx);
            assert_eq!(
                this.pending_workspace_selection_for(1)
                    .map(|pending| pending.workspace_id),
                Some(first_workspace)
            );
            let visible_target = this.target_pane;
            this.select_pane(1, first_pane, window, cx);
            assert_eq!(this.target_pane, visible_target);
            assert!(
                this.send_layout_to(1, LayoutCommand::ActivateTab { tab_id: second_tab })
                    .is_none(),
                "other Layout commands must not supersede Workspace navigation"
            );
            this.select_workspace(1, second_workspace, window, cx);
            assert_eq!(
                this.pending_workspace_selection_for(1)
                    .map(|pending| pending.workspace_id),
                Some(second_workspace),
                "the last click must replace an in-flight navigation intent"
            );
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            condr.pending_workspace_selection_for(1).is_none()
                && condr
                    .active_session()
                    .and_then(|session| session.active_workspace_id())
                    == Some(second_workspace)
                && condr.active_dock_surface == Some(second_surface)
        })
    }));
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds,
        "a superseded Workspace must never replace the visible Dock"
    );

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let (intermediate, bootstrap, generation, server_id, runtime_epoch, session_id) = {
                let connection = this.connection(1).unwrap();
                let mut intermediate = Session::restore(connection.snapshot.clone()).unwrap();
                assert!(intermediate.activate_workspace(first_workspace));
                (
                    intermediate.clone(),
                    bootstrap_for_session(connection, &intermediate),
                    connection.connect_generation,
                    connection.server_id.unwrap(),
                    connection.runtime_epoch.unwrap(),
                    connection.session_id.unwrap(),
                )
            };
            this.pending_workspace_selections.insert(
                1,
                PendingWorkspaceSelection {
                    connection_key: 1,
                    workspace_id: second_workspace,
                    pane_id: None,
                    connect_generation: generation,
                    server_id,
                    runtime_epoch,
                    session_id,
                    request_id: u64::MAX,
                    applied_sequence: None,
                },
            );
            let effect = this.handle_incoming(1, generation, Incoming::Bootstrap(bootstrap), cx);
            assert!(
                !effect.rebuild,
                "an intermediate Bootstrap must not replace the visible Dock"
            );
            assert_eq!(this.active_dock_surface, Some(second_surface));
            assert_eq!(
                this.presented_workspace_id(1, &intermediate),
                Some(second_workspace)
            );
            if effect.notify {
                cx.notify();
            }
        });
    });
    window.update(|window, cx| _ = window.draw(cx));
    for pane_id in &second_panes {
        assert!(window.debug_bounds(terminal_selector(*pane_id)).is_some());
    }
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds,
        "rendering an intermediate Bootstrap must keep the cached surface intact"
    );

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let (bootstrap, generation, server_id, session_id, sequence) = {
                let connection = this.connection(1).unwrap();
                let mut final_session = Session::restore(connection.snapshot.clone()).unwrap();
                assert!(final_session.activate_workspace(second_workspace));
                (
                    bootstrap_for_session(connection, &final_session),
                    connection.connect_generation,
                    connection.server_id.unwrap(),
                    connection.session_id.unwrap(),
                    connection.sequence,
                )
            };
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::LayoutApplied {
                    server_id,
                    session_id,
                    request_id: u64::MAX,
                    sequence,
                }),
                cx,
            );
            let effect = this.handle_incoming(1, generation, Incoming::Bootstrap(bootstrap), cx);
            assert!(effect.rebuild);
            assert!(this.pending_workspace_selection_for(1).is_none());
            if effect.rebuild {
                this.rebuild_dock(window, cx);
            }
            if effect.notify {
                cx.notify();
            }
        });
    });
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds,
        "confirming the already-visible target must only switch presentation state"
    );

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            const STALE_RATIO_REQUEST: u64 = u64::MAX - 2;
            const LATEST_RATIO_REQUEST: u64 = u64::MAX - 1;
            let (
                generation,
                server_id,
                session_id,
                sequence,
                intermediate_bootstrap,
                final_bootstrap,
                final_projection,
            ) = {
                let connection = this.connection(1).unwrap();
                let mut intermediate = Session::restore(connection.snapshot.clone()).unwrap();
                assert!(intermediate.set_tab_split_ratios(second_tab, &[0.45]));
                let mut final_session = intermediate.clone();
                assert!(final_session.set_tab_split_ratios(second_tab, &[0.35]));
                let final_projection = final_session.tab(second_tab).unwrap().layout().clone();
                (
                    connection.connect_generation,
                    connection.server_id.unwrap(),
                    connection.session_id.unwrap(),
                    connection.sequence,
                    bootstrap_for_session(connection, &intermediate),
                    bootstrap_for_session(connection, &final_session),
                    final_projection,
                )
            };
            let surface = this.dock_surfaces.get_mut(&second_surface).unwrap();
            surface.projection = Some(final_projection.clone());
            surface.pending_projection_request = Some(LATEST_RATIO_REQUEST);
            surface.pending_projection_applied_sequence = None;

            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::LayoutApplied {
                    server_id,
                    session_id,
                    request_id: STALE_RATIO_REQUEST,
                    sequence,
                }),
                cx,
            );
            let effect = this.handle_incoming(
                1,
                generation,
                Incoming::Bootstrap(intermediate_bootstrap),
                cx,
            );
            assert!(effect.rebuild);
            this.rebuild_dock(window, cx);
            let surface = this.dock_surfaces.get(&second_surface).unwrap();
            assert_eq!(
                surface.pending_projection_request,
                Some(LATEST_RATIO_REQUEST)
            );
            assert_eq!(surface.projection.as_ref(), Some(&final_projection));

            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::LayoutApplied {
                    server_id,
                    session_id,
                    request_id: LATEST_RATIO_REQUEST,
                    sequence,
                }),
                cx,
            );
            let effect =
                this.handle_incoming(1, generation, Incoming::Bootstrap(final_bootstrap), cx);
            assert!(effect.rebuild);
            this.rebuild_dock(window, cx);
            let surface = this.dock_surfaces.get(&second_surface).unwrap();
            assert!(surface.pending_projection_request.is_none());
            assert!(surface.pending_projection_applied_sequence.is_none());
            assert_eq!(surface.projection.as_ref(), Some(&final_projection));
        });
    });
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds,
        "stale ratio acknowledgements must not replace the latest Dock projection"
    );

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_pane(1, first_pane, window, cx);
            assert_eq!(
                this.pending_workspace_selection_for(1)
                    .and_then(|pending| pending.pane_id),
                Some(first_pane)
            );
            assert_eq!(this.active_dock_surface, Some(second_surface));
        });
    });
    window.update(|window, cx| _ = window.draw(cx));
    for pane_id in &second_panes {
        assert!(window.debug_bounds(terminal_selector(*pane_id)).is_some());
    }
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            condr.pending_workspace_selection_for(1).is_none()
                && condr.active_dock_surface == Some(first_surface)
                && condr.target_pane == Some((1, first_pane))
        })
    }));
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_pane(1, second_panes[0], window, cx);
            assert_eq!(this.active_dock_surface, Some(first_surface));
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            condr.pending_workspace_selection_for(1).is_none()
                && condr.active_dock_surface == Some(second_surface)
                && condr.target_pane == Some((1, second_panes[0]))
        })
    }));
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds,
        "cross-Workspace Pane navigation must swap cached surfaces without rebuilding"
    );

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let (endpoint, snapshot, terminals) = {
                let connection = this.connection(1).unwrap();
                let mut remote_session = Session::restore(connection.snapshot.clone()).unwrap();
                assert!(remote_session.focus_pane(second_panes[0]));
                (
                    connection.endpoint.clone(),
                    remote_session.snapshot(),
                    connection.terminals.clone(),
                )
            };
            let mut remote = ServerConnection::new(2, "Remote".into(), endpoint);
            remote.status = ConnectionStatus::Connected;
            remote.server_id = Some(ServerId(200));
            remote.runtime_epoch = Some(RuntimeEpoch(201));
            remote.session_id = Some(SessionId(202));
            remote.snapshot = snapshot;
            remote.terminals = terminals;
            remote.subscribed = true;
            remote.controlling = true;
            this.connections.push(remote);

            const A_REQUEST: u64 = u64::MAX - 10;
            const B_REQUEST: u64 = u64::MAX - 11;
            let (a_generation, a_server_id, a_runtime_epoch, a_session_id, sequence) = {
                let connection = this.connection(1).unwrap();
                (
                    connection.connect_generation,
                    connection.server_id.unwrap(),
                    connection.runtime_epoch.unwrap(),
                    connection.session_id.unwrap(),
                    connection.sequence,
                )
            };
            let (b_generation, b_server_id, b_runtime_epoch, b_session_id) = {
                let connection = this.connection(2).unwrap();
                (
                    connection.connect_generation,
                    connection.server_id.unwrap(),
                    connection.runtime_epoch.unwrap(),
                    connection.session_id.unwrap(),
                )
            };
            let mut a_final =
                Session::restore(this.connection(1).unwrap().snapshot.clone()).unwrap();
            assert!(a_final.activate_workspace(first_workspace));
            let a_bootstrap = bootstrap_for_session(this.connection(1).unwrap(), &a_final);
            this.pending_workspace_selections.insert(
                1,
                PendingWorkspaceSelection {
                    connection_key: 1,
                    workspace_id: first_workspace,
                    pane_id: None,
                    connect_generation: a_generation,
                    server_id: a_server_id,
                    runtime_epoch: a_runtime_epoch,
                    session_id: a_session_id,
                    request_id: A_REQUEST,
                    applied_sequence: None,
                },
            );
            this.pending_workspace_selections.insert(
                2,
                PendingWorkspaceSelection {
                    connection_key: 2,
                    workspace_id: first_workspace,
                    pane_id: None,
                    connect_generation: b_generation,
                    server_id: b_server_id,
                    runtime_epoch: b_runtime_epoch,
                    session_id: b_session_id,
                    request_id: B_REQUEST,
                    applied_sequence: None,
                },
            );
            this.pending_presentation_request = Some((2, B_REQUEST));
            this.handle_incoming(
                1,
                a_generation,
                Incoming::Message(ServerMessage::LayoutApplied {
                    server_id: a_server_id,
                    session_id: a_session_id,
                    request_id: A_REQUEST,
                    sequence,
                }),
                cx,
            );
            let a_effect =
                this.handle_incoming(1, a_generation, Incoming::Bootstrap(a_bootstrap), cx);
            assert!(!a_effect.rebuild);
            assert!(!a_effect.rebuild_active);
            assert_eq!(this.active_connection, 1);
            assert_eq!(this.active_dock_surface, Some(second_surface));

            let rejection = this.handle_incoming(
                2,
                b_generation,
                Incoming::Message(ServerMessage::LayoutRejected {
                    server_id: b_server_id,
                    session_id: b_session_id,
                    request_id: B_REQUEST,
                    reason: "synthetic rejection".into(),
                }),
                cx,
            );
            assert!(rejection.rebuild_active);
            this.rebuild_dock(window, cx);
            assert_eq!(this.active_connection, 1);
            assert_eq!(this.active_dock_surface, Some(first_surface));
            assert!(this.pending_workspace_selections.is_empty());
            assert!(this.pending_presentation_request.is_none());

            let mut reset = Session::restore(this.connection(1).unwrap().snapshot.clone()).unwrap();
            assert!(reset.activate_workspace(second_workspace));
            let reset_bootstrap = bootstrap_for_session(this.connection(1).unwrap(), &reset);
            let reset_effect =
                this.handle_incoming(1, a_generation, Incoming::Bootstrap(reset_bootstrap), cx);
            assert!(reset_effect.rebuild);
            this.rebuild_dock(window, cx);
            assert_eq!(this.active_dock_surface, Some(second_surface));

            this.select_pane(2, second_panes[0], window, cx);
            assert_eq!(this.active_connection, 2);
            assert_eq!(
                this.active_dock_surface,
                Some(DockSurfaceKey {
                    connection_key: 2,
                    tab_id: second_tab,
                })
            );
            assert!(this.pending_workspace_selections.is_empty());
        });
    });
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window
            .debug_bounds(terminal_selector(second_panes[0]))
            .is_some(),
        "an already-focused Pane on another Server must render in the same update"
    );
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_server(1, window, cx);
            assert_eq!(this.active_dock_surface, Some(second_surface));

            const REMOVE_REQUEST: u64 = u64::MAX - 12;
            let (remote_generation, remote_server, remote_epoch, remote_session) = {
                let remote = this.connection(2).unwrap();
                (
                    remote.connect_generation,
                    remote.server_id.unwrap(),
                    remote.runtime_epoch.unwrap(),
                    remote.session_id.unwrap(),
                )
            };
            this.pending_workspace_selections.insert(
                2,
                PendingWorkspaceSelection {
                    connection_key: 2,
                    workspace_id: first_workspace,
                    pane_id: None,
                    connect_generation: remote_generation,
                    server_id: remote_server,
                    runtime_epoch: remote_epoch,
                    session_id: remote_session,
                    request_id: REMOVE_REQUEST,
                    applied_sequence: None,
                },
            );
            this.pending_presentation_request = Some((2, REMOVE_REQUEST));
            this.select_workspace(1, second_workspace, window, cx);
            assert!(this.pending_presentation_request.is_none());
            assert!(this.pending_workspace_selection_for(2).is_some());
            assert_eq!(this.active_dock_surface, Some(second_surface));

            this.pending_presentation_request = Some((2, REMOVE_REQUEST));
            let generation = this.connection(1).unwrap().connect_generation;
            let mut changed_authority =
                Session::restore(this.connection(1).unwrap().snapshot.clone()).unwrap();
            assert!(changed_authority.activate_workspace(first_workspace));
            let changed_bootstrap =
                bootstrap_for_session(this.connection(1).unwrap(), &changed_authority);
            let held =
                this.handle_incoming(1, generation, Incoming::Bootstrap(changed_bootstrap), cx);
            assert!(!held.rebuild);
            assert_eq!(this.active_dock_surface, Some(second_surface));

            this.remove_server(2, window, cx);
            assert!(this.connection(2).is_none());
            assert!(this.pending_presentation_request.is_none());
            assert_eq!(this.active_dock_surface, Some(first_surface));

            let mut reset = Session::restore(this.connection(1).unwrap().snapshot.clone()).unwrap();
            assert!(reset.activate_workspace(second_workspace));
            let reset_bootstrap = bootstrap_for_session(this.connection(1).unwrap(), &reset);
            let reset =
                this.handle_incoming(1, generation, Incoming::Bootstrap(reset_bootstrap), cx);
            assert!(reset.rebuild);
            this.rebuild_dock(window, cx);
            assert_eq!(this.active_dock_surface, Some(second_surface));
        });
    });
    let rebuilds = window.read(|app| view.read(app).dock_rebuild_count);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_workspace(1, first_workspace, window, cx)
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            condr
                .active_session()
                .and_then(|session| session.active_workspace_id())
                == Some(first_workspace)
                && condr.active_dock_surface == Some(first_surface)
        })
    }));
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds,
        "returning to a cached Dock at the same size must only switch Entities"
    );

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let (generation, server_id, session_id, mut active_view) = {
                let connection = this.connection(1).unwrap();
                (
                    connection.connect_generation,
                    connection.server_id.unwrap(),
                    connection.session_id.unwrap(),
                    connection.terminals[&first_pane].view.clone(),
                )
            };
            active_view.revision += 1;
            let active = this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::TerminalFrame(TerminalFrameBatch {
                    server_id,
                    session_id,
                    panes: vec![PaneTerminalFrame {
                        pane_id: first_pane,
                        frame: TerminalViewFrame::Full(active_view),
                    }],
                })),
                cx,
            );
            assert!(active.notify, "the mounted Dock must repaint for its Pane");

            let inactive_pane = second_panes[0];
            let mut inactive_view = this.connection(1).unwrap().terminals[&inactive_pane]
                .view
                .clone();
            inactive_view.revision += 1;
            let inactive = this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::TerminalFrame(TerminalFrameBatch {
                    server_id,
                    session_id,
                    panes: vec![PaneTerminalFrame {
                        pane_id: inactive_pane,
                        frame: TerminalViewFrame::Full(inactive_view),
                    }],
                })),
                cx,
            );
            assert!(
                !inactive.notify,
                "an inactive cached Dock must absorb Terminal state without repainting"
            );
        });
    });

    window.update(|window, cx| _ = window.draw(cx));
    let inactive_bounds = window.read(|app| {
        view.read(app).dock_surfaces[&second_surface]
            .area
            .read(app)
            .bounds()
    });
    let inactive_geometry = window.read(|app| {
        let condr = view.read(app);
        second_panes
            .iter()
            .map(|pane_id| {
                condr
                    .terminal_geometry
                    .get(&(1, *pane_id))
                    .map(|geometry| (geometry.bounds, geometry.cell_size))
            })
            .collect::<Vec<_>>()
    });
    assert!(inactive_geometry.iter().all(Option::is_some));
    let active_bounds = window.read(|app| {
        view.read(app).dock_surfaces[&first_surface]
            .area
            .read(app)
            .bounds()
    });
    window.simulate_resize(size(px(1560.), px(860.)));
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).dock_surfaces[&first_surface]
                .area
                .read(app)
                .bounds()
                .size
                .width
                != active_bounds.size.width
        })
    }));
    assert_eq!(
        window.read(|app| {
            view.read(app).dock_surfaces[&second_surface]
                .area
                .read(app)
                .bounds()
        }),
        inactive_bounds,
        "an inactive cached Dock must not participate in layout"
    );
    assert_eq!(
        window.read(|app| {
            let condr = view.read(app);
            second_panes
                .iter()
                .map(|pane_id| {
                    condr
                        .terminal_geometry
                        .get(&(1, *pane_id))
                        .map(|geometry| (geometry.bounds, geometry.cell_size))
                })
                .collect::<Vec<_>>()
        }),
        inactive_geometry,
        "inactive TerminalElements must not prepaint or request a resize"
    );

    let rebuilds_before_resized_switch = window.read(|app| view.read(app).dock_rebuild_count);
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let mut session = Session::restore(this.connection(1).unwrap().snapshot.clone())
                .expect("client Session should restore");
            assert!(session.activate_workspace(second_workspace));
            this.connection_mut(1).unwrap().snapshot = session.snapshot();
            this.refresh_target_pane(1);
            this.rebuild_dock(window, cx);
            cx.notify();
        });
    });
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds_before_resized_switch + 1,
        "a stale inactive Dock should be prepared at the current size before it is shown"
    );

    window.update(|window, cx| _ = window.draw(cx));
    let first_frame = second_panes
        .iter()
        .map(|pane_id| {
            window
                .debug_bounds(terminal_selector(*pane_id))
                .expect("target Pane should render on the first frame")
        })
        .collect::<Vec<_>>();
    let pane_width = first_frame[0].size.width + first_frame[1].size.width;
    assert!(
        (first_frame[0].size.width / pane_width - 0.35).abs() < 0.03,
        "the target Dock must use its authoritative split ratio before it is shown"
    );
    assert!(first_frame[0].right() <= first_frame[1].left());
    assert!(first_frame[1].left() - first_frame[0].right() <= px(8.));
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    let settled_frame = second_panes
        .iter()
        .map(|pane_id| {
            window
                .debug_bounds(terminal_selector(*pane_id))
                .expect("target Pane should remain rendered")
        })
        .collect::<Vec<_>>();
    for (first, settled) in first_frame.iter().zip(&settled_frame) {
        assert!((first.origin.x - settled.origin.x).abs() <= px(1.));
        assert!((first.origin.y - settled.origin.y).abs() <= px(1.));
        assert!((first.size.width - settled.size.width).abs() <= px(1.));
        assert!((first.size.height - settled.size.height).abs() <= px(1.));
    }
    assert_eq!(
        window.read(|app| view.read(app).dock_rebuild_count),
        rebuilds_before_resized_switch + 1,
        "settling the first frame must not replace the Dock a second time"
    );

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CloseWorkspace {
                workspace_id: first_workspace,
            });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            let condr = view.read(app);
            condr
                .active_session()
                .is_some_and(|session| session.workspace(first_workspace).is_none())
                && !condr.dock_surfaces.contains_key(&first_surface)
        })
    }));
}

#[test]
fn the_settings_button_opens_a_separate_window_that_applies_a_theme_mode() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);
    let main_window = window.update(|window, _| window.window_handle());

    let settings_button = window
        .debug_bounds("open-settings")
        .expect("the sidebar should render a Settings button");
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    let settings_handle = window
        .windows()
        .into_iter()
        .find(|handle| *handle != main_window)
        .expect("the Settings button should open a second window");
    let settings = VisualTestContext::from_window(settings_handle, window).into_mut();
    settings.update(|window, cx| _ = window.draw(cx));
    assert!(
        settings.debug_bounds("settings-content").is_some(),
        "the Settings window should render its content"
    );

    // Opening again re-activates the existing window instead of adding another.
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.windows().len(), 2);

    window.update(|window, cx| {
        view.update(cx, |this, cx| this.set_appearance(Appearance::Dark, cx));
        _ = window.draw(cx);
    });
    window.run_until_parked();
    assert!(
        window.update(|_, cx| cx.theme().is_dark()),
        "choosing Dark should switch the theme mode"
    );
    assert_eq!(
        window.windows().len(),
        2,
        "switching the theme must keep the Settings window open"
    );

    settings.update(|window, _| window.remove_window());
    window.run_until_parked();
    assert_eq!(window.windows().len(), 1);

    // Closing the main window takes a reopened Settings window with it.
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.windows().len(), 2);
    window.update(|window, _| window.remove_window());
    window.run_until_parked();
    assert!(
        cx.windows().is_empty(),
        "closing the main window must close Settings too"
    );
}

#[test]
fn the_terminal_settings_controls_drive_the_preferences_and_reset() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);
    let owner = view.downgrade();
    let main_window = window.update(|window, _| window.window_handle());

    let settings_button = window.debug_bounds("open-settings").unwrap();
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    let settings_handle = window
        .windows()
        .into_iter()
        .find(|handle| *handle != main_window)
        .expect("Settings should open in its own window");
    let settings = VisualTestContext::from_window(settings_handle, window).into_mut();
    settings.update(|window, cx| _ = window.draw(cx));
    let settings_view = settings.read(|app| {
        view.read(app)
            .settings_view
            .as_ref()
            .and_then(|view| view.upgrade())
            .expect("the Settings window view is recorded")
    });
    let select_state = settings.read(|app| settings_view.read(app).color_scheme.clone());

    // Colors: the real Select, opened by click, filtered by typing, confirmed by Enter.
    let select = settings
        .debug_bounds("terminal-color-scheme")
        .expect("the Colors select should render");
    settings.simulate_click(select.center(), Modifiers::default());
    settings.run_until_parked();
    settings.simulate_input("Kanagawa Dragon");
    settings.simulate_keystrokes("enter");
    settings.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).terminal_color_scheme.clone()),
        "Kanagawa Dragon",
        "confirming a scheme in the select must store it"
    );
    let expected = crate::color_scheme::palette("Kanagawa Dragon").unwrap();
    assert_eq!(
        window.read(|app| app.global::<TerminalPalette>().background),
        expected.background,
        "the chosen scheme must reach the terminal palette"
    );
    assert!(window.read(|app| color_scheme_is_dirty(&owner, app)));

    // Reset All for Colors: preference and widget both return to Default.
    settings.update(|window, cx| reset_color_scheme(&owner, &select_state, window, cx));
    settings.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).terminal_color_scheme.clone()),
        ""
    );
    assert!(!window.read(|app| color_scheme_is_dirty(&owner, app)));
    assert_eq!(
        window.read(|app| select_state.read(app).selected_value().cloned()),
        Some("Default".into()),
        "the select must show Default after a reset"
    );
    assert_eq!(
        window.read(|app| app.global::<TerminalPalette>().background),
        TerminalPalette::default().background
    );

    // Font: the field keeps what was typed; the theme gets the normalized value.
    window.update(|_, cx| select_terminal_font_family(&settings_view, "Cascadia Mono".into(), cx));
    assert_eq!(
        window.read(|app| terminal_font_family(&settings_view, app)),
        "Cascadia Mono"
    );
    assert_eq!(
        window.read(|app| app.theme().mono_font_family.clone()),
        "Cascadia Mono"
    );
    window.update(|_, cx| select_terminal_font_family(&settings_view, "".into(), cx));
    assert_eq!(
        window.read(|app| terminal_font_family(&settings_view, app)),
        "",
        "a half-edited field must not be rewritten under the user"
    );
    assert_eq!(
        window.read(|app| app.theme().mono_font_family.clone()),
        TerminalFont::default().family,
        "an empty family falls back to the default font"
    );

    // Shell: the Server page edits the selected Server; the Server stores the value and
    // reports it back, so the connection's settings follow the field.
    assert_eq!(
        window.read(|app| settings_view.read(app).selected_server),
        1,
        "the Server tab starts on the active connection"
    );
    window.update(|_, cx| select_settings_server(&settings_view, 1, cx));
    assert_eq!(window.read(|app| server_shell(&settings_view, app)), "");
    window.update(|_, cx| select_server_shell(&settings_view, " nu ".into(), cx));
    assert_eq!(
        window.read(|app| server_shell(&settings_view, app)),
        " nu ",
        "the field keeps what was typed"
    );
    assert!(
        wait_until_event_driven(window, |window| {
            window.read(|app| {
                view.read(app)
                    .connection(1)
                    .is_some_and(|connection| connection.settings.shell == "nu")
            })
        }),
        "the Server must store the trimmed shell and publish it"
    );
    window.update(|_, cx| select_server_shell(&settings_view, "".into(), cx));
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .is_some_and(|connection| connection.settings.shell.is_empty())
        })
    }));

    // Font size: the real stepper buttons, one pixel per click, clamped to the bounds.
    let default_size = f64::from(TerminalFont::default().size);
    settings.update(|window, cx| _ = window.draw(cx));
    let decrease = settings
        .debug_bounds("terminal-font-size-decrease")
        .expect("the Font size stepper should render");
    settings.simulate_click(decrease.center(), Modifiers::default());
    settings.run_until_parked();
    assert_eq!(
        window.read(|app| terminal_font_size(&settings_view, app)),
        default_size - 1.
    );
    assert_eq!(
        window.read(|app| app.theme().mono_font_size),
        px(TerminalFont::default().size - 1.)
    );
    settings.update(|window, cx| _ = window.draw(cx));
    let increase = settings
        .debug_bounds("terminal-font-size-increase")
        .unwrap();
    settings.simulate_click(increase.center(), Modifiers::default());
    settings.simulate_click(increase.center(), Modifiers::default());
    settings.run_until_parked();
    assert_eq!(
        window.read(|app| app.theme().mono_font_size),
        px(TerminalFont::default().size + 1.)
    );
    window.update(|_, cx| step_terminal_font_size(&settings_view, -100., cx));
    assert_eq!(
        window.read(|app| app.theme().mono_font_size),
        px(TerminalFont::MIN_SIZE),
        "stepping never leaves the allowed range"
    );
    window.update(|_, cx| select_terminal_font_size(&settings_view, default_size, cx));
    assert_eq!(
        window.read(|app| app.theme().mono_font_size),
        px(TerminalFont::default().size)
    );

    settings.update(|window, _| window.remove_window());
    window.run_until_parked();
}

#[test]
fn the_sidebar_keeps_its_dragged_width_across_window_resizes() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));

    let initial = window.read(|app| view.read(app).sidebar_width);
    let sidebar = window.debug_bounds("condr-sidebar").unwrap();
    assert_eq!(sidebar.size.width, initial);

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
