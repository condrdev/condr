//! `WatchTerminals` (ADR 0041): a Client limits its terminal frames to some Panes, while
//! every Pane's record and every reliable event still reach it.

use super::*;
use condr_core::protocol::PaneTerminalFrame;

/// Two Panes whose terminals are views without a PTY behind them: the first titled
/// `build`, the second exited and ringing.
fn two_panes(handle: &ServerHandle) -> (PaneId, PaneId) {
    let mut state = handle.state.lock().unwrap();
    for _ in 0..2 {
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
    }
    let panes = state
        .session
        .workspaces()
        .iter()
        .map(|workspace| workspace.tabs()[0].focused_pane().unwrap().id())
        .collect::<Vec<_>>();
    for &pane_id in &panes {
        state
            .terminal_views
            .insert(pane_id, latest_terminal_view(screen(1)));
        state.closing_terminals.insert(pane_id);
    }
    state.terminal_titles.insert(panes[0], "build".into());
    state.exited_terminals.insert(panes[1]);
    state.pending_terminal_bells.insert(panes[1]);
    (panes[0], panes[1])
}

/// One row whose last cell names the revision, so the next one is a delta.
fn screen(revision: u64) -> TerminalView {
    terminal_test_view(revision, &format!("abc{revision}"))
}

/// What a terminal's monitor does with a new screen: keep it as the latest and render it
/// for every Client that watches the Pane.
fn publish(handle: &ServerHandle, pane_id: PaneId, revision: u64) {
    let view = screen(revision);
    let clients = handle
        .state
        .lock()
        .unwrap()
        .publish_terminal(pane_id, TerminalViewFrame::Full(view))
        .expect("a full frame is kept");
    for client_id in clients {
        flush_terminal_render(&handle.state, client_id);
    }
}

fn connect_watching(endpoint: &Endpoint, panes: &[PaneId]) -> (EndpointStream, SessionBootstrap) {
    let connection = ClientConnection::connect_watching(
        endpoint,
        "phone",
        panes.iter().copied().collect(),
        crate::ConnectionCancellation::default(),
    )
    .unwrap();
    let bootstrap = connection.bootstrap().unwrap().clone();
    let stream = connection.into_stream();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    (stream, bootstrap)
}

fn watch(stream: &mut EndpointStream, panes: &[PaneId]) {
    condr_core::protocol::write_message(
        stream,
        &ClientMessage::WatchTerminals {
            panes: panes.iter().copied().collect(),
        },
    )
    .unwrap();
}

/// Waits for the Server to have read everything sent before; frames are not waited for.
fn round_trip(stream: &mut EndpointStream, server_id: ServerId) {
    condr_core::protocol::write_message(
        stream,
        &ClientMessage::Ping {
            server_id,
            nonce: 77,
        },
    )
    .unwrap();
    wait_for_message(stream, |message| {
        assert!(
            !matches!(message, ServerMessage::TerminalFrame(_)),
            "no frame is due: {message:?}"
        );
        matches!(message, ServerMessage::Pong { nonce: 77, .. })
    });
}

fn next_frames(stream: &mut EndpointStream) -> Vec<PaneTerminalFrame> {
    match wait_for_message(stream, |message| {
        matches!(message, ServerMessage::TerminalFrame(_))
    }) {
        ServerMessage::TerminalFrame(batch) => batch.panes,
        _ => unreachable!(),
    }
}

fn only_client(handle: &ServerHandle) -> u64 {
    *handle
        .state
        .lock()
        .unwrap()
        .watched_terminals
        .keys()
        .next()
        .expect("one Client sent a set")
}

