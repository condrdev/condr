use super::*;

#[test]
fn stale_connection_result_cannot_replace_the_current_attempt() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection_mut(1).unwrap();
            let original_endpoint = connection.endpoint.clone();
            let original_generation = connection.connect_generation;
            connection.connect_generation = original_generation.wrapping_add(1);
            connection.status = ConnectionStatus::Connecting;

            this.handle_connection_result(
                ConnectionResult {
                    key: 1,
                    generation: original_generation,
                    endpoint: Endpoint::tcp("127.0.0.1:9".parse().unwrap()),
                    result: Err("stale failure".into()),
                },
                window,
                cx,
            );

            let connection = this.connection_mut(1).unwrap();
            assert_eq!(connection.endpoint, original_endpoint);
            assert!(connection.status == ConnectionStatus::Connecting);
            assert!(connection.error.is_none());
            connection.connect_generation = original_generation;
            connection.status = ConnectionStatus::Connected;
        });
    });
}

#[test]
fn reliable_sequence_gap_bootstraps_and_restores_subscription() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);

    let original_sequence = window.read(|app| view.read(app).connection(1).unwrap().sequence);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.connect_generation;
            let server_id = connection.server_id.unwrap();
            let session_id = connection.session_id.unwrap();
            assert!(connection.subscribed);

            for sequence in [
                original_sequence.saturating_add(2),
                original_sequence.saturating_add(3),
            ] {
                this.handle_incoming(
                    1,
                    generation,
                    Incoming::Message(ServerMessage::Event {
                        server_id,
                        session_id,
                        sequence,
                        event: SessionEvent::LayoutChanged,
                    }),
                    cx,
                );
            }

            let connection = this.connection(1).unwrap();
            assert_eq!(
                connection.bootstrap_resync_session_id,
                connection.session_id
            );
            assert!(!connection.subscribed);
            assert!(!connection.can_mutate());
        });
    });

    assert!(
        wait_until_event_driven(window, |window| {
            window.read(|app| {
                view.read(app).connection(1).is_some_and(|connection| {
                    connection.subscribed
                        && !connection.subscription_pending
                        && connection.bootstrap_resync_session_id.is_none()
                        && connection.can_mutate()
                })
            })
        }),
        "GUI did not restore its reliable subscription after a sequence gap"
    );
}

#[test]
fn denied_replacement_connection_retries_after_the_controller_releases() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);
    let endpoint = window.read(|app| {
        view.read(app)
            .connection(1)
            .expect("local connection exists")
            .endpoint
            .clone()
    });

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.disconnect_server(1);
            cx.notify();
        });
    });

    let contender = ClientConnection::connect(&endpoint, "control-contender").unwrap();
    let session_id = contender.bootstrap().session_id;
    let mut contender_stream = contender.into_stream();
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        condr_core::protocol::write_message(
            &mut contender_stream,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        match condr_core::protocol::read_message(&mut contender_stream).unwrap() {
            ServerMessage::ControlGranted { .. } => break,
            ServerMessage::ControlDenied { .. } => {
                assert!(
                    Instant::now() < deadline,
                    "the disconnected GUI never released control"
                );
                std::thread::sleep(TEST_POLL_INTERVAL);
            }
            message => panic!("unexpected control response: {message:?}"),
        }
    }

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.start_connect(1);
            cx.notify();
        });
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).connection(1).is_some_and(|connection| {
                connection.status == ConnectionStatus::Connected
                    && !connection.controlling
                    && connection.error.as_deref() == Some(CONTROL_BUSY_REASON)
            })
        })
    }));

    condr_core::protocol::write_message(
        &mut contender_stream,
        &ClientMessage::ReleaseControl { session_id },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message(&mut contender_stream).unwrap(),
        ServerMessage::ControlReleased { .. }
    ));
    assert!(
        wait_until(window, |window| {
            window.read(|app| {
                view.read(app)
                    .connection(1)
                    .is_some_and(ServerConnection::can_mutate)
            })
        }),
        "replacement connection did not retry control acquisition"
    );
}

