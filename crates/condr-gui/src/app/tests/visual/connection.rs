use super::*;
use crate::app::tests::tcp;
use condr_server::StaticKey;

#[test]
fn a_new_panes_title_survives_its_first_visual_frame_and_is_pruned_on_close() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.connect_generation;
            let server_id = connection.server_id.unwrap();
            let session_id = connection.session_id.unwrap();
            let sequence = connection.sequence;
            let original = connection.snapshot.clone();
            let mut session = Session::restore(original.clone()).unwrap();
            session.create_workspace(std::env::temp_dir()).unwrap();
            let pane_id = session
                .active_workspace()
                .unwrap()
                .active_tab()
                .focused_pane()
                .unwrap()
                .id();
            for (offset, event) in [
                SessionEvent::LayoutChanged {
                    snapshot: session.snapshot(),
                    zoomed_panes: Vec::new(),
                },
                SessionEvent::TerminalTitleChanged {
                    pane_id,
                    title: Some("agent title".into()),
                },
            ]
            .into_iter()
            .enumerate()
            {
                this.handle_incoming(
                    1,
                    generation,
                    Incoming::Message(ServerMessage::Event {
                        server_id,
                        session_id,
                        sequence: sequence + offset as u64 + 1,
                        event,
                    }),
                    cx,
                );
            }
            assert!(!this.connection(1).unwrap().terminals.contains_key(&pane_id));
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::TerminalFrame(TerminalFrameBatch {
                    server_id,
                    session_id,
                    panes: vec![PaneTerminalFrame {
                        pane_id,
                        frame: TerminalViewFrame::Full(crate::app::tests::terminal_view(
                            1, "first",
                        )),
                    }],
                })),
                cx,
            );
            let connection = this.connection(1).unwrap();
            assert_eq!(
                connection.terminal_titles.get(&pane_id).map(String::as_str),
                Some("agent title")
            );
            assert!(connection.terminals.contains_key(&pane_id));
            assert!(connection.bootstrap_resync_session_id.is_none());
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::Event {
                    server_id,
                    session_id,
                    sequence: sequence + 3,
                    event: SessionEvent::LayoutChanged {
                        snapshot: original,
                        zoomed_panes: Vec::new(),
                    },
                }),
                cx,
            );
            assert!(
                !this
                    .connection(1)
                    .unwrap()
                    .terminal_titles
                    .contains_key(&pane_id)
            );
        });
    });
}

#[test]
fn stale_connection_result_cannot_replace_the_current_attempt() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
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
                    endpoint: tcp("127.0.0.1:9"),
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
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);

    let original_sequence = window.read(|app| view.read(app).connection(1).unwrap().sequence);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.connect_generation;
            let server_id = connection.server_id.unwrap();
            let session_id = connection.session_id.unwrap();
            let snapshot = connection.snapshot.clone();
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
                        event: SessionEvent::LayoutChanged {
                            snapshot: snapshot.clone(),
                            zoomed_panes: Vec::new(),
                        },
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

/// A layout change is applied from the event itself: no Bootstrap round trip, so the
/// GUI never drops into the "not synchronized" state that dims every control.
#[test]
fn in_sequence_layout_change_applies_without_a_bootstrap_resync() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.connect_generation;
            let server_id = connection.server_id.unwrap();
            let session_id = connection.session_id.unwrap();
            let sequence = connection.sequence + 1;
            // The test Server starts empty; the event may announce a Workspace the GUI
            // has never seen, exactly like a `condr workspace create` from a Pane.
            let mut session = Session::restore(connection.snapshot.clone()).unwrap();
            if session.active_workspace().is_none() {
                session.create_workspace(std::env::temp_dir()).unwrap();
            }
            let tab_id = session.active_workspace().unwrap().active_tab().id();
            assert!(session.rename_tab(tab_id, "renamed by event"));
            let snapshot = session.snapshot();
            assert!(connection.can_mutate());

            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::Event {
                    server_id,
                    session_id,
                    sequence,
                    event: SessionEvent::LayoutChanged {
                        snapshot: snapshot.clone(),
                        zoomed_panes: Vec::new(),
                    },
                }),
                cx,
            );

            let connection = this.connection(1).unwrap();
            assert_eq!(
                connection.snapshot, snapshot,
                "the event's structure is applied"
            );
            assert_eq!(connection.sequence, sequence);
            assert!(
                connection.bootstrap_resync_session_id.is_none(),
                "an in-sequence layout change must not request a Bootstrap"
            );
            assert!(connection.can_mutate(), "the UI stays enabled throughout");
        });
    });
}

