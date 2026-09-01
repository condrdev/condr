use super::*;

#[cfg(unix)]
#[test]
fn terminal_tab_and_backtab_keys_reach_the_pty() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_component::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            });
        });
    });

    let mut pane_id = None;
    assert!(wait_until(window, |window| {
        pane_id = window.read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        });
        let Some(pane_id) = pane_id else {
            return false;
        };
        let focus = window.read(|app| {
            view.read(app)
                .panels
                .get(&(1, pane_id))
                .map(|panel| panel.read(app).focus_handle.clone())
        });
        window.debug_bounds(terminal_selector(pane_id)).is_some()
            && focus.is_some_and(|focus| window.update(|window, _| focus.is_focused(window)))
    }));
    let pane_id = pane_id.unwrap();

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.terminal_command(
                1,
                pane_id,
                TerminalCommand::Text(
                    "stty -echo -icanon min 1 time 0; printf 'CONDR_TAB_READY\\n'; bytes=$(dd bs=1 count=4 2>/dev/null | od -An -tx1 | tr -d '[:space:]'); stty sane; printf 'CONDR_TAB_BYTES_%s\\n' \"$bytes\"\r"
                        .into(),
                ),
            );
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        terminal_contains(window, &view, 1, pane_id, "CONDR_TAB_READY")
    }));
    let focus = window.read(|app| {
        view.read(app)
            .panels
            .get(&(1, pane_id))
            .map(|panel| panel.read(app).focus_handle.clone())
    });
    assert!(focus.is_some_and(|focus| window.update(|window, _| focus.is_focused(window))));

    window.simulate_keystrokes("tab shift-tab");

    assert!(
        wait_until_event_driven(window, |window| {
            terminal_contains(window, &view, 1, pane_id, "CONDR_TAB_BYTES_091b5b5a")
        }),
        "Tab or BackTab was handled as GUI focus navigation instead of terminal input"
    );
}

#[test]
fn terminal_drag_selection_updates_locally() {
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

    let mut pane_id = None;
    assert!(wait_until(window, |window| {
        pane_id = window.read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        });
        pane_id.is_some_and(|pane_id| window.debug_bounds(terminal_selector(pane_id)).is_some())
    }));
    let pane_id = pane_id.unwrap();
    let render_cache = window.read(|app| {
        view.read(app)
            .panels
            .get(&(1, pane_id))
            .unwrap()
            .read(app)
            .render_cache
            .clone()
    });
    let mut last_revision = None;
    let mut stable_since = Instant::now();
    assert!(wait_until(window, |window| {
        window.update(|window, cx| _ = window.draw(cx));
        let (revision, resize_pending) = window.read(|app| {
            let condr = view.read(app);
            (
                condr.terminal(1, pane_id).unwrap().view.revision,
                condr.pending_sizes.contains_key(&(1, pane_id)),
            )
        });
        if last_revision != Some(revision) {
            last_revision = Some(revision);
            stable_since = Instant::now();
        }
        !resize_pending
            && render_cache.borrow().shaped_cells() > 0
            && stable_since.elapsed() >= Duration::from_millis(50)
    }));
    let shaped_before_drag = render_cache.borrow().shaped_cells();
    let terminal = window.debug_bounds(terminal_selector(pane_id)).unwrap();
    let start = point(
        terminal.left() + terminal.size.width * 0.25,
        terminal.top() + terminal.size.height * 0.35,
    );
    let end = point(
        terminal.left() + terminal.size.width * 0.75,
        terminal.top() + terminal.size.height * 0.65,
    );

    window.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    let dragging = window.read(|app| view.read(app).terminal_selection.unwrap());
    assert!(dragging.dragging);
    assert_eq!((dragging.connection_key, dragging.pane_id), (1, pane_id));
    assert_ne!(dragging.range.start, dragging.range.end);
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(
        render_cache.borrow().shaped_cells(),
        shaped_before_drag,
        "selection-only frames must reuse shaped terminal cells"
    );

    window.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    let completed = window.read(|app| view.read(app).terminal_selection.unwrap());
    assert!(!completed.dragging);
    assert_eq!(completed.range, dragging.range);

    let revision = window.read(|app| view.read(app).terminal(1, pane_id).unwrap().view.revision);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.terminal_command(
                1,
                pane_id,
                TerminalCommand::Text("printf 'CONDR_SELECTION_OUTPUT\\n'\r".into()),
            );
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .terminal(1, pane_id)
                .is_some_and(|terminal| terminal.view.revision > revision)
        })
    }));
    let after_output = window.read(|app| view.read(app).terminal_selection.unwrap());
    assert_eq!(after_output.range, completed.range);
}

