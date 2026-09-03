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

#[cfg(unix)]
#[test]
fn terminal_pageup_reaches_the_pty_while_shift_pageup_scrolls_history() {
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
                    "i=0; while [ $i -lt 80 ]; do printf 'CONDR_SCROLL_%03d\\n' \"$i\"; i=$((i+1)); done; stty -echo -icanon min 1 time 0; printf 'CONDR_PAGEUP_READY\\n'; bytes=$(dd bs=1 count=4 2>/dev/null | od -An -tx1 | tr -d '[:space:]'); stty sane; printf 'CONDR_PAGEUP_BYTES_%s\\n' \"$bytes\"\r"
                        .into(),
                ),
            );
        });
    });
    assert!(wait_until_event_driven(window, |window| {
        terminal_contains(window, &view, 1, pane_id, "CONDR_PAGEUP_READY")
    }));

    window.simulate_keystrokes("pageup");
    assert!(wait_until_event_driven(window, |window| {
        terminal_contains(window, &view, 1, pane_id, "CONDR_PAGEUP_BYTES_1b5b357e")
    }));

    window.simulate_keystrokes("shift-pageup");
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .terminal(1, pane_id)
                .is_some_and(|terminal| terminal.view.display_offset > 0)
        })
    }));
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

#[test]
fn scrollback_selection_tracks_authoritative_view_offset_for_copy() {
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
    assert!(wait_until_event_driven(window, |window| {
        pane_id = window.read(|app| {
            view.read(app)
                .active_session()?
                .active_workspace()
                .map(|workspace| workspace.active_tab().focused_pane().id())
        });
        pane_id.is_some()
    }));
    let pane_id = pane_id.unwrap();
    let (outgoing, outgoing_rx) = std::sync::mpsc::channel();

    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let range = TerminalSelection {
                start: TerminalPosition {
                    row: 1,
                    column: 1,
                    side: TerminalSide::Left,
                },
                end: TerminalPosition {
                    row: 1,
                    column: 3,
                    side: TerminalSide::Right,
                },
                display_offset: 3,
            };
            this.terminal_selection = Some(LocalTerminalSelection {
                connection_key: 1,
                pane_id,
                range,
                dragging: false,
            });

            let (generation, server_id, session_id, mut terminal_view) = {
                let connection = this.connection(1).unwrap();
                (
                    connection.connect_generation,
                    connection.server_id.unwrap(),
                    connection.session_id.unwrap(),
                    connection.terminals[&pane_id].view.clone(),
                )
            };
            terminal_view.revision += 1;
            terminal_view.display_offset = 7;
            this.connection_mut(1).unwrap().io = Some(ClientIo {
                outgoing,
                _incoming_task: Task::ready(()),
            });
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

            let updated = this.terminal_selection.unwrap().range;
            assert_eq!((updated.start, updated.end), (range.start, range.end));
            assert_eq!(updated.display_offset, 7);
            assert!(this.copy_terminal_selection(1, pane_id, cx));
        });
    });

    assert!(matches!(
        outgoing_rx.recv().unwrap(),
        ClientMessage::Terminal {
            pane_id: copied_pane,
            command: TerminalCommand::Copy { selection },
            ..
        } if copied_pane == pane_id && selection.display_offset == 7
    ));
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
fn terminal_double_click_and_clipboard_shortcut_copy_a_word() {
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
    window.simulate_keystrokes("ctrl-shift-c");
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
    window.simulate_keystrokes("ctrl-shift-c");
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

#[test]
fn terminal_link_hover_and_modified_click_open_the_url() {
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
    let uri = "https://example.com/path";
    let row = 0;
    let hover_column = 10;

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            connection.io = None;
            connection.connect_generation = connection.connect_generation.wrapping_add(1);
        });
    });
    window.run_until_parked();
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
            assert!(usize::from(terminal_view.size.columns) >= uri.len());
            for column in 0..terminal_view.size.columns {
                let cell = &mut terminal_view.cells[usize::from(row)
                    * usize::from(terminal_view.size.columns)
                    + usize::from(column)];
                cell.text = " ".into();
                cell.flags = 0;
                cell.hyperlink = None;
            }
            for (column, text) in uri.chars().enumerate() {
                terminal_view.cells
                    [usize::from(row) * usize::from(terminal_view.size.columns) + column]
                    .text = text.to_string().into();
            }
            terminal_view.cursor = None;
            terminal_view.revision += 1;
            let effect = this.handle_incoming(
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
            if effect.notify {
                cx.notify();
            }
        });
    });
    window.update(|window, cx| _ = window.draw(cx));

    let (geometry, render_cache, terminal_size) = window.read(|app| {
        let condr = view.read(app);
        (
            condr.terminal_geometry[&(1, pane_id)],
            condr.panels[&(1, pane_id)].read(app).render_cache.clone(),
            condr.terminal(1, pane_id).unwrap().view.size,
        )
    });
    let link_cell = point(
        geometry.bounds.left() + geometry.cell_size.width * (f32::from(hover_column) + 0.5),
        geometry.bounds.top() + geometry.cell_size.height * (f32::from(row) + 0.5),
    );
    let off_link_column = u16::try_from(uri.len()).unwrap();
    assert!(terminal_size.columns > off_link_column);
    let off_link_cell = point(
        geometry.bounds.left() + geometry.cell_size.width * (f32::from(off_link_column) + 0.5),
        geometry.bounds.top() + geometry.cell_size.height * (f32::from(row) + 0.5),
    );
    let shaped_before_hover = render_cache.borrow().shaped_cells();
    window.simulate_mouse_move(link_cell, None, Modifiers::default());
    window.update(|window, cx| _ = window.draw(cx));
    assert_eq!(
        window.read(|app| {
            view.read(app)
                .hovered_link
                .as_ref()
                .map(|(_, _, link)| (link.uri.to_string(), link.position))
        }),
        Some((
            uri.to_owned(),
            TerminalMousePosition {
                row,
                column: hover_column,
            },
        )),
        "moving over a URL must cache it without Ctrl/Cmd"
    );
    assert!(
        render_cache.borrow().shaped_cells() > shaped_before_hover,
        "hovering a URL must render its underline"
    );

    window.simulate_mouse_down(link_cell, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(link_cell, MouseButton::Left, Modifiers::default());
    assert_eq!(
        window.opened_url(),
        None,
        "a plain click must not activate a terminal link"
    );

    let secondary = Modifiers::secondary_key();
    window.simulate_mouse_down(link_cell, MouseButton::Left, secondary);
    assert_eq!(
        window.opened_url(),
        None,
        "pressing a link must wait for a matching release"
    );
    assert!(window.read(|app| view.read(app).pressed_terminal_link.is_some()));
    window.simulate_mouse_move(off_link_cell, MouseButton::Left, secondary);
    window.simulate_mouse_up(off_link_cell, MouseButton::Left, secondary);
    assert_eq!(
        window.opened_url(),
        None,
        "releasing away from the pressed link must cancel activation"
    );
    assert!(window.read(|app| view.read(app).pressed_terminal_link.is_none()));

    window.simulate_mouse_move(link_cell, None, secondary);
    window.simulate_mouse_down(link_cell, MouseButton::Left, secondary);
    assert_eq!(window.opened_url(), None);
    window.simulate_mouse_up(link_cell, MouseButton::Left, secondary);
    assert_eq!(window.opened_url().as_deref(), Some(uri));

    assert!(window.read(|app| view.read(app).hovered_link.is_some()));

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
            terminal_view.cells[usize::from(row) * usize::from(terminal_view.size.columns)
                + usize::from(hover_column)]
            .text = " ".into();
            terminal_view.revision += 1;
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
        });
    });
    assert!(
        window.read(|app| view.read(app).hovered_link.is_none()),
        "a terminal frame must invalidate a link no longer under the pointer"
    );
}

