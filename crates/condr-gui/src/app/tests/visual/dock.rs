use super::*;
use condr_core::PaneLayout;
use gpui_kit::{Pixels, Point};

/// Where the window looks: connection, Workspace, Tab.
fn presented(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
) -> Option<(u64, WorkspaceId, TabId)> {
    window.read(|app| {
        view.read(app)
            .presented()
            .map(|(key, _, workspace_id, tab_id)| (key, workspace_id, tab_id))
    })
}

fn create_workspace(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
    root: &TestDirectory,
) -> (WorkspaceId, TabId, PaneId) {
    let before = window.read(|app| {
        view.read(app)
            .active_session()
            .map_or(0, |session| session.workspaces().len())
    });
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: root.0.clone(),
            });
        });
    });
    let mut created = None;
    assert!(wait_until(window, |window| {
        created = window.read(|app| {
            let condr = view.read(app);
            let session = condr.active_session()?;
            let workspace = session.workspaces().get(before)?;
            let tab = workspace.tabs().first()?;
            let surface = DockSurfaceKey {
                connection_key: 1,
                tab_id: tab.id(),
            };
            // The Client that asked for the Workspace shows it (ADR 0021).
            (condr.active_dock_surface == Some(surface)
                && condr.dock_surfaces.contains_key(&surface))
            .then(|| (workspace.id(), tab.id(), tab.focused_pane().unwrap().id()))
        });
        created.is_some()
    }));
    created.unwrap()
}

#[test]
fn two_gui_clients_keep_independent_views_until_explicit_activation() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("two-views-first");
    let second_root = TestDirectory::new("two-views-second");
    let mut first_cx = TestAppContext::single();
    first_cx.update(gpui_kit::init);
    let (server, endpoint) = start_server();
    let (first, first_window, _server) =
        connected_condr_with(&mut first_cx, server, endpoint.clone());
    let (first_workspace, first_tab, _) = create_workspace(first_window, &first, &first_root);
    let (second_workspace, second_tab, _) = create_workspace(first_window, &first, &second_root);

    let mut second_cx = TestAppContext::single();
    second_cx.update(gpui_kit::init);
    let (second, second_window) = connected_condr_at(&mut second_cx, endpoint.clone());
    assert!(wait_until(second_window, |window| {
        window.read(|app| {
            let connection = second.read(app).connection(1).unwrap();
            connection.is_synchronized() && connection.control_denied.is_some()
        })
    }));
    assert_eq!(
        presented(second_window, &second),
        Some((1, first_workspace, first_tab))
    );

    // A creation moves its requester, while the other GUI only learns the structure.
    first_window.update(|_, cx| {
        first.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateTab {
                workspace_id: second_workspace,
                name: None,
                cwd_from: None,
            });
        });
    });
    let mut created_tab = None;
    assert!(wait_until(first_window, |window| {
        created_tab = window.read(|app| {
            let session = first.read(app).active_session()?;
            Some(session.workspace(second_workspace)?.tabs().get(1)?.id())
        });
        created_tab
            .is_some_and(|tab_id| presented(window, &first) == Some((1, second_workspace, tab_id)))
    }));
    let created_tab = created_tab.unwrap();
    assert!(wait_until(second_window, |window| {
        window.read(|app| {
            second
                .read(app)
                .active_session()
                .is_some_and(|session| session.tab(created_tab).is_some())
        })
    }));
    assert_eq!(
        presented(second_window, &second),
        Some((1, first_workspace, first_tab))
    );

    let first_requests = first_window.read(|app| {
        first
            .read(app)
            .connection(1)
            .unwrap()
            .next_layout_request_id
    });
    let second_requests = second_window.read(|app| {
        second
            .read(app)
            .connection(1)
            .unwrap()
            .next_layout_request_id
    });
    second_window.update(|window, cx| {
        second.update(cx, |this, cx| {
            this.select_workspace(1, second_workspace, window, cx);
        });
    });
    assert_eq!(
        presented(second_window, &second),
        Some((1, second_workspace, second_tab))
    );
    assert_eq!(
        presented(first_window, &first),
        Some((1, second_workspace, created_tab))
    );

    // Both controller and viewer can browse different Tabs without sending commands.
    first_window.update(|window, cx| {
        first.update(cx, |this, cx| {
            this.activate_tab_on(1, second_tab, window, cx)
        });
    });
    second_window.update(|window, cx| {
        second.update(cx, |this, cx| {
            this.activate_tab_on(1, created_tab, window, cx)
        });
    });
    assert_eq!(
        first_window.read(|app| first
            .read(app)
            .connection(1)
            .unwrap()
            .next_layout_request_id),
        first_requests
    );
    assert_eq!(
        second_window.read(|app| second
            .read(app)
            .connection(1)
            .unwrap()
            .next_layout_request_id),
        second_requests
    );

    // Wait for both clients to apply the same reliable structural event, then check
    // that neither client's Tab choice was overwritten by it or the other GUI.
    first_window.update(|_, cx| {
        first.update(cx, |this, _| {
            this.send_layout(LayoutCommand::RenameWorkspace {
                workspace_id: second_workspace,
                name: "shared structure".into(),
            });
        });
    });
    for (view, window) in [(&first, &mut *first_window), (&second, &mut *second_window)] {
        assert!(wait_until(window, |window| {
            window.read(|app| {
                view.read(app).active_session().is_some_and(|session| {
                    session.workspace(second_workspace).unwrap().name() == "shared structure"
                })
            })
        }));
    }
    assert_eq!(
        presented(first_window, &first),
        Some((1, second_workspace, second_tab))
    );
    assert_eq!(
        presented(second_window, &second),
        Some((1, second_workspace, created_tab))
    );

    first_window.update(|window, cx| {
        first.update(cx, |this, cx| {
            this.select_workspace(1, first_workspace, window, cx)
        });
    });
    second_window.update(|_, cx| {
        second.update(cx, |this, _| {
            assert!(this.connection_mut(1).unwrap().request_snapshot());
        });
    });
    assert!(wait_until(second_window, |window| {
        window.read(|app| second.read(app).connection(1).unwrap().is_synchronized())
    }));
    assert_eq!(
        presented(first_window, &first),
        Some((1, first_workspace, first_tab))
    );
    assert_eq!(
        presented(second_window, &second),
        Some((1, second_workspace, created_tab))
    );

    // Only an explicit activation asks both viewers to show the same target.
    let mut cli = ClientConnection::connect_overview(&endpoint, "activate-both").unwrap();
    cli.layout(LayoutCommand::ActivateTab { tab_id: second_tab })
        .unwrap()
        .unwrap();
    for (view, window) in [(&first, &mut *first_window), (&second, &mut *second_window)] {
        assert!(wait_until(window, |window| {
            presented(window, view) == Some((1, second_workspace, second_tab))
        }));
    }
}