#[cfg(unix)]
#[test]
fn terminal_right_click_reports_to_the_pty_and_shift_left_drag_selects_locally() {
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

    let mut pane_id = None;
    assert!(wait_until(window, |window| {
        pane_id = window.read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        });
        pane_id.is_some_and(|pane_id| window.debug_bounds(terminal_selector(pane_id)).is_some())
    }));
    let pane_id = pane_id.unwrap();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.terminal_command(
                1,
                pane_id,
                TerminalCommand::Text(
                    "stty raw -echo; printf 'CONDR_MOUSE_READY\\r\\n\\033[?1002h\\033[?1006h'; bytes=$(dd bs=1 count=18 2>/dev/null | od -An -tx1 | tr -d ' \\n'); stty sane; printf '\\r\\nCONDR_MOUSE_BYTES_%s\\r\\n' \"$bytes\"\r"
                        .into(),
                ),
            );
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        terminal_contains(window, &view, 1, pane_id, "CONDR_MOUSE_READY")
            && window.read(|app| {
                view.read(app).terminal(1, pane_id).is_some_and(|terminal| {
                    terminal.view.mouse_tracking == TerminalMouseTracking::Drag
                })
            })
    }));
    window.update(|window, cx| _ = window.draw(cx));

    let geometry = window.read(|app| {
        view.read(app)
            .terminal_geometry
            .get(&(1, pane_id))
            .copied()
            .unwrap()
    });
    let click = point(
        geometry.bounds.left() + geometry.cell_size.width * 2.5,
        geometry.bounds.top() + geometry.cell_size.height * 2.5,
    );
    window.simulate_mouse_down(click, MouseButton::Right, Modifiers::default());
    window.simulate_mouse_up(click, MouseButton::Right, Modifiers::default());
    assert!(wait_until_event_driven(window, |window| {
        terminal_contains(
            window,
            &view,
            1,
            pane_id,
            "CONDR_MOUSE_BYTES_1b5b3c323b333b334d1b5b3c323b333b336d",
        )
    }));
    assert!(window.read(|app| view.read(app).terminal_selection.is_none()));

    let end = point(
        click.x + geometry.cell_size.width * 4.,
        click.y + geometry.cell_size.height,
    );
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    window.simulate_mouse_down(click, MouseButton::Left, shift);
    window.simulate_mouse_move(end, MouseButton::Left, shift);
    window.simulate_mouse_up(end, MouseButton::Left, shift);
    let selection = window.read(|app| view.read(app).terminal_selection.unwrap());
    assert!(!selection.dragging);
    assert_ne!(selection.range.start, selection.range.end);

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let (generation, server_id, session_id, mut terminal_view) = {
                let connection = this.connection(1).unwrap();
                (
                    connection.connect_generation,
                    connection.server_id.unwrap(),
                    connection.session_id.unwrap(),
                    connection.terminals[&pane_id].view.clone(),
                )
            };
            this.last_terminal_mouse_motion = Some(ReportedTerminalMouseMotion {
                connection_key: 1,
                pane_id,
                mouse_tracking: terminal_view.mouse_tracking,
                event: TerminalMouseEvent::Motion {
                    button: Some(TerminalMouseButton::Left),
                    position: TerminalMousePosition { row: 0, column: 0 },
                    modifiers: Default::default(),
                },
            });
            terminal_view.revision += 1;
            terminal_view.mouse_tracking = TerminalMouseTracking::None;
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
            assert!(this.last_terminal_mouse_motion.is_none());
        });
    });
}