#[test]
fn server_disconnect_reconnect_and_remove_preserve_runtime() {
    let _serial_guard = acquire_visual_test_lock();
    let workspace_root = TestDirectory::new("reconnect-cached-dock");
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
    let mut active_surface = None;
    assert!(wait_until_event_driven(window, |window| {
        active_surface = window.read(|app| {
            let condr = view.read(app);
            let session = condr.active_session()?;
            let tab = session.active_workspace()?.active_tab();
            let surface_key = DockSurfaceKey {
                connection_key: 1,
                tab_id: tab.id(),
            };
            condr
                .dock_surfaces
                .contains_key(&surface_key)
                .then_some((surface_key, tab.focused_pane().id()))
        });
        active_surface.is_some()
    }));
    let (surface_key, pane_id) = active_surface.unwrap();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                pane_id,
                direction: SplitDirection::Horizontal,
            });
        });
    });
    let mut pane_ids = Vec::new();
    assert!(wait_until(window, |window| {
        pane_ids = window.read(|app| {
            let condr = view.read(app);
            condr
                .active_session()
                .and_then(|session| session.tab(surface_key.tab_id).cloned())
                .map(|tab| tab.panes().iter().map(|pane| pane.id()).collect())
                .unwrap_or_default()
        });
        pane_ids.len() == 2
    }));
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let mut local = Session::restore(this.connection(1).unwrap().snapshot.clone()).unwrap();
            assert!(local.set_tab_split_ratios(surface_key.tab_id, &[0.72]));
            let local_layout = local.tab(surface_key.tab_id).unwrap().layout().clone();
            let surface = this.dock_surfaces.get(&surface_key).unwrap();
            let available_size = surface.area.read(cx).bounds().size;
            let area = surface.area.clone();
            let dock_layout = this.build_dock_layout(1, &local_layout, available_size, cx);
            let surface = this.dock_surfaces.get_mut(&surface_key).unwrap();
            surface.programmatic_layout_events =
                surface.programmatic_layout_events.saturating_add(1);
            surface.projection = Some(local_layout);
            surface.pending_projection_request = Some(u64::MAX);
            surface.pending_projection_applied_sequence = None;
            area.update(cx, |dock, cx| {
                dock.set_center(dock_layout, window, cx);
            });
        });
    });
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    let optimistic_first = window.debug_bounds(terminal_selector(pane_ids[0])).unwrap();
    let optimistic_second = window.debug_bounds(terminal_selector(pane_ids[1])).unwrap();
    let optimistic_width = optimistic_first.size.width + optimistic_second.size.width;
    assert!((optimistic_first.size.width / optimistic_width - 0.72).abs() < 0.03);

    let identity = window.read(|app| {
        let connection = view.read(app).connection(1).unwrap();
        (
            connection.server_id,
            connection.runtime_epoch,
            connection.session_id,
        )
    });

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let authoritative = Session::restore(this.connection(1).unwrap().snapshot.clone())
                .unwrap()
                .tab(surface_key.tab_id)
                .unwrap()
                .layout()
                .clone();
            let rebuilds = this.dock_rebuild_count;
            this.pending_sizes
                .insert((1, pane_id), condr_core::TerminalSize::new(99, 199));
            assert!(this.disconnect_server(1));
            this.refresh_target_pane(1);
            this.rebuild_dock(window, cx);
            assert!(!this.pending_sizes.contains_key(&(1, pane_id)));
            let surface = this.dock_surfaces.get(&surface_key).unwrap();
            assert!(surface.pending_projection_request.is_none());
            assert!(surface.pending_projection_applied_sequence.is_none());
            assert_eq!(surface.projection.as_ref(), Some(&authoritative));
            assert_eq!(this.dock_rebuild_count, rebuilds + 1);
            cx.notify();
        });
    });
    window.update(|window, cx| _ = window.draw(cx));
    let restored_first = window.debug_bounds(terminal_selector(pane_ids[0])).unwrap();
    let restored_second = window.debug_bounds(terminal_selector(pane_ids[1])).unwrap();
    let restored_width = restored_first.size.width + restored_second.size.width;
    assert!(
        (restored_first.size.width / restored_width - 0.5).abs() < 0.03,
        "disconnect must restore the authoritative split before reconnect"
    );
    assert!(window.read(|app| {
        view.read(app).connection(1).is_some_and(|connection| {
            connection.status == ConnectionStatus::Disconnected && !connection.can_mutate()
        })
    }));

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.start_connect(1);
            let generation = this.connection(1).unwrap().connect_generation;
            this.start_connect(1);
            assert_eq!(
                this.connection(1).unwrap().connect_generation,
                generation,
                "a duplicate reconnect must share the in-flight attempt"
            );
            cx.notify();
        });
    });
    let reconnected = wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .is_some_and(ServerConnection::can_mutate)
        })
    });
    assert!(reconnected, "Server did not reconnect");
    assert_eq!(
        window.read(|app| {
            let connection = view.read(app).connection(1).unwrap();
            (
                connection.server_id,
                connection.runtime_epoch,
                connection.session_id,
            )
        }),
        identity,
        "reconnect should retain the Server runtime"
    );
    assert!(window.read(|app| {
        let condr = view.read(app);
        let session = condr.active_session().unwrap();
        let surface = condr.dock_surfaces.get(&surface_key).unwrap();
        surface.pending_projection_request.is_none()
            && surface.pending_projection_applied_sequence.is_none()
            && surface.projection.as_ref()
                == Some(session.tab(surface_key.tab_id).unwrap().layout())
    }));

    window.update(|window, cx| {
        view.update(cx, |this, cx| this.remove_server(1, window, cx));
    });
    assert!(window.read(|app| view.read(app).connections.is_empty()));
    window.quit();
}