#[test]
fn the_view_is_local_follows_activation_and_survives_a_closed_tab() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("view-first");
    let second_root = TestDirectory::new("view-second");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (server, endpoint) = start_server();
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint.clone());

    let (first_workspace, first_tab, first_pane) = create_workspace(window, &view, &first_root);
    let (second_workspace, second_tab, _) = create_workspace(window, &view, &second_root);
    assert_eq!(
        presented(window, &view),
        Some((1, second_workspace, second_tab))
    );

    // Selecting a Workspace or Tab is this Client's own business: no Layout command
    // leaves, and going back to a Tab whose Dock exists replaces nothing.
    let request_id = |window: &mut VisualTestContext| {
        window.read(|app| view.read(app).connection(1).unwrap().next_layout_request_id)
    };
    let rebuilds =
        |window: &mut VisualTestContext| window.read(|app| view.read(app).dock_rebuild_count);
    let (requests_before, rebuilds_before) = (request_id(window), rebuilds(window));
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_workspace(1, first_workspace, window, cx)
        });
    });
    window.run_until_parked();
    assert_eq!(
        presented(window, &view),
        Some((1, first_workspace, first_tab))
    );
    assert_eq!(
        window.read(|app| view.read(app).active_dock_surface),
        Some(DockSurfaceKey {
            connection_key: 1,
            tab_id: first_tab,
        })
    );
    assert_eq!(
        request_id(window),
        requests_before,
        "a view change is not a Layout command"
    );
    assert_eq!(
        rebuilds(window),
        rebuilds_before,
        "a cached Dock is shown, not rebuilt"
    );
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_workspace(1, first_workspace, window, cx);
            this.select_server(1, window, cx);
        });
    });
    window.run_until_parked();
    assert_eq!(
        rebuilds(window),
        rebuilds_before,
        "reselecting the shown Workspace does nothing"
    );
    assert_eq!(
        Session::restore(_server.handle.snapshot())
            .unwrap()
            .workspaces()
            .len(),
        2,
        "the Server structure is untouched by browsing"
    );

    // Pane focus inside a Tab is Session structure: selecting the other Pane tells the
    // Server, and the target follows at once.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                focus: true,
                pane_id: first_pane,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    let mut split_pane = None;
    assert!(wait_until(window, |window| {
        split_pane = window.read(|app| {
            let session = view.read(app).active_session()?;
            let tab = session.tab(first_tab)?;
            (tab.panes().len() == 2).then(|| tab.focused_pane().unwrap().id())
        });
        split_pane.is_some()
    }));
    let split_pane = split_pane.unwrap();
    assert_ne!(split_pane, first_pane);
    let requests_before = request_id(window);
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            assert!(this.select_pane(1, first_pane, window, cx))
        });
    });
    assert_eq!(
        window.read(|app| view.read(app).target_pane),
        Some((1, first_pane))
    );
    assert_eq!(
        request_id(window),
        requests_before + 1,
        "Pane focus goes to the Server"
    );
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session.tab(first_tab).unwrap().focused_pane().unwrap().id() == first_pane
            })
        })
    }));

    // Another client (the CLI, an agent) asking every viewer to show a Tab moves this
    // view too; the Session it reads stays the same.
    let sequence_before = window.read(|app| view.read(app).connection(1).unwrap().sequence);
    let mut cli = ClientConnection::connect_overview(&endpoint, "cli").unwrap();
    cli.layout(LayoutCommand::ActivateTab { tab_id: second_tab })
        .unwrap()
        .unwrap();
    assert!(wait_until(window, |window| {
        presented(window, &view) == Some((1, second_workspace, second_tab))
    }));
    assert!(window.read(|app| view.read(app).connection(1).unwrap().sequence) > sequence_before);
    cli.layout(LayoutCommand::ActivateWorkspace {
        workspace_id: first_workspace,
    })
    .unwrap()
    .unwrap();
    assert!(wait_until(window, |window| {
        presented(window, &view) == Some((1, first_workspace, first_tab))
    }));

    // A second Tab in the shown Workspace: created here, so shown here. Closing it moves
    // the view to the Tab that takes its place, as a browser does.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateTab {
                workspace_id: first_workspace,
                name: None,
                cwd_from: Some(first_pane),
            });
        });
    });
    let mut new_tab = None;
    assert!(wait_until(window, |window| {
        new_tab = window.read(|app| {
            let session = view.read(app).active_session()?;
            let workspace = session.workspace(first_workspace)?;
            (workspace.tabs().len() == 2).then(|| workspace.tabs()[1].id())
        });
        new_tab.is_some()
            && presented(window, &view) == Some((1, first_workspace, new_tab.unwrap()))
    }));
    let new_tab = new_tab.unwrap();
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.send_layout(LayoutCommand::MoveTab {
                tab_id: new_tab,
                target_index: 0,
            });
            this.activate_tab_on(1, new_tab, window, cx);
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session.workspace(first_workspace).unwrap().tabs()[0].id() == new_tab
            })
        })
    }));
    assert_eq!(
        presented(window, &view),
        Some((1, first_workspace, new_tab))
    );
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CloseTab { tab_id: new_tab });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.tab(new_tab).is_none())
        })
    }));
    assert_eq!(
        presented(window, &view),
        Some((1, first_workspace, first_tab)),
        "the Tab that took the closed one's place is shown"
    );

    // Closing the shown Workspace moves to its neighbour the same way.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CloseWorkspace {
                workspace_id: first_workspace,
            });
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspace(first_workspace).is_none())
        })
    }));
    assert_eq!(
        presented(window, &view),
        Some((1, second_workspace, second_tab))
    );
}