#[test]
fn terminal_double_click_and_ctrl_c_copy_a_word() {
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

    let mut pane_id = None;
    assert!(wait_until(window, |window| {
        pane_id = window.read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        });
        pane_id.is_some_and(|pane_id| window.debug_bounds(terminal_selector(pane_id)).is_some())
    }));
    let pane_id = pane_id.unwrap();

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.terminal_command(
                1,
                pane_id,
                TerminalCommand::Text("echo CONDR_COPY_WORD\r".into()),
            );
        });
    });

    let word = "CONDR_COPY_WORD";
    let word_chars = word.chars().collect::<Vec<_>>();
    let mut word_cell = None;
    assert!(wait_until_event_driven(window, |window| {
        word_cell = window.read(|app| {
            let terminal = &view.read(app).terminal(1, pane_id)?.view;
            for row in 0..terminal.size.rows {
                for column in 0..terminal.size.columns {
                    let matches = word_chars.iter().enumerate().all(|(offset, expected)| {
                        let Ok(offset) = u16::try_from(offset) else {
                            return false;
                        };
                        terminal
                            .cell(row, column.saturating_add(offset))
                            .and_then(|cell| cell.text.chars().next())
                            == Some(*expected)
                    });
                    if matches {
                        return Some((row, column));
                    }
                }
            }
            None
        });
        word_cell.is_some()
    }));
    let (row, column) = word_cell.unwrap();
    let click = window.read(|app| {
        let geometry = view
            .read(app)
            .terminal_geometry
            .get(&(1, pane_id))
            .copied()
            .unwrap();
        point(
            geometry.bounds.left() + geometry.cell_size.width * (f32::from(column) + 0.5),
            geometry.bounds.top() + geometry.cell_size.height * (f32::from(row) + 0.5),
        )
    });

    window.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: click,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    window.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: click,
        modifiers: Modifiers::default(),
        click_count: 2,
    });

    let selection = window.read(|app| view.read(app).terminal_selection.unwrap());
    assert!(!selection.dragging);
    assert_eq!(selection.range.start.column, column);
    assert_eq!(
        selection.range.end.column,
        column + u16::try_from(word_chars.len()).unwrap() - 1
    );

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            connection.subscribed = false;
            connection.bootstrap_resync_session_id = connection.session_id;
        });
    });
    window.simulate_keystrokes("ctrl-c");
    assert!(
        window.read(|app| view.read(app).terminal_selection.is_some()),
        "a rejected Copy must preserve the local selection"
    );
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            connection.subscribed = true;
            connection.bootstrap_resync_session_id = None;
        });
    });
    window.simulate_keystrokes("ctrl-c");
    assert!(window.read(|app| view.read(app).terminal_selection.is_some()));
    assert!(wait_until_event_driven(window, |window| {
        window
            .read_from_clipboard()
            .and_then(|item| item.text())
            .is_some_and(|text| text == word)
    }));
}

#[test]
fn terminal_clipboard_shortcuts_paste_through_tcp_server() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_component::init);
    let (server, endpoint) = start_tcp_server();
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
            });
        });
    });

    let mut pane_id = None;
    assert!(wait_until(window, |window| {
        pane_id = window.read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        });
        let Some(pane_id) = pane_id else {
            return false;
        };
        let (terminal_ready, focus) = window.read(|app| {
            let condr = view.read(app);
            (
                condr
                    .connection(1)
                    .and_then(|connection| connection.terminals.get(&pane_id))
                    .is_some_and(|terminal| !terminal.exited),
                condr
                    .panels
                    .get(&(1, pane_id))
                    .map(|panel| panel.read(app).focus_handle.clone()),
            )
        });
        terminal_ready
            && window.debug_bounds(terminal_selector(pane_id)).is_some()
            && focus.is_some_and(|focus| window.update(|window, _| focus.is_focused(window)))
    }));
    let pane_id = pane_id.unwrap();

    let shift_insert_marker = "CONDR_SHIFT_INSERT_PASTE";
    window.write_to_clipboard(ClipboardItem::new_string(shift_insert_marker.into()));
    window.simulate_keystrokes("shift-insert");
    assert!(wait_until_event_driven(window, |window| {
        terminal_contains(window, &view, 1, pane_id, shift_insert_marker)
    }));

    #[cfg(windows)]
    {
        let ctrl_v_marker = "CONDR_CTRL_V_PASTE";
        window.write_to_clipboard(ClipboardItem::new_string(ctrl_v_marker.into()));
        window.simulate_keystrokes("ctrl-v");
        assert!(wait_until_event_driven(window, |window| {
            terminal_contains(window, &view, 1, pane_id, ctrl_v_marker)
        }));
    }

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.prompt_rename_server_on(1, "Local".into(), window, cx)
        });
    });
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.update(|window, cx| window.has_focused_input(cx)));

    let dialog_marker = "CONDR_DIALOG_MUST_NOT_PASTE_INTO_TERMINAL";
    window.write_to_clipboard(ClipboardItem::new_string(dialog_marker.into()));
    window.simulate_keystrokes("shift-insert");
    window.run_until_parked();
    assert!(!terminal_contains(window, &view, 1, pane_id, dialog_marker));
}
