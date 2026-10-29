//! A panic on a thread that holds no lock ends only the work it was doing (ADR 0053).

use super::*;
use condr_core::{AgentKind, AgentState};

/// A connected, subscribed Client and the Pane of the live Workspace it created.
fn live_pane(
    endpoint: &Endpoint,
    handle: &ServerHandle,
) -> (EndpointStream, ServerId, SessionId, PaneId) {
    let connection = ClientConnection::connect(endpoint, "panics").unwrap();
    let bootstrap = connection.bootstrap().unwrap().clone();
    let (server_id, session_id) = (bootstrap.server_id, bootstrap.session_id);
    let mut stream = connection.into_stream();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    subscribe(&mut stream, session_id, bootstrap.sequence);
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
    wait_for_message(&mut stream, |message| {
        matches!(message, ServerMessage::LayoutApplied { request_id: 1, .. })
    });
    let pane_id = handle.state.lock().unwrap().session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
        .id();
    (stream, server_id, session_id, pane_id)
}

#[test]
fn a_monitor_that_panics_ends_its_pane() {
    let (handle, endpoint, thread) = start();
    let (mut stream, server_id, session_id, pane_id) = live_pane(&endpoint, &handle);
    handle
        .state
        .lock()
        .unwrap()
        .injected_panics
        .insert(("monitor", pane_id.as_u64()));

    // Output wakes the monitor, which panics on its next frame.
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text("echo condr-wake\r".into()),
    );
    wait_for_message(&mut stream, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::TerminalExited { pane_id: exited },
                ..
            } if *exited == pane_id
        )
    });
    let after = ClientConnection::connect(&endpoint, "after-panic").unwrap();
    assert!(
        after
            .bootstrap()
            .unwrap()
            .terminals
            .iter()
            .any(|terminal| terminal.pane_id == pane_id && terminal.exited),
        "a new Client sees the Pane ended"
    );

    handle.stop();
    drop((stream, after));
    thread.join().unwrap().unwrap();
}

#[test]
fn a_probe_that_panics_leaves_its_pane_running_with_its_agent_unknown() {
    let (handle, endpoint, thread) = start();
    let (mut stream, server_id, session_id, pane_id) = live_pane(&endpoint, &handle);
    {
        let mut state = handle.state.lock().unwrap();
        state.agents.insert(
            pane_id,
            AgentSnapshot {
                session_id: None,
                kind: AgentKind::Claude,
                state: AgentState::Working,
                detail: None,
            },
        );
        state.injected_panics.insert(("probe", pane_id.as_u64()));
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    while handle.state.lock().unwrap().agents[&pane_id].state != AgentState::Unknown {
        assert!(Instant::now() < deadline, "the Agent never turned Unknown");
        thread::sleep(Duration::from_millis(20));
    }
    send_terminal(
        &mut stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text("echo condr-alive\r".into()),
    );
    let mut views = std::collections::HashMap::new();
    wait_for_terminal(&mut stream, &mut views, pane_id, |view| {
        view_text(view).contains("condr-alive")
    });

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn a_request_that_panics_is_answered_with_an_error() {
    let (handle, endpoint, thread) = start();
    let connection = ClientConnection::connect(&endpoint, "panics").unwrap();
    let bootstrap = connection.bootstrap().unwrap().clone();
    let mut stream = connection.into_stream();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let workspace_id = {
        let mut state = handle.state.lock().unwrap();
        let workspace_id = state
            .session
            .create_workspace(std::env::temp_dir())
            .unwrap();
        state
            .injected_panics
            .insert(("git-log", workspace_id.as_u64()));
        workspace_id
    };
    let log = |stream: &mut EndpointStream, request_id| {
        condr_core::protocol::write_message(
            stream,
            &ClientMessage::GitLog {
                server_id: bootstrap.server_id,
                session_id: bootstrap.session_id,
                request_id,
                workspace_id,
                from: None,
                skip: 0,
                limit: 10,
            },
        )
        .unwrap();
        let reply = wait_for_message(
            stream,
            |message| matches!(message, ServerMessage::GitLog { request_id: id, .. } if *id == request_id),
        );
        let ServerMessage::GitLog { result, .. } = reply else {
            unreachable!("the predicate only accepts GitLog");
        };
        result
    };

    let failed = log(&mut stream, 1).unwrap_err();
    assert!(failed.contains("internal error"), "{failed}");
    // The next request is answered as it always was.
    assert!(log(&mut stream, 2).is_err_and(|reason| !reason.contains("internal error")));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}