#[test]
fn denied_replacement_connection_retries_after_the_controller_releases() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
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
    let session_id = contender.bootstrap().unwrap().session_id;
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
                .then_some((surface_key, tab.focused_pane().unwrap().id()))
        });
        active_surface.is_some()
    }));
    let (surface_key, pane_id) = active_surface.unwrap();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::SplitPane {
                focus: true,
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
            let local_layout = local
                .tab(surface_key.tab_id)
                .unwrap()
                .layout()
                .unwrap()
                .clone();
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
                .unwrap()
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
                == Some(session.tab(surface_key.tab_id).unwrap().layout().unwrap())
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
        ServerConfig::at_socket(endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path.clone()),
    );

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, mut server) = connected_condr_with(&mut cx, server, endpoint.clone());
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
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
            .unwrap()
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

    let mut replacement = start_server_with_config(
        ServerConfig::at_socket(endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path),
    );
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
fn invalid_saved_server_protects_the_list_but_allows_preferences() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("invalid-client-config");
    let config_path = directory.0.join("config.toml");
    let original = "# keep these entries\n[[client.servers]]\nname = 'Old TCP'\naddress = '127.0.0.1:4242'\nserver_key = 'legacy'\n[[client.servers]]\nname = 'Valid SSH'\naddress = 'ssh://build-box'\n";
    std::fs::write(&config_path, original).unwrap();
    let config = config::LoadedConfig::read(Some(config_path.clone()));
    assert!(config.servers_error.is_some());
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let view_holder = Rc::new(RefCell::new(None));
    let holder = view_holder.clone();
    let (_root, window) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            Condr::new(
                Endpoint::local(directory.0.join("unused.sock")),
                Some(Err("offline for config test".into())),
                config,
                window,
                cx,
            )
        });
        holder.borrow_mut().replace(view.clone());
        Root::new(view, window, cx)
    });
    let view = view_holder.borrow_mut().take().unwrap();
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            assert!(this.servers_error.is_some());
            this.prompt_add_server(window, cx);
            assert!(!window.has_active_dialog(cx));
            assert!(
                this.app_error
                    .as_ref()
                    .unwrap()
                    .contains("fix the file and restart")
            );
            // Even direct writeback and mutations must respect the failed load.
            this.connections.push(ServerConnection::new(
                2,
                "Unsaved".into(),
                Endpoint::parse("ssh://new-box", None).unwrap(),
            ));
            assert!(!this.apply_server_edit(2, "Edited", "ssh://edited", "", window, cx));
            this.remove_server(2, window, cx);
            assert_eq!(this.connection(2).unwrap().label, "Unsaved");
            this.save_servers(cx);
            assert!(this.config_save.is_none());
        });
    });
    window.run_until_parked();
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), original);
    window.update(|_, cx| view.update(cx, |this, cx| this.set_appearance(Appearance::Dark, cx)));
    window.run_until_parked();
    let saved = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved.contains("appearance = \"dark\""));
    assert!(saved.contains("# keep these entries"));
    assert!(saved.contains("address = '127.0.0.1:4242'"));
    assert!(saved.contains("address = 'ssh://build-box'"));
    assert!(!saved.contains("new-box"));
    window.quit();
}

