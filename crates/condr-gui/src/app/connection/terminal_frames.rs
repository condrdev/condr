use super::*;

#[derive(Default)]
pub(in crate::app) struct TerminalVisualSlot {
    pub(in crate::app) state: Mutex<TerminalVisualSlotState>,
}

#[derive(Default)]
pub(in crate::app) struct TerminalVisualSlotState {
    pub(in crate::app) generation: u64,
    pub(in crate::app) pending: Option<PendingTerminalVisual>,
    pub(in crate::app) signaled: bool,
}

pub(in crate::app) struct PendingTerminalVisual {
    pub(in crate::app) server_id: ServerId,
    pub(in crate::app) session_id: SessionId,
    pub(in crate::app) panes: HashMap<PaneId, TerminalViewFrame>,
}

impl TerminalVisualSlot {
    /// Merges a batch into the pending visual, all panes or none: a `take` racing with this
    /// call sees either the previous pending state or the fully merged one, never a batch
    /// with some panes committed and the failing pane removed.
    pub(in crate::app) fn publish(&self, batch: TerminalFrameBatch) -> Result<Option<u64>, ()> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.pending.as_ref().is_some_and(|pending| {
            pending.server_id != batch.server_id || pending.session_id != batch.session_id
        }) {
            return Err(());
        }
        // Read-only pre-validation: every merge that could fail is checked before any
        // frame moves, so no frame needs cloning and a failure leaves the slot untouched.
        let mut seen = HashMap::new();
        for pane in &batch.panes {
            let previous = seen
                .get(&pane.pane_id)
                .copied()
                .or_else(|| state.pending.as_ref()?.panes.get(&pane.pane_id));
            if let Some(previous) = previous {
                check_terminal_frame_merge(previous, &pane.frame)?;
            }
            seen.insert(pane.pane_id, &pane.frame);
        }
        let pending = state.pending.get_or_insert_with(|| PendingTerminalVisual {
            server_id: batch.server_id,
            session_id: batch.session_id,
            panes: HashMap::new(),
        });
        for pane in batch.panes {
            let frame = match pending.panes.remove(&pane.pane_id) {
                Some(previous) => match merge_terminal_frames(previous, pane.frame) {
                    Ok(frame) => frame,
                    Err(()) => {
                        // Pre-validation covers every server-produced batch; should a merge
                        // still fail, invalidate the whole slot under this lock so a `take`
                        // never observes a half-committed batch.
                        state.generation = state.generation.wrapping_add(1);
                        state.pending = None;
                        state.signaled = false;
                        return Err(());
                    }
                },
                None => pane.frame,
            };
            pending.panes.insert(pane.pane_id, frame);
        }
        if state.signaled {
            Ok(None)
        } else {
            state.signaled = true;
            Ok(Some(state.generation))
        }
    }

    pub(in crate::app) fn take(&self, generation: u64) -> Option<TerminalFrameBatch> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.generation != generation {
            return None;
        }
        state.signaled = false;
        let pending = state.pending.take()?;
        Some(TerminalFrameBatch {
            server_id: pending.server_id,
            session_id: pending.session_id,
            panes: pending
                .panes
                .into_iter()
                .map(|(pane_id, frame)| PaneTerminalFrame { pane_id, frame })
                .collect(),
        })
    }

    pub(in crate::app) fn advance(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.pending = None;
        state.signaled = false;
    }
}

pub(in crate::app) struct TerminalFrameChunkAssembly {
    pub(in crate::app) server_id: ServerId,
    pub(in crate::app) session_id: SessionId,
    pub(in crate::app) pane_id: PaneId,
    pub(in crate::app) revision: u64,
    pub(in crate::app) next_chunk_index: u32,
    pub(in crate::app) chunk_count: u32,
    pub(in crate::app) payload: Vec<u8>,
}

