use super::*;
#[cfg(target_os = "linux")]
use condr_core::{
    TerminalModifiers, TerminalMouseButton, TerminalMouseEvent, TerminalMousePosition,
};

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn controller_can_acknowledge_attention_while_terminal_is_closing() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let (server_id, session_id, pane_id) = {
        let mut state = handle.state.lock().unwrap();
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .unwrap()
            .id();
        state.closing_terminals.insert(pane_id);
        state.pending_terminal_bells.insert(pane_id);
        (state.server_id, state.session_id, pane_id)
    };
    acquire_control(&mut stream, session_id);

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
        assert_eq!(state.focused_terminal, Some(pane_id));
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
    acquire_control(&mut stream, session_id);
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

#[test]
fn controller_is_exclusive_and_released_on_disconnect() {
    let (handle, endpoint, thread) = start();
    let mut first = connect_and_bootstrap(&endpoint);
    let mut second = connect_and_bootstrap(&endpoint);
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    acquire_control(&mut first, session_id);
    condr_core::protocol::write_message(&mut second, &ClientMessage::AcquireControl { session_id })
        .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut second).unwrap(),
        ServerMessage::ControlDenied { .. }
    ));
    condr_core::protocol::write_message(
        &mut second,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 73,
            command: LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
                root_directory: std::env::temp_dir(),
            },
        },
    )
    .unwrap();
    // Control is exclusive, but layout is not gated on it: the denied client still
    // changes structure, as the CLI in a Pane does while the GUI holds control.
    // The response follows the events the change raised on this subscribed stream.
    let applied = std::iter::from_fn(|| {
        Some(condr_core::protocol::read_message::<_, ServerMessage>(&mut second).unwrap())
    })
    .find(|message| !matches!(message, ServerMessage::Event { .. }))
    .unwrap();
    assert!(
        matches!(
            applied,
            ServerMessage::LayoutApplied {
                server_id: applied_server,
                session_id: applied_session,
                request_id: 73,
                ..
            } if applied_server == server_id && applied_session == session_id
        ),
        "{applied:?}"
    );
    drop(first);
    thread::sleep(Duration::from_millis(20));
    acquire_control(&mut second, session_id);
    handle.stop();
    drop(second);
    thread.join().unwrap().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn releasing_control_releases_reported_mouse_before_focus() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    acquire_control(&mut stream, session_id);
    subscribe(&mut stream, session_id, 0);

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
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
    let pane_id = handle
        .state
        .lock()
        .unwrap()
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
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
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text(
            "stty raw -echo; printf 'controller-cleanup-ready\\r\\n\\033[?1002h\\033[?1006h\\033[?1004h'; bytes=$(dd bs=1 count=36 2>/dev/null | od -An -tx1 | tr -d ' \\n'); printf '\\033[?1002l\\033[?1006l\\033[?1004l'; stty sane; printf '\\r\\ncontroller-cleanup-bytes_%s\\r\\n' \"$bytes\"\r"
                .into(),
        ),
    );
    wait_for_terminal(&mut stream, &mut views, pane_id, |view| {
        view.mouse_tracking == condr_core::TerminalMouseTracking::Drag
            && view_text(view).contains("controller-cleanup-ready")
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
    condr_core::protocol::write_message(&mut stream, &ClientMessage::ReleaseControl { session_id })
        .unwrap();
    assert!(matches!(
        wait_for_message(&mut stream, |message| matches!(
            message,
            ServerMessage::ControlReleased { .. }
        )),
        ServerMessage::ControlReleased {
            server_id: released_server,
            session_id: released_session,
        } if released_server == server_id && released_session == session_id
    ));

    let expected = "controller-cleanup-bytes_1b5b491b5b3c303b383b344d1b5b3c34303b31303b354d1b5b3c383b31303b356d1b5b4f";
    wait_for_terminal_text(&mut stream, &mut views, pane_id, expected);

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}
