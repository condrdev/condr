use super::*;

struct PeerRegistration {
    state: Arc<Mutex<RuntimeState>>,
    client_id: u64,
}

impl Drop for PeerRegistration {
    fn drop(&mut self) {
        self.state
            .lock()
            .expect("server state lock poisoned")
            .tcp_peers
            .remove(&self.client_id);
    }
}

pub(super) fn handle_client(
    mut stream: EndpointStream,
    client_id: u64,
    state: Arc<Mutex<RuntimeState>>,
    stop: Arc<AtomicBool>,
    lifecycle: Arc<ServerLifecycle>,
) {
    if stream
        .set_handshake_timeout(Some(HANDSHAKE_TIMEOUT))
        .is_err()
    {
        return;
    }
    let hello = match condr_core::protocol::read_message::<_, ClientMessage>(&mut stream) {
        Ok(ClientMessage::Hello(hello)) => hello,
        Ok(_) => {
            let _ = send_error(&mut stream, &state, "expected Hello as first message");
            return;
        }
        Err(error) => {
            let _ = send_error(
                &mut stream,
                &state,
                &format!("invalid handshake frame: {error}"),
            );
            return;
        }
    };

    let (server_id, runtime_epoch, session_id) = {
        let state = state.lock().expect("server state lock poisoned");
        (state.server_id, state.runtime_epoch, state.session_id)
    };
    if let VersionCheck::Incompatible(reason) = check_version(hello.version) {
        let _ = send_message(
            &mut stream,
            &ServerMessage::Welcome {
                version: PROTOCOL_VERSION,
                server_id,
                runtime_epoch,
                session_id,
                error: Some(reason),
            },
        );
        return;
    }

    // Reading Hello proved the peer held its invite, if it used one; record the device
    // before it learns anything about this Server.
    if let Err(error) = stream.complete_pairing(&hello.client_name) {
        let _ = send_error(&mut stream, &state, &format!("pairing failed: {error}"));
        return;
    }
    // A TCP peer stays addressable by its device key until it leaves, so revoking that
    // key can close the connection from another client's thread. The store is read again
    // under the same lock the revocation scan takes: a device revoked between its
    // handshake and this point is refused here, one revoked later is found in the table.
    let mut peer_registration = None;
    if let Some(peer_key) = stream.peer_key() {
        let Ok(peer) = stream.try_clone() else { return };
        let mut state_guard = state.lock().expect("server state lock poisoned");
        match stream.peer_authorized() {
            Ok(true) => {
                state_guard.tcp_peers.insert(client_id, (peer_key, peer));
                peer_registration = Some(PeerRegistration {
                    state: Arc::clone(&state),
                    client_id,
                });
                drop(state_guard);
                // Best effort: a failed bookkeeping write must not cost the connection.
                let _ = stream.record_peer_seen();
            }
            Ok(false) => {
                drop(state_guard);
                let _ = send_error(&mut stream, &state, "device revoked");
                return;
            }
            Err(error) => {
                drop(state_guard);
                let _ = send_error(
                    &mut stream,
                    &state,
                    &format!("authorization check failed: {error}"),
                );
                return;
            }
        }
    }

    if send_message(
        &mut stream,
        &ServerMessage::Welcome {
            version: PROTOCOL_VERSION,
            server_id,
            runtime_epoch,
            session_id,
            error: None,
        },
    )
    .is_err()
    {
        return;
    }
    let _ = stream.set_handshake_timeout(None);

    let mut writer_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    };
    let lag_notice = frame_message(&ServerMessage::SubscriptionRejected {
        server_id,
        session_id,
        reason: "client fell behind the reliable event stream".into(),
    })
    .expect("lag notice must fit a protocol frame");
    let (outbound, outbound_rx) = ClientWriter::channel_with_lag_notice(lag_notice);
    let writer_state = Arc::downgrade(&state);
    let writer = thread::spawn(move || {
        while let Some(item) = outbound_rx.recv() {
            let result = match item {
                ClientWriteItem::Reliable(data) => send_framed(&mut writer_stream, &data),
                ClientWriteItem::ReliableBatch(frames) => frames
                    .into_iter()
                    .try_for_each(|data| send_framed(&mut writer_stream, &data)),
                ClientWriteItem::ClosingReliable { data, delivered } => {
                    let result = send_framed(&mut writer_stream, &data);
                    let _ = delivered.send(result.is_ok());
                    break;
                }
                ClientWriteItem::Render { data, slot_drained } => {
                    if slot_drained && let Some(state) = writer_state.upgrade() {
                        flush_terminal_render(&state, client_id);
                    }
                    send_framed(&mut writer_stream, &data)
                }
            };
            if result.is_err() {
                break;
            }
        }
        outbound_rx.close();
    });

    let mut stopping_server = false;
    loop {
        let message = match condr_core::protocol::read_message_with_limit::<_, ClientMessage>(
            &mut stream,
            condr_core::protocol::MAX_IMAGE_FRAME_SIZE,
        ) {
            // Only an image may use the large allowance; anything else that size is a fault.
            Ok((message, size)) if size <= message.frame_limit() => message,
            Ok((_, size)) => {
                let _ = queue_message(
                    &outbound,
                    ServerMessage::Error {
                        message: format!(
                            "invalid client frame: {size} bytes exceeds the {} byte limit",
                            condr_core::protocol::MAX_FRAME_SIZE
                        ),
                    },
                );
                break;
            }
            Err(error @ (FramingError::Oversized { .. } | FramingError::Codec(_))) => {
                let _ = queue_message(
                    &outbound,
                    ServerMessage::Error {
                        message: format!("invalid client frame: {error}"),
                    },
                );
                break;
            }
            Err(_) => break,
        };
        let mut started_terminals = Vec::new();
        let mut removed_terminals = Vec::new();
        let should_close = match message {
            ClientMessage::SnapshotRequest { session_id } => {
                queue_runtime_bootstrap(&state, client_id, session_id, &outbound)
            }
            ClientMessage::OverviewRequest { session_id } => {
                let response = {
                    let state = state.lock().expect("server state lock poisoned");
                    if session_id == state.session_id {
                        ServerMessage::Overview(SessionOverview {
                            server_id: state.server_id,
                            runtime_epoch: state.runtime_epoch,
                            session_id: state.session_id,
                            sequence: state.sequence,
                            snapshot: state.session.snapshot(),
                            terminals: state
                                .terminals
                                .keys()
                                .filter(|pane_id| !state.closing_terminals.contains(pane_id))
                                .map(|pane_id| PaneTerminalMetadata {
                                    pane_id: *pane_id,
                                    title: state.terminal_titles.get(pane_id).cloned(),
                                    exited: state.exited_terminals.contains(pane_id),
                                })
                                .collect(),
                            agents: state
                                .agents
                                .iter()
                                .filter(|(pane_id, _)| {
                                    !state.closing_terminals.contains(pane_id)
                                        && !state.exited_terminals.contains(pane_id)
                                })
                                .map(|(pane_id, agent)| PaneAgentSnapshot {
                                    pane_id: *pane_id,
                                    agent: *agent,
                                })
                                .collect(),
                            zoomed_panes: state.zoomed_panes(),
                        })
                    } else {
                        ServerMessage::Error {
                            message: "unknown Session".into(),
                        }
                    }
                };
                match response {
                    ServerMessage::Overview(overview) => match frame_overview_messages(overview) {
                        Ok(frames) => {
                            outbound.send_reliable_batch(frames)
                                == Err(ReliableSendError::Disconnected)
                        }
                        Err(error) => queue_message(
                            &outbound,
                            ServerMessage::Error {
                                message: format!("cannot encode Session overview: {error}"),
                            },
                        ),
                    },
                    response => queue_message(&outbound, response),
                }
            }
            ClientMessage::Subscribe {
                session_id,
                after_sequence,
            } => {
                let shared_state = Arc::clone(&state);
                let mut state = state.lock().expect("server state lock poisoned");
                let (responses, should_subscribe) = {
                    if session_id != state.session_id {
                        (
                            vec![ServerMessage::SubscriptionRejected {
                                server_id: state.server_id,
                                session_id: state.session_id,
                                reason: "unknown Session".into(),
                            }],
                            false,
                        )
                    } else if after_sequence > state.sequence {
                        (
                            vec![ServerMessage::SubscriptionRejected {
                                server_id: state.server_id,
                                session_id: state.session_id,
                                reason: "event cursor is ahead of the server".into(),
                            }],
                            false,
                        )
                    } else if state
                        .events
                        .front()
                        .is_some_and(|event| after_sequence.saturating_add(1) < event.sequence)
                    {
                        (
                            vec![ServerMessage::SubscriptionRejected {
                                server_id: state.server_id,
                                session_id: state.session_id,
                                reason: "event cursor expired".into(),
                            }],
                            false,
                        )
                    } else {
                        let mut responses = state
                            .events
                            .iter()
                            .filter(|event| event.sequence > after_sequence)
                            .map(|event| ServerMessage::Event {
                                server_id: state.server_id,
                                session_id: state.session_id,
                                sequence: event.sequence,
                                event: event.event.clone(),
                            })
                            .collect::<Vec<_>>();
                        responses.push(ServerMessage::Subscribed {
                            server_id: state.server_id,
                            session_id: state.session_id,
                            sequence: state.sequence,
                        });
                        (responses, true)
                    }
                };
                if should_subscribe {
                    state.ensure_subscriber(client_id, outbound.clone());
                } else if let Some(subscriber) = state.subscribers.remove(&client_id) {
                    subscriber.writer.clear_render();
                }
                let queued = responses
                    .into_iter()
                    .try_for_each(|response| queue_response(&outbound, response));
                if queued.is_err() {
                    // Lagged or dead: either way this subscription did not reach the client.
                    state.subscribers.remove(&client_id);
                }
                let should_flush = should_subscribe && queued.is_ok();
                drop(state);
                if should_flush {
                    flush_terminal_render(&shared_state, client_id);
                }
                queued == Err(ReliableSendError::Disconnected)
            }
            ClientMessage::Ping { server_id, nonce } => {
                let state = state.lock().expect("server state lock poisoned");
                if server_id != state.server_id {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "unknown Server".into(),
                        },
                    )
                } else {
                    queue_message(
                        &outbound,
                        ServerMessage::Pong {
                            server_id,
                            nonce,
                            sequence: state.sequence,
                        },
                    )
                }
            }
            // Control changes commit only once their response is queued: a response lost to
            // writer lag must not leave the client believing the opposite of the Server.
            ClientMessage::AcquireControl { session_id } => {
                let mut state = state.lock().expect("server state lock poisoned");
                let (response, grant) = if session_id != state.session_id {
                    (
                        ServerMessage::ControlDenied {
                            server_id: state.server_id,
                            session_id,
                            reason: "unknown Session".into(),
                        },
                        false,
                    )
                } else if state.active_controller.is_none()
                    || state.active_controller == Some(client_id)
                {
                    (
                        ServerMessage::ControlGranted {
                            server_id: state.server_id,
                            session_id,
                        },
                        true,
                    )
                } else {
                    (
                        ServerMessage::ControlDenied {
                            server_id: state.server_id,
                            session_id,
                            reason: "another client controls this Session".into(),
                        },
                        false,
                    )
                };
                match queue_response(&outbound, response) {
                    Ok(()) => {
                        if grant {
                            state.active_controller = Some(client_id);
                        }
                        false
                    }
                    Err(ReliableSendError::Lagged) => false,
                    Err(ReliableSendError::Disconnected) => true,
                }
            }
            ClientMessage::ReleaseControl { session_id } => {
                let mut state = state.lock().expect("server state lock poisoned");
                let (response, release) = if state.session_id != session_id {
                    (
                        ServerMessage::Error {
                            message: "unknown Session".into(),
                        },
                        false,
                    )
                } else if state.active_controller == Some(client_id) {
                    (
                        ServerMessage::ControlReleased {
                            server_id: state.server_id,
                            session_id,
                        },
                        true,
                    )
                } else {
                    (
                        ServerMessage::ControlDenied {
                            server_id: state.server_id,
                            session_id,
                            reason: "client does not control this Session".into(),
                        },
                        false,
                    )
                };
                match queue_response(&outbound, response) {
                    Ok(()) => {
                        if release {
                            state.clear_controller_terminal_state();
                            state.active_controller = None;
                        }
                        false
                    }
                    Err(ReliableSendError::Lagged) => false,
                    Err(ReliableSendError::Disconnected) => true,
                }
            }
            ClientMessage::Layout {
                server_id,
                session_id,
                request_id,
                command,
            } => {
                let external = matches!(
                    command,
                    LayoutCommand::CreateWorkspace { .. }
                        | LayoutCommand::CreateWorktree { .. }
                        | LayoutCommand::OpenWorktree { .. }
                        | LayoutCommand::RemoveWorktree { .. }
                );
                if external {
                    let operation = lifecycle.begin_operation();
                    let launch = state
                        .lock()
                        .expect("server state lock poisoned")
                        .shell_launch();
                    let plan = if operation.is_some() {
                        let mut state = state.lock().expect("server state lock poisoned");
                        plan_client_external_layout(
                            &mut state,
                            client_id,
                            server_id,
                            session_id,
                            request_id,
                            lifecycle.is_stopping(),
                            &command,
                        )
                    } else {
                        let state = state.lock().expect("server state lock poisoned");
                        Err(Box::new(ServerMessage::LayoutRejected {
                            server_id: state.server_id,
                            session_id: state.session_id,
                            request_id,
                            reason: "Server is stopping".into(),
                        }))
                    };
                    match plan.and_then(|plan| {
                        prepare_external_layout(plan).map_err(|reason| {
                            Box::new(ServerMessage::LayoutRejected {
                                server_id,
                                session_id,
                                request_id,
                                reason,
                            })
                        })
                    }) {
                        Err(message) => queue_message(&outbound, *message),
                        Ok(mut prepared) => {
                            let probes = {
                                let state = state.lock().expect("server state lock poisoned");
                                if let Some(message) = layout_authority_error(
                                    &state,
                                    client_id,
                                    server_id,
                                    session_id,
                                    request_id,
                                    lifecycle.is_stopping(),
                                ) {
                                    Err(Box::new(message))
                                } else if let PreparedExternalLayout::RemoveWorktreeReady {
                                    workspace_id,
                                    ..
                                } = &prepared
                                {
                                    Ok(state.terminal_cwd_probes_for_workspace(*workspace_id))
                                } else {
                                    Ok(Vec::new())
                                }
                            };
                            let approval = probes.and_then(|probes| {
                                let observations = observe_terminal_cwds(probes);
                                let mut state = state.lock().expect("server state lock poisoned");
                                if let Some(message) = layout_authority_error(
                                    &state,
                                    client_id,
                                    server_id,
                                    session_id,
                                    request_id,
                                    lifecycle.is_stopping(),
                                ) {
                                    Err(Box::new(message))
                                } else {
                                    state.record_terminal_cwd_observations(observations);
                                    approve_external_layout(&mut state, &mut prepared).map_err(
                                        |reason| {
                                            Box::new(ServerMessage::LayoutRejected {
                                                server_id: state.server_id,
                                                session_id: state.session_id,
                                                request_id,
                                                reason,
                                            })
                                        },
                                    )
                                }
                            });
                            match approval {
                                Err(message) => {
                                    let failed = queue_message(&outbound, *message);
                                    let cleanup_failed = cancel_prepared_external_layout(prepared)
                                        .is_err_and(|message| {
                                            eprintln!("condr-server: {message}");
                                            queue_message(
                                                &outbound,
                                                ServerMessage::Error { message },
                                            )
                                        });
                                    failed || cleanup_failed
                                }
                                Ok(()) => {
                                    match finish_external_layout(prepared, &launch) {
                                        Err(failure) => {
                                            let ExternalLayoutFinishError {
                                                message,
                                                restarted,
                                                exited,
                                                cwds,
                                            } = failure;
                                            let stopping = lifecycle.is_stopping();
                                            let clients = {
                                                let mut state = state
                                                    .lock()
                                                    .expect("server state lock poisoned");
                                                state.record_terminal_cwds(cwds);
                                                if stopping {
                                                    Vec::new()
                                                } else {
                                                    let mut cleared_agents = Vec::new();
                                                    for (pane_id, runtime, updates) in restarted {
                                                        if state.session.pane(pane_id).is_none()
                                                            || state
                                                                .terminals
                                                                .contains_key(&pane_id)
                                                        {
                                                            continue;
                                                        }
                                                        state.exited_terminals.remove(&pane_id);
                                                        if state.agents.remove(&pane_id).is_some() {
                                                            cleared_agents.push(pane_id);
                                                        }
                                                        started_terminals.push(
                                                            state.install_terminal(
                                                                pane_id, runtime, updates,
                                                            ),
                                                        );
                                                    }
                                                    for pane_id in cleared_agents {
                                                        state.publish_background(
                                                            SessionEvent::AgentChanged {
                                                                pane_id,
                                                                agent: None,
                                                            },
                                                        );
                                                    }
                                                    for (pane_id, runtime) in exited {
                                                        state.restore_exited_terminal(
                                                            pane_id, runtime,
                                                        );
                                                    }
                                                    state
                                                        .subscribers
                                                        .keys()
                                                        .copied()
                                                        .collect::<Vec<_>>()
                                                }
                                            };
                                            for client_id in clients {
                                                flush_terminal_render(&state, client_id);
                                            }
                                            queue_message(
                                                &outbound,
                                                ServerMessage::LayoutRejected {
                                                    server_id,
                                                    session_id,
                                                    request_id,
                                                    reason: message,
                                                },
                                            )
                                        }
                                        Ok(prepared) => {
                                            let mut state =
                                                state.lock().expect("server state lock poisoned");
                                            // A successful Git removal cannot be rolled back, so its
                                            // matching Session close must commit during shutdown.
                                            let stopping = lifecycle.is_stopping()
                                                && !matches!(
                                                    &prepared,
                                                    PreparedExternalLayout::RemoveWorktree { .. }
                                                );
                                            if let Some(message) = layout_authority_error(
                                                &state, client_id, server_id, session_id,
                                                request_id, stopping,
                                            ) {
                                                drop(state);
                                                let failed = queue_message(&outbound, message);
                                                let cleanup_failed =
                                                    cancel_prepared_external_layout(prepared)
                                                        .is_err_and(|message| {
                                                            eprintln!("condr-server: {message}");
                                                            queue_message(
                                                                &outbound,
                                                                ServerMessage::Error { message },
                                                            )
                                                        });
                                                failed || cleanup_failed
                                            } else {
                                                match apply_prepared_external_layout(
                                                    &mut state, prepared,
                                                ) {
                                                    Ok(effect) => {
                                                        started_terminals
                                                            .extend(effect.started_terminals);
                                                        removed_terminals =
                                                            effect.removed_terminals;
                                                        state.publish_layout_change(
                                                            client_id,
                                                            &outbound,
                                                            request_id,
                                                            effect.result,
                                                        )
                                                    }
                                                    Err(reason) => queue_message(
                                                        &outbound,
                                                        ServerMessage::LayoutRejected {
                                                            server_id: state.server_id,
                                                            session_id: state.session_id,
                                                            request_id,
                                                            reason,
                                                        },
                                                    ),
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    let probes = {
                        let state = state.lock().expect("server state lock poisoned");
                        if let Some(message) = layout_authority_error(
                            &state,
                            client_id,
                            server_id,
                            session_id,
                            request_id,
                            lifecycle.is_stopping(),
                        ) {
                            Err(message)
                        } else {
                            Ok(state.terminal_cwd_probes_for_layout(&command))
                        }
                    };
                    match probes {
                        Err(message) => queue_message(&outbound, message),
                        Ok(probes) => {
                            let observations = observe_terminal_cwds(probes);
                            let mut state = state.lock().expect("server state lock poisoned");
                            if let Some(message) = layout_authority_error(
                                &state,
                                client_id,
                                server_id,
                                session_id,
                                request_id,
                                lifecycle.is_stopping(),
                            ) {
                                queue_message(&outbound, message)
                            } else {
                                state.record_terminal_cwd_observations(observations);
                                match apply_layout_command(&mut state, command) {
                                    Ok(effect) => {
                                        started_terminals.extend(effect.started_terminals);
                                        removed_terminals = effect.removed_terminals;
                                        state.publish_layout_change(
                                            client_id,
                                            &outbound,
                                            request_id,
                                            effect.result,
                                        )
                                    }
                                    Err(reason) => queue_message(
                                        &outbound,
                                        ServerMessage::LayoutRejected {
                                            server_id: state.server_id,
                                            session_id: state.session_id,
                                            request_id,
                                            reason,
                                        },
                                    ),
                                }
                            }
                        }
                    }
                }
            }
            ClientMessage::Terminal {
                server_id,
                session_id,
                pane_id,
                command,
            } => {
                let mut state = state.lock().expect("server state lock poisoned");
                let is_copy = matches!(&command, TerminalCommand::Copy { .. });
                // Reading and typing are open to every client, the CLI in a Pane
                // included (ADR 0009); focus, mouse, resize, scroll and selection stay
                // with the controller because they describe one viewer's state.
                let needs_control = !matches!(
                    &command,
                    TerminalCommand::Copy { .. }
                        | TerminalCommand::Text(_)
                        | TerminalCommand::Paste(_)
                        | TerminalCommand::Key { .. }
                );
                let focus = match &command {
                    TerminalCommand::Focus(focused) => Some(*focused),
                    _ => None,
                };
                if server_id != state.server_id {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "unknown Server".into(),
                        },
                    )
                } else if session_id != state.session_id {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "unknown Session".into(),
                        },
                    )
                } else if lifecycle.is_stopping() {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "Server is stopping".into(),
                        },
                    )
                } else if needs_control && state.active_controller != Some(client_id) {
                    queue_message(
                        &outbound,
                        ServerMessage::ControlDenied {
                            server_id,
                            session_id,
                            reason: "acquire Session control before using a terminal".into(),
                        },
                    )
                } else if state.session.pane(pane_id).is_none() {
                    // A focus change for a Pane that is already gone is not a fault: the
                    // client tells us it stopped looking at a Tab it just closed, and the
                    // close event has simply not reached it yet.
                    if focus.is_some() {
                        false
                    } else {
                        queue_message(
                            &outbound,
                            ServerMessage::Error {
                                message: "unknown Pane".into(),
                            },
                        )
                    }
                } else if state.exited_terminals.contains(&pane_id) && focus.is_none() {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "terminal has exited".into(),
                        },
                    )
                } else if state.closing_terminals.contains(&pane_id) && focus.is_none() {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "terminal is closing".into(),
                        },
                    )
                } else {
                    let unavailable = state.exited_terminals.contains(&pane_id)
                        || state.closing_terminals.contains(&pane_id);
                    let result = if !unavailable && !state.terminals.contains_key(&pane_id) {
                        Err(io::Error::new(io::ErrorKind::NotFound, "unknown Pane"))
                    } else {
                        if focus == Some(true) && state.focused_terminal != Some(pane_id) {
                            state.clear_terminal_focus();
                        }
                        if let Some(focused) = focus {
                            state.record_terminal_focus(pane_id, focused);
                        }
                        if unavailable {
                            Ok(None)
                        } else {
                            state.terminals[&pane_id].execute(command)
                        }
                    };
                    match result {
                        Ok(text) if is_copy => queue_message(
                            &outbound,
                            ServerMessage::TerminalCopied { pane_id, text },
                        ),
                        Ok(_) => false,
                        Err(error) => queue_message(
                            &outbound,
                            ServerMessage::Error {
                                message: error.to_string(),
                            },
                        ),
                    }
                }
            }
            ClientMessage::PasteImage {
                server_id,
                session_id,
                pane_id,
                format,
                bytes,
            } => {
                let _operation = lifecycle.begin_operation();
                // Validate, write the file with the lock released, validate again: a Pane
                // can close while a 16 MiB image is being written, and the bytes must not
                // end up in whatever took its place.
                let target = |state: &RuntimeState| -> Result<(), &'static str> {
                    if server_id != state.server_id {
                        Err("unknown Server")
                    } else if session_id != state.session_id {
                        Err("unknown Session")
                    } else if lifecycle.is_stopping() {
                        Err("Server is stopping")
                    } else if state.session.pane(pane_id).is_none()
                        || !state.terminals.contains_key(&pane_id)
                    {
                        Err("unknown Pane")
                    } else if state.exited_terminals.contains(&pane_id) {
                        Err("terminal has exited")
                    } else if state.closing_terminals.contains(&pane_id) {
                        Err("terminal is closing")
                    } else {
                        Ok(())
                    }
                };
                let checked = {
                    let state = state.lock().expect("server state lock poisoned");
                    target(&state)
                        .map(|()| state.terminal_instances.get(&pane_id).copied())
                        .map_err(str::to_owned)
                };
                let result = checked
                    .and_then(|instance_id| {
                        if bytes.len() > condr_core::protocol::MAX_CLIPBOARD_IMAGE_BYTES {
                            Err("image exceeds 16 MiB".to_owned())
                        } else {
                            Ok(instance_id)
                        }
                    })
                    .and_then(|instance_id| {
                        clipboard_image::stage(client_id, format, &bytes)
                            .map(|path| (path, instance_id))
                            .map_err(|error| format!("could not stage image: {error}"))
                    })
                    .and_then(|(path, instance_id)| {
                        let mut state = state.lock().expect("server state lock poisoned");
                        let pasted = target(&state).map_err(str::to_owned).and_then(|()| {
                            if state.terminal_instances.get(&pane_id).copied() != instance_id {
                                return Err("terminal changed during image upload".to_owned());
                            }
                            state.terminals[&pane_id]
                                .execute(TerminalCommand::Paste(
                                    path.to_string_lossy().into_owned(),
                                ))
                                .map(|_| ())
                                .map_err(|error| error.to_string())
                        });
                        match pasted {
                            Ok(()) => {
                                state.staged_images.entry(client_id).or_default().push(path);
                                Ok(())
                            }
                            Err(message) => {
                                clipboard_image::remove([path]);
                                Err(message)
                            }
                        }
                    });
                match result {
                    Ok(()) => false,
                    Err(message) => queue_message(&outbound, ServerMessage::Error { message }),
                }
            }
            ClientMessage::ReadPane {
                server_id,
                session_id,
                pane_id,
                lines,
            } => {
                let state = state.lock().expect("server state lock poisoned");
                let error = if server_id != state.server_id {
                    Some("unknown Server")
                } else if session_id != state.session_id {
                    Some("unknown Session")
                } else if !state.terminals.contains_key(&pane_id) {
                    Some("unknown Pane")
                } else {
                    None
                };
                match error {
                    Some(message) => queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: message.into(),
                        },
                    ),
                    None => {
                        let text = state.terminals[&pane_id].recent_text(lines as usize);
                        drop(state);
                        queue_message(&outbound, ServerMessage::PaneText { pane_id, text })
                    }
                }
            }
            ClientMessage::Agent {
                server_id,
                session_id,
                command,
            } => {
                let installations = if matches!(
                    command,
                    condr_core::protocol::AgentCommand::Available
                        | condr_core::protocol::AgentCommand::Start { .. }
                ) {
                    condr_core::agent_discovery::discover()
                } else {
                    Vec::new()
                };
                let mut state = state.lock().expect("server state lock poisoned");
                if server_id != state.server_id
                    || session_id != state.session_id
                    || lifecycle.is_stopping()
                {
                    queue_message(
                        &outbound,
                        ServerMessage::AgentResult {
                            result: Err(condr_core::protocol::AgentError {
                                code: "invalid_agent_request".into(),
                                message: "unknown Server/Session or Server is stopping".into(),
                            }),
                        },
                    );
                } else {
                    state.handle_agent(client_id, &outbound, command, installations);
                }
                false
            }
            ClientMessage::SetServerSettings { server_id, shell } => {
                set_server_shell(&state, server_id, &shell, client_id, &outbound)
            }
            ClientMessage::StopServer { server_id } => {
                let known_server = state.lock().expect("server state lock poisoned").server_id;
                if server_id != known_server {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "unknown Server".into(),
                        },
                    )
                } else {
                    lifecycle.begin_stop();
                    if let Ok(data) = frame_message(&ServerMessage::ServerStopping)
                        && let Ok(receipt) = outbound.send_closing_reliable(data)
                    {
                        let _ = receipt.wait(STOP_ACK_TIMEOUT);
                    }
                    stopping_server = true;
                    true
                }
            }
            ClientMessage::RevokeDevice { key } => {
                let response = if !stream.may_administer() {
                    ServerMessage::Error {
                        message: "only the Server host may revoke devices".into(),
                    }
                } else if let Ok(key) = crate::noise::PublicKey::parse(&key) {
                    let state = state.lock().expect("server state lock poisoned");
                    let mut disconnected = 0;
                    for (id, (peer_key, peer)) in &state.tcp_peers {
                        if *id != client_id && *peer_key == key {
                            let _ = peer.shutdown();
                            disconnected += 1;
                        }
                    }
                    ServerMessage::DevicesRevoked { disconnected }
                } else {
                    ServerMessage::Error {
                        message: "expected a full device fingerprint".into(),
                    }
                };
                queue_message(&outbound, response)
            }
            ClientMessage::ConnectedDevices => {
                let response = if !stream.may_administer() {
                    ServerMessage::Error {
                        message: "only the Server host may list connected devices".into(),
                    }
                } else {
                    let state = state.lock().expect("server state lock poisoned");
                    let mut keys = state
                        .tcp_peers
                        .values()
                        .map(|(key, _)| key.to_hex())
                        .collect::<Vec<_>>();
                    keys.sort();
                    keys.dedup();
                    ServerMessage::ConnectedDevices { keys }
                };
                queue_message(&outbound, response)
            }
            ClientMessage::Detach => true,
            ClientMessage::Hello(Hello { .. }) => false,
        };
        for (pane_id, instance_id, updates, agent_probe, cwd_probe, notice_probe, view_source) in
            started_terminals
        {
            monitor_terminal(TerminalMonitor {
                pane_id,
                instance_id,
                updates,
                agent_probe,
                cwd_probe,
                notice_probe,
                view_source,
                state: Arc::clone(&state),
                lifecycle: Arc::clone(&lifecycle),
            });
        }
        drop(removed_terminals);
        if should_close {
            break;
        }
    }

    let mut state = state.lock().expect("server state lock poisoned");
    state.agent_control.waiters.remove(&client_id);
    state.subscribers.remove(&client_id);
    if let Some(staged) = state.staged_images.remove(&client_id) {
        clipboard_image::remove(staged);
    }
    if state.active_controller == Some(client_id) {
        state.clear_controller_terminal_state();
        state.active_controller = None;
    }
    drop(state);
    drop(peer_registration);
    if stopping_server {
        stop.store(true, Ordering::Release);
    }
    drop(outbound);
    let _ = writer.join();
}