#[test]
fn a_client_watching_nothing_gets_events_and_no_frames() {
    let (handle, endpoint, thread) = start();
    let (first, second) = two_panes(&handle);
    let (mut stream, bootstrap) = connect_watching(&endpoint, &[]);
    subscribe(&mut stream, bootstrap.session_id, bootstrap.sequence);
    let client_id = only_client(&handle);

    publish(&handle, first, 2);
    publish(&handle, second, 2);
    {
        let state = handle.state.lock().unwrap();
        let subscriber = &state.subscribers[&client_id];
        assert!(subscriber.pending_terminals.is_empty(), "nothing to render");
        assert!(subscriber.terminal_baselines.is_empty());
    }
    handle
        .state
        .lock()
        .unwrap()
        .publish_background(SessionEvent::TerminalTitleChanged {
            pane_id: second,
            title: Some("tests".into()),
        });
    let event = wait_for_message(&mut stream, |message| {
        assert!(
            !matches!(message, ServerMessage::TerminalFrame(_)),
            "a frame reached a Client watching nothing"
        );
        matches!(message, ServerMessage::Event { .. })
    });
    assert!(matches!(
        event,
        ServerMessage::Event {
            event: SessionEvent::TerminalTitleChanged { pane_id, .. },
            ..
        } if pane_id == second
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn a_pane_added_to_the_set_arrives_whole_and_then_as_deltas() {
    let (handle, endpoint, thread) = start();
    let (first, second) = two_panes(&handle);
    let (mut stream, bootstrap) = connect_watching(&endpoint, &[]);
    subscribe(&mut stream, bootstrap.session_id, bootstrap.sequence);
    publish(&handle, first, 2);

    watch(&mut stream, &[first]);
    let frames = next_frames(&mut stream);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pane_id, first);
    assert!(
        matches!(&frames[0].frame, TerminalViewFrame::Full(view) if view.revision == 2),
        "{frames:?}"
    );

    publish(&handle, second, 3);
    publish(&handle, first, 3);
    let frames = next_frames(&mut stream);
    assert_eq!(frames.len(), 1, "only the watched Pane: {frames:?}");
    assert_eq!(frames[0].pane_id, first);
    assert!(matches!(
        &frames[0].frame,
        TerminalViewFrame::Delta(delta) if delta.base_revision == 2 && delta.revision == 3
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn a_removed_pane_stops_and_loses_its_baseline_while_a_kept_one_goes_on_with_deltas() {
    let (handle, endpoint, thread) = start();
    let (first, second) = two_panes(&handle);
    let (mut stream, bootstrap) = connect_watching(&endpoint, &[first, second]);
    subscribe(&mut stream, bootstrap.session_id, bootstrap.sequence);
    let client_id = only_client(&handle);
    publish(&handle, first, 2);
    publish(&handle, second, 2);
    let mut seen = std::collections::HashSet::new();
    while seen.len() < 2 {
        for frame in next_frames(&mut stream) {
            assert!(matches!(frame.frame, TerminalViewFrame::Full(_)));
            seen.insert(frame.pane_id);
        }
    }

    watch(&mut stream, &[first]);
    round_trip(&mut stream, bootstrap.server_id);
    {
        let state = handle.state.lock().unwrap();
        let subscriber = &state.subscribers[&client_id];
        assert!(subscriber.terminal_baselines.contains_key(&first));
        assert!(!subscriber.terminal_baselines.contains_key(&second));
    }
    publish(&handle, second, 3);
    publish(&handle, first, 3);
    let frames = next_frames(&mut stream);
    assert_eq!(frames.len(), 1, "only the kept Pane: {frames:?}");
    assert_eq!(frames[0].pane_id, first);
    assert!(matches!(
        &frames[0].frame,
        TerminalViewFrame::Delta(delta) if delta.base_revision == 2 && delta.revision == 3
    ));

    // Its baseline gone, the Pane that comes back starts from a full frame.
    watch(&mut stream, &[first, second]);
    let frames = next_frames(&mut stream);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pane_id, second);
    assert!(matches!(&frames[0].frame, TerminalViewFrame::Full(view) if view.revision == 3));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn a_set_sent_before_the_snapshot_request_limits_the_views_but_not_the_records() {
    let (handle, endpoint, thread) = start();
    let (first, second) = two_panes(&handle);

    let (stream, bootstrap) = connect_watching(&endpoint, &[first]);
    let record = |pane_id| {
        bootstrap
            .terminals
            .iter()
            .find(|terminal| terminal.pane_id == pane_id)
            .expect("every Pane has a record")
    };
    let watched = record(first);
    assert_eq!(view_text(watched.view.as_ref().unwrap()), "abc1");
    assert_eq!(watched.title.as_deref(), Some("build"));
    assert!(!watched.exited && !watched.attention);
    let unwatched = record(second);
    assert!(unwatched.view.is_none());
    assert!(unwatched.exited && unwatched.attention);

    // A Client that never sends a set gets every Pane's view, as before.
    let desktop = ClientConnection::connect(&endpoint, "desktop").unwrap();
    assert!(
        desktop
            .bootstrap()
            .unwrap()
            .terminals
            .iter()
            .all(|terminal| terminal.view.is_some())
    );

    handle.stop();
    drop(stream);
    drop(desktop);
    thread.join().unwrap().unwrap();
}

#[test]
fn a_set_changed_during_a_bootstrap_applies_when_it_finishes() {
    let endpoint = test_endpoint();
    let mut state = RuntimeState::new(endpoint.as_local_path().unwrap());
    for _ in 0..2 {
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
    }
    let [first, second] = [0, 1].map(|index| {
        state.session.workspaces()[index].tabs()[0]
            .focused_pane()
            .unwrap()
            .id()
    });
    for pane_id in [first, second] {
        state
            .terminal_views
            .insert(pane_id, latest_terminal_view(screen(1)));
        state.closing_terminals.insert(pane_id);
    }
    let (writer, receiver) = ClientWriter::channel();
    state.watch_terminals(7, [first].into());
    state.ensure_subscriber(7, writer);

    let capture = state.capture_bootstrap(state.watched_terminals.get(&7));
    state.begin_bootstrap(7);
    state.watch_terminals(7, [second].into());
    assert!(
        state.subscribers[&7].pending_terminals.is_empty(),
        "nothing renders before the Bootstrap is out"
    );

    let framed = frame_bootstrap_messages(capture.materialize()).unwrap();
    assert_eq!(
        framed.baselines.len(),
        1,
        "the Bootstrap carried the old set"
    );
    state.finish_bootstrap(7, framed).unwrap();
    let subscriber = &state.subscribers[&7];
    assert!(
        subscriber.terminal_baselines.is_empty(),
        "the Pane that left lost the baseline the Bootstrap gave it"
    );
    assert_eq!(
        subscriber.pending_terminals,
        [second].into(),
        "the Pane that joined waits for a full frame"
    );

    let state = Arc::new(Mutex::new(state));
    flush_terminal_render(&state, 7);
    assert!(matches!(
        receiver.recv(),
        Some(ClientWriteItem::ReliableBatch(_))
    ));
    let Some(ClientWriteItem::Render { data, .. }) = receiver.recv() else {
        panic!("the joined Pane renders once the Bootstrap is out");
    };
    let message: ServerMessage = condr_core::protocol::read_message(&mut data.as_slice()).unwrap();
    assert!(matches!(
        message,
        ServerMessage::TerminalFrame(batch)
            if batch.panes.len() == 1
                && batch.panes[0].pane_id == second
                && matches!(batch.panes[0].frame, TerminalViewFrame::Full(_))
    ));
}

#[test]
fn the_set_survives_a_rejected_subscription_and_the_resubscription() {
    let (handle, endpoint, thread) = start();
    let (first, second) = two_panes(&handle);
    let (mut stream, bootstrap) = connect_watching(&endpoint, &[first]);
    let session_id = bootstrap.session_id;
    subscribe(&mut stream, session_id, bootstrap.sequence);
    let client_id = only_client(&handle);

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Subscribe {
            session_id,
            after_sequence: u64::MAX,
        },
    )
    .unwrap();
    wait_for_message(&mut stream, |message| {
        matches!(message, ServerMessage::SubscriptionRejected { .. })
    });
    {
        let state = handle.state.lock().unwrap();
        assert!(state.subscribers.is_empty());
        assert_eq!(state.watched_terminals[&client_id], [first].into());
    }

    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::SnapshotRequest { session_id },
    )
    .unwrap();
    let ServerMessage::Bootstrap(header) = wait_for_message(&mut stream, |message| {
        matches!(message, ServerMessage::Bootstrap(_))
    }) else {
        unreachable!()
    };
    let bootstrap = condr_client::frames::read_bootstrap_batches(&mut stream, header).unwrap();
    for terminal in &bootstrap.terminals {
        assert_eq!(terminal.view.is_some(), terminal.pane_id == first);
    }
    subscribe(&mut stream, session_id, bootstrap.sequence);
    publish(&handle, second, 2);
    publish(&handle, first, 2);
    let frames = next_frames(&mut stream);
    assert_eq!(frames.len(), 1, "only the watched Pane: {frames:?}");
    assert_eq!(frames[0].pane_id, first);

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}
