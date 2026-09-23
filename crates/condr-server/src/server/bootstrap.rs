use super::*;

/// Queues a direct response. `Err(Lagged)` means the response was dropped in favour of the
/// lag notice: the client will re-Bootstrap, so callers must not commit state that only this
/// response would have told the client about.
pub(super) fn queue_response(
    outbound: &ClientWriter,
    message: ServerMessage,
) -> Result<(), ReliableSendError> {
    let data = frame_message(&message).map_err(|_| ReliableSendError::Disconnected)?;
    outbound.send_reliable(data)
}

/// Queues a direct response and reports whether the connection must close. A lagged writer
/// keeps its connection: the queued lag notice drives recovery.
pub(super) fn queue_message(outbound: &ClientWriter, message: ServerMessage) -> bool {
    queue_response(outbound, message) == Err(ReliableSendError::Disconnected)
}

pub(super) fn frame_overview_messages(mut overview: SessionOverview) -> io::Result<Vec<Vec<u8>>> {
    // A near-limit structural snapshot and 1024 titles need separate frames. Neither
    // frame contains VT cells, and the writer keeps the pair contiguous.
    let terminals = ServerMessage::OverviewTerminals {
        server_id: overview.server_id,
        session_id: overview.session_id,
        terminals: std::mem::take(&mut overview.terminals),
    };
    Ok(vec![
        frame_message(&ServerMessage::Overview(overview))?,
        frame_message(&terminals)?,
    ])
}

pub(super) fn queue_runtime_bootstrap(
    state: &Arc<Mutex<RuntimeState>>,
    client_id: u64,
    session_id: SessionId,
    outbound: &ClientWriter,
) -> bool {
    // Fence order for a lagged writer: the lag notice already sits in the reliable queue and
    // the writer refuses everything else. Only reopen it (`resume_after_lag`) once this client
    // is fenced by `begin_bootstrap`, so background events land in `deferred_reliable` instead
    // of slipping between the notice and the Bootstrap. Rejections reopen it just before they
    // are queued, which still keeps them behind the notice.
    let reject = |message: ServerMessage| {
        outbound.resume_after_lag();
        queue_message(outbound, message)
    };
    let probes = {
        let state = state.lock().expect("server state lock poisoned");
        if session_id != state.session_id {
            return reject(ServerMessage::SnapshotRejected {
                server_id: state.server_id,
                session_id: state.session_id,
                reason: "unknown Session".into(),
            });
        }
        state.terminal_cwd_probes()
    };
    let observations = observe_terminal_cwds(probes);
    let (capture, fenced) = {
        let mut state = state.lock().expect("server state lock poisoned");
        if session_id != state.session_id {
            return reject(ServerMessage::SnapshotRejected {
                server_id: state.server_id,
                session_id: state.session_id,
                reason: "unknown Session".into(),
            });
        }
        state.record_terminal_cwd_observations(observations);
        let capture = state.capture_bootstrap();
        let fenced = state.subscribers.contains_key(&client_id);
        if fenced {
            state.begin_bootstrap(client_id);
        }
        (capture, fenced)
    };

    let bootstrap_server_id = capture.server_id;
    let bootstrap_session_id = capture.session_id;
    let framed = match frame_bootstrap_messages(capture.materialize()) {
        Ok(framed) => framed,
        Err(error) => {
            if fenced {
                state
                    .lock()
                    .expect("server state lock poisoned")
                    .abort_bootstrap(client_id);
            }
            let _ = reject(ServerMessage::SnapshotRejected {
                server_id: bootstrap_server_id,
                session_id: bootstrap_session_id,
                reason: format!("cannot encode Session Bootstrap: {error}"),
            });
            return true;
        }
    };

    let result = if fenced {
        let mut state = state.lock().expect("server state lock poisoned");
        // Fenced and encoded: reopening now puts the Bootstrap right behind the lag notice.
        outbound.resume_after_lag();
        let result = state.finish_bootstrap(client_id, framed);
        if result.is_err() {
            state.subscribers.remove(&client_id);
        }
        result
    } else {
        outbound.resume_after_lag();
        outbound.send_reliable_batch(framed.frames)
    };
    if result.is_ok() && fenced {
        flush_terminal_render(state, client_id);
    }
    result == Err(ReliableSendError::Disconnected)
}