#[test]
fn disconnect_and_drop_cancel_pending_connection_attempts() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| view.update(cx, |this, _| {
        let connection = this.connection_mut(1).unwrap();
        let endpoint = connection.endpoint.clone();
        let cancellation = connection.cancellation.clone();
        connection.status = ConnectionStatus::Connecting;
        this.disconnect_server(1);
        assert!(matches!(ClientConnection::connect_cancellable(&endpoint, "cancelled", cancellation), Err(error) if error.kind() == std::io::ErrorKind::Interrupted));
        let connection = ServerConnection::new(2, "Dropped".into(), endpoint.clone());
        let cancellation = connection.cancellation.clone();
        drop(connection);
        assert!(matches!(ClientConnection::connect_cancellable(&endpoint, "dropped", cancellation), Err(error) if error.kind() == std::io::ErrorKind::Interrupted));
    }));
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
        cx.update(gpui_kit::init);
        let initial = ClientConnection::connect(&endpoint, "condr-test").unwrap();
        let bootstrap = initial.bootstrap().unwrap().clone();
        assert_eq!(bootstrap.server_id, server.handle.server_id());
        let view_holder = Rc::new(RefCell::new(None));
        let view_holder_for_window = view_holder.clone();
        let (_root, window) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                Condr::new(
                    endpoint.clone(),
                    Some(Ok(initial)),
                    config::LoadedConfig::read(Some(config_path.clone())),
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
        submit_text_dialog(
            window,
            &format!(
                "tcp://{}@127.0.0.1:4242",
                StaticKey::from_private([7; 32]).public()
            ),
        );
        assert!(window.read(|app| {
            view.read(app)
                .connections
                .iter()
                .any(|connection| connection.endpoint.tcp_host_port() == Some(("127.0.0.1", 4242)))
        }));
    }

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let initial = ClientConnection::connect(&endpoint, "condr-test").unwrap();
    let view_holder = Rc::new(RefCell::new(None));
    let view_holder_for_window = view_holder.clone();
    let (_root, window) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            Condr::new(
                endpoint,
                Some(Ok(initial)),
                config::LoadedConfig::read(Some(config_path)),
                window,
                cx,
            )
        });
        view_holder_for_window.borrow_mut().replace(view.clone());
        Root::new(view, window, cx)
    });
    let view = view_holder.borrow_mut().take().unwrap();
    assert!(window.read(|app| {
        view.read(app)
            .connections
            .iter()
            .any(|connection| connection.endpoint.tcp_host_port() == Some(("127.0.0.1", 4242)))
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
        ServerConfig::at_socket(endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path),
    );

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
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
        .debug_bounds("open-project")
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
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.prompt_text(
                "Rename Server",
                "Save",
                "Local".into(),
                |this, name, _, _| {
                    this.connection_mut(1).unwrap().label = name;
                    true
                },
                window,
                cx,
            )
        });
    });
    submit_text_dialog(window, "Build Server");

    assert_eq!(
        window.read(|app| view.read(app).connection(1).unwrap().label.clone()),
        "Build Server"
    );
}

#[test]
fn editing_a_server_changes_its_name_and_address_but_never_the_local_one() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.connections
                .push(ServerConnection::new(2, "lab".into(), tcp("10.0.0.5:4242")));
            assert!(this.apply_server_edit(2, "Lab box", "lab.local", "5555", window, cx));
            let connection = this.connection(2).unwrap();
            assert_eq!(connection.label, "Lab box");
            assert_eq!(
                connection.endpoint.tcp_host_port(),
                Some(("lab.local", 5555))
            );
            assert!(
                !this.apply_server_edit(2, "", "lab.local", "5555", window, cx),
                "a blank name is refused"
            );
            assert!(
                !this.apply_server_edit(2, "Lab box", "lab.local", "0", window, cx),
                "port 0 is refused"
            );
            assert!(this.app_error.is_some());

            // The Local Server has no edit dialog at all.
            this.prompt_edit_server_on(1, window, cx);
        });
    });
    window.run_until_parked();
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));

    window.update(|window, cx| {
        view.update(cx, |this, cx| this.prompt_edit_server_on(2, window, cx));
    });
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    assert!(window.update(|window, cx| window.has_focused_input(cx)));
}