#[test]
fn terminal_focus_changes_report_to_the_pty_without_leasing_the_focused_panel() {
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

    window.update(|window, _| window.activate_window());
    window.run_until_parked();
    assert!(
        window.update(|window, _| window.is_window_active()),
        "the test window must be active for terminal focus to be reported"
    );
    assert_eq!(
        window.read(|app| view.read(app).reported_terminal_focus),
        Some((1, pane_id)),
        "activating the window should report terminal focus to the PTY"
    );

    let app_focus = window.read(|app| view.read(app).focus_handle.clone());
    let terminal_focus = window.read(|app| {
        view.read(app).panels[&(1, pane_id)]
            .read(app)
            .focus_handle
            .clone()
    });
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let session_id = this.connection(1).unwrap().session_id.unwrap();
            this.connection_mut(1).unwrap().bootstrap_resync_session_id = Some(session_id);
            this.sync_terminal_focus(window, cx);
        });
    });
    assert_eq!(
        window.read(|app| view.read(app).reported_terminal_focus),
        Some((1, pane_id)),
        "Bootstrap resync alone must not report a focused terminal as unfocused"
    );
    window.update(|window, cx| app_focus.focus(window, cx));
    window.update(|window, cx| _ = window.draw(cx));
    window.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).reported_terminal_focus),
        None,
        "a blur during Bootstrap resync must still be reported"
    );
    assert_eq!(
        window.read(|app| view.read(app).focused_terminal),
        None,
        "the GUI must also consider the terminal locally unfocused"
    );
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            connection.send(ClientMessage::Terminal {
                server_id: connection.server_id.unwrap(),
                session_id: connection.session_id.unwrap(),
                pane_id,
                // BEL first, then a marker split so the echoed command line never
                // matches it. The shell is pwsh on Windows and a POSIX shell elsewhere.
                #[cfg(windows)]
                command: TerminalCommand::Text(
                    "[Console]::Write([char]7); Write-Host ('focus-blur-command-' + 'finished')\r"
                        .into(),
                ),
                #[cfg(not(windows))]
                command: TerminalCommand::Text(
                    "printf '\\a'; printf 'focus-blur-command-'; printf 'finished\\n'\r".into(),
                ),
            });
        });
    });
    assert!(
        wait_until(window, |window| terminal_contains(
            window,
            &view,
            1,
            pane_id,
            "focus-blur-command-finished"
        )),
        "the shell must execute the BEL command before attention is asserted"
    );
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .unwrap()
                .attention
                .contains(&pane_id)
        })
    }));
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().bootstrap_resync_session_id = None;
        });
    });
    window.update(|window, cx| terminal_focus.focus(window, cx));
    window.update(|window, cx| _ = window.draw(cx));
    window.run_until_parked();

    let send_attention = |window: &mut VisualTestContext, attention| {
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                let (generation, server_id, session_id, sequence) = {
                    let connection = this.connection(1).unwrap();
                    (
                        connection.connect_generation,
                        connection.server_id.unwrap(),
                        connection.session_id.unwrap(),
                        connection.sequence + 1,
                    )
                };
                let effect = this.handle_incoming(
                    1,
                    generation,
                    Incoming::Message(ServerMessage::Event {
                        server_id,
                        session_id,
                        sequence,
                        event: SessionEvent::TerminalAttentionChanged { pane_id, attention },
                    }),
                    cx,
                );
                if effect.notify {
                    cx.notify();
                }
            });
        });
    };
    send_attention(window, true);
    assert!(
        !window.read(|app| view
            .read(app)
            .connection(1)
            .unwrap()
            .attention
            .contains(&pane_id)),
        "a bell from the actually focused terminal must not create attention"
    );

    // Blurring a Pane runs the Panel's focus callback while GPUI holds its lease.
    window.update(|window, cx| app_focus.focus(window, cx));
    window.update(|window, cx| _ = window.draw(cx));
    window.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).reported_terminal_focus),
        None,
        "moving focus out of a Pane should report the terminal as unfocused"
    );
    assert_eq!(
        window.read(|app| view.read(app).target_pane),
        Some((1, pane_id)),
        "blurring the terminal does not change the selected Pane"
    );

    send_attention(window, true);
    assert!(
        window.read(|app| view
            .read(app)
            .connection(1)
            .unwrap()
            .attention
            .contains(&pane_id)),
        "a selected but unfocused terminal must retain bell attention"
    );

    window.update(|window, cx| terminal_focus.focus(window, cx));
    window.update(|window, cx| _ = window.draw(cx));
    window.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).reported_terminal_focus),
        Some((1, pane_id))
    );
    assert!(
        !window.read(|app| view
            .read(app)
            .connection(1)
            .unwrap()
            .attention
            .contains(&pane_id)),
        "successfully reporting terminal focus must clear bell attention"
    );

    window.update(|window, cx| app_focus.focus(window, cx));
    window.update(|window, cx| _ = window.draw(cx));
    window.run_until_parked();
    send_attention(window, true);
    send_attention(window, false);
    assert!(
        !window.read(|app| view
            .read(app)
            .connection(1)
            .unwrap()
            .attention
            .contains(&pane_id)),
        "the ordered focus clear must cancel a delayed pre-focus bell"
    );
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().controlling = false;
        });
    });
    send_attention(window, true);
    assert!(
        window.read(|app| view
            .read(app)
            .connection(1)
            .unwrap()
            .attention
            .contains(&pane_id)),
        "attention is Server state and is kept without control; the pane header and \
         sidebar gate on `controlling` instead (3c92fb8)"
    );

    window.update(|window, cx| terminal_focus.focus(window, cx));
    window.update(|window, cx| _ = window.draw(cx));
    window.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).focused_terminal),
        Some((1, pane_id))
    );
    assert_eq!(
        window.read(|app| view.read(app).reported_terminal_focus),
        None,
        "a viewer tracks local focus even though it cannot report Focus(true)"
    );

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1).unwrap().controlling = true;
            this.sync_terminal_focus(window, cx);
        });
    });
    assert_eq!(
        window.read(|app| view.read(app).reported_terminal_focus),
        Some((1, pane_id)),
        "control reacquisition must report existing local focus without a new focus event"
    );
}

