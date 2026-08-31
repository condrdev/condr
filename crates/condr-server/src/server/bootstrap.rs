use super::*;

pub(super) fn queue_message(outbound: &ClientWriter, message: ServerMessage) -> bool {
    frame_message(&message)
        .and_then(|data| {
            outbound
                .send_reliable(data)
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "client writer stopped"))
        })
        .is_err()
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
    let probes = {
        let state = state.lock().expect("server state lock poisoned");
        if session_id != state.session_id {
            return queue_message(
                outbound,
                ServerMessage::SnapshotRejected {
                    server_id: state.server_id,
                    session_id: state.session_id,
                    reason: "unknown Session".into(),
                },
            );
        }
        state.terminal_cwd_probes()
    };
    let observations = observe_terminal_cwds(probes);
    let (capture, fenced) = {
        let mut state = state.lock().expect("server state lock poisoned");
        if session_id != state.session_id {
            return queue_message(
                outbound,
                ServerMessage::SnapshotRejected {
                    server_id: state.server_id,
                    session_id: state.session_id,
                    reason: "unknown Session".into(),
                },
            );
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
            let _ = queue_message(
                outbound,
                ServerMessage::SnapshotRejected {
                    server_id: bootstrap_server_id,
                    session_id: bootstrap_session_id,
                    reason: format!("cannot encode Session Bootstrap: {error}"),
                },
            );
            return true;
        }
    };

    let failed = if fenced {
        let mut state = state.lock().expect("server state lock poisoned");
        let failed = state.finish_bootstrap(client_id, framed);
        if failed {
            state.subscribers.remove(&client_id);
        }
        failed
    } else {
        outbound.send_reliable_batch(framed.frames).is_err()
    };
    if !failed && fenced {
        flush_terminal_render(state, client_id);
    }
    failed
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
        if let Some(frame) = frame {
            frames.push(PaneTerminalFrame {
                pane_id: pane.pane_id,
                frame,
            });
        }
        baselines.push((pane.pane_id, pane.current));
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
    for (record_index, record) in records.into_iter().enumerate() {
        let record_index = u32::try_from(record_index)
            .map_err(|_| io::Error::other("too many Bootstrap records"))?;
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