pub(in crate::app) fn read_bootstrap_batches(
    reader: &mut impl std::io::Read,
    header: BootstrapHeader,
) -> Result<SessionBootstrap, String> {
    let batch_count = header.batch_count;
    let mut assembler = BootstrapAssembler::new(header)?;
    for _ in 0..batch_count {
        let message: ServerMessage = condr_core::protocol::read_message(reader)
            .map_err(|error| format!("cannot read Bootstrap batch: {error}"))?;
        let ServerMessage::BootstrapBatch(batch) = message else {
            return Err(format!("expected Bootstrap batch, received {message:?}"));
        };
        assembler.push(batch)?;
    }
    assembler.finish()
}

pub(in crate::app) fn assemble_terminal_frame_chunk(
    assembly: &mut Option<TerminalFrameChunkAssembly>,
    chunk: TerminalFrameChunk,
) -> Result<Option<PaneTerminalFrame>, String> {
    if chunk.chunk_count == 0 || chunk.chunk_index >= chunk.chunk_count {
        return Err("terminal frame chunk has an invalid range".into());
    }
    if chunk.payload.is_empty() || chunk.payload.len() > MAX_CHUNK_PAYLOAD_SIZE {
        return Err("terminal frame chunk has an invalid payload size".into());
    }

    let state = assembly.get_or_insert_with(|| TerminalFrameChunkAssembly {
        server_id: chunk.server_id,
        session_id: chunk.session_id,
        pane_id: chunk.pane_id,
        revision: chunk.revision,
        next_chunk_index: 0,
        chunk_count: chunk.chunk_count,
        payload: Vec::new(),
    });
    if state.server_id != chunk.server_id
        || state.session_id != chunk.session_id
        || state.pane_id != chunk.pane_id
        || state.revision != chunk.revision
        || state.chunk_count != chunk.chunk_count
        || state.next_chunk_index != chunk.chunk_index
    {
        return Err("terminal frame chunks are not one contiguous record".into());
    }
    let next_size = state
        .payload
        .len()
        .checked_add(chunk.payload.len())
        .ok_or_else(|| "terminal frame record size overflowed usize".to_string())?;
    if next_size > MAX_CHUNKED_RECORD_SIZE {
        return Err("terminal frame record exceeds the protocol limit".into());
    }
    state
        .payload
        .try_reserve(chunk.payload.len())
        .map_err(|_| "terminal frame record allocation failed".to_string())?;
    state.payload.extend_from_slice(&chunk.payload);
    state.next_chunk_index += 1;
    if state.next_chunk_index != state.chunk_count {
        return Ok(None);
    }

    let completed = assembly
        .take()
        .expect("terminal frame chunk assembly must exist");
    // A frame from a newer protocol leaves the visual baseline behind: resynchronize.
    let Some(pane) = decode_pane_terminal_frame(&completed.payload)? else {
        return Err("terminal frame from a newer protocol".into());
    };
    if pane.pane_id != completed.pane_id
        || terminal_view_frame_revision(&pane.frame) != completed.revision
    {
        return Err("terminal frame chunk metadata does not match its payload".into());
    }
    Ok(Some(pane))
}

pub(in crate::app) fn terminal_chunk_identity_matches(
    assembly: &mut Option<TerminalFrameChunkAssembly>,
    chunk: &TerminalFrameChunk,
    server_id: ServerId,
    session_id: SessionId,
) -> Result<bool, String> {
    if chunk.server_id == server_id && chunk.session_id == session_id {
        return Ok(true);
    }
    if assembly.take().is_some() {
        return Err("terminal frame chunk identity changed during a record".into());
    }
    Ok(false)
}

