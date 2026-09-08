use super::*;

pub(super) struct LatestTerminalView {
    pub(super) view: Arc<TerminalView>,
    pub(super) hyperlinks: TerminalHyperlinkBudget,
    pub(super) producer_frame: Option<TerminalViewFrame>,
}

pub(super) struct TerminalRenderSnapshot {
    pub(super) client_id: u64,
    pub(super) generation: u64,
    pub(super) server_id: ServerId,
    pub(super) session_id: SessionId,
    pub(super) panes: Vec<TerminalRenderPaneSnapshot>,
}

pub(super) struct TerminalRenderPaneSnapshot {
    pub(super) pane_id: PaneId,
    pub(super) baseline: Option<Arc<TerminalView>>,
    pub(super) current: Arc<TerminalView>,
    pub(super) producer_frame: Option<TerminalViewFrame>,
}

pub(super) struct PreparedTerminalRender {
    client_id: u64,
    generation: u64,
    server_id: ServerId,
    session_id: SessionId,
    pending: Vec<PaneId>,
    pub(super) baselines: Vec<(PaneId, Arc<TerminalView>)>,
    pub(super) frames: Vec<PaneTerminalFrame>,
}

enum TerminalRenderCommit {
    Done,
    Retry,
}

impl RuntimeState {
    pub(super) fn publish_terminal(
        &mut self,
        pane_id: PaneId,
        frame: TerminalViewFrame,
    ) -> Option<Vec<u64>> {
        match frame {
            TerminalViewFrame::Full(mut view) => {
                let hyperlinks = TerminalHyperlinkBudget::new(&mut view);
                self.terminal_views.insert(
                    pane_id,
                    LatestTerminalView {
                        view: Arc::new(view),
                        hyperlinks,
                        producer_frame: None,
                    },
                );
            }
            TerminalViewFrame::Delta(delta) => {
                let latest = self.terminal_views.get_mut(&pane_id)?;
                let delta = latest
                    .hyperlinks
                    .apply_delta(Arc::make_mut(&mut latest.view), delta)
                    .ok()?;
                latest.producer_frame = Some(TerminalViewFrame::Delta(delta));
            }
        }
        let client_ids = self.subscribers.keys().copied().collect::<Vec<_>>();
        for subscriber in self.subscribers.values_mut() {
            subscriber.pending_terminals.insert(pane_id);
            subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        }
        Some(client_ids)
    }

    pub(super) fn terminal_render_snapshot(
        &mut self,
        client_id: u64,
    ) -> Option<TerminalRenderSnapshot> {
        let subscriber = self.subscribers.get_mut(&client_id)?;
        if subscriber.bootstrap_pending {
            return None;
        }
        subscriber
            .pending_terminals
            .retain(|pane_id| self.terminal_views.contains_key(pane_id));
        subscriber
            .terminal_baselines
            .retain(|pane_id, _| self.terminal_views.contains_key(pane_id));
        let panes = subscriber
            .pending_terminals
            .iter()
            .filter_map(|pane_id| {
                let latest = self.terminal_views.get(pane_id)?;
                Some(TerminalRenderPaneSnapshot {
                    pane_id: *pane_id,
                    baseline: subscriber.terminal_baselines.get(pane_id).cloned(),
                    current: Arc::clone(&latest.view),
                    producer_frame: latest.producer_frame.clone(),
                })
            })
            .collect::<Vec<_>>();
        (!panes.is_empty()).then_some(TerminalRenderSnapshot {
            client_id,
            generation: subscriber.render_generation,
            server_id: self.server_id,
            session_id: self.session_id,
            panes,
        })
    }

    fn commit_terminal_render(
        &mut self,
        prepared: PreparedTerminalRender,
        frames: Option<Vec<Vec<u8>>>,
    ) -> TerminalRenderCommit {
        let Some(subscriber) = self.subscribers.get_mut(&prepared.client_id) else {
            return TerminalRenderCommit::Done;
        };
        if subscriber.render_generation != prepared.generation {
            return TerminalRenderCommit::Retry;
        }
        if let Some(frames) = frames {
            match subscriber.try_send_terminal_render(frames, prepared.baselines) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => return TerminalRenderCommit::Done,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.subscribers.remove(&prepared.client_id);
                    return TerminalRenderCommit::Done;
                }
            }
        } else {
            for (pane_id, view) in prepared.baselines {
                subscriber.terminal_baselines.insert(pane_id, view);
            }
        }
        for pane_id in prepared.pending {
            subscriber.pending_terminals.remove(&pane_id);
        }
        subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        TerminalRenderCommit::Done
    }
}

impl ClientSubscriber {
    pub(super) fn try_send_terminal_render(
        &mut self,
        frames: Vec<Vec<u8>>,
        baselines: Vec<(PaneId, Arc<TerminalView>)>,
    ) -> Result<(), mpsc::TrySendError<Vec<Vec<u8>>>> {
        self.writer.try_send_render(frames)?;
        for (pane_id, view) in baselines {
            self.terminal_baselines.insert(pane_id, view);
        }
        Ok(())
    }
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

fn append_terminal_frame_batch(
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

fn terminal_frame_revision(frame: &TerminalViewFrame) -> u64 {
    match frame {
        TerminalViewFrame::Full(view) => view.revision,
        TerminalViewFrame::Delta(delta) => delta.revision,
    }
}
