use super::*;
#[cfg(target_os = "linux")]
use condr_core::{
    TerminalModifiers, TerminalMouseButton, TerminalMouseEvent, TerminalMousePosition,
};

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn a_client_can_acknowledge_attention_while_terminal_is_closing() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let (server_id, session_id, pane_id) = {
        let mut state = handle.state.lock().unwrap();
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = state.session.workspaces()[0].tabs()[0]
            .focused_pane()
            .unwrap()
            .id();
        state.closing_terminals.insert(pane_id);
        state.pending_terminal_bells.insert(pane_id);
        (state.server_id, state.session_id, pane_id)
    };

    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Focus(true),
    );
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Ping {
            server_id,
            nonce: 7,
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::Pong { nonce: 7, .. }
    ));
    {
        let state = handle.state.lock().unwrap();
        assert_eq!(state.focused_terminal.map(|(pane, _)| pane), Some(pane_id));
        assert!(!state.pending_terminal_bells.contains(&pane_id));
    }

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

/// Closing a Tab from the GUI races its focus bookkeeping: the Pane is gone on the
/// Server before the client's `Focus(false)` for it arrives. That is not an error worth
/// showing; typing into a missing Pane still is.
#[test]
fn focus_changes_for_a_missing_pane_are_ignored_but_input_is_rejected() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let (server_id, session_id) = {
        let state = handle.state.lock().unwrap();
        (state.server_id, state.session_id)
    };
    let missing = PaneId::from_u64(424_242);

    for focused in [false, true] {
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            missing,
            TerminalCommand::Focus(focused),
        );
    }
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Ping {
            server_id,
            nonce: 9,
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut stream),
        ServerMessage::Pong { nonce: 9, .. }
    ));

    send_terminal(
        &mut stream,
        server_id,
        session_id,
        missing,
        TerminalCommand::Text("ls".into()),
    );
    assert!(matches!(
        read_server(&mut stream),
        ServerMessage::Error { message } if message == "unknown Pane"
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

/// Without Session control every Client may focus a Pane: the last report wins, the
/// previous holder's late unfocus cannot take it back, and leaving gives it up (ADR 0036).
#[test]
fn focus_follows_the_last_client_and_ends_when_it_leaves() {
    let (handle, endpoint, thread) = start();
    let mut first = connect_and_bootstrap(&endpoint);
    let mut second = connect_and_bootstrap(&endpoint);
    let (server_id, session_id, pane_id) = {
        let mut state = handle.state.lock().unwrap();
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = state.session.workspaces()[0].tabs()[0]
            .focused_pane()
            .unwrap()
            .id();
        // Focus is recorded for a closing Pane without a PTY behind it.
        state.closing_terminals.insert(pane_id);
        (state.server_id, state.session_id, pane_id)
    };
    let focus = |stream: &mut EndpointStream, focused: bool, nonce: u64| {
        send_terminal(
            stream,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Focus(focused),
        );
        condr_core::protocol::write_message(stream, &ClientMessage::Ping { server_id, nonce })
            .unwrap();
        assert!(matches!(
            read_server(stream),
            ServerMessage::Pong { nonce: answered, .. } if answered == nonce
        ));
    };
    let focused = || {
        handle
            .state
            .lock()
            .unwrap()
            .focused_terminal
            .map(|(pane, _)| pane)
    };

    focus(&mut first, true, 1);
    focus(&mut second, true, 2);
    focus(&mut first, false, 3);
    assert_eq!(focused(), Some(pane_id), "a stale unfocus is ignored");
    {
        let state = handle.state.lock().unwrap();
        assert_eq!(
            state.sizing_client,
            state.focused_terminal.map(|(_, client)| client),
            "the last Client to focus sizes the terminals"
        );
    }

    drop(second);
    let deadline = Instant::now() + Duration::from_secs(5);
    while focused().is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        focused(),
        None,
        "the holder's focus ends with its connection"
    );
    assert_eq!(handle.state.lock().unwrap().sizing_client, None);

    handle.stop();
    drop(first);
    thread.join().unwrap().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn the_focused_client_sizes_and_leaving_releases_its_mouse_before_its_focus() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let mut observer = connect_and_bootstrap(&endpoint);
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    subscribe(&mut stream, session_id, 0);
    subscribe(&mut observer, session_id, 0);

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            },
        },
    )
    .unwrap();
    let message = wait_for_message(&mut stream, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let ServerMessage::Event { sequence, .. } = message else {
        unreachable!("predicate only accepts LayoutChanged events");
    };
    assert_layout_applied(&mut stream, server_id, session_id, 1, sequence);
    let pane_id = handle.state.lock().unwrap().session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
        .id();
    let mut views = std::collections::HashMap::new();
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Resize(TerminalSize::new(8, 160)),
    );
    wait_for_terminal(&mut stream, &mut views, pane_id, |view| {
        view.size.columns == 160
    });

    // The focused Client sizes the terminal: another Client's resize is dropped. The
    // observer's echo is handled after its resize on the same connection.
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Focus(true),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while handle.state.lock().unwrap().sizing_client.is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    send_terminal(
        &mut observer,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Resize(TerminalSize::new(8, 120)),
    );
    send_terminal(
        &mut observer,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text("echo sizing-held\r".into()),
    );
    let mut observer_views = std::collections::HashMap::new();
    let view = wait_for_terminal_text(&mut observer, &mut observer_views, pane_id, "sizing-held");
    assert_eq!(view.size.columns, 160);

    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text(
            "stty raw -echo; printf 'viewer-cleanup-ready\\r\\n\\033[?1002h\\033[?1006h\\033[?1004h'; bytes=$(dd bs=1 count=36 2>/dev/null | od -An -tx1 | tr -d ' \\n'); printf '\\033[?1002l\\033[?1006l\\033[?1004l'; stty sane; printf '\\r\\nviewer-cleanup-bytes_%s\\r\\n' \"$bytes\"\r"
                .into(),
        ),
    );
    wait_for_terminal(&mut stream, &mut views, pane_id, |view| {
        view.mouse_tracking == condr_core::TerminalMouseTracking::Drag
            && view_text(view).contains("viewer-cleanup-ready")
    });

    let press_position = TerminalMousePosition { row: 3, column: 7 };
    let motion_position = TerminalMousePosition { row: 4, column: 9 };
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Focus(true),
    );
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Mouse(TerminalMouseEvent::Button {
            button: TerminalMouseButton::Left,
            pressed: true,
            position: press_position,
            modifiers: TerminalModifiers::default(),
        }),
    );
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Mouse(TerminalMouseEvent::Motion {
            button: Some(TerminalMouseButton::Left),
            position: motion_position,
            modifiers: TerminalModifiers {
                alt: true,
                ..TerminalModifiers::default()
            },
        }),
    );
    // Leaving gives all of it up; the other Client sees the Pane hear it, then sizes it.
    drop(stream);
    let expected = "viewer-cleanup-bytes_1b5b491b5b3c303b383b344d1b5b3c34303b31303b354d1b5b3c383b31303b356d1b5b4f";
    wait_for_terminal_text(&mut observer, &mut observer_views, pane_id, expected);
    send_terminal(
        &mut observer,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Resize(TerminalSize::new(8, 120)),
    );
    wait_for_terminal(&mut observer, &mut observer_views, pane_id, |view| {
        view.size.columns == 120
    });

    handle.stop();
    drop(observer);
    thread.join().unwrap().unwrap();
}
