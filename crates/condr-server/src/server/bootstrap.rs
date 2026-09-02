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

pub(super) fn send_bootstrap(
    stream: &mut EndpointStream,
    state: &Arc<Mutex<RuntimeState>>,
) -> io::Result<()> {
    let probes = state
        .lock()
        .expect("server state lock poisoned")
        .terminal_cwd_probes();
    let observations = observe_terminal_cwds(probes);
    let capture = {
        let mut state = state.lock().expect("server state lock poisoned");
        state.record_terminal_cwd_observations(observations);
        state.capture_bootstrap()
    };
    for frame in frame_bootstrap_messages(capture.materialize())?.frames {
        send_framed(stream, &frame)?;
    }
    Ok(())
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

pub(super) fn send_error(
    stream: &mut EndpointStream,
    state: &Arc<Mutex<RuntimeState>>,
    message: &str,
) -> io::Result<()> {
    let state = state.lock().expect("server state lock poisoned");
    send_message(
        stream,
        &ServerMessage::Welcome {
            version: PROTOCOL_VERSION,
            server_id: state.server_id,
            runtime_epoch: state.runtime_epoch,
            session_id: state.session_id,
            error: Some(message.into()),
        },
    )
}

pub(super) fn send_message(stream: &mut EndpointStream, message: &ServerMessage) -> io::Result<()> {
    condr_core::protocol::write_message(stream, message)
        .map_err(|error| io::Error::other(error.to_string()))
}

pub(super) fn flush_terminal_render(state: &Arc<Mutex<RuntimeState>>, client_id: u64) {
    loop {
        let Some(snapshot) = state
            .lock()
            .expect("server state lock poisoned")
            .terminal_render_snapshot(client_id)
        else {
            return;
        };
        let mut prepared = prepare_terminal_render(snapshot);
        let frames = if prepared.frames.is_empty() {
            None
        } else {
            match frame_terminal_batches(
                prepared.server_id,
                prepared.session_id,
                std::mem::take(&mut prepared.frames),
            ) {
                Ok(frames) => Some(frames),
                Err(_) => return,
            }
        };
        let outcome = state
            .lock()
            .expect("server state lock poisoned")
            .commit_terminal_render(prepared, frames);
        if matches!(outcome, TerminalRenderCommit::Done) {
            return;
        }
    }
}

pub(super) fn prepare_terminal_render(snapshot: TerminalRenderSnapshot) -> PreparedTerminalRender {
    let mut pending = Vec::with_capacity(snapshot.panes.len());
    let mut baselines = Vec::with_capacity(snapshot.panes.len());
    let mut frames = Vec::with_capacity(snapshot.panes.len());
    for pane in snapshot.panes {
        pending.push(pane.pane_id);
        if pane
            .baseline
            .as_ref()
            .is_some_and(|baseline| pane.current.revision <= baseline.revision)
        {
            continue;
        }
        let frame = match (&pane.baseline, &pane.producer_frame) {
            (Some(baseline), Some(TerminalViewFrame::Delta(delta)))
                if baseline.revision == delta.base_revision =>
            {
                pane.producer_frame
            }
            (baseline, _) => TerminalView::frame_from(baseline.as_deref(), &pane.current),
        };
        let Some(mut frame) = frame else {
            continue;
        };
        let projected = frame.normalize_hyperlinks_for_wire();
        let baseline = if projected {
            match &frame {
                TerminalViewFrame::Full(view) => Arc::new(view.clone()),
                TerminalViewFrame::Delta(_) => {
                    let mut view = pane
                        .baseline
                        .expect("a prepared terminal delta has a client baseline")
                        .as_ref()
                        .clone();
                    view.apply_frame(frame.clone())
                        .expect("a prepared terminal delta matches its client baseline");
                    Arc::new(view)
                }
            }
        } else {
            pane.current
        };
        frames.push(PaneTerminalFrame {
            pane_id: pane.pane_id,
            frame,
        });
        baselines.push((pane.pane_id, baseline));
    }
    PreparedTerminalRender {
        client_id: snapshot.client_id,
        generation: snapshot.generation,
        server_id: snapshot.server_id,
        session_id: snapshot.session_id,
        pending,
        baselines,
        frames,
    }
}

pub(super) fn frame_message(message: &ServerMessage) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    condr_core::protocol::write_message(&mut data, message)
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(data)
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

pub(super) fn frame_terminal_batches(
    server_id: ServerId,
    session_id: SessionId,
    panes: Vec<PaneTerminalFrame>,
) -> io::Result<Vec<Vec<u8>>> {
    let mut frames = Vec::new();
    let mut batch = Vec::new();
    let empty = ServerMessage::TerminalFrame(TerminalFrameBatch {
        server_id,
        session_id,
        panes: Vec::new(),
    });
    let overhead = usize::try_from(
        bincode::serialized_size(&empty).map_err(|error| io::Error::other(error.to_string()))?,
    )
    .map_err(|_| io::Error::other("terminal frame size does not fit usize"))?;
    let mut batch_size = overhead;
    for pane in panes {
        let pane_size = usize::try_from(
            bincode::serialized_size(&pane).map_err(|error| io::Error::other(error.to_string()))?,
        )
        .map_err(|_| io::Error::other("terminal Pane frame size does not fit usize"))?;
        if overhead.saturating_add(pane_size) > MAX_FRAME_SIZE {
            append_terminal_frame_batch(
                &mut frames,
                server_id,
                session_id,
                std::mem::take(&mut batch),
            )?;
            batch_size = overhead;

            let revision = terminal_frame_revision(&pane.frame);
            let pane_id = pane.pane_id;
            let payload = encode_pane_terminal_frame(&pane).map_err(io::Error::other)?;
            let chunk_count = u32::try_from(payload.len().div_ceil(MAX_CHUNK_PAYLOAD_SIZE))
                .map_err(|_| io::Error::other("too many terminal frame chunks"))?;
            if chunk_count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "terminal Pane frame encoded to an empty payload",
                ));
            }
            for (chunk_index, payload) in payload.chunks(MAX_CHUNK_PAYLOAD_SIZE).enumerate() {
                let chunk_index = u32::try_from(chunk_index)
                    .map_err(|_| io::Error::other("too many terminal frame chunks"))?;
                frames.push(frame_message(&ServerMessage::TerminalFrameChunk(
                    TerminalFrameChunk {
                        server_id,
                        session_id,
                        pane_id,
                        revision,
                        chunk_index,
                        chunk_count,
                        payload: payload.to_vec(),
                    },
                ))?);
            }
            continue;
        }
        if !batch.is_empty() && batch_size.saturating_add(pane_size) > MAX_FRAME_SIZE {
            append_terminal_frame_batch(
                &mut frames,
                server_id,
                session_id,
                std::mem::take(&mut batch),
            )?;
            batch_size = overhead;
        }
        batch_size += pane_size;
        batch.push(pane);
    }
    append_terminal_frame_batch(&mut frames, server_id, session_id, batch)?;
    Ok(frames)
}

pub(super) fn append_terminal_frame_batch(
    frames: &mut Vec<Vec<u8>>,
    server_id: ServerId,
    session_id: SessionId,
    panes: Vec<PaneTerminalFrame>,
) -> io::Result<()> {
    if !panes.is_empty() {
        frames.push(frame_message(&ServerMessage::TerminalFrame(
            TerminalFrameBatch {
                server_id,
                session_id,
                panes,
            },
        ))?);
    }
    Ok(())
}

pub(super) fn terminal_frame_revision(frame: &TerminalViewFrame) -> u64 {
    match frame {
        TerminalViewFrame::Full(view) => view.revision,
        TerminalViewFrame::Delta(delta) => delta.revision,
    }
}

pub(super) fn send_framed(stream: &mut EndpointStream, data: &[u8]) -> io::Result<()> {
    stream.write_all(data)?;
    stream.flush()
}