pub(in crate::app) fn enforce_terminal_chunk_reliable_fence(
    assembly: &mut Option<TerminalFrameChunkAssembly>,
    message: &ServerMessage,
) -> Result<(), String> {
    if assembly.is_none()
        || matches!(
            message,
            ServerMessage::TerminalFrame(_)
                | ServerMessage::TerminalFrameChunk(_)
                | ServerMessage::TerminalClipboard { .. }
                | ServerMessage::Event {
                    event: SessionEvent::Activated { .. }
                        | SessionEvent::AgentChanged { .. }
                        | SessionEvent::WorkspaceGitChanged { .. }
                        | SessionEvent::WorkspaceFilesChanged { .. }
                        | SessionEvent::TerminalTitleChanged { .. }
                        | SessionEvent::TerminalAttentionChanged { .. },
                    ..
                }
        )
    {
        return Ok(());
    }
    *assembly = None;
    Err("reliable server message interrupted a terminal frame record".into())
}

pub(in crate::app) fn terminal_view_frame_revision(frame: &TerminalViewFrame) -> u64 {
    match frame {
        TerminalViewFrame::Full(view) => view.revision,
        TerminalViewFrame::Delta(delta) => delta.revision,
    }
}

pub(in crate::app) fn publish_terminal_batch(
    visual_slot: &TerminalVisualSlot,
    incoming: &async_channel::Sender<Incoming>,
    resync_pending: &mut bool,
    batch: TerminalFrameBatch,
) -> Result<(), ()> {
    match visual_slot.publish(batch) {
        Ok(Some(generation)) => incoming
            .send_blocking(Incoming::VisualReady(generation))
            .map_err(|_| ()),
        Ok(None) => Ok(()),
        Err(()) => request_terminal_resync(visual_slot, incoming, resync_pending),
    }
}

pub(in crate::app) fn apply_terminal_frame_batch(
    terminals: &mut HashMap<PaneId, ClientTerminal>,
    terminal_hyperlinks: &mut HashMap<PaneId, TerminalHyperlinkBudget>,
    panes: Vec<PaneTerminalFrame>,
) -> Result<Vec<PaneId>, ()> {
    let mut pane_ids = Vec::with_capacity(panes.len());
    let mut seen = HashSet::with_capacity(panes.len());
    for pane in &panes {
        if !seen.insert(pane.pane_id) {
            return Err(());
        }
        match terminals.get(&pane.pane_id) {
            Some(terminal) => {
                if !terminal_hyperlinks.contains_key(&pane.pane_id) {
                    return Err(());
                }
                terminal.view.validate_frame(&pane.frame).map_err(|_| ())?;
            }
            // A Pane the structure announced but whose terminal has not been seen yet:
            // its first frame is a full view, which stands on its own.
            None if matches!(pane.frame, TerminalViewFrame::Full(_)) => {}
            None => return Err(()),
        }
    }
    for pane in panes {
        match pane.frame {
            TerminalViewFrame::Full(mut view) => {
                terminal_hyperlinks.insert(pane.pane_id, TerminalHyperlinkBudget::new(&mut view));
                let view = Arc::new(view);
                match terminals.get_mut(&pane.pane_id) {
                    Some(terminal) => terminal.view = view,
                    None => {
                        terminals.insert(
                            pane.pane_id,
                            ClientTerminal {
                                view,
                                exited: false,
                            },
                        );
                    }
                }
            }
            TerminalViewFrame::Delta(delta) => {
                let terminal = terminals
                    .get_mut(&pane.pane_id)
                    .expect("prevalidated terminal still exists");
                terminal_hyperlinks
                    .get_mut(&pane.pane_id)
                    .expect("prevalidated terminal hyperlink budget still exists")
                    .apply_delta(Arc::make_mut(&mut terminal.view), delta)
                    .expect("prevalidated terminal delta remains valid");
            }
        }
        pane_ids.push(pane.pane_id);
    }
    Ok(pane_ids)
}

pub(in crate::app) fn clear_pending_sizes_for_bootstrap(
    pending_sizes: &mut HashMap<(ConnectionKey, PaneId), TerminalSize>,
    key: ConnectionKey,
) {
    pending_sizes.retain(|(connection_key, _), _| *connection_key != key);
}

pub(in crate::app) fn request_terminal_resync(
    visual_slot: &TerminalVisualSlot,
    incoming: &async_channel::Sender<Incoming>,
    resync_pending: &mut bool,
) -> Result<(), ()> {
    visual_slot.advance();
    *resync_pending = true;
    incoming
        .send_blocking(Incoming::TerminalResync)
        .map_err(|_| ())
}