/// Dragging a Pane header onto another Pane rearranges the Tab through the Server: an
/// edge splits the target and puts the dragged Pane on that side, the centre swaps them.
#[test]
fn dragging_a_pane_header_moves_it_beside_or_swaps_it_with_the_target() {
    let _serial_guard = acquire_visual_test_lock();
    let root = TestDirectory::new("pane-drag");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (_, tab_id, first) = create_workspace(window, &view, &root);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                focus: true,
                pane_id: first,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    let layout = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .and_then(|session| session.tab(tab_id)?.layout().cloned())
        })
    };
    let mut second = None;
    assert!(wait_until(window, |window| {
        second = match layout(window) {
            Some(PaneLayout::Split { second, .. }) => match *second {
                PaneLayout::Pane(id) if id != first => Some(id),
                _ => None,
            },
            _ => None,
        };
        second.is_some()
    }));
    let second = second.unwrap();
    // H(first, second): the first Pane on the left, the new one on the right.
    let drag = |window: &mut VisualTestContext, pane: PaneId, to: Point<Pixels>| {
        window.update(|window, cx| _ = window.draw(cx));
        let header = window
            .debug_bounds(leaked_selector(format!(
                "terminal-pane-header-{}",
                pane.as_u64()
            )))
            .expect("the Pane header renders");
        window.simulate_mouse_down(header.center(), MouseButton::Left, Modifiers::default());
        window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        window.update(|window, cx| _ = window.draw(cx));
        window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    };
    let body = |window: &mut VisualTestContext, pane: PaneId| {
        window
            .debug_bounds(leaked_selector(format!("terminal-pane-{}", pane.as_u64())))
            .expect("the Pane body renders")
    };

    // The first Pane dropped below the second: V(second, first) replaces the split.
    // While it hovers there, the Kit's placeholder covers the target's lower half.
    let target = body(window, second);
    let below = point(target.center().x, target.bottom() - px(10.));
    let header = window
        .debug_bounds(leaked_selector(format!(
            "terminal-pane-header-{}",
            first.as_u64()
        )))
        .unwrap();
    window.simulate_mouse_down(header.center(), MouseButton::Left, Modifiers::default());
    // The first move starts the drag; the next one reaches the target.
    window.simulate_mouse_move(below, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.simulate_mouse_move(below, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    let placeholder = window
        .debug_bounds("condr-drop-placeholder")
        .expect("hovering a drop zone shows the placeholder");
    // The group frame also holds the header and border, so compare loosely: the block
    // starts around the Pane's middle and reaches its bottom edge.
    assert!(
        placeholder.top() > target.top()
            && placeholder.top() < target.center().y
            && placeholder.bottom() >= target.bottom() - px(4.),
        "the placeholder covers the hovered Pane's lower half: {placeholder:?} vs {target:?}"
    );
    window.simulate_mouse_move(below, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(below, MouseButton::Left, Modifiers::default());
    assert!(
        wait_until(window, |window| {
            layout(window)
                == Some(PaneLayout::Split {
                    direction: SplitDirection::Vertical,
                    ratio: 0.5,
                    first: Box::new(PaneLayout::Pane(second)),
                    second: Box::new(PaneLayout::Pane(first)),
                })
        }),
        "an edge drop moves the Pane to that side: {:?}",
        layout(window)
    );
    assert_eq!(
        window.read(|app| view.read(app).target_pane),
        Some((1, second)),
        "dragging a Pane away does not target it"
    );

    // Dropped on the centre, the two Panes trade places and the split stays.
    let target = body(window, second);
    drag(window, first, target.center());
    assert!(
        wait_until(window, |window| {
            layout(window)
                == Some(PaneLayout::Split {
                    direction: SplitDirection::Vertical,
                    ratio: 0.5,
                    first: Box::new(PaneLayout::Pane(first)),
                    second: Box::new(PaneLayout::Pane(second)),
                })
        }),
        "a centre drop swaps the Panes: {:?}",
        layout(window)
    );

    // Dropping a Pane on itself is not a rearrangement.
    let before = layout(window);
    let requests = window.read(|app| view.read(app).connection(1).unwrap().next_layout_request_id);
    let target = body(window, first);
    drag(
        window,
        first,
        point(target.left() + px(10.), target.center().y),
    );
    window.run_until_parked();
    assert_eq!(layout(window), before);
    assert_eq!(
        window.read(|app| view.read(app).connection(1).unwrap().next_layout_request_id),
        requests,
        "no Layout command leaves for a self drop"
    );
}

/// Clicking a Pane's header targets that Pane, as clicking its terminal does.
#[test]
fn clicking_a_pane_header_targets_the_pane() {
    let _serial_guard = acquire_visual_test_lock();
    let root = TestDirectory::new("pane-header-click");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (_, tab_id, first) = create_workspace(window, &view, &root);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                focus: true,
                pane_id: first,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    let mut second = None;
    assert!(wait_until(window, |window| {
        second = window.read(|app| {
            let session = view.read(app).active_session()?;
            let tab = session.tab(tab_id)?;
            (tab.panes().len() == 2).then(|| tab.focused_pane().unwrap().id())
        });
        second.is_some()
    }));
    let second = second.unwrap();
    assert!(wait_until(window, |window| {
        window.read(|app| view.read(app).target_pane) == Some((1, second))
    }));

    window.update(|window, cx| _ = window.draw(cx));
    let header = window
        .debug_bounds(leaked_selector(format!(
            "terminal-pane-header-{}",
            first.as_u64()
        )))
        .unwrap();
    window.simulate_click(header.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.read(|app| view.read(app).target_pane) == Some((1, first))
        }),
        "a header click targets its Pane"
    );
}