#[test]
fn replacement_server_restores_structure_with_fresh_terminal_state() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("persistent-restart");
    let workspace_root = directory.0.join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let endpoint = Endpoint::local(directory.0.join("server.sock"));
    let snapshot_path = directory.0.join("session.snapshot");
    let server = start_server_with_config(
        ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
    );

    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, mut server) = connected_condr_with(&mut cx, server, endpoint.clone());
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: workspace_root.clone(),
            });
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .active_workspace()
                    .is_some_and(|workspace| workspace.root_directory() == workspace_root.as_path())
            })
        })
    }));

    let (server_id, runtime_epoch, expected_snapshot, pane_id) = window.read(|app| {
        let condr = view.read(app);
        let connection = condr.connection(1).unwrap();
        let pane_id = condr
            .active_session()
            .unwrap()
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        (
            connection.server_id,
            connection.runtime_epoch,
            connection.snapshot.clone(),
            pane_id,
        )
    });
    let old_terminal_marker = "CONDR_PH6_OLD_TERMINAL_STATE";
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.terminal_command(
                1,
                pane_id,
                TerminalCommand::Text(old_terminal_marker.into()),
            );
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .and_then(|connection| connection.terminals.get(&pane_id))
                .is_some_and(|terminal| {
                    terminal
                        .view
                        .cells
                        .iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                        .contains(old_terminal_marker)
                })
        })
    }));

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            connection.send(ClientMessage::StopServer {
                server_id: connection.server_id.unwrap(),
            });
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .is_some_and(|connection| connection.status == ConnectionStatus::Disconnected)
        })
    }));
    server.stop();

    let mut replacement =
        start_server_with_config(ServerConfig::new(endpoint).with_snapshot_path(snapshot_path));
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.start_connect(1);
            cx.notify();
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .is_some_and(ServerConnection::can_mutate)
        })
    }));

    window.read(|app| {
        let condr = view.read(app);
        let connection = condr.connection(1).unwrap();
        assert_eq!(connection.server_id, server_id);
        assert_ne!(connection.runtime_epoch, runtime_epoch);
        assert_eq!(connection.snapshot, expected_snapshot);
        assert_eq!(connection.terminals.len(), 1);
        assert!(connection.agents.is_empty());
        assert!(
            !connection.terminals[&pane_id]
                .view
                .cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
                .contains(old_terminal_marker)
        );
        assert_eq!(
            condr
                .active_session()
                .unwrap()
                .active_workspace()
                .unwrap()
                .root_directory(),
            workspace_root.as_path()
        );
    });
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds(terminal_selector(pane_id)).is_some(),
        "restored Pane should remain visible after reconnecting to the replacement Server"
    );
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            connection.send(ClientMessage::StopServer {
                server_id: connection.server_id.unwrap(),
            });
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .is_some_and(|connection| connection.status == ConnectionStatus::Disconnected)
        })
    }));
    replacement.stop();
    window.quit();
}

#[test]
fn added_server_survives_gui_restart() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("client-config");
    let config_path = directory.0.join("config.toml");
    let (server, endpoint) = start_server();

    {
        let mut cx = TestAppContext::single();
        cx.update(gpui_component::init);
        let initial = ClientConnection::connect(&endpoint, "condr-gui-test").unwrap();
        let bootstrap = initial.bootstrap().clone();
        assert_eq!(bootstrap.server_id, server.handle.server_id());
        let view_holder = Rc::new(RefCell::new(None));
        let view_holder_for_window = view_holder.clone();
        let (_root, window) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                Condr::new(
                    endpoint.clone(),
                    Ok(initial),
                    Some(config_path.clone()),
                    window,
                    cx,
                )
            });
            view_holder_for_window.borrow_mut().replace(view.clone());
            Root::new(view, window, cx)
        });
        let view = view_holder.borrow_mut().take().unwrap();
        window.update(|window, cx| {
            view.update(cx, |this, cx| this.prompt_add_server(window, cx));
        });
        submit_text_dialog(window, "127.0.0.1:4242");
        assert!(window.read(|app| {
            view.read(app).connections.iter().any(|connection| {
                connection.endpoint == Endpoint::tcp("127.0.0.1:4242".parse().unwrap())
            })
        }));
    }

    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let initial = ClientConnection::connect(&endpoint, "condr-gui-test").unwrap();
    let view_holder = Rc::new(RefCell::new(None));
    let view_holder_for_window = view_holder.clone();
    let (_root, window) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Condr::new(endpoint, Ok(initial), Some(config_path), window, cx));
        view_holder_for_window.borrow_mut().replace(view.clone());
        Root::new(view, window, cx)
    });
    let view = view_holder.borrow_mut().take().unwrap();
    assert!(window.read(|app| {
        view.read(app).connections.iter().any(|connection| {
            connection.endpoint == Endpoint::tcp("127.0.0.1:4242".parse().unwrap())
        })
    }));
}