/// The Server's one answer to a handshake; with a `refusal` the caller closes the stream.
pub(super) fn send_welcome(
    stream: &mut EndpointStream,
    state: &Arc<Mutex<RuntimeState>>,
    refusal: Option<Refusal>,
) -> io::Result<()> {
    let (server_id, session_id) = {
        let state = state.lock().expect("server state lock poisoned");
        (state.server_id, state.session_id)
    };
    let welcome = Welcome::new(server_id, session_id, refusal);
    condr_core::protocol::write_message(stream, &welcome)
        .map_err(|error| io::Error::other(error.to_string()))
}

pub(super) fn frame_message(message: &ServerMessage) -> io::Result<Vec<u8>> {
    condr_core::protocol::encode_frame(message, MAX_FRAME_SIZE)
        .map_err(|error| io::Error::other(error.to_string()))
}

pub(super) fn validate_persistable_snapshot(
    snapshot: &condr_core::SessionSnapshot,
) -> Result<(), String> {
    let bytes = snapshot
        .to_bytes()
        .map_err(|error| format!("Session Snapshot cannot be encoded: {error}"))?;
    if bytes.len() > MAX_PERSISTED_SNAPSHOT_BYTES {
        return Err(format!(
            "Session Snapshot is {} bytes; Server limit is {MAX_PERSISTED_SNAPSHOT_BYTES} bytes",
            bytes.len()
        ));
    }
    Ok(())
}

pub(super) fn validate_snapshot_root_paths(
    snapshot: &condr_core::SessionSnapshot,
) -> Result<(), String> {
    if let Some(root) = snapshot.root_paths().find(|root| !root.is_absolute()) {
        return Err(format!(
            "Workspace root is not absolute on this Server: {}",
            root.display()
        ));
    }
    Ok(())
}

pub(super) struct FramedBootstrap {
    pub(super) frames: Vec<Vec<u8>>,
    pub(super) baselines: TerminalBaselines,
}

pub(super) fn frame_bootstrap_messages(bootstrap: SessionBootstrap) -> io::Result<FramedBootstrap> {
    let SessionBootstrap {
        server_id,
        runtime_epoch,
        session_id,
        sequence,
        snapshot,
        settings,
        terminals,
        agents,
        workspace_git,
        zoomed_panes,
    } = bootstrap;
    let mut records = Vec::with_capacity(
        terminals.len() + agents.len() + workspace_git.len() + zoomed_panes.len(),
    );
    records.extend(terminals.into_iter().map(BootstrapRecord::Terminal));
    records.extend(agents.into_iter().map(BootstrapRecord::Agent));
    records.extend(workspace_git.into_iter().map(BootstrapRecord::WorkspaceGit));
    records.extend(zoomed_panes.into_iter().map(BootstrapRecord::ZoomedPane));
    let (batches, baselines) = split_bootstrap_records(server_id, session_id, records)?;
    let batch_count =
        u32::try_from(batches.len()).map_err(|_| io::Error::other("too many Bootstrap batches"))?;
    let header = BootstrapHeader {
        server_id,
        runtime_epoch,
        session_id,
        sequence,
        snapshot,
        settings,
        batch_count,
    };

    let mut frames = Vec::with_capacity(batches.len() + 1);
    frames.push(frame_message(&ServerMessage::Bootstrap(header))?);
    for batch in batches {
        frames.push(frame_message(&ServerMessage::BootstrapBatch(batch))?);
    }
    Ok(FramedBootstrap { frames, baselines })
}