/// H(V(a, b), c) with b dropped left of c becomes H(a, H(b, c)); every Pane stays inside
/// the Dock and the Server's ratios stay where the move put them.
#[test]
fn moving_a_pane_into_a_nested_split_keeps_every_pane_on_screen() {
    let _serial_guard = acquire_visual_test_lock();
    let root = TestDirectory::new("pane-move-nested");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let (_, tab_id, a) = create_workspace(window, &view, &root);
    let layout = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .and_then(|session| session.tab(tab_id)?.layout().cloned())
        })
    };
    let split = |window: &mut VisualTestContext, pane: PaneId, direction: SplitDirection| {
        window.update(|_, cx| {
            view.update(cx, |this, _| {
                this.send_layout(LayoutCommand::SplitPane {
                    focus: true,
                    pane_id: pane,
                    direction,
                });
            });
        });
        let known: Vec<PaneId> = window.read(|app| {
            let session = view.read(app).active_session().unwrap();
            let tab = session.tab(tab_id).unwrap();
            tab.panes().iter().map(|pane| pane.id()).collect()
        });
        let mut created = None;
        assert!(wait_until(window, |window| {
            created = window.read(|app| {
                let session = view.read(app).active_session()?;
                let tab = session.tab(tab_id)?;
                tab.panes()
                    .iter()
                    .map(|pane| pane.id())
                    .find(|id| !known.contains(id))
            });
            created.is_some()
        }));
        created.unwrap()
    };
    let c = split(window, a, SplitDirection::Horizontal);
    let b = split(window, a, SplitDirection::Vertical);
    assert_eq!(
        layout(window),
        Some(PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.5,
            first: Box::new(PaneLayout::Split {
                direction: SplitDirection::Vertical,
                ratio: 0.5,
                first: Box::new(PaneLayout::Pane(a)),
                second: Box::new(PaneLayout::Pane(b)),
            }),
            second: Box::new(PaneLayout::Pane(c)),
        })
    );

    window.update(|window, cx| _ = window.draw(cx));
    let bounds = |window: &mut VisualTestContext, selector: String| {
        window.debug_bounds(leaked_selector(selector)).unwrap()
    };
    let header = bounds(window, format!("terminal-pane-header-{}", b.as_u64()));
    let target = bounds(window, format!("terminal-pane-{}", c.as_u64()));
    let left = point(target.left() + px(10.), target.center().y);
    window.simulate_mouse_down(header.center(), MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(left, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    window.simulate_mouse_move(left, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(left, MouseButton::Left, Modifiers::default());
    let expected = PaneLayout::Split {
        direction: SplitDirection::Horizontal,
        ratio: 0.5,
        first: Box::new(PaneLayout::Pane(a)),
        second: Box::new(PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.5,
            first: Box::new(PaneLayout::Pane(b)),
            second: Box::new(PaneLayout::Pane(c)),
        }),
    };
    assert!(
        wait_until(window, |window| layout(window) == Some(expected.clone())),
        "{:?}",
        layout(window)
    );
    // Let any Dock feedback settle, then check nothing rewrote the ratios.
    for _ in 0..5 {
        window.run_until_parked();
        window.update(|window, cx| _ = window.draw(cx));
    }
    assert_eq!(
        layout(window),
        Some(expected),
        "the Dock must not feed ratios back"
    );

    let viewport = window.update(|window, _| window.viewport_size());
    for pane in [a, b, c] {
        let body = bounds(window, format!("terminal-pane-{}", pane.as_u64()));
        assert!(
            body.right() <= viewport.width + px(1.) && body.size.width > px(50.),
            "Pane {} at {body:?} must stay inside the window {viewport:?}",
            pane.as_u64()
        );
    }
    let a_bounds = bounds(window, format!("terminal-pane-{}", a.as_u64()));
    let b_bounds = bounds(window, format!("terminal-pane-{}", b.as_u64()));
    assert!(
        (a_bounds.size.width - b_bounds.size.width * 2.).abs() < px(12.),
        "a takes half, b a quarter: {a_bounds:?} vs {b_bounds:?}"
    );
}