#[test]
fn corrupt_snapshot_connects_to_an_operable_start_page() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("corrupt-snapshot");
    let endpoint = Endpoint::local(directory.0.join("server.sock"));
    let snapshot_path = directory.0.join("session.snapshot");
    std::fs::write(&snapshot_path, b"not a Condr snapshot").unwrap();
    let server = start_server_with_config(
        ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path),
    );

    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    assert!(window.read(|app| {
        view.read(app)
            .active_session()
            .is_some_and(|session| session.workspaces().is_empty())
    }));
    assert!(window.read(|app| {
        view.read(app)
            .connection(1)
            .is_some_and(|connection| connection.terminals.is_empty())
    }));
    window.update(|window, cx| _ = window.draw(cx));
    let new_workspace = window
        .debug_bounds("new-terminal-workspace")
        .expect("Start Page should offer New Workspace after a corrupt snapshot");
    window.simulate_click(new_workspace.center(), Modifiers::default());
    assert!(window.did_prompt_for_paths());
    window.simulate_path_prompt_response(|_| None);
    window.quit();
}

#[test]
fn text_dialog_actions_are_compact_and_submit() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.prompt_rename_server_on(1, "Local".into(), window, cx)
        });
    });
    submit_text_dialog(window, "Build Server");

    assert_eq!(
        window.read(|app| view.read(app).connection(1).unwrap().label.clone()),
        "Build Server"
    );
}

#[test]
fn server_events_wake_gui_without_polling_clock() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            });
        });
    });

    assert!(
        wait_until_event_driven(window, |window| {
            window.read(|app| {
                view.read(app)
                    .active_session()
                    .is_some_and(|session| session.active_workspace().is_some())
            })
        }),
        "server events should wake GPUI without a timer tick"
    );
}

#[test]
fn chosen_appearance_persists_and_survives_gui_restart() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("client-appearance");
    let config_path = directory.0.join("config.toml");
    let (server, endpoint) = start_server();

    {
        let mut cx = TestAppContext::single();
        cx.update(gpui_component::init);
        let initial = ClientConnection::connect(&endpoint, "condr-gui-test").unwrap();
        let view_holder = Rc::new(RefCell::new(None));
        let view_holder_for_window = view_holder.clone();
        let (_root, window) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                Condr::new(
                    endpoint.clone(),
                    Ok(initial),
                    Some(config_path.clone()),
                    window,
                    cx,
                )
            });
            view_holder_for_window.borrow_mut().replace(view.clone());
            Root::new(view, window, cx)
        });
        let view = view_holder.borrow_mut().take().unwrap();
        assert!(!window.update(|_, cx| cx.theme().is_dark()));

        window.update(|_, cx| {
            view.update(cx, |this, cx| this.set_appearance(Appearance::Dark, cx));
        });
        assert!(window.update(|_, cx| cx.theme().is_dark()));
        assert!(
            window.read(|app| view.read(app).app_error.is_none()),
            "saving the appearance must not report a config error"
        );
        assert!(
            std::fs::read_to_string(&config_path)
                .unwrap()
                .contains("appearance = \"dark\""),
            "the appearance must reach the client config file"
        );
    }

    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    assert!(
        !cx.update(|cx| cx.theme().is_dark()),
        "gpui_component::init starts every process in Light"
    );
    let initial = ClientConnection::connect(&endpoint, "condr-gui-test").unwrap();
    let view_holder = Rc::new(RefCell::new(None));
    let view_holder_for_window = view_holder.clone();
    let (_root, window) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Condr::new(endpoint, Ok(initial), Some(config_path), window, cx));
        view_holder_for_window.borrow_mut().replace(view.clone());
        Root::new(view, window, cx)
    });
    let view = view_holder.borrow_mut().take().unwrap();
    assert!(
        window.update(|_, cx| cx.theme().is_dark()),
        "a restarted GUI must restore the saved appearance"
    );
    assert_eq!(
        window.read(|app| view.read(app).appearance),
        Appearance::Dark
    );
    drop(server);
}
