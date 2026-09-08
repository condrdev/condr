use super::*;

#[test]
fn cached_dock_navigation_avoids_visible_rebuilds_and_background_layout() {
    let _serial_guard = acquire_visual_test_lock();
    let first_root = TestDirectory::new("cached-dock-first");
    let second_root = TestDirectory::new("cached-dock-second");
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
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
                focus: true,
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
                name: None,
                focus: true,
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
                focus: true,
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
    let next_request_id =
        window.read(|app| view.read(app).connection(1).unwrap().next_layout_request_id);
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.select_workspace(1, second_workspace, window, cx);
        });
    });
    window.run_until_parked();
    assert!(window.read(|app| { view.read(app).pending_workspace_selection_for(1).is_none() }));
    assert_eq!(
        window.read(|app| view.read(app).connection(1).unwrap().next_layout_request_id),
        next_request_id,
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
                    result: Default::default(),
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
                    result: Default::default(),
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
                    result: Default::default(),
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
                    result: Default::default(),
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
                    connection.terminals[&first_pane].view.as_ref().clone(),
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
            assert!(
                !active.notify && !active.rebuild,
                "a frame repaints its Panel entity only, never the root or the Dock"
            );

            let inactive_pane = second_panes[0];
            let mut inactive_view = this.connection(1).unwrap().terminals[&inactive_pane]
                .view
                .as_ref()
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