/// Whether [`merge_terminal_frames`] would succeed, without consuming either frame.
pub(in crate::app) fn check_terminal_frame_merge(
    previous: &TerminalViewFrame,
    next: &TerminalViewFrame,
) -> Result<(), ()> {
    match (previous, next) {
        (_, TerminalViewFrame::Full(_)) => Ok(()),
        (TerminalViewFrame::Full(view), TerminalViewFrame::Delta(delta)) => {
            view.validate_delta(delta).map_err(|_| ())
        }
        (TerminalViewFrame::Delta(previous), TerminalViewFrame::Delta(next)) => {
            check_terminal_delta_merge(previous, next)
        }
    }
}

fn check_terminal_delta_merge(
    previous: &TerminalViewDelta,
    next: &TerminalViewDelta,
) -> Result<(), ()> {
    if previous.revision != next.base_revision {
        return Err(());
    }
    for run in previous.runs.iter().chain(&next.runs) {
        let len = u32::try_from(run.cells.len()).map_err(|_| ())?;
        if run.cells.is_empty() || run.start.checked_add(len).is_none() {
            return Err(());
        }
    }
    Ok(())
}

pub(in crate::app) fn merge_terminal_frames(
    previous: TerminalViewFrame,
    next: TerminalViewFrame,
) -> Result<TerminalViewFrame, ()> {
    // Every wire frame is already within the hyperlink limits on its own; only merging a
    // link-bearing delta into an earlier frame can push the union past them.
    let next_adds_links = match &next {
        TerminalViewFrame::Delta(delta) => delta
            .runs
            .iter()
            .flat_map(|run| &run.cells)
            .any(|cell| cell.hyperlink.is_some()),
        TerminalViewFrame::Full(_) => false,
    };
    let mut merged = match (previous, next) {
        (TerminalViewFrame::Full(mut view), TerminalViewFrame::Delta(delta)) => {
            view.apply_frame(TerminalViewFrame::Delta(delta))
                .map_err(|_| ())?;
            Ok(TerminalViewFrame::Full(view))
        }
        (_, TerminalViewFrame::Full(view)) => Ok(TerminalViewFrame::Full(view)),
        (TerminalViewFrame::Delta(previous), TerminalViewFrame::Delta(next)) => {
            merge_terminal_deltas(previous, next).map(TerminalViewFrame::Delta)
        }
    }?;
    if next_adds_links {
        let _ = merged.normalize_hyperlinks_for_wire();
    }
    Ok(merged)
}

pub(in crate::app) fn merge_terminal_deltas(
    previous: TerminalViewDelta,
    next: TerminalViewDelta,
) -> Result<TerminalViewDelta, ()> {
    check_terminal_delta_merge(&previous, &next)?;
    let mut cells = BTreeMap::new();
    for run in previous.runs.into_iter().chain(next.runs) {
        for (offset, cell) in run.cells.into_iter().enumerate() {
            let offset = u32::try_from(offset).map_err(|_| ())?;
            let index = run.start.checked_add(offset).ok_or(())?;
            cells.insert(index, cell);
        }
    }
    let mut runs: Vec<TerminalCellRun> = Vec::new();
    for (index, cell) in cells {
        if let Some(run) = runs.last_mut()
            && run
                .start
                .checked_add(u32::try_from(run.cells.len()).map_err(|_| ())?)
                == Some(index)
        {
            run.cells.push(cell);
        } else {
            runs.push(TerminalCellRun {
                start: index,
                cells: vec![cell],
            });
        }
    }
    Ok(TerminalViewDelta {
        base_revision: previous.base_revision,
        revision: next.revision,
        display_offset: next.display_offset,
        mouse_tracking: next.mouse_tracking,
        cursor: next.cursor,
        selection: next.selection,
        runs,
    })
}