#[test]
fn selected_block_elements_stay_visible_in_the_selection_text_color() {
    let _serial_guard = acquire_visual_test_lock();
    let workspace_root = TestDirectory::new("selected-block-elements");
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
    let mut pane_id = None;
    assert!(wait_until_event_driven(window, |window| {
        pane_id = window.read(|app| {
            let session = view.read(app).active_session()?;
            Some(session.active_workspace()?.active_tab().focused_pane().id())
        });
        pane_id.is_some()
    }));
    let pane_id = pane_id.unwrap();

    // A scheme with a selection text color, as Gruvbox and most of the collection have.
    let selection_text: gpui::Hsla = gpui::rgb(0x123456).into();
    let palette = TerminalPalette {
        selection_text: Some(selection_text),
        ..TerminalPalette::default()
    };
    let selection_background: Background = palette.selection.into();
    let block_foreground: Background = selection_text.into();
    window.update(|_, cx| cx.set_global(palette));

    let cell = |text: &str| TerminalCell {
        text: text.into(),
        foreground: TerminalColor::Named(256),
        background: TerminalColor::Named(257),
        flags: 0,
        hyperlink: None,
    };
    let terminal = TerminalView {
        revision: 1,
        size: TerminalSize::new(1, 2),
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cells: vec![cell("█"), cell("x")],
        cursor: None,
    };
    let selection = TerminalSelection {
        start: TerminalPosition {
            row: 0,
            column: 0,
            side: TerminalSide::Left,
        },
        end: TerminalPosition {
            row: 0,
            column: 0,
            side: TerminalSide::Right,
        },
        display_offset: 0,
    };
    let (_, prepaint) = window.draw(point(px(0.), px(0.)), size(px(200.), px(40.)), |_, cx| {
        TerminalElement::new(
            view.clone(),
            TerminalElementProps {
                focus_handle: cx.focus_handle(),
                connection_key: 1,
                pane_id,
                terminal,
                marked_text: None,
                selection: Some(selection),
                hovered_link: None,
                runtime_epoch: None,
                render_cache: Rc::new(RefCell::new(TerminalRenderCache::default())),
                scroll_remainder: Rc::new(RefCell::new(point(0., 0.))),
            },
        )
    });

    let selection_index = prepaint
        .quads
        .iter()
        .position(|quad| quad.background == selection_background)
        .expect("the selection background is painted");
    let block_index = prepaint
        .quads
        .iter()
        .position(|quad| quad.background == block_foreground)
        .expect("the selected █ is painted as a quad in the selection text color");
    assert!(
        block_index > selection_index,
        "the block element must paint above the selection: block at {block_index}, selection at {selection_index}"
    );

    let linked_cache = Rc::new(RefCell::new(TerminalRenderCache::default()));
    let linked_block = TerminalView {
        revision: 2,
        size: TerminalSize::new(1, 1),
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cells: vec![TerminalCell {
            hyperlink: Some("https://example.com".into()),
            ..cell("█")
        }],
        cursor: None,
    };
    window.draw(point(px(0.), px(0.)), size(px(100.), px(40.)), |_, cx| {
        TerminalElement::new(
            view.clone(),
            TerminalElementProps {
                focus_handle: cx.focus_handle(),
                connection_key: 1,
                pane_id,
                terminal: linked_block,
                marked_text: None,
                selection: None,
                hovered_link: None,
                runtime_epoch: None,
                render_cache: linked_cache.clone(),
                scroll_remainder: Rc::new(RefCell::new(point(0., 0.))),
            },
        )
    });
    assert_eq!(
        linked_cache.borrow().shaped_cells(),
        1,
        "a linked block glyph must be shaped so its underline is visible"
    );
}