fn set_server_shell(
    state: &Arc<Mutex<RuntimeState>>,
    server_id: ServerId,
    shell: &str,
    client_id: u64,
    outbound: &ClientWriter,
) -> bool {
    let shell = shell.trim();
    let settings_write = Arc::clone(
        &state
            .lock()
            .expect("server state lock poisoned")
            .settings_write,
    );
    // Keep disk completion and the published value in the same order across clients,
    // without holding the Session lock during file or lock I/O.
    let _write = settings_write.lock().expect("settings write lock poisoned");
    let path = {
        let state = state.lock().expect("server state lock poisoned");
        if server_id != state.server_id {
            return queue_message(
                outbound,
                ServerMessage::Error {
                    message: "unknown Server".into(),
                },
            );
        }
        if shell.len() > MAX_SHELL_SETTING_BYTES {
            return queue_message(
                outbound,
                ServerMessage::Error {
                    message: format!("shell setting exceeds {MAX_SHELL_SETTING_BYTES} bytes"),
                },
            );
        }
        if state.settings.shell == shell {
            return false;
        }
        state.config_path.clone()
    };
    if let Some(path) = path
        && let Err(error) = save_shell(&path, shell)
    {
        return queue_message(
            outbound,
            ServerMessage::Error {
                message: format!(
                    "failed to save the shell preference to {}: {error}",
                    path.display()
                ),
            },
        );
    }
    state
        .lock()
        .expect("server state lock poisoned")
        .set_shell(shell, Some((client_id, outbound)))
}