pub(super) fn split_bootstrap_records(
    server_id: ServerId,
    session_id: SessionId,
    records: Vec<BootstrapRecord>,
) -> io::Result<(Vec<BootstrapBatch>, TerminalBaselines)> {
    let mut batches = Vec::new();
    let mut baselines = Vec::new();
    let mut total_payload_size = 0usize;
    for (record_index, mut record) in records.into_iter().enumerate() {
        let record_index = u32::try_from(record_index)
            .map_err(|_| io::Error::other("too many Bootstrap records"))?;
        if let BootstrapRecord::Terminal(terminal) = &mut record {
            let _ = terminal.view.normalize_hyperlinks_for_wire();
        }
        let payload = encode_bootstrap_record(&record).map_err(io::Error::other)?;
        total_payload_size = total_payload_size
            .checked_add(payload.len())
            .ok_or_else(|| io::Error::other("Bootstrap aggregate size overflowed usize"))?;
        if total_payload_size > MAX_BOOTSTRAP_TOTAL_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Bootstrap exceeds the {MAX_BOOTSTRAP_TOTAL_SIZE}-byte aggregate limit"),
            ));
        }
        let chunk_count = u32::try_from(payload.len().div_ceil(MAX_CHUNK_PAYLOAD_SIZE))
            .map_err(|_| io::Error::other("too many Bootstrap record chunks"))?;
        for (chunk_index, payload) in payload.chunks(MAX_CHUNK_PAYLOAD_SIZE).enumerate() {
            if batches.len() >= MAX_BOOTSTRAP_BATCHES as usize {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Bootstrap exceeds the {MAX_BOOTSTRAP_BATCHES}-batch limit"),
                ));
            }
            let batch_index = u32::try_from(batches.len())
                .map_err(|_| io::Error::other("too many Bootstrap batches"))?;
            let chunk_index = u32::try_from(chunk_index)
                .map_err(|_| io::Error::other("too many Bootstrap record chunks"))?;
            let batch = BootstrapBatch {
                server_id,
                session_id,
                batch_index,
                record_index,
                chunk_index,
                chunk_count,
                payload: payload.to_vec(),
            };
            batches.push(batch);
        }
        if chunk_count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Bootstrap record encoded to an empty payload",
            ));
        }
        if let BootstrapRecord::Terminal(terminal) = record {
            baselines.push((terminal.pane_id, Arc::new(terminal.view)));
        }
    }
    Ok((batches, baselines))
}

pub(super) fn send_framed(stream: &mut EndpointStream, data: &[u8]) -> io::Result<()> {
    stream.write_all(data)?;
    stream.flush()
}

impl RuntimeState {
    pub(super) fn capture_bootstrap(&self) -> BootstrapCapture {
        #[cfg(test)]
        self.bootstrap_captures.fetch_add(1, Ordering::Relaxed);
        let mut terminals = Vec::new();
        for workspace in self.session.workspaces() {
            for tab in workspace.tabs() {
                for pane in tab.panes() {
                    if (self.terminals.contains_key(&pane.id())
                        || self.closing_terminals.contains(&pane.id()))
                        && let Some(latest) = self.terminal_views.get(&pane.id())
                    {
                        terminals.push(BootstrapTerminalCapture {
                            pane_id: pane.id(),
                            view: Arc::clone(&latest.view),
                            exited: self.exited_terminals.contains(&pane.id()),
                            title: self.terminal_titles.get(&pane.id()).cloned(),
                            attention: self.pending_terminal_bells.contains(&pane.id()),
                        });
                    }
                }
            }
        }
        BootstrapCapture {
            settings: self.settings.clone(),
            server_id: self.server_id,
            runtime_epoch: self.runtime_epoch,
            session_id: self.session_id,
            sequence: self.sequence,
            snapshot: self.session.snapshot(),
            terminals,
            agents: self
                .agents
                .iter()
                .filter(|(pane_id, _)| !self.closing_terminals.contains(pane_id))
                .filter(|(pane_id, _)| !self.exited_terminals.contains(pane_id))
                .map(|(&pane_id, agent)| PaneAgentSnapshot {
                    pane_id,
                    agent: agent.clone(),
                })
                .collect(),
            workspace_git: self
                .workspace_git
                .iter()
                .map(|(&workspace_id, repository)| workspace_git_snapshot(workspace_id, repository))
                .collect(),
            zoomed_panes: self.zoomed_panes(),
        }
    }

    #[cfg(test)]
    pub(super) fn bootstrap(&self) -> SessionBootstrap {
        self.capture_bootstrap().materialize()
    }
}