#[test]
fn ssh_addresses_add_edit_and_use_remote_paths() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| {
        view.update(cx, |this, cx| this.prompt_add_server(window, cx));
    });
    submit_text_dialog(window, "ssh://user@127.0.0.1:1?bin=/opt/a%20b/condr");
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let key = this.active_connection;
            let Endpoint::Ssh(ssh) = &this.connection(key).unwrap().endpoint else {
                panic!("expected SSH");
            };
            assert_eq!(ssh.binary(), "/opt/a b/condr");
            this.disconnect_server(key);
            assert!(this.apply_server_edit(
                key,
                "Build",
                "ssh://builder@127.0.0.1:1?bin=/opt/condr",
                "",
                window,
                cx
            ));
            assert_eq!(
                this.connection(key).unwrap().endpoint.to_string(),
                "ssh://builder@127.0.0.1:1?bin=/opt/condr"
            );
            this.prompt_edit_server_on(key, window, cx);
        });
    });
    window.run_until_parked();
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    assert!(window.update(|window, cx| window.has_focused_input(cx)));
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.prompt_directory_on(
                this.active_connection,
                "New Workspace".into(),
                "Create",
                |_, _, _, _| {},
                window,
                cx,
            )
        });
    });
    window.run_until_parked();
    assert!(!window.did_prompt_for_paths());
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    window.quit();
}

#[test]
fn server_events_wake_gui_without_polling_clock() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
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
fn an_unwatched_agent_completion_posts_a_system_notification_for_its_pane() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    cx.update(|cx| cx.set_app_identity("dev.condr.gui", "Condr"));
    let (view, window, _server) = connected_condr(&mut cx);
    let pane_id = window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.connect_generation;
            let server_id = connection.server_id.unwrap();
            let session_id = connection.session_id.unwrap();
            let sequence = connection.sequence;
            let mut session = Session::restore(connection.snapshot.clone()).unwrap();
            session.create_workspace(std::env::temp_dir()).unwrap();
            let pane_id = session
                .active_workspace()
                .unwrap()
                .active_tab()
                .focused_pane()
                .unwrap()
                .id();
            let agent = |state| {
                Some(AgentSnapshot {
                    session_id: None,
                    kind: AgentKind::Claude,
                    state,
                })
            };
            for (offset, event) in [
                SessionEvent::LayoutChanged {
                    snapshot: session.snapshot(),
                    zoomed_panes: Vec::new(),
                },
                SessionEvent::AgentChanged {
                    pane_id,
                    agent: agent(AgentState::Working),
                },
                SessionEvent::AgentChanged {
                    pane_id,
                    agent: agent(AgentState::Idle),
                },
            ]
            .into_iter()
            .enumerate()
            {
                this.handle_incoming(
                    1,
                    generation,
                    Incoming::Message(ServerMessage::Event {
                        server_id,
                        session_id,
                        sequence: sequence + offset as u64 + 1,
                        event,
                    }),
                    cx,
                );
            }
            pane_id
        })
    });
    let shown = cx.shown_system_notifications();
    assert_eq!(
        shown.len(),
        1,
        "only the Working -> Idle transition notifies"
    );
    assert_eq!(shown[0].title.as_ref(), "Claude finished");
    assert_eq!(
        shown[0].tag,
        crate::app::notifications::agent_notification_tag(1, pane_id)
    );
}

#[test]
fn an_unexpected_disconnect_reconnects_on_its_own_and_says_so() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let generation = this.connection(1).unwrap().connect_generation;
            this.handle_incoming(
                1,
                generation,
                Incoming::Disconnected("the device closed the connection".into()),
                cx,
            );
            let connection = this.connection(1).unwrap();
            assert!(connection.status == ConnectionStatus::Disconnected);
            assert!(connection.reconnect_deadline.is_some());
            assert_eq!(
                connection.error.as_deref(),
                Some("the device closed the connection")
            );
            cx.notify();
        });
    });
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds("connection-status-1").is_some(),
        "the start page reports the reconnect"
    );
    // The faked disconnect leaves the old socket open, so the Server may keep control
    // with it for a while; the connection itself must come back and settle.
    assert!(
        wait_until(window, |window| {
            window.read(|app| {
                view.read(app).connection(1).is_some_and(|connection| {
                    connection.status == ConnectionStatus::Connected
                        && connection.reconnect_deadline.is_none()
                })
            })
        }),
        "the GUI did not reconnect on its own"
    );
}
