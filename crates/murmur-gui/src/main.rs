mod terminal_element;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_component::dialog::{Cancel, Confirm, DialogButtonProps, DialogFooter};
use gpui_component::dock::{
    BasePanel, DockArea, DockAreaRenderer, DockEvent, DockLayout, PanelEvent, PanelInfo,
    PanelState, TabGroupRenderer, TilesRenderer,
};
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui_component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu,
    SidebarMenuItem,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Root, Selectable as _, Sizable as _,
    StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_component_assets::Assets;
use murmur_core::protocol::{
    BootstrapAssembler, BootstrapHeader, ClientMessage, LayoutCommand, MAX_CHUNK_PAYLOAD_SIZE,
    MAX_CHUNKED_RECORD_SIZE, PaneTerminalFrame, PaneTerminalSnapshot, RuntimeEpoch, ServerId,
    ServerMessage, SessionBootstrap, SessionEvent, SessionId, TerminalFrameBatch,
    TerminalFrameChunk, WorkspaceGitSnapshot, decode_pane_terminal_frame,
};
use murmur_core::{
    AgentSnapshot, AgentTracker, PaneDirection, PaneId, PaneLayout, Session, SessionSnapshot,
    SplitDirection, TabId, TerminalCellRun, TerminalCommand, TerminalKey, TerminalModifiers,
    TerminalPosition, TerminalScroll, TerminalSelection, TerminalSize, TerminalViewDelta,
    TerminalViewFrame, WorkspaceId,
};
use murmur_server::{ClientConnection, Endpoint, ServerConfig};

use crate::terminal_element::{TerminalElement, TerminalElementProps, TerminalRenderCache};

actions!(
    murmur,
    [
        AddServer,
        ReconnectServer,
        NewWorkspace,
        NewTab,
        RenameWorkspace,
        RenameTab,
        MoveWorkspaceUp,
        MoveWorkspaceDown,
        MoveTabLeft,
        MoveTabRight,
        ClosePane,
        CloseTab,
        CloseWorkspace,
        NextTab,
        PreviousTab,
        SplitRight,
        SplitDown,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        ResizeLeft,
        ResizeRight,
        ResizeUp,
        ResizeDown,
        SwapLeft,
        SwapRight,
        SwapUp,
        SwapDown,
        ToggleZoom
    ]
);

pub(crate) type ConnectionKey = u64;

const DEFAULT_WINDOW_SIZE: Size<Pixels> = size(px(1280.0), px(720.0));
const CONNECTION_RESULT_BUFFER_CAPACITY: usize = 16;
const SERVER_EVENT_BUFFER_CAPACITY: usize = 256;
const CONTROL_RETRY_DELAY: Duration = Duration::from_millis(50);
const MAX_CONTROL_RETRY_ATTEMPTS: u8 = 20;
const CONTROL_BUSY_REASON: &str = "another client controls this Session";

fn default_worktree_branch(workspace_name: &str) -> String {
    let slug = workspace_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-');
    format!("worktree/{}", if slug.is_empty() { "change" } else { slug })
}

fn default_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(DEFAULT_WINDOW_SIZE, cx)),
        ..Default::default()
    }
}

enum Incoming {
    Bootstrap(SessionBootstrap),
    Message(ServerMessage),
    VisualReady(u64),
    TerminalResync,
    Disconnected(String),
}

#[derive(Default)]
struct TerminalVisualSlot {
    state: Mutex<TerminalVisualSlotState>,
}

#[derive(Default)]
struct TerminalVisualSlotState {
    generation: u64,
    pending: Option<PendingTerminalVisual>,
    signaled: bool,
}

struct PendingTerminalVisual {
    server_id: ServerId,
    session_id: SessionId,
    panes: HashMap<PaneId, TerminalViewFrame>,
}

impl TerminalVisualSlot {
    fn publish(&self, batch: TerminalFrameBatch) -> Result<Option<u64>, ()> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let pending = state.pending.get_or_insert_with(|| PendingTerminalVisual {
            server_id: batch.server_id,
            session_id: batch.session_id,
            panes: HashMap::new(),
        });
        if pending.server_id != batch.server_id || pending.session_id != batch.session_id {
            return Err(());
        }
        for pane in batch.panes {
            match pending.panes.remove(&pane.pane_id) {
                Some(previous) => {
                    pending
                        .panes
                        .insert(pane.pane_id, merge_terminal_frames(previous, pane.frame)?);
                }
                None => {
                    pending.panes.insert(pane.pane_id, pane.frame);
                }
            }
        }
        if state.signaled {
            Ok(None)
        } else {
            state.signaled = true;
            Ok(Some(state.generation))
        }
    }

    fn take(&self, generation: u64) -> Option<TerminalFrameBatch> {
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

    fn advance(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.pending = None;
        state.signaled = false;
    }
}

struct TerminalFrameChunkAssembly {
    server_id: ServerId,
    session_id: SessionId,
    pane_id: PaneId,
    revision: u64,
    next_chunk_index: u32,
    chunk_count: u32,
    payload: Vec<u8>,
}

fn read_bootstrap_batches(
    reader: &mut impl std::io::Read,
    header: BootstrapHeader,
) -> Result<SessionBootstrap, String> {
    let batch_count = header.batch_count;
    let mut assembler = BootstrapAssembler::new(header)?;
    for _ in 0..batch_count {
        let message: ServerMessage = murmur_core::protocol::read_message(reader)
            .map_err(|error| format!("cannot read Bootstrap batch: {error}"))?;
        let ServerMessage::BootstrapBatch(batch) = message else {
            return Err(format!("expected Bootstrap batch, received {message:?}"));
        };
        assembler.push(batch)?;
    }
    assembler.finish()
}

fn assemble_terminal_frame_chunk(
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
    let pane = decode_pane_terminal_frame(&completed.payload)?;
    if pane.pane_id != completed.pane_id
        || terminal_view_frame_revision(&pane.frame) != completed.revision
    {
        return Err("terminal frame chunk metadata does not match its payload".into());
    }
    Ok(Some(pane))
}

fn terminal_chunk_identity_matches(
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

fn enforce_terminal_chunk_reliable_fence(
    assembly: &mut Option<TerminalFrameChunkAssembly>,
    message: &ServerMessage,
) -> Result<(), String> {
    if assembly.is_none()
        || matches!(
            message,
            ServerMessage::TerminalFrame(_) | ServerMessage::TerminalFrameChunk(_)
        )
    {
        return Ok(());
    }
    *assembly = None;
    Err("reliable server message interrupted a terminal frame record".into())
}

fn terminal_view_frame_revision(frame: &TerminalViewFrame) -> u64 {
    match frame {
        TerminalViewFrame::Full(view) => view.revision,
        TerminalViewFrame::Delta(delta) => delta.revision,
    }
}

fn publish_terminal_batch(
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

fn apply_terminal_frame_batch(
    terminals: &mut HashMap<PaneId, PaneTerminalSnapshot>,
    panes: Vec<PaneTerminalFrame>,
) -> Result<Vec<PaneId>, ()> {
    let mut pane_ids = Vec::with_capacity(panes.len());
    let mut staged = Vec::with_capacity(panes.len());
    let mut seen = HashSet::with_capacity(panes.len());
    for pane in panes {
        if !seen.insert(pane.pane_id) {
            return Err(());
        }
        let mut view = terminals.get(&pane.pane_id).ok_or(())?.view.clone();
        view.apply_frame(pane.frame).map_err(|_| ())?;
        pane_ids.push(pane.pane_id);
        staged.push((pane.pane_id, view));
    }
    for (pane_id, view) in staged {
        terminals
            .get_mut(&pane_id)
            .expect("staged terminal still exists")
            .view = view;
    }
    Ok(pane_ids)
}

fn clear_pending_sizes_for_bootstrap(
    pending_sizes: &mut HashMap<(ConnectionKey, PaneId), TerminalSize>,
    key: ConnectionKey,
) {
    pending_sizes.retain(|(connection_key, _), _| *connection_key != key);
}

fn request_terminal_resync(
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

fn merge_terminal_frames(
    previous: TerminalViewFrame,
    next: TerminalViewFrame,
) -> Result<TerminalViewFrame, ()> {
    match (previous, next) {
        (TerminalViewFrame::Full(mut view), TerminalViewFrame::Delta(delta)) => {
            view.apply_frame(TerminalViewFrame::Delta(delta))
                .map_err(|_| ())?;
            Ok(TerminalViewFrame::Full(view))
        }
        (_, TerminalViewFrame::Full(view)) => Ok(TerminalViewFrame::Full(view)),
        (TerminalViewFrame::Delta(previous), TerminalViewFrame::Delta(next)) => {
            merge_terminal_deltas(previous, next).map(TerminalViewFrame::Delta)
        }
    }
}

fn merge_terminal_deltas(
    previous: TerminalViewDelta,
    next: TerminalViewDelta,
) -> Result<TerminalViewDelta, ()> {
    if previous.revision != next.base_revision {
        return Err(());
    }
    let mut cells = BTreeMap::new();
    for run in previous.runs.into_iter().chain(next.runs) {
        if run.cells.is_empty() {
            return Err(());
        }
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
        cursor: next.cursor,
        runs,
    })
}

#[derive(Default)]
struct IncomingEffect {
    rebuild: bool,
    notify: bool,
}

struct ClientIo {
    outgoing: mpsc::Sender<ClientMessage>,
    _incoming_task: Task<()>,
}

impl ClientIo {
    fn start(
        connection: ClientConnection,
        key: ConnectionKey,
        connection_generation: u64,
        initial_server_id: ServerId,
        initial_session_id: SessionId,
        window: &Window,
        cx: &Context<Murmur>,
    ) -> std::io::Result<Self> {
        let mut reader = connection.into_stream();
        let mut writer = reader.try_clone()?;
        let (outgoing, outgoing_rx) = mpsc::channel();
        let (incoming_tx, incoming_rx) = async_channel::bounded(SERVER_EVENT_BUFFER_CAPACITY);
        let writer_events = incoming_tx.clone();
        let visual_slot = Arc::new(TerminalVisualSlot::default());

        thread::Builder::new()
            .name("murmur-client-writer".into())
            .spawn(move || {
                while let Ok(message) = outgoing_rx.recv() {
                    if let Err(error) = murmur_core::protocol::write_message(&mut writer, &message)
                    {
                        let _ =
                            writer_events.send_blocking(Incoming::Disconnected(error.to_string()));
                        break;
                    }
                }
            })?;
        let reader_visual_slot = Arc::clone(&visual_slot);
        thread::Builder::new()
            .name("murmur-client-reader".into())
            .spawn(move || {
                let mut server_id = initial_server_id;
                let mut session_id = initial_session_id;
                let mut resync_pending = false;
                let mut terminal_chunk_assembly = None;
                loop {
                    let message = match murmur_core::protocol::read_message(&mut reader) {
                        Ok(message) => message,
                        Err(error) => {
                            let _ = incoming_tx
                                .send_blocking(Incoming::Disconnected(error.to_string()));
                            break;
                        }
                    };
                    if let Err(error) = enforce_terminal_chunk_reliable_fence(
                        &mut terminal_chunk_assembly,
                        &message,
                    ) {
                        match &message {
                            ServerMessage::Bootstrap(_) | ServerMessage::ServerStopping => {}
                            ServerMessage::Welcome { .. } | ServerMessage::BootstrapBatch(_) => {
                                let _ = incoming_tx.send_blocking(Incoming::Disconnected(error));
                                break;
                            }
                            _ => {
                                if request_terminal_resync(
                                    &reader_visual_slot,
                                    &incoming_tx,
                                    &mut resync_pending,
                                )
                                .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                    match message {
                        ServerMessage::TerminalFrame(batch) => {
                            if resync_pending {
                                continue;
                            }
                            if terminal_chunk_assembly.is_some() {
                                terminal_chunk_assembly = None;
                                if request_terminal_resync(
                                    &reader_visual_slot,
                                    &incoming_tx,
                                    &mut resync_pending,
                                )
                                .is_err()
                                {
                                    break;
                                }
                                continue;
                            }
                            if batch.server_id != server_id || batch.session_id != session_id {
                                continue;
                            }
                            if publish_terminal_batch(
                                &reader_visual_slot,
                                &incoming_tx,
                                &mut resync_pending,
                                batch,
                            )
                            .is_err()
                            {
                                break;
                            }
                        }
                        ServerMessage::TerminalFrameChunk(chunk) => {
                            if resync_pending {
                                continue;
                            }
                            match terminal_chunk_identity_matches(
                                &mut terminal_chunk_assembly,
                                &chunk,
                                server_id,
                                session_id,
                            ) {
                                Ok(true) => {}
                                Ok(false) => continue,
                                Err(_) => {
                                    if request_terminal_resync(
                                        &reader_visual_slot,
                                        &incoming_tx,
                                        &mut resync_pending,
                                    )
                                    .is_err()
                                    {
                                        break;
                                    }
                                    continue;
                                }
                            }
                            match assemble_terminal_frame_chunk(&mut terminal_chunk_assembly, chunk)
                            {
                                Ok(Some(pane)) => {
                                    if publish_terminal_batch(
                                        &reader_visual_slot,
                                        &incoming_tx,
                                        &mut resync_pending,
                                        TerminalFrameBatch {
                                            server_id,
                                            session_id,
                                            panes: vec![pane],
                                        },
                                    )
                                    .is_err()
                                    {
                                        break;
                                    }
                                }
                                Ok(None) => {}
                                Err(_) => {
                                    terminal_chunk_assembly = None;
                                    if request_terminal_resync(
                                        &reader_visual_slot,
                                        &incoming_tx,
                                        &mut resync_pending,
                                    )
                                    .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                        }
                        ServerMessage::Bootstrap(header) => {
                            let bootstrap = match read_bootstrap_batches(&mut reader, header) {
                                Ok(bootstrap) => bootstrap,
                                Err(error) => {
                                    let _ =
                                        incoming_tx.send_blocking(Incoming::Disconnected(error));
                                    break;
                                }
                            };
                            server_id = bootstrap.server_id;
                            session_id = bootstrap.session_id;
                            resync_pending = false;
                            terminal_chunk_assembly = None;
                            reader_visual_slot.advance();
                            if incoming_tx
                                .send_blocking(Incoming::Bootstrap(bootstrap))
                                .is_err()
                            {
                                break;
                            }
                        }
                        ServerMessage::BootstrapBatch(_) => {
                            let _ = incoming_tx.send_blocking(Incoming::Disconnected(
                                "unexpected Bootstrap batch without a header".into(),
                            ));
                            break;
                        }
                        message => {
                            if incoming_tx
                                .send_blocking(Incoming::Message(message))
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            })?;

        let incoming_task = cx.spawn_in(window, async move |owner, cx| {
            while let Ok(first) = incoming_rx.recv().await {
                let mut incoming = Vec::with_capacity(SERVER_EVENT_BUFFER_CAPACITY.min(16));
                incoming.push(first);
                while let Ok(next) = incoming_rx.try_recv() {
                    incoming.push(next);
                }
                if owner
                    .update_in(cx, |this, window, cx| {
                        let mut effect = IncomingEffect::default();
                        for incoming in incoming {
                            let incoming = match incoming {
                                Incoming::VisualReady(generation) => {
                                    let Some(batch) = visual_slot.take(generation) else {
                                        continue;
                                    };
                                    Incoming::Message(ServerMessage::TerminalFrame(batch))
                                }
                                incoming => incoming,
                            };
                            let next =
                                this.handle_incoming(key, connection_generation, incoming, cx);
                            effect.rebuild |= next.rebuild;
                            effect.notify |= next.notify;
                        }
                        if effect.rebuild && key == this.active_connection {
                            this.rebuild_dock(window, cx);
                        }
                        if effect.notify || effect.rebuild {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        Ok(Self {
            outgoing,
            _incoming_task: incoming_task,
        })
    }
}

struct ConnectionResult {
    key: ConnectionKey,
    generation: u64,
    endpoint: Endpoint,
    result: Result<ClientConnection, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
}

struct ServerConnection {
    key: ConnectionKey,
    label: String,
    endpoint: Endpoint,
    status: ConnectionStatus,
    server_id: Option<ServerId>,
    runtime_epoch: Option<RuntimeEpoch>,
    session_id: Option<SessionId>,
    sequence: u64,
    snapshot: SessionSnapshot,
    terminals: HashMap<PaneId, PaneTerminalSnapshot>,
    agents: HashMap<PaneId, AgentSnapshot>,
    agent_trackers: HashMap<PaneId, AgentTracker>,
    workspace_git: HashMap<WorkspaceId, WorkspaceGitSnapshot>,
    zoomed_panes: HashSet<PaneId>,
    io: Option<ClientIo>,
    connect_generation: u64,
    controlling: bool,
    control_retry_attempts: u8,
    control_retry_scheduled: bool,
    terminal_resync_pending: bool,
    error: Option<String>,
}

impl ServerConnection {
    fn new(key: ConnectionKey, label: String, endpoint: Endpoint) -> Self {
        Self {
            key,
            label,
            endpoint,
            status: ConnectionStatus::Disconnected,
            server_id: None,
            runtime_epoch: None,
            session_id: None,
            sequence: 0,
            snapshot: Session::new().snapshot(),
            terminals: HashMap::new(),
            agents: HashMap::new(),
            agent_trackers: HashMap::new(),
            workspace_git: HashMap::new(),
            zoomed_panes: HashSet::new(),
            io: None,
            connect_generation: 0,
            controlling: false,
            control_retry_attempts: 0,
            control_retry_scheduled: false,
            terminal_resync_pending: false,
            error: None,
        }
    }

    fn can_mutate(&self) -> bool {
        self.status == ConnectionStatus::Connected && self.controlling
    }

    fn apply_bootstrap(&mut self, bootstrap: SessionBootstrap) -> bool {
        let previous_layout = self.dock_projection();
        self.server_id = Some(bootstrap.server_id);
        self.runtime_epoch = Some(bootstrap.runtime_epoch);
        self.session_id = Some(bootstrap.session_id);
        self.sequence = bootstrap.sequence;
        self.snapshot = bootstrap.snapshot;
        self.terminals = bootstrap
            .terminals
            .into_iter()
            .map(|terminal| (terminal.pane_id, terminal))
            .collect();
        self.agents = bootstrap
            .agents
            .into_iter()
            .map(|agent| (agent.pane_id, agent.agent))
            .collect();
        self.agent_trackers
            .retain(|pane_id, _| self.agents.contains_key(pane_id));
        for (&pane_id, agent) in &self.agents {
            self.agent_trackers
                .entry(pane_id)
                .and_modify(|tracker| tracker.update(agent.state, false))
                .or_insert_with(|| AgentTracker::new(agent.state));
        }
        self.workspace_git = bootstrap
            .workspace_git
            .into_iter()
            .map(|git| (git.workspace_id, git))
            .collect();
        self.zoomed_panes = bootstrap.zoomed_panes.into_iter().collect();
        self.status = ConnectionStatus::Connected;
        self.terminal_resync_pending = false;
        self.error = None;
        previous_layout != self.dock_projection()
    }

    fn dock_projection(&self) -> Option<PaneLayout> {
        let session = Session::restore(self.snapshot.clone()).ok()?;
        let tab = session.active_workspace()?.active_tab();
        self.zoomed_panes
            .iter()
            .copied()
            .find(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .map(PaneLayout::Pane)
            .or_else(|| Some(tab.layout().clone()))
    }

    fn send(&mut self, message: ClientMessage) {
        let failed = self
            .io
            .as_ref()
            .is_none_or(|io| io.outgoing.send(message).is_err());
        if failed {
            self.status = ConnectionStatus::Disconnected;
            self.controlling = false;
            self.error = Some("Disconnected from murmur-server".into());
            self.io = None;
        }
    }

    fn request_snapshot(&mut self) {
        if self.terminal_resync_pending {
            return;
        }
        let Some(session_id) = self.session_id else {
            return;
        };
        self.terminal_resync_pending = true;
        self.send(ClientMessage::SnapshotRequest { session_id });
    }
}

#[derive(Clone, Copy, PartialEq)]
struct TerminalGeometry {
    bounds: Bounds<Pixels>,
    cell_size: Size<Pixels>,
}

#[derive(Clone, Copy)]
struct LocalTerminalSelection {
    connection_key: ConnectionKey,
    pane_id: PaneId,
    range: TerminalSelection,
    dragging: bool,
}

struct TerminalPanel {
    connection_key: ConnectionKey,
    pane_id: PaneId,
    owner: WeakEntity<Murmur>,
    focus_handle: FocusHandle,
    render_cache: Rc<RefCell<TerminalRenderCache>>,
}

struct MurmurDockRenderer;

impl DockAreaRenderer for MurmurDockRenderer {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div()
            .id("murmur-dock-area")
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_row()
    }

    fn center_frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div()
            .id("murmur-dock-center")
            .flex()
            .flex_1()
            .flex_col()
            .overflow_hidden()
    }

    fn split_frame(
        &self,
        node: gpui_component::dock::NodeId,
        _: Axis,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(("murmur-dock-split", node.as_u64()))
            .size_full()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(MurmurTabGroupRenderer)
    }

    fn tiles_renderer(&self) -> Rc<dyn TilesRenderer> {
        Rc::new(MurmurTilesRenderer)
    }
}

struct MurmurTabGroupRenderer;

impl TabGroupRenderer for MurmurTabGroupRenderer {
    fn frame(
        &self,
        _: &gpui_component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("murmur-tab-group")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
    }

    fn content_frame(
        &self,
        _: &gpui_component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("murmur-tab-content")
            .size_full()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
    }

    fn render_tab_bar(
        &self,
        _: &gpui_component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        Empty.into_any_element()
    }
}

struct MurmurTilesRenderer;

impl TilesRenderer for MurmurTilesRenderer {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div().id("murmur-tiles").size_full().overflow_hidden()
    }

    fn render_drag_bar(
        &self,
        _: &gpui_component::dock::TileContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        Empty.into_any_element()
    }
}

impl TerminalPanel {
    fn new(
        connection_key: ConnectionKey,
        pane_id: PaneId,
        owner: WeakEntity<Murmur>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            connection_key,
            pane_id,
            owner,
            focus_handle: cx.focus_handle(),
            render_cache: Rc::new(RefCell::new(TerminalRenderCache::default())),
        }
    }
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for TerminalPanel {
    fn panel_name(&self) -> &'static str {
        "MurmurTerminal"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = self.owner.upgrade();
        let (terminal, active, controlling, marked_text, selection, runtime_epoch) = owner
            .as_ref()
            .map(|owner| {
                let app = owner.read(cx);
                let active = app.target_pane == Some((self.connection_key, self.pane_id));
                (
                    app.terminal(self.connection_key, self.pane_id).cloned(),
                    active,
                    app.connection(self.connection_key)
                        .is_some_and(ServerConnection::can_mutate),
                    active.then(|| app.marked_text.clone()).flatten(),
                    app.selection_for(self.connection_key, self.pane_id),
                    app.connection(self.connection_key)
                        .and_then(|connection| connection.runtime_epoch),
                )
            })
            .unwrap_or((None, false, false, None, None, None));

        let key = self.connection_key;
        let pane_id = self.pane_id;
        let focus = self.focus_handle.clone();
        let click_owner = self.owner.clone();
        let right_click_owner = self.owner.clone();
        let body = div()
            .id(format!("terminal-pane-{key}-{}", pane_id.as_u64()))
            .debug_selector(move || format!("terminal-pane-{}", pane_id.as_u64()))
            .key_context("Murmur")
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                focus.focus(window, cx);
                let _ = click_owner.update(cx, |app, cx| app.select_pane(key, pane_id, cx));
            })
            .on_mouse_down(MouseButton::Right, move |_, _, cx| {
                let _ = right_click_owner.update(cx, |app, cx| {
                    app.set_target_pane(key, pane_id, cx);
                });
            })
            .size_full()
            .overflow_hidden()
            .p_1()
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(cx.theme().mono_font_size)
            .line_height(relative(1.35))
            .border_1()
            .border_color(if active {
                cx.theme().ring
            } else {
                cx.theme().border
            })
            .child(if let (Some(owner), Some(terminal)) = (owner, terminal) {
                TerminalElement::new(
                    owner,
                    TerminalElementProps {
                        focus_handle: self.focus_handle.clone(),
                        connection_key: key,
                        pane_id,
                        terminal: terminal.view,
                        marked_text,
                        selection,
                        runtime_epoch,
                        render_cache: self.render_cache.clone(),
                    },
                )
                .into_any_element()
            } else {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Terminal unavailable")
                    .into_any_element()
            });

        body.context_menu(move |menu, window, cx| {
            menu.menu_with_enable("Split Right", Box::new(SplitRight), controlling)
                .menu_with_enable("Split Down", Box::new(SplitDown), controlling)
                .separator()
                .submenu("Swap", window, cx, move |menu, _, _| {
                    menu.menu_with_enable("Left", Box::new(SwapLeft), controlling)
                        .menu_with_enable("Right", Box::new(SwapRight), controlling)
                        .menu_with_enable("Up", Box::new(SwapUp), controlling)
                        .menu_with_enable("Down", Box::new(SwapDown), controlling)
                })
                .separator()
                .menu_with_enable("Toggle Zoom", Box::new(ToggleZoom), controlling)
                .menu_with_enable("Close Pane", Box::new(ClosePane), controlling)
        })
    }
}

pub(crate) struct Murmur {
    connections: Vec<ServerConnection>,
    active_connection: ConnectionKey,
    next_connection_key: ConnectionKey,
    connect_results_tx: async_channel::Sender<ConnectionResult>,
    _connect_results_task: Task<()>,
    dock_area: Entity<DockArea>,
    _dock_subscription: Subscription,
    rebuilding_dock: bool,
    #[cfg(feature = "test-support")]
    dock_rebuild_count: usize,
    panels: HashMap<(ConnectionKey, PaneId), Entity<TerminalPanel>>,
    target_pane: Option<(ConnectionKey, PaneId)>,
    focus_handle: FocusHandle,
    terminal_selection: Option<LocalTerminalSelection>,
    pending_sizes: HashMap<(ConnectionKey, PaneId), TerminalSize>,
    terminal_geometry: HashMap<(ConnectionKey, PaneId), TerminalGeometry>,
    marked_text: Option<String>,
    app_error: Option<String>,
}

impl Murmur {
    fn new(
        endpoint: Endpoint,
        initial: Result<ClientConnection, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let dock_area = cx.new(|cx| {
            DockArea::new("murmur-workspace", None, window, cx)
                .with_renderer(Rc::new(MurmurDockRenderer))
        });
        let dock_subscription = cx.subscribe_in(
            &dock_area,
            window,
            |this, dock, event: &DockEvent, _, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.on_dock_layout_changed(dock, cx);
                }
            },
        );
        let (connect_results_tx, connect_results_rx) =
            async_channel::bounded(CONNECTION_RESULT_BUFFER_CAPACITY);
        let mut connection = ServerConnection::new(1, "Local".into(), endpoint);
        if let Err(error) = Self::install_connection(&mut connection, initial, window, cx) {
            connection.status = ConnectionStatus::Disconnected;
            connection.error = Some(error);
        }

        let mut this = Self {
            connections: vec![connection],
            active_connection: 1,
            next_connection_key: 2,
            connect_results_tx,
            _connect_results_task: Task::ready(()),
            dock_area,
            _dock_subscription: dock_subscription,
            rebuilding_dock: false,
            #[cfg(feature = "test-support")]
            dock_rebuild_count: 0,
            panels: HashMap::new(),
            target_pane: None,
            focus_handle: cx.focus_handle(),
            terminal_selection: None,
            pending_sizes: HashMap::new(),
            terminal_geometry: HashMap::new(),
            marked_text: None,
            app_error: None,
        };
        this.refresh_target_pane(1);
        this.acquire_and_subscribe(1);

        this._connect_results_task = cx.spawn_in(window, async move |owner, cx| {
            while let Ok(result) = connect_results_rx.recv().await {
                if owner
                    .update_in(cx, |this, window, cx| {
                        this.handle_connection_result(result, window, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = owner.update(cx, |this, cx| this.rebuild_dock(window, cx));
        });
        this
    }

    fn install_connection(
        connection: &mut ServerConnection,
        result: Result<ClientConnection, String>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Result<(), String> {
        let client = result?;
        let bootstrap = client.bootstrap().clone();
        let io = ClientIo::start(
            client,
            connection.key,
            connection.connect_generation,
            bootstrap.server_id,
            bootstrap.session_id,
            window,
            cx,
        )
        .map_err(|error| error.to_string())?;
        connection.apply_bootstrap(bootstrap);
        connection.io = Some(io);
        connection.controlling = false;
        connection.control_retry_attempts = 0;
        connection.control_retry_scheduled = false;
        Ok(())
    }

    fn connection(&self, key: ConnectionKey) -> Option<&ServerConnection> {
        self.connections
            .iter()
            .find(|connection| connection.key == key)
    }

    fn connection_mut(&mut self, key: ConnectionKey) -> Option<&mut ServerConnection> {
        self.connections
            .iter_mut()
            .find(|connection| connection.key == key)
    }

    fn active_connection(&self) -> Option<&ServerConnection> {
        self.connection(self.active_connection)
    }

    fn active_session(&self) -> Option<Session> {
        Session::restore(self.active_connection()?.snapshot.clone()).ok()
    }

    fn terminal(
        &self,
        connection_key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<&PaneTerminalSnapshot> {
        self.connection(connection_key)?.terminals.get(&pane_id)
    }

    fn acquire_and_subscribe(&mut self, key: ConnectionKey) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let Some(session_id) = connection.session_id else {
            return;
        };
        connection.send(ClientMessage::AcquireControl { session_id });
        connection.send(ClientMessage::Subscribe {
            session_id,
            after_sequence: connection.sequence,
        });
    }

    fn schedule_control_retry(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if connection.status != ConnectionStatus::Connected
            || connection.controlling
            || connection.control_retry_scheduled
            || connection.control_retry_attempts >= MAX_CONTROL_RETRY_ATTEMPTS
        {
            return;
        }
        let Some(session_id) = connection.session_id else {
            return;
        };
        let generation = connection.connect_generation;
        connection.control_retry_scheduled = true;
        connection.control_retry_attempts = connection.control_retry_attempts.saturating_add(1);

        cx.spawn(async move |owner, cx| {
            cx.background_executor().timer(CONTROL_RETRY_DELAY).await;
            owner
                .update(cx, |this, cx| {
                    let Some(connection) = this.connection_mut(key) else {
                        return;
                    };
                    if connection.connect_generation != generation {
                        return;
                    }
                    connection.control_retry_scheduled = false;
                    if connection.status == ConnectionStatus::Connected
                        && !connection.controlling
                        && connection.session_id == Some(session_id)
                    {
                        connection.send(ClientMessage::AcquireControl { session_id });
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    fn start_connect(&mut self, key: ConnectionKey) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if connection.status == ConnectionStatus::Connecting {
            return;
        }
        if let Some(io) = connection.io.take() {
            let _ = io.outgoing.send(ClientMessage::Detach);
        }
        connection.connect_generation = connection.connect_generation.wrapping_add(1);
        let generation = connection.connect_generation;
        connection.status = ConnectionStatus::Connecting;
        connection.controlling = false;
        connection.control_retry_attempts = 0;
        connection.control_retry_scheduled = false;
        connection.error = None;
        let endpoint = connection.endpoint.clone();
        let sender = self.connect_results_tx.clone();
        thread::spawn(move || {
            let connected_endpoint = if endpoint == ServerConfig::default().endpoint {
                murmur_server::ensure_local_server().unwrap_or(endpoint)
            } else {
                endpoint
            };
            let result = ClientConnection::connect(&connected_endpoint, "murmur-gui")
                .map_err(|error| error.to_string());
            let _ = sender.send_blocking(ConnectionResult {
                key,
                generation,
                endpoint: connected_endpoint,
                result,
            });
        });
    }

    fn disconnect_server(&mut self, key: ConnectionKey) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some(io) = connection.io.take() {
            let _ = io.outgoing.send(ClientMessage::Detach);
        }
        connection.connect_generation = connection.connect_generation.wrapping_add(1);
        connection.status = ConnectionStatus::Disconnected;
        connection.controlling = false;
        connection.control_retry_attempts = 0;
        connection.control_retry_scheduled = false;
        connection.error = None;
    }

    fn remove_server(&mut self, key: ConnectionKey, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect_server(key);
        let Some(index) = self
            .connections
            .iter()
            .position(|connection| connection.key == key)
        else {
            return;
        };
        let was_active = self.active_connection == key;
        self.connections.remove(index);
        self.panels
            .retain(|(connection_key, _), _| *connection_key != key);
        self.pending_sizes
            .retain(|(connection_key, _), _| *connection_key != key);
        self.terminal_geometry
            .retain(|(connection_key, _), _| *connection_key != key);
        if self
            .terminal_selection
            .is_some_and(|selection| selection.connection_key == key)
        {
            self.terminal_selection = None;
        }
        if was_active {
            self.active_connection = self
                .connections
                .get(index)
                .or_else(|| self.connections.last())
                .map_or(0, |connection| connection.key);
            self.refresh_target_pane(self.active_connection);
            self.rebuild_dock(window, cx);
        }
        cx.notify();
    }

    fn handle_connection_result(
        &mut self,
        result: ConnectionResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ConnectionResult {
            key,
            generation,
            endpoint,
            result,
        } = result;
        if !self.connection(key).is_some_and(|connection| {
            connection.status == ConnectionStatus::Connecting
                && connection.connect_generation == generation
        }) {
            return;
        }
        let installed = if let Some(connection) = self.connection_mut(key) {
            connection.endpoint = endpoint;
            match Self::install_connection(connection, result, window, cx) {
                Ok(()) => true,
                Err(error) => {
                    connection.status = ConnectionStatus::Disconnected;
                    connection.error = Some(error);
                    false
                }
            }
        } else {
            false
        };
        if installed {
            self.acquire_and_subscribe(key);
            self.refresh_target_pane(key);
            if key == self.active_connection {
                self.rebuild_dock(window, cx);
            }
        }
    }

    fn handle_incoming(
        &mut self,
        key: ConnectionKey,
        generation: u64,
        incoming: Incoming,
        cx: &mut Context<Self>,
    ) -> IncomingEffect {
        let Some(index) = self
            .connections
            .iter()
            .position(|connection| connection.key == key)
        else {
            return IncomingEffect::default();
        };
        if self.connections[index].connect_generation != generation {
            return IncomingEffect::default();
        }
        let message = match incoming {
            Incoming::Bootstrap(bootstrap) => {
                let rebuild = self.connections[index].apply_bootstrap(bootstrap);
                clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
                self.refresh_target_pane(key);
                return IncomingEffect {
                    rebuild,
                    notify: true,
                };
            }
            Incoming::Message(message) => message,
            Incoming::TerminalResync => {
                self.connections[index].request_snapshot();
                return IncomingEffect::default();
            }
            Incoming::VisualReady(_) => return IncomingEffect::default(),
            Incoming::Disconnected(error) => {
                let connection = &mut self.connections[index];
                connection.status = ConnectionStatus::Disconnected;
                connection.controlling = false;
                connection.control_retry_attempts = 0;
                connection.control_retry_scheduled = false;
                connection.error = Some(format!("Server connection closed: {error}"));
                connection.io = None;
                return IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                };
            }
        };

        match message {
            ServerMessage::Bootstrap(_)
            | ServerMessage::BootstrapBatch(_)
            | ServerMessage::TerminalFrameChunk(_) => {
                self.connections[index].request_snapshot();
                IncomingEffect::default()
            }
            ServerMessage::Event {
                server_id,
                session_id,
                sequence,
                event,
            } => {
                let connection = &mut self.connections[index];
                if connection.server_id != Some(server_id)
                    || connection.session_id != Some(session_id)
                {
                    return IncomingEffect::default();
                }
                if sequence != connection.sequence.saturating_add(1) {
                    if sequence > connection.sequence {
                        connection.request_snapshot();
                    }
                    return IncomingEffect::default();
                }
                self.connections[index].sequence = sequence;
                let mut notify = false;
                match event {
                    SessionEvent::LayoutChanged => {
                        self.connections[index].request_snapshot();
                    }
                    SessionEvent::TerminalExited { pane_id } => {
                        if let Some(terminal) = self.connections[index].terminals.get_mut(&pane_id)
                        {
                            terminal.exited = true;
                            notify = true;
                        } else {
                            self.connections[index].request_snapshot();
                        }
                    }
                    SessionEvent::AgentChanged { pane_id, agent } => {
                        let visible = self.active_connection == key
                            && self.target_pane == Some((key, pane_id));
                        if let Some(agent) = agent {
                            self.connections[index].agents.insert(pane_id, agent);
                            self.connections[index]
                                .agent_trackers
                                .entry(pane_id)
                                .and_modify(|tracker| tracker.update(agent.state, visible))
                                .or_insert_with(|| AgentTracker::new(agent.state));
                        } else {
                            self.connections[index].agents.remove(&pane_id);
                            self.connections[index].agent_trackers.remove(&pane_id);
                        }
                        notify = true;
                    }
                    SessionEvent::WorkspaceGitChanged { workspace_id, git } => {
                        if let Some(git) = git {
                            self.connections[index]
                                .workspace_git
                                .insert(workspace_id, git);
                        } else {
                            self.connections[index].workspace_git.remove(&workspace_id);
                        }
                        notify = true;
                    }
                }
                IncomingEffect {
                    rebuild: false,
                    notify,
                }
            }
            ServerMessage::TerminalFrame(batch) => {
                if self.connections[index].server_id != Some(batch.server_id)
                    || self.connections[index].session_id != Some(batch.session_id)
                {
                    return IncomingEffect::default();
                }

                let pane_ids = match apply_terminal_frame_batch(
                    &mut self.connections[index].terminals,
                    batch.panes,
                ) {
                    Ok(pane_ids) => pane_ids,
                    Err(()) => {
                        self.connections[index].request_snapshot();
                        return IncomingEffect::default();
                    }
                };
                for pane_id in &pane_ids {
                    let terminal_size = self.connections[index]
                        .terminals
                        .get(pane_id)
                        .expect("applied terminal still exists")
                        .view
                        .size;
                    let pending_key = (key, *pane_id);
                    if self.pending_sizes.get(&pending_key) == Some(&terminal_size) {
                        self.pending_sizes.remove(&pending_key);
                    }
                }
                IncomingEffect {
                    rebuild: false,
                    notify: !pane_ids.is_empty(),
                }
            }
            ServerMessage::ControlGranted { .. } => {
                self.connections[index].controlling = true;
                self.connections[index].control_retry_attempts = 0;
                self.connections[index].control_retry_scheduled = false;
                self.connections[index].error = None;
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::ControlReleased { .. } => {
                self.connections[index].controlling = false;
                self.connections[index].control_retry_attempts = 0;
                self.connections[index].control_retry_scheduled = false;
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::ControlDenied { reason, .. } => {
                let retry_control = reason == CONTROL_BUSY_REASON;
                self.connections[index].controlling = false;
                self.connections[index].error = Some(reason);
                if retry_control {
                    self.schedule_control_retry(key, cx);
                }
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::Error { message } => {
                self.connections[index].error = Some(message);
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::TerminalCopied { text, .. } => {
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                IncomingEffect::default()
            }
            ServerMessage::ServerStopping => {
                let connection = &mut self.connections[index];
                connection.status = ConnectionStatus::Disconnected;
                connection.controlling = false;
                connection.control_retry_attempts = 0;
                connection.control_retry_scheduled = false;
                connection.error = Some("murmur-server stopped".into());
                connection.io = None;
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::Welcome { .. }
            | ServerMessage::Subscribed { .. }
            | ServerMessage::Pong { .. } => IncomingEffect::default(),
        }
    }

    fn refresh_target_pane(&mut self, key: ConnectionKey) {
        let target = self.connection_focused_pane(key);
        if key == self.active_connection {
            self.target_pane = target.map(|pane_id| (key, pane_id));
            if let Some(pane_id) = target {
                self.mark_agent_seen(key, pane_id);
            }
        }
    }

    fn mark_agent_seen(&mut self, key: ConnectionKey, pane_id: PaneId) {
        if let Some(tracker) = self
            .connection_mut(key)
            .and_then(|connection| connection.agent_trackers.get_mut(&pane_id))
        {
            tracker.mark_seen();
        }
    }

    fn connection_focused_pane(&self, key: ConnectionKey) -> Option<PaneId> {
        self.connection(key)
            .and_then(|connection| Session::restore(connection.snapshot.clone()).ok())
            .and_then(|session| Some(session.active_workspace()?.active_tab().focused_pane().id()))
    }

    fn select_server(&mut self, key: ConnectionKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection(key).is_none() {
            return;
        }
        self.active_connection = key;
        self.refresh_target_pane(key);
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    fn select_workspace(&mut self, key: ConnectionKey, workspace_id: WorkspaceId) {
        self.active_connection = key;
        self.send_layout_to(key, LayoutCommand::ActivateWorkspace { workspace_id });
    }

    pub(crate) fn select_pane(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if self.set_target_pane(key, pane_id, cx).is_none()
            || self.connection_focused_pane(key) == Some(pane_id)
        {
            return;
        }
        self.send_layout_to(key, LayoutCommand::FocusPane { pane_id });
    }

    fn set_target_pane(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if !self
            .connection(key)
            .is_some_and(ServerConnection::can_mutate)
        {
            return None;
        }
        let changed = self.active_connection != key || self.target_pane != Some((key, pane_id));
        self.active_connection = key;
        self.target_pane = Some((key, pane_id));
        self.mark_agent_seen(key, pane_id);
        if changed {
            cx.notify();
        }
        Some(changed)
    }

    fn send_layout(&mut self, command: LayoutCommand) {
        self.send_layout_to(self.active_connection, command);
    }

    fn send_layout_to(&mut self, key: ConnectionKey, command: LayoutCommand) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return;
        };
        if connection.can_mutate() {
            connection.send(ClientMessage::Layout {
                server_id,
                session_id,
                command,
            });
        }
    }

    fn terminal_command(&mut self, key: ConnectionKey, pane_id: PaneId, command: TerminalCommand) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return;
        };
        let read_only = matches!(&command, TerminalCommand::Copy { .. });
        if (read_only || connection.can_mutate())
            && !connection
                .terminals
                .get(&pane_id)
                .is_some_and(|terminal| terminal.exited)
        {
            connection.send(ClientMessage::Terminal {
                server_id,
                session_id,
                pane_id,
                command,
            });
        }
    }

    fn focused_pane(&self) -> Option<PaneId> {
        self.target_pane
            .filter(|(key, _)| *key == self.active_connection)
            .map(|(_, pane_id)| pane_id)
            .or_else(|| {
                Some(
                    self.active_session()?
                        .active_workspace()?
                        .active_tab()
                        .focused_pane()
                        .id(),
                )
            })
    }

    fn rebuild_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "test-support")]
        {
            self.dock_rebuild_count += 1;
        }
        let Some(connection) = self.active_connection() else {
            return;
        };
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            self.target_pane = None;
            return;
        };
        let tab = workspace.active_tab();
        let focused = tab.focused_pane().id();
        let layout = connection
            .zoomed_panes
            .iter()
            .copied()
            .find(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .map(PaneLayout::Pane)
            .unwrap_or_else(|| tab.layout().clone());
        let key = connection.key;
        self.target_pane = Some((key, focused));
        let dock_layout = self.build_dock_layout(key, &layout, cx);
        let focus = self
            .panels
            .get(&(key, focused))
            .expect("focused terminal Panel exists in the rebuilt Dock")
            .read(cx)
            .focus_handle
            .clone();
        self.rebuilding_dock = true;
        self.dock_area.update(cx, |dock, cx| {
            dock.set_locked(true, window, cx);
            dock.set_center(dock_layout, window, cx);
        });
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let should_focus = owner
                .update(cx, |this, _| {
                    this.rebuilding_dock = false;
                    this.active_connection == key && this.target_pane == Some((key, focused))
                })
                .unwrap_or(false);
            if should_focus && !window.has_active_dialog(cx) {
                focus.focus(window, cx);
            }
        });
    }

    fn build_dock_layout(
        &mut self,
        key: ConnectionKey,
        layout: &PaneLayout,
        cx: &mut Context<Self>,
    ) -> DockLayout {
        match layout {
            PaneLayout::Pane(pane_id) => {
                let panel = self
                    .panels
                    .entry((key, *pane_id))
                    .or_insert_with(|| {
                        let owner = cx.weak_entity();
                        cx.new(|cx| TerminalPanel::new(key, *pane_id, owner, cx))
                    })
                    .clone();
                DockLayout::tabs().panel(panel)
            }
            PaneLayout::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let first = self.build_dock_layout(key, first, cx);
                let second = self.build_dock_layout(key, second, cx);
                let split = match direction {
                    SplitDirection::Horizontal => DockLayout::h_split(),
                    SplitDirection::Vertical => DockLayout::v_split(),
                };
                split
                    .child(first, Some(px(1000. * ratio)))
                    .child(second, Some(px(1000. * (1. - ratio))))
            }
        }
    }

    fn on_dock_layout_changed(&mut self, dock: &Entity<DockArea>, cx: &mut Context<Self>) {
        if self.rebuilding_dock {
            return;
        }
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(tab) = session
            .active_workspace()
            .map(|workspace| workspace.active_tab())
        else {
            return;
        };
        if self
            .active_connection()
            .is_some_and(|connection| !connection.zoomed_panes.is_empty())
        {
            return;
        }
        let mut ratios = Vec::new();
        collect_dock_ratios(&dock.read(cx).dump(cx).center, &mut ratios);
        let mut expected = Vec::new();
        collect_layout_ratios(tab.layout(), &mut expected);
        if ratios.len() == expected.len()
            && ratios
                .iter()
                .zip(expected)
                .any(|(actual, expected)| (actual - expected).abs() > 0.001)
        {
            self.send_layout(LayoutCommand::SetSplitRatios {
                tab_id: tab.id(),
                ratios,
            });
        }
    }

    fn reconnect_active(&mut self) {
        self.start_connect(self.active_connection);
    }

    fn new_workspace_on(&mut self, key: ConnectionKey, root_directory: PathBuf) {
        self.active_connection = key;
        self.target_pane = None;
        self.send_layout_to(key, LayoutCommand::CreateWorkspace { root_directory });
    }

    fn choose_workspace_directory_on(
        &mut self,
        key: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(label) = self
            .connection(key)
            .map(|connection| connection.label.clone())
        else {
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(format!("New Workspace on {label}").into()),
        });
        let owner = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let path = paths.await.ok()?.ok()??.into_iter().next()?;
                owner
                    .update(cx, |this, _| this.new_workspace_on(key, path))
                    .ok()?;
                Some(())
            })
            .detach();
    }

    fn new_tab(&mut self) {
        let Some(workspace_id) = self
            .active_session()
            .and_then(|session| session.active_workspace_id())
        else {
            return;
        };
        self.new_tab_on(self.active_connection, workspace_id);
    }

    fn new_tab_on(&mut self, key: ConnectionKey, workspace_id: WorkspaceId) {
        self.active_connection = key;
        self.target_pane = None;
        self.send_layout_to(key, LayoutCommand::CreateTab { workspace_id });
    }

    fn cycle_tab(&mut self, step: isize) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tabs = workspace.tabs();
        let Some(active_ix) = tabs
            .iter()
            .position(|tab| tab.id() == workspace.active_tab().id())
        else {
            return;
        };
        let target_ix = (active_ix as isize + step).rem_euclid(tabs.len() as isize) as usize;
        self.send_layout(LayoutCommand::ActivateTab {
            tab_id: tabs[target_ix].id(),
        });
    }

    fn move_workspace(&mut self, step: isize) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace_id) = session.active_workspace_id() else {
            return;
        };
        let Some(index) = session
            .workspaces()
            .iter()
            .position(|workspace| workspace.id() == workspace_id)
        else {
            return;
        };
        let target = (index as isize + step).clamp(0, session.workspaces().len() as isize - 1);
        self.send_layout(LayoutCommand::MoveWorkspace {
            workspace_id,
            target_index: target as u32,
        });
    }

    fn move_tab(&mut self, step: isize) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tab_id = workspace.active_tab().id();
        let Some(index) = workspace.tabs().iter().position(|tab| tab.id() == tab_id) else {
            return;
        };
        let target = (index as isize + step).clamp(0, workspace.tabs().len() as isize - 1);
        self.send_layout(LayoutCommand::MoveTab {
            tab_id,
            target_index: target as u32,
        });
    }

    fn split(&mut self, direction: SplitDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::SplitPane { pane_id, direction });
        }
    }

    fn focus_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::FocusPaneDirection { pane_id, direction });
        }
    }

    fn resize_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::ResizePane {
                pane_id,
                direction,
                amount: 0.05,
            });
        }
    }

    fn swap_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::SwapPane { pane_id, direction });
        }
    }

    fn toggle_zoom(&mut self) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::TogglePaneZoom { pane_id });
        }
    }

    fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane_id) = self.focused_pane() else {
            return;
        };
        let closes_workspace = self.active_session().is_some_and(|session| {
            session.workspaces().iter().any(|workspace| {
                workspace.tabs().len() == 1
                    && workspace.tabs()[0].panes().len() == 1
                    && workspace.tabs()[0].panes()[0].id() == pane_id
            })
        });
        self.confirm_close_on(
            self.active_connection,
            LayoutCommand::ClosePane { pane_id },
            closes_workspace,
            window,
            cx,
        );
    }

    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        self.close_tab_id(
            self.active_connection,
            workspace.active_tab().id(),
            workspace.tabs().len() == 1,
            window,
            cx,
        );
    }

    fn close_tab_id(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        closes_workspace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_close_on(
            key,
            LayoutCommand::CloseTab { tab_id },
            closes_workspace,
            window,
            cx,
        );
    }

    fn close_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace_id) = self
            .active_session()
            .and_then(|session| session.active_workspace_id())
        else {
            return;
        };
        self.close_workspace_id(self.active_connection, workspace_id, window, cx);
    }

    fn close_workspace_id(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_close_on(
            key,
            LayoutCommand::CloseWorkspace { workspace_id },
            true,
            window,
            cx,
        );
    }

    fn confirm_close_on(
        &mut self,
        key: ConnectionKey,
        command: LayoutCommand,
        closes_workspace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !closes_workspace {
            self.send_layout_to(key, command);
            return;
        }
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let owner = owner.clone();
                let command = command.clone();
                alert
                    .confirm()
                    .title("Close Workspace?")
                    .description("The workspace terminals will be stopped. Files are not deleted.")
                    .on_ok(move |_, _, cx| {
                        let _ =
                            owner.update(cx, |this, _| this.send_layout_to(key, command.clone()));
                        true
                    })
            });
        });
    }

    fn prompt_text(
        &mut self,
        title: &'static str,
        ok_text: &'static str,
        initial: String,
        apply: impl Fn(&mut Murmur, String) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let owner = cx.weak_entity();
        let apply = Rc::new(apply);
        window.defer(cx, move |window, cx| {
            let input_for_content = input.clone();
            let input_for_ok = input.clone();
            let owner = owner.clone();
            let apply = apply.clone();
            window.open_dialog(cx, move |dialog, _, _| {
                let input_for_content = input_for_content.clone();
                let input_for_ok = input_for_ok.clone();
                let owner = owner.clone();
                let apply = apply.clone();
                dialog
                    .title(title)
                    .content(move |content, _, _| {
                        content.child(Input::new(&input_for_content).w_full())
                    })
                    .footer(
                        DialogFooter::new()
                            .child(
                                Button::new("dialog-cancel")
                                    .debug_selector(|| "dialog-cancel".into())
                                    .label("Cancel")
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(Cancel), cx)
                                    }),
                            )
                            .child(
                                Button::new("dialog-primary-action")
                                    .debug_selector(|| "dialog-primary-action".into())
                                    .primary()
                                    .label(ok_text)
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(Confirm { secondary: false }),
                                            cx,
                                        )
                                    }),
                            ),
                    )
                    .on_ok(move |_, _, cx| {
                        let value = input_for_ok.read(cx).value().trim().to_owned();
                        if value.is_empty() {
                            return false;
                        }
                        let apply = apply.clone();
                        let _ = owner.update(cx, |this, cx| {
                            apply(this, value);
                            cx.notify();
                        });
                        true
                    })
            });
            input.update(cx, |input, cx| {
                input.focus(window, cx);
                input.select_all(window, cx);
            });
        });
    }

    fn prompt_add_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_text(
            "Add Server",
            "Add",
            "127.0.0.1:7341".into(),
            |this, value| {
                if let Ok(address) = value.parse::<SocketAddr>() {
                    let endpoint = Endpoint::tcp(address);
                    if this.connections.iter().any(|c| c.endpoint == endpoint) {
                        return;
                    }
                    this.app_error = None;
                    let key = this.next_connection_key;
                    this.next_connection_key += 1;
                    this.connections.push(ServerConnection::new(
                        key,
                        address.to_string(),
                        endpoint,
                    ));
                    this.active_connection = key;
                    this.target_pane = None;
                    this.start_connect(key);
                } else {
                    this.app_error = Some("Invalid server address".into());
                }
            },
            window,
            cx,
        );
    }

    fn prompt_rename_server_on(
        &mut self,
        key: ConnectionKey,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Rename Server",
            "Save",
            name,
            move |this, name| {
                if let Some(connection) = this.connection_mut(key) {
                    connection.label = name;
                }
            },
            window,
            cx,
        );
    }

    fn confirm_delete_server_on(
        &mut self,
        key: ConnectionKey,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let owner = owner.clone();
                alert
                    .title(format!("Delete \"{name}\"?"))
                    .description("This removes the Server from Murmur. Its terminals keep running.")
                    .button_props(
                        DialogButtonProps::default()
                            .ok_text("Delete")
                            .ok_variant(ButtonVariant::Danger)
                            .show_cancel(true)
                            .on_ok(move |_, window, cx| {
                                let _ = owner
                                    .update(cx, |this, cx| this.remove_server(key, window, cx));
                                true
                            }),
                    )
            });
        });
    }

    fn prompt_rename_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        self.prompt_rename_workspace_on(
            self.active_connection,
            workspace.id(),
            workspace.name().to_owned(),
            window,
            cx,
        );
    }

    fn prompt_rename_workspace_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Rename Workspace",
            "Save",
            name,
            move |this, name| {
                this.send_layout_to(key, LayoutCommand::RenameWorkspace { workspace_id, name });
            },
            window,
            cx,
        );
    }

    fn prompt_create_worktree_on(
        &mut self,
        key: ConnectionKey,
        parent_workspace_id: WorkspaceId,
        workspace_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Create Worktree",
            "Create",
            default_worktree_branch(&workspace_name),
            move |this, branch| {
                this.send_layout_to(
                    key,
                    LayoutCommand::CreateWorktree {
                        parent_workspace_id,
                        branch,
                    },
                );
            },
            window,
            cx,
        );
    }

    fn choose_worktree_directory_on(
        &mut self,
        key: ConnectionKey,
        parent_workspace_id: WorkspaceId,
        workspace_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(format!("Open Existing Worktree from {workspace_name}").into()),
        });
        let owner = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let root_directory = paths.await.ok()?.ok()??.into_iter().next()?;
                owner
                    .update(cx, |this, _| {
                        this.send_layout_to(
                            key,
                            LayoutCommand::OpenWorktree {
                                parent_workspace_id,
                                root_directory,
                            },
                        )
                    })
                    .ok()?;
                Some(())
            })
            .detach();
    }

    fn confirm_remove_worktree_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        workspace_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let owner = owner.clone();
                alert
                    .title(format!("Remove worktree \"{workspace_name}\"?"))
                    .description(
                        "The worktree directory will be deleted. Modified or untracked files prevent removal; the branch is kept.",
                    )
                    .button_props(
                        DialogButtonProps::default()
                            .ok_text("Remove")
                            .ok_variant(ButtonVariant::Danger)
                            .show_cancel(true)
                            .on_ok(move |_, _, cx| {
                                let _ = owner.update(cx, |this, _| {
                                    this.send_layout_to(
                                        key,
                                        LayoutCommand::RemoveWorktree { workspace_id },
                                    )
                                });
                                true
                            }),
                    )
            });
        });
    }

    fn prompt_rename_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tab = workspace.active_tab();
        self.prompt_rename_tab_on(
            self.active_connection,
            tab.id(),
            tab.name().to_owned(),
            window,
            cx,
        );
    }

    fn prompt_rename_tab_on(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Rename Tab",
            "Save",
            name,
            move |this, name| {
                this.send_layout_to(key, LayoutCommand::RenameTab { tab_id, name });
            },
            window,
            cx,
        );
    }

    fn dismiss_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            window.close_dialog(cx);
        }
    }

    fn action_add_server(&mut self, _: &AddServer, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.prompt_add_server(window, cx);
    }

    fn action_reconnect(
        &mut self,
        _: &ReconnectServer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.reconnect_active();
        cx.notify();
    }

    fn action_new_workspace(
        &mut self,
        _: &NewWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.choose_workspace_directory_on(self.active_connection, window, cx);
    }

    fn action_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.new_tab();
    }

    fn action_rename_workspace(
        &mut self,
        _: &RenameWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.prompt_rename_workspace(window, cx);
    }

    fn action_rename_tab(&mut self, _: &RenameTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.prompt_rename_tab(window, cx);
    }

    fn action_move_workspace_up(
        &mut self,
        _: &MoveWorkspaceUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_workspace(-1);
    }

    fn action_move_workspace_down(
        &mut self,
        _: &MoveWorkspaceDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_workspace(1);
    }

    fn action_move_tab_left(
        &mut self,
        _: &MoveTabLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_tab(-1);
    }

    fn action_move_tab_right(
        &mut self,
        _: &MoveTabRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_tab(1);
    }

    fn action_close_pane(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.close_pane(window, cx);
    }

    fn action_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.close_tab(window, cx);
    }

    fn action_close_workspace(
        &mut self,
        _: &CloseWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.close_workspace(window, cx);
    }

    fn action_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.cycle_tab(1);
    }

    fn action_previous_tab(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.cycle_tab(-1);
    }

    fn action_split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.split(SplitDirection::Horizontal);
    }

    fn action_split_down(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.split(SplitDirection::Vertical);
    }

    fn action_focus_left(&mut self, _: &FocusLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Left);
    }

    fn action_focus_right(&mut self, _: &FocusRight, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Right);
    }

    fn action_focus_up(&mut self, _: &FocusUp, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Up);
    }

    fn action_focus_down(&mut self, _: &FocusDown, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Down);
    }

    fn action_resize_left(&mut self, _: &ResizeLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Left);
    }

    fn action_resize_right(
        &mut self,
        _: &ResizeRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Right);
    }

    fn action_resize_up(&mut self, _: &ResizeUp, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Up);
    }

    fn action_resize_down(&mut self, _: &ResizeDown, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Down);
    }

    fn action_swap_left(&mut self, _: &SwapLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Left);
    }

    fn action_swap_right(&mut self, _: &SwapRight, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Right);
    }

    fn action_swap_up(&mut self, _: &SwapUp, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Up);
    }

    fn action_swap_down(&mut self, _: &SwapDown, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Down);
    }

    fn action_toggle_zoom(&mut self, _: &ToggleZoom, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.toggle_zoom();
    }

    pub(crate) fn resize_terminal(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        size: TerminalSize,
        cx: &mut Context<Self>,
    ) {
        let pending_key = (key, pane_id);
        if self.pending_sizes.get(&pending_key) == Some(&size)
            || self
                .terminal(key, pane_id)
                .is_some_and(|terminal| terminal.view.size == size)
        {
            return;
        }
        self.pending_sizes.insert(pending_key, size);
        self.terminal_command(key, pane_id, TerminalCommand::Resize(size));
        cx.notify();
    }

    pub(crate) fn update_terminal_geometry(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        bounds: Bounds<Pixels>,
        cell_size: Size<Pixels>,
    ) {
        self.terminal_geometry
            .insert((key, pane_id), TerminalGeometry { bounds, cell_size });
    }

    pub(crate) fn terminal_layout_is_current(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        bounds: Bounds<Pixels>,
        cell_size: Size<Pixels>,
        terminal_size: TerminalSize,
    ) -> bool {
        self.terminal_geometry.get(&(key, pane_id)) == Some(&TerminalGeometry { bounds, cell_size })
            && (self.pending_sizes.get(&(key, pane_id)) == Some(&terminal_size)
                || self
                    .terminal(key, pane_id)
                    .is_some_and(|terminal| terminal.view.size == terminal_size))
    }

    pub(crate) fn begin_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminal(key, pane_id) else {
            return;
        };
        let display_offset = terminal.view.display_offset;
        let multi_click_range = match click_count {
            2 => terminal
                .view
                .word_selection_at(position.row, position.column),
            3.. => terminal.view.line_selection_at(position.row),
            _ => None,
        };
        self.terminal_selection = Some(LocalTerminalSelection {
            connection_key: key,
            pane_id,
            range: multi_click_range.unwrap_or(TerminalSelection {
                start: position,
                end: position,
                display_offset,
            }),
            dragging: click_count == 1,
        });
        cx.notify();
    }

    pub(crate) fn update_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = &mut self.terminal_selection
            && selection.dragging
            && selection.connection_key == key
            && selection.pane_id == pane_id
            && selection.range.end != position
        {
            selection.range.end = position;
            cx.notify();
        }
    }

    pub(crate) fn is_selecting(&self, key: ConnectionKey, pane_id: PaneId) -> bool {
        self.terminal_selection.is_some_and(|selection| {
            selection.dragging && selection.connection_key == key && selection.pane_id == pane_id
        })
    }

    pub(crate) fn end_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        cx: &mut Context<Self>,
    ) {
        self.update_selection(key, pane_id, position, cx);
        if let Some(selection) = &mut self.terminal_selection
            && selection.connection_key == key
            && selection.pane_id == pane_id
        {
            selection.dragging = false;
        }
    }

    fn selection_for(&self, key: ConnectionKey, pane_id: PaneId) -> Option<TerminalSelection> {
        self.terminal_selection
            .filter(|selection| selection.connection_key == key && selection.pane_id == pane_id)
            .map(|selection| selection.range)
    }

    fn copy_terminal_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(selection) = self.selection_for(key, pane_id) else {
            return false;
        };
        let Some(columns) = self
            .terminal(key, pane_id)
            .map(|terminal| terminal.view.size.columns)
        else {
            return false;
        };
        if selection.selected_cell_range(columns).is_none() {
            return false;
        }

        self.terminal_command(key, pane_id, TerminalCommand::Copy { selection });
        self.clear_selection(cx);
        true
    }

    fn paste_into_terminal(&mut self, key: ConnectionKey, pane_id: PaneId, cx: &mut Context<Self>) {
        self.clear_selection(cx);
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.terminal_command(key, pane_id, TerminalCommand::Paste(text));
        }
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.terminal_selection.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn scroll_terminal(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        lines: i32,
        cx: &mut Context<Self>,
    ) {
        if lines != 0 {
            self.clear_selection(cx);
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Scroll(TerminalScroll::Lines(lines)),
            );
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((key, pane_id)) = self.target_pane else {
            return;
        };
        let stroke = &event.keystroke;
        if let Some(action) = fixed_shortcut(stroke) {
            window.dispatch_action(action, cx);
            cx.stop_propagation();
            return;
        }
        let modifiers = stroke.modifiers;
        let explicit_copy_paste = modifiers.platform || (modifiers.control && modifiers.shift);
        let retained_selection_copy = modifiers.control
            && !modifiers.alt
            && self
                .selection_for(key, pane_id)
                .and_then(|selection| {
                    let columns = self.terminal(key, pane_id)?.view.size.columns;
                    selection.selected_cell_range(columns)
                })
                .is_some();
        if stroke.key == "c" && (explicit_copy_paste || retained_selection_copy) {
            self.copy_terminal_selection(key, pane_id, cx);
            cx.stop_propagation();
            return;
        }
        if explicit_copy_paste && stroke.key == "v" {
            self.paste_into_terminal(key, pane_id, cx);
            cx.stop_propagation();
            return;
        }
        if modifiers.shift && stroke.key == "pageup" {
            self.clear_selection(cx);
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Scroll(TerminalScroll::PageUp),
            );
            cx.stop_propagation();
            return;
        }
        if modifiers.shift && stroke.key == "pagedown" {
            self.clear_selection(cx);
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Scroll(TerminalScroll::PageDown),
            );
            cx.stop_propagation();
            return;
        }

        let key_code = match stroke.key.as_str() {
            "enter" => Some(TerminalKey::Enter),
            "tab" if modifiers.shift => Some(TerminalKey::BackTab),
            "tab" => Some(TerminalKey::Tab),
            "backspace" => Some(TerminalKey::Backspace),
            "delete" => Some(TerminalKey::Delete),
            "escape" => Some(TerminalKey::Escape),
            "up" => Some(TerminalKey::Up),
            "down" => Some(TerminalKey::Down),
            "right" => Some(TerminalKey::Right),
            "left" => Some(TerminalKey::Left),
            "home" => Some(TerminalKey::Home),
            "end" => Some(TerminalKey::End),
            "pageup" => Some(TerminalKey::PageUp),
            "pagedown" => Some(TerminalKey::PageDown),
            "insert" => Some(TerminalKey::Insert),
            key if key.len() > 1 && key.starts_with('f') => key[1..]
                .parse::<u8>()
                .ok()
                .filter(|number| (1..=12).contains(number))
                .map(TerminalKey::Function),
            _ if modifiers.control || modifiers.alt || modifiers.platform => {
                Some(TerminalKey::Character(
                    stroke
                        .key_char
                        .clone()
                        .unwrap_or_else(|| stroke.key.clone()),
                ))
            }
            _ => None,
        };
        if let Some(key_code) = key_code {
            self.clear_selection(cx);
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Key {
                    key: key_code,
                    modifiers: TerminalModifiers {
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                        control: modifiers.control,
                        platform: modifiers.platform,
                    },
                },
            );
            cx.stop_propagation();
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let items = self.connections.iter().map(|connection| {
            let key = connection.key;
            let active_server = key == self.active_connection;
            let connected = connection.can_mutate();
            let label = match connection.status {
                ConnectionStatus::Connecting => format!("{} (connecting)", connection.label),
                ConnectionStatus::Connected => connection.label.clone(),
                ConnectionStatus::Disconnected => format!("{} (offline)", connection.label),
            };
            let workspaces = Session::restore(connection.snapshot.clone())
                .ok()
                .map(|session| {
                    let active_workspace = session.active_workspace_id();
                    session
                        .workspaces()
                        .iter()
                        .map(|workspace| {
                            let workspace_id = workspace.id();
                            let workspace_name = workspace.name().to_owned();
                            let git = connection.workspace_git.get(&workspace_id);
                            let branch = git.and_then(|git| git.branch.clone());
                            let supports_worktrees = git.is_some_and(|git| !git.linked_worktree)
                                && workspace.worktree().is_none();
                            let managed_worktree = workspace
                                .worktree()
                                .is_some_and(|association| association.is_managed());
                            let agents = workspace
                                .tabs()
                                .iter()
                                .flat_map(|tab| tab.panes())
                                .filter_map(|pane| {
                                    let pane_id = pane.id();
                                    let agent = connection.agents.get(&pane_id)?;
                                    let state = connection
                                        .agent_trackers
                                        .get(&pane_id)
                                        .map(|tracker| tracker.display_state().label())
                                        .unwrap_or_else(|| agent.state.label());
                                    let agent_owner = owner.clone();
                                    Some(
                                        SidebarMenuItem::new(agent.kind.label())
                                            .icon(IconName::Bot)
                                            .active(
                                                active_server
                                                    && self.target_pane == Some((key, pane_id)),
                                            )
                                            .suffix(move |_, cx| {
                                                div()
                                                    .debug_selector(move || {
                                                        format!("agent-{key}-{}", pane_id.as_u64())
                                                    })
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(state)
                                            })
                                            .disable(!connected)
                                            .on_click(move |_, _, cx| {
                                                let _ = agent_owner.update(cx, |this, cx| {
                                                    this.select_pane(key, pane_id, cx)
                                                });
                                            }),
                                    )
                                })
                                .collect::<Vec<_>>();
                            let owner = owner.clone();
                            let menu_owner = owner.clone();
                            SidebarMenuItem::new(workspace_name.clone())
                                .icon(IconName::Folder)
                                .active(active_server && active_workspace == Some(workspace_id))
                                .default_open(true)
                                .children(agents)
                                .disable(!connected)
                                .context_menu(move |menu, _, _| {
                                    let rename_owner = menu_owner.clone();
                                    let create_owner = menu_owner.clone();
                                    let open_owner = menu_owner.clone();
                                    let remove_owner = menu_owner.clone();
                                    let close_owner = menu_owner.clone();
                                    let rename_name = workspace_name.clone();
                                    let create_name = workspace_name.clone();
                                    let open_name = workspace_name.clone();
                                    let remove_name = workspace_name.clone();
                                    let menu = menu.item(
                                        PopupMenuItem::new("Rename Workspace…")
                                            .disabled(!connected)
                                            .on_click(move |_, window, cx| {
                                                let name = rename_name.clone();
                                                let _ = rename_owner.update(cx, |this, cx| {
                                                    this.prompt_rename_workspace_on(
                                                        key,
                                                        workspace_id,
                                                        name,
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    );
                                    let menu = if supports_worktrees {
                                        menu.separator()
                                            .item(
                                                PopupMenuItem::new("Create Worktree…")
                                                    .disabled(!connected)
                                                    .on_click(move |_, window, cx| {
                                                        let name = create_name.clone();
                                                        let _ =
                                                            create_owner.update(cx, |this, cx| {
                                                                this.prompt_create_worktree_on(
                                                                    key,
                                                                    workspace_id,
                                                                    name,
                                                                    window,
                                                                    cx,
                                                                )
                                                            });
                                                    }),
                                            )
                                            .item(
                                                PopupMenuItem::new("Open Existing Worktree…")
                                                    .disabled(!connected)
                                                    .on_click(move |_, window, cx| {
                                                        let name = open_name.clone();
                                                        let _ =
                                                            open_owner.update(cx, |this, cx| {
                                                                this.choose_worktree_directory_on(
                                                                    key,
                                                                    workspace_id,
                                                                    name,
                                                                    window,
                                                                    cx,
                                                                )
                                                            });
                                                    }),
                                            )
                                    } else {
                                        menu
                                    };
                                    let menu = if managed_worktree {
                                        menu.separator().item(
                                            PopupMenuItem::new("Remove Worktree…")
                                                .disabled(!connected)
                                                .on_click(move |_, window, cx| {
                                                    let name = remove_name.clone();
                                                    let _ = remove_owner.update(cx, |this, cx| {
                                                        this.confirm_remove_worktree_on(
                                                            key,
                                                            workspace_id,
                                                            name,
                                                            window,
                                                            cx,
                                                        )
                                                    });
                                                }),
                                        )
                                    } else {
                                        menu
                                    };
                                    menu.separator().item(
                                        PopupMenuItem::new("Close Workspace")
                                            .disabled(!connected)
                                            .on_click(move |_, window, cx| {
                                                let _ = close_owner.update(cx, |this, cx| {
                                                    this.close_workspace_id(
                                                        key,
                                                        workspace_id,
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    )
                                })
                                .when_some(branch, |item, branch| {
                                    item.suffix(move |_, cx| {
                                        div()
                                            .debug_selector(move || {
                                                format!("workspace-{key}-{}", workspace_id.as_u64())
                                            })
                                            .max_w(px(84.0))
                                            .truncate()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(branch.clone())
                                    })
                                })
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, _| {
                                        this.select_workspace(key, workspace_id)
                                    });
                                })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let select_owner = owner.clone();
            let new_workspace_owner = owner.clone();
            let server_menu_owner = owner.clone();
            let server_name = connection.label.clone();
            let status = connection.status;
            let new_workspace_label = connection.label.clone();
            SidebarMenuItem::new(label)
                .icon(IconName::HardDrive)
                .active(active_server)
                .default_open(true)
                .children(workspaces)
                .context_menu(move |menu, _, _| {
                    let rename_owner = server_menu_owner.clone();
                    let connection_owner = server_menu_owner.clone();
                    let delete_owner = server_menu_owner.clone();
                    let rename_name = server_name.clone();
                    let delete_name = server_name.clone();
                    let (connection_label, connection_disabled) = match status {
                        ConnectionStatus::Connected => ("Disconnect", false),
                        ConnectionStatus::Disconnected => ("Connect", false),
                        ConnectionStatus::Connecting => ("Connecting…", true),
                    };
                    menu.item(PopupMenuItem::new("Rename Server…").on_click(
                        move |_, window, cx| {
                            let name = rename_name.clone();
                            let _ = rename_owner.update(cx, |this, cx| {
                                this.prompt_rename_server_on(key, name, window, cx)
                            });
                        },
                    ))
                    .item(
                        PopupMenuItem::new(connection_label)
                            .disabled(connection_disabled)
                            .on_click(move |_, _, cx| {
                                let _ = connection_owner.update(cx, |this, cx| {
                                    match status {
                                        ConnectionStatus::Connected => this.disconnect_server(key),
                                        ConnectionStatus::Disconnected => this.start_connect(key),
                                        ConnectionStatus::Connecting => {}
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Delete Server").on_click(move |_, window, cx| {
                            let name = delete_name.clone();
                            let _ = delete_owner.update(cx, |this, cx| {
                                this.confirm_delete_server_on(key, name, window, cx)
                            });
                        }),
                    )
                })
                .suffix(move |_, _| {
                    let owner = new_workspace_owner.clone();
                    let tooltip = format!("New Workspace on {new_workspace_label}…");
                    Button::new(("new-workspace", key))
                        .debug_selector(move || format!("new-workspace-server-{key}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Plus)
                        .tooltip(tooltip)
                        .disabled(!connected)
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            let _ = owner.update(cx, |this, cx| {
                                this.choose_workspace_directory_on(key, window, cx)
                            });
                        })
                })
                .on_click(move |_, window, cx| {
                    let _ = select_owner.update(cx, |this, cx| this.select_server(key, window, cx));
                })
        });

        let add_owner = cx.weak_entity();
        let reconnect_owner = cx.weak_entity();
        let reconnect_visible = self
            .active_connection()
            .is_some_and(|connection| connection.status == ConnectionStatus::Disconnected);
        Sidebar::new("murmur-sidebar")
            .collapsible(SidebarCollapsible::None)
            .w_full()
            .header(
                SidebarHeader::new()
                    .child(Icon::new(IconName::SquareTerminal))
                    .child(div().flex_1().font_semibold().child(murmur_core::APP_NAME)),
            )
            .child(SidebarGroup::new("Servers").child(SidebarMenu::new().children(items)))
            .footer(
                SidebarFooter::new()
                    .child(
                        Button::new("add-server")
                            .debug_selector(|| "add-server".into())
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip("Add Server")
                            .on_click(move |_, window, cx| {
                                let _ = add_owner
                                    .update(cx, |this, cx| this.prompt_add_server(window, cx));
                            }),
                    )
                    .when(reconnect_visible, |footer| {
                        footer.child(
                            Button::new("reconnect-server")
                                .ghost()
                                .small()
                                .icon(IconName::LoaderCircle)
                                .tooltip("Reconnect")
                                .on_click(move |_, _, cx| {
                                    let _ = reconnect_owner.update(cx, |this, cx| {
                                        this.reconnect_active();
                                        cx.notify();
                                    });
                                }),
                        )
                    }),
            )
    }

    fn render_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connection) = self.active_connection() else {
            return div().size_full().into_any_element();
        };
        let error = self.app_error.clone().or_else(|| connection.error.clone());
        let can_mutate = connection.can_mutate();
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            return div()
                .size_full()
                .child("Invalid Session state")
                .into_any_element();
        };
        let Some(workspace) = session.active_workspace() else {
            let new_owner = cx.weak_entity();
            let key = connection.key;
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .when_some(error, |view, error| {
                    view.child(div().text_sm().text_color(cx.theme().danger).child(error))
                })
                .child(
                    Button::new("new-terminal-workspace")
                        .debug_selector(|| "new-terminal-workspace".into())
                        .primary()
                        .icon(IconName::SquareTerminal)
                        .label("New Workspace…")
                        .disabled(!can_mutate)
                        .on_click(move |_, window, cx| {
                            let _ = new_owner.update(cx, |this, cx| {
                                this.choose_workspace_directory_on(key, window, cx)
                            });
                        }),
                )
                .into_any_element();
        };

        let key = connection.key;
        let workspace_id = workspace.id();
        let active_tab = workspace.active_tab().id();
        let closes_workspace = workspace.tabs().len() == 1;
        let tab_buttons = workspace.tabs().iter().map(|tab| {
            let tab_id = tab.id();
            let tab_name = tab.name().to_owned();
            let activate_owner = cx.weak_entity();
            let menu_owner = cx.weak_entity();
            h_flex()
                .id(("tab-menu", tab_id.as_u64()))
                .child(
                    Button::new(("tab", tab_id.as_u64()))
                        .debug_selector(move || format!("tab-{}", tab_id.as_u64()))
                        .ghost()
                        .small()
                        .selected(tab_id == active_tab)
                        .label(tab.name().to_owned())
                        .disabled(!can_mutate)
                        .on_click(move |_, _, cx| {
                            let _ = activate_owner.update(cx, |this, _| {
                                this.send_layout_to(key, LayoutCommand::ActivateTab { tab_id })
                            });
                        }),
                )
                .context_menu(move |menu, _, _| {
                    let rename_owner = menu_owner.clone();
                    let close_owner = menu_owner.clone();
                    let rename_name = tab_name.clone();
                    menu.item(
                        PopupMenuItem::new("Rename Tab…")
                            .disabled(!can_mutate)
                            .on_click(move |_, window, cx| {
                                let name = rename_name.clone();
                                let _ = rename_owner.update(cx, |this, cx| {
                                    this.prompt_rename_tab_on(key, tab_id, name, window, cx)
                                });
                            }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Close Tab")
                            .disabled(!can_mutate)
                            .on_click(move |_, window, cx| {
                                let _ = close_owner.update(cx, |this, cx| {
                                    this.close_tab_id(key, tab_id, closes_workspace, window, cx)
                                });
                            }),
                    )
                })
        });
        let new_tab_owner = cx.weak_entity();

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_shrink_0()
                    .gap_1()
                    .px_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(tab_buttons)
                    .child(
                        Button::new("new-tab")
                            .debug_selector(|| "new-tab".into())
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip("New Tab")
                            .disabled(!can_mutate)
                            .on_click(move |_, _, cx| {
                                let _ = new_tab_owner
                                    .update(cx, |this, _| this.new_tab_on(key, workspace_id));
                            }),
                    )
                    .when_some(error, |row, error| {
                        row.child(
                            div()
                                .ml_auto()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    }),
            )
            .child(div().min_h_0().flex_1().child(self.dock_area.clone()))
            .into_any_element()
    }
}

impl Focusable for Murmur {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Murmur {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        adjusted_range.replace(
            0..self
                .marked_text
                .as_ref()
                .map_or(0, |text| text.encode_utf16().count()),
        );
        Some(self.marked_text.clone().unwrap_or_default())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let (Some(text), Some((key, pane_id))) = (item.text(), self.target_pane) {
            self.clear_selection(cx);
            self.terminal_command(key, pane_id, TerminalCommand::Paste(text));
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = None;
        if !text.is_empty()
            && let Some((key, pane_id)) = self.target_pane
        {
            self.clear_selection(cx);
            self.terminal_command(key, pane_id, TerminalCommand::Text(text.into()));
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let key = self.target_pane?;
        let geometry = self.terminal_geometry.get(&key)?;
        let cursor = self.terminal(key.0, key.1)?.view.cursor?;
        Some(Bounds::new(
            point(
                geometry.bounds.left() + geometry.cell_size.width * f32::from(cursor.column),
                geometry.bounds.top() + geometry.cell_size.height * f32::from(cursor.row),
            ),
            geometry.cell_size,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

impl Render for Murmur {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog_layer = Root::render_dialog_layer(window, cx);
        div()
            .key_context("Murmur")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::action_add_server))
            .on_action(cx.listener(Self::action_reconnect))
            .on_action(cx.listener(Self::action_new_workspace))
            .on_action(cx.listener(Self::action_new_tab))
            .on_action(cx.listener(Self::action_rename_workspace))
            .on_action(cx.listener(Self::action_rename_tab))
            .on_action(cx.listener(Self::action_move_workspace_up))
            .on_action(cx.listener(Self::action_move_workspace_down))
            .on_action(cx.listener(Self::action_move_tab_left))
            .on_action(cx.listener(Self::action_move_tab_right))
            .on_action(cx.listener(Self::action_close_pane))
            .on_action(cx.listener(Self::action_close_tab))
            .on_action(cx.listener(Self::action_close_workspace))
            .on_action(cx.listener(Self::action_next_tab))
            .on_action(cx.listener(Self::action_previous_tab))
            .on_action(cx.listener(Self::action_split_right))
            .on_action(cx.listener(Self::action_split_down))
            .on_action(cx.listener(Self::action_focus_left))
            .on_action(cx.listener(Self::action_focus_right))
            .on_action(cx.listener(Self::action_focus_up))
            .on_action(cx.listener(Self::action_focus_down))
            .on_action(cx.listener(Self::action_resize_left))
            .on_action(cx.listener(Self::action_resize_right))
            .on_action(cx.listener(Self::action_resize_up))
            .on_action(cx.listener(Self::action_resize_down))
            .on_action(cx.listener(Self::action_swap_left))
            .on_action(cx.listener(Self::action_swap_right))
            .on_action(cx.listener(Self::action_swap_up))
            .on_action(cx.listener(Self::action_swap_down))
            .on_action(cx.listener(Self::action_toggle_zoom))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_resizable("murmur-shell")
                    .child(
                        resizable_panel()
                            .size(px(240.))
                            .size_range(px(180.)..px(360.))
                            .flex_none()
                            .child(
                                div()
                                    .debug_selector(|| "murmur-sidebar".into())
                                    .size_full()
                                    .child(self.render_sidebar(cx)),
                            ),
                    )
                    .child(self.render_workspace(cx)),
            )
            .children(dialog_layer)
    }
}

fn collect_layout_ratios(layout: &PaneLayout, ratios: &mut Vec<f32>) {
    if let PaneLayout::Split {
        ratio,
        first,
        second,
        ..
    } = layout
    {
        ratios.push(*ratio);
        collect_layout_ratios(first, ratios);
        collect_layout_ratios(second, ratios);
    }
}

fn collect_dock_ratios(state: &PanelState, ratios: &mut Vec<f32>) {
    if let PanelInfo::Stack { sizes, .. } = &state.info
        && sizes.len() == 2
    {
        let total = sizes[0] + sizes[1];
        if total > px(0.) {
            ratios.push((sizes[0] / total).clamp(0.1, 0.9));
        }
    }
    for child in &state.children {
        collect_dock_ratios(child, ratios);
    }
}

fn fixed_shortcut(stroke: &Keystroke) -> Option<Box<dyn Action>> {
    let modifiers = stroke.modifiers;
    if modifiers.platform || modifiers.function {
        return None;
    }
    if !modifiers.control && modifiers.alt {
        // Windows GPUI turns Shift+= / Shift+- into + / _ and clears Shift.
        let key_char = stroke.key_char.as_deref();
        if (modifiers.shift && stroke.key == "=") || stroke.key == "+" || key_char == Some("+") {
            return Some(Box::new(SplitRight));
        }
        if (modifiers.shift && stroke.key == "-") || stroke.key == "_" || key_char == Some("_") {
            return Some(Box::new(SplitDown));
        }
    }
    match (
        modifiers.control,
        modifiers.alt,
        modifiers.shift,
        stroke.key.as_str(),
    ) {
        (true, false, true, "t") => Some(Box::new(NewTab)),
        (true, false, true, "w") => Some(Box::new(ClosePane)),
        (true, false, false, "tab") => Some(Box::new(NextTab)),
        (true, false, true, "tab") => Some(Box::new(PreviousTab)),
        (false, true, false, "left") => Some(Box::new(FocusLeft)),
        (false, true, false, "right") => Some(Box::new(FocusRight)),
        (false, true, false, "up") => Some(Box::new(FocusUp)),
        (false, true, false, "down") => Some(Box::new(FocusDown)),
        (false, true, true, "left") => Some(Box::new(ResizeLeft)),
        (false, true, true, "right") => Some(Box::new(ResizeRight)),
        (false, true, true, "up") => Some(Box::new(ResizeUp)),
        (false, true, true, "down") => Some(Box::new(ResizeDown)),
        _ => None,
    }
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewTab, Some("Murmur")),
        KeyBinding::new("ctrl-shift-w", ClosePane, Some("Murmur")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Murmur")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Murmur")),
        KeyBinding::new("alt-shift-=", SplitRight, Some("Murmur")),
        KeyBinding::new("alt-shift--", SplitDown, Some("Murmur")),
        KeyBinding::new("alt-left", FocusLeft, Some("Murmur")),
        KeyBinding::new("alt-right", FocusRight, Some("Murmur")),
        KeyBinding::new("alt-up", FocusUp, Some("Murmur")),
        KeyBinding::new("alt-down", FocusDown, Some("Murmur")),
        KeyBinding::new("alt-shift-left", ResizeLeft, Some("Murmur")),
        KeyBinding::new("alt-shift-right", ResizeRight, Some("Murmur")),
        KeyBinding::new("alt-shift-up", ResizeUp, Some("Murmur")),
        KeyBinding::new("alt-shift-down", ResizeDown, Some("Murmur")),
    ]);
}

fn main() {
    let endpoint =
        murmur_server::ensure_local_server().unwrap_or_else(|_| ServerConfig::default().endpoint);
    let initial =
        ClientConnection::connect(&endpoint, "murmur-gui").map_err(|error| error.to_string());
    let app = gpui_platform::application().with_assets(Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        bind_keys(cx);
        let window_options = default_window_options(cx);
        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| Murmur::new(endpoint, initial, window, cx));
                let root = cx.new(|cx| Root::new(view, window, cx));
                window.resize(DEFAULT_WINDOW_SIZE);
                root
            })
            .expect("failed to open Murmur window");
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use gpui::Keystroke;
    use murmur_core::protocol::{
        BootstrapBatch, BootstrapHeader, BootstrapRecord, PaneTerminalFrame, PaneTerminalSnapshot,
        RuntimeEpoch, ServerId, ServerMessage, SessionId, TerminalFrameBatch, TerminalFrameChunk,
        encode_bootstrap_record, encode_pane_terminal_frame,
    };
    use murmur_core::{
        Session, TerminalCell, TerminalCellRun, TerminalColor, TerminalSize, TerminalView,
        TerminalViewDelta, TerminalViewFrame,
    };

    use super::{
        FocusLeft, NextTab, PreviousTab, SplitDown, SplitRight, TerminalVisualSlot,
        apply_terminal_frame_batch, assemble_terminal_frame_chunk,
        clear_pending_sizes_for_bootstrap, enforce_terminal_chunk_reliable_fence, fixed_shortcut,
        merge_terminal_deltas, read_bootstrap_batches, terminal_chunk_identity_matches,
    };

    fn terminal_cell(text: &str) -> TerminalCell {
        TerminalCell {
            text: text.into(),
            foreground: TerminalColor::Named(0),
            background: TerminalColor::Named(0),
            flags: 0,
        }
    }

    fn terminal_view(revision: u64, text: &str) -> TerminalView {
        let cells = text
            .chars()
            .map(|character| terminal_cell(&character.to_string()))
            .collect::<Vec<_>>();
        TerminalView {
            revision,
            size: TerminalSize::new(1, u16::try_from(cells.len()).unwrap()),
            display_offset: 0,
            cells,
            cursor: None,
        }
    }

    fn pane_id() -> murmur_core::PaneId {
        let mut session = Session::new();
        session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id()
    }

    #[test]
    fn authoritative_bootstrap_releases_pending_resize_for_retry() {
        let pane_id = pane_id();
        let mut pending_sizes = std::collections::HashMap::from([
            ((1, pane_id), TerminalSize::new(40, 100)),
            ((2, pane_id), TerminalSize::new(20, 80)),
        ]);

        clear_pending_sizes_for_bootstrap(&mut pending_sizes, 1);

        assert!(!pending_sizes.contains_key(&(1, pane_id)));
        assert_eq!(
            pending_sizes.get(&(2, pane_id)),
            Some(&TerminalSize::new(20, 80))
        );
    }

    #[test]
    fn terminal_frame_chunks_are_exposed_only_after_complete_reassembly() {
        let pane_id = pane_id();
        let expected = PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Full(terminal_view(7, "chunked")),
        };
        let payload = encode_pane_terminal_frame(&expected).unwrap();
        let midpoint = payload.len() / 2;
        let chunks = [&payload[..midpoint], &payload[midpoint..]];
        let mut assembly = None;

        for (chunk_index, payload) in chunks.into_iter().enumerate() {
            let assembled = assemble_terminal_frame_chunk(
                &mut assembly,
                TerminalFrameChunk {
                    server_id: ServerId(1),
                    session_id: SessionId(1),
                    pane_id,
                    revision: 7,
                    chunk_index: chunk_index as u32,
                    chunk_count: 2,
                    payload: payload.to_vec(),
                },
            )
            .unwrap();
            if chunk_index == 0 {
                assert!(assembled.is_none());
            } else {
                assert_eq!(assembled, Some(expected.clone()));
            }
        }
        assert!(assembly.is_none());
    }

    #[test]
    fn terminal_chunk_identity_mismatch_aborts_the_in_progress_record() {
        let pane_id = pane_id();
        let mut assembly = None;
        assemble_terminal_frame_chunk(
            &mut assembly,
            TerminalFrameChunk {
                server_id: ServerId(1),
                session_id: SessionId(2),
                pane_id,
                revision: 7,
                chunk_index: 0,
                chunk_count: 2,
                payload: vec![1],
            },
        )
        .unwrap();

        let mismatched = TerminalFrameChunk {
            server_id: ServerId(9),
            session_id: SessionId(2),
            pane_id,
            revision: 7,
            chunk_index: 1,
            chunk_count: 2,
            payload: vec![2],
        };
        assert!(
            terminal_chunk_identity_matches(&mut assembly, &mismatched, ServerId(1), SessionId(2))
                .is_err()
        );
        assert!(assembly.is_none());
    }

    #[test]
    fn reliable_message_is_a_terminal_chunk_protocol_fence() {
        let pane_id = pane_id();
        let mut assembly = None;
        assemble_terminal_frame_chunk(
            &mut assembly,
            TerminalFrameChunk {
                server_id: ServerId(1),
                session_id: SessionId(2),
                pane_id,
                revision: 7,
                chunk_index: 0,
                chunk_count: 2,
                payload: vec![1],
            },
        )
        .unwrap();

        let reliable = ServerMessage::ControlGranted {
            server_id: ServerId(1),
            session_id: SessionId(2),
        };
        assert!(enforce_terminal_chunk_reliable_fence(&mut assembly, &reliable).is_err());
        assert!(assembly.is_none());
    }

    #[test]
    fn terminal_frame_batch_is_atomic_when_a_later_pane_has_a_gap() {
        let first_pane = pane_id();
        let second_pane = pane_id();
        let first_view = terminal_view(1, "a");
        let second_view = terminal_view(1, "b");
        let mut terminals = std::collections::HashMap::from([
            (
                first_pane,
                PaneTerminalSnapshot {
                    pane_id: first_pane,
                    view: first_view.clone(),
                    exited: false,
                },
            ),
            (
                second_pane,
                PaneTerminalSnapshot {
                    pane_id: second_pane,
                    view: second_view.clone(),
                    exited: false,
                },
            ),
        ]);
        let batch = vec![
            PaneTerminalFrame {
                pane_id: first_pane,
                frame: TerminalViewFrame::Delta(TerminalViewDelta {
                    base_revision: 1,
                    revision: 2,
                    display_offset: 0,
                    cursor: None,
                    runs: vec![TerminalCellRun {
                        start: 0,
                        cells: vec![terminal_cell("x")],
                    }],
                }),
            },
            PaneTerminalFrame {
                pane_id: second_pane,
                frame: TerminalViewFrame::Delta(TerminalViewDelta {
                    base_revision: 99,
                    revision: 100,
                    display_offset: 0,
                    cursor: None,
                    runs: vec![TerminalCellRun {
                        start: 0,
                        cells: vec![terminal_cell("y")],
                    }],
                }),
            },
        ];

        assert!(apply_terminal_frame_batch(&mut terminals, batch).is_err());
        assert_eq!(terminals[&first_pane].view, first_view);
        assert_eq!(terminals[&second_pane].view, second_view);
    }

    #[test]
    fn incomplete_bootstrap_batches_never_produce_a_partial_snapshot() {
        let mut session = Session::new();
        session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let terminal = PaneTerminalSnapshot {
            pane_id,
            view: terminal_view(3, "restore"),
            exited: false,
        };
        let payload =
            encode_bootstrap_record(&BootstrapRecord::Terminal(terminal.clone())).unwrap();
        let midpoint = payload.len() / 2;
        let header = BootstrapHeader {
            server_id: ServerId(1),
            runtime_epoch: RuntimeEpoch(2),
            session_id: SessionId(1),
            sequence: 4,
            snapshot: session.snapshot(),
            batch_count: 2,
        };
        let batches = [
            BootstrapBatch {
                server_id: ServerId(1),
                session_id: SessionId(1),
                batch_index: 0,
                record_index: 0,
                chunk_index: 0,
                chunk_count: 2,
                payload: payload[..midpoint].to_vec(),
            },
            BootstrapBatch {
                server_id: ServerId(1),
                session_id: SessionId(1),
                batch_index: 1,
                record_index: 0,
                chunk_index: 1,
                chunk_count: 2,
                payload: payload[midpoint..].to_vec(),
            },
        ];
        let mut complete = Vec::new();
        for batch in &batches {
            murmur_core::protocol::write_message(
                &mut complete,
                &ServerMessage::BootstrapBatch(batch.clone()),
            )
            .unwrap();
        }
        let assembled = read_bootstrap_batches(&mut complete.as_slice(), header.clone()).unwrap();
        assert_eq!(assembled.terminals, vec![terminal]);

        let mut incomplete = Vec::new();
        murmur_core::protocol::write_message(
            &mut incomplete,
            &ServerMessage::BootstrapBatch(batches[0].clone()),
        )
        .unwrap();
        assert!(read_bootstrap_batches(&mut incomplete.as_slice(), header).is_err());
    }

    #[test]
    fn terminal_shortcut_fallback_maps_only_fixed_chords() {
        let action = |keys: &str| fixed_shortcut(&Keystroke::parse(keys).unwrap());

        assert!(action("ctrl-tab").unwrap().as_any().is::<NextTab>());
        assert!(
            action("ctrl-shift-tab")
                .unwrap()
                .as_any()
                .is::<PreviousTab>()
        );
        assert!(action("alt-shift-=").unwrap().as_any().is::<SplitRight>());
        assert!(action("alt-shift--").unwrap().as_any().is::<SplitDown>());
        assert!(action("alt-+").unwrap().as_any().is::<SplitRight>());
        assert!(action("alt-_").unwrap().as_any().is::<SplitDown>());
        assert!(action("alt-left").unwrap().as_any().is::<FocusLeft>());
        assert!(action("ctrl-p").is_none());
    }

    #[test]
    fn gui_visual_slot_composes_pending_deltas_into_one_signal() {
        let pane_id = pane_id();
        let slot = TerminalVisualSlot::default();
        let batch = |frame| TerminalFrameBatch {
            server_id: ServerId(1),
            session_id: SessionId(2),
            panes: vec![PaneTerminalFrame { pane_id, frame }],
        };
        let first = TerminalViewDelta {
            base_revision: 1,
            revision: 2,
            display_offset: 0,
            cursor: None,
            runs: vec![TerminalCellRun {
                start: 1,
                cells: vec![terminal_cell("X")],
            }],
        };
        let second = TerminalViewDelta {
            base_revision: 2,
            revision: 3,
            display_offset: 0,
            cursor: None,
            runs: vec![TerminalCellRun {
                start: 2,
                cells: vec![terminal_cell("Y")],
            }],
        };

        assert_eq!(
            slot.publish(batch(TerminalViewFrame::Delta(first)))
                .unwrap(),
            Some(0)
        );
        assert_eq!(
            slot.publish(batch(TerminalViewFrame::Delta(second)))
                .unwrap(),
            None
        );
        let mut view = terminal_view(1, "abcd");
        let pending = slot.take(0).unwrap();
        assert_eq!(pending.panes.len(), 1);
        view.apply_frame(pending.panes.into_iter().next().unwrap().frame)
            .unwrap();
        assert_eq!(view.revision, 3);
        assert_eq!(view.cells[1].text.as_str(), "X");
        assert_eq!(view.cells[2].text.as_str(), "Y");
    }

    #[test]
    fn bootstrap_generation_discards_an_old_visual_signal() {
        let pane_id = pane_id();
        let slot = TerminalVisualSlot::default();
        let batch = |revision, text| TerminalFrameBatch {
            server_id: ServerId(1),
            session_id: SessionId(2),
            panes: vec![PaneTerminalFrame {
                pane_id,
                frame: TerminalViewFrame::Full(terminal_view(revision, text)),
            }],
        };

        assert_eq!(slot.publish(batch(1, "old")).unwrap(), Some(0));
        slot.advance();
        assert_eq!(slot.publish(batch(2, "new")).unwrap(), Some(1));
        assert!(slot.take(0).is_none());
        let current = slot.take(1).unwrap();
        assert!(matches!(
            &current.panes[0].frame,
            TerminalViewFrame::Full(view) if view.revision == 2
        ));
    }

    #[test]
    fn delta_composition_keeps_later_cell_values_and_latest_metadata() {
        let previous = TerminalViewDelta {
            base_revision: 4,
            revision: 5,
            display_offset: 0,
            cursor: None,
            runs: vec![TerminalCellRun {
                start: 1,
                cells: vec![terminal_cell("A"), terminal_cell("B")],
            }],
        };
        let next = TerminalViewDelta {
            base_revision: 5,
            revision: 6,
            display_offset: 3,
            cursor: None,
            runs: vec![TerminalCellRun {
                start: 2,
                cells: vec![terminal_cell("C")],
            }],
        };

        let merged = merge_terminal_deltas(previous, next).unwrap();
        assert_eq!((merged.base_revision, merged.revision), (4, 6));
        assert_eq!(merged.display_offset, 3);
        assert_eq!(merged.runs.len(), 1);
        assert_eq!(merged.runs[0].start, 1);
        assert_eq!(merged.runs[0].cells[0].text.as_str(), "A");
        assert_eq!(merged.runs[0].cells[1].text.as_str(), "C");
    }

    #[cfg(feature = "test-support")]
    mod visual {
        use std::cell::RefCell;
        use std::ops::Deref;
        use std::rc::Rc;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::thread::JoinHandle;
        use std::time::{Duration, Instant};

        use gpui::{
            AppContext as _, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
            TestAppContext, VisualTestContext, point, px, size,
        };
        use gpui_component::{Root, WindowExt as _};
        #[cfg(target_os = "linux")]
        use murmur_core::SplitDirection;
        use murmur_core::protocol::{ClientMessage, LayoutCommand, ServerMessage};
        use murmur_core::{PaneId, TabId, TerminalCommand, WorkspaceId};
        use murmur_server::{BoundServer, ClientConnection, Endpoint, ServerConfig, ServerHandle};

        use super::super::{
            CONTROL_BUSY_REASON, ConnectionResult, ConnectionStatus, DEFAULT_WINDOW_SIZE, Murmur,
            ServerConnection, default_window_options,
        };

        const TEST_TIMEOUT: Duration = Duration::from_secs(5);
        const TEST_POLL_INTERVAL: Duration = Duration::from_millis(2);
        static NEXT_TEST_SERVER_ID: AtomicU64 = AtomicU64::new(1);

        struct TestServer {
            handle: ServerHandle,
            thread: Option<JoinHandle<std::io::Result<()>>>,
        }

        impl TestServer {
            fn stop(&mut self) {
                self.handle.stop();
                if let Some(thread) = self.thread.take() {
                    thread.join().unwrap().unwrap();
                }
            }
        }

        impl Drop for TestServer {
            fn drop(&mut self) {
                self.handle.stop();
                if let Some(thread) = self.thread.take() {
                    let _ = thread.join();
                }
            }
        }

        fn start_server() -> (TestServer, Endpoint) {
            let endpoint = Endpoint::local(std::env::temp_dir().join(format!(
                "murmur-gui-{}-{}.sock",
                std::process::id(),
                NEXT_TEST_SERVER_ID.fetch_add(1, Ordering::Relaxed),
            )));
            let server = start_server_with_config(ServerConfig::ephemeral(endpoint.clone()));
            (server, endpoint)
        }

        fn start_server_with_config(config: ServerConfig) -> TestServer {
            let server = BoundServer::bind(config).unwrap();
            let handle = server.handle();
            let thread = std::thread::spawn(move || server.run());
            TestServer {
                handle,
                thread: Some(thread),
            }
        }

        fn connected_murmur(
            cx: &mut TestAppContext,
        ) -> (Entity<Murmur>, &mut VisualTestContext, TestServer) {
            let (server, endpoint) = start_server();
            connected_murmur_with(cx, server, endpoint)
        }

        fn connected_murmur_with(
            cx: &mut TestAppContext,
            server: TestServer,
            endpoint: Endpoint,
        ) -> (Entity<Murmur>, &mut VisualTestContext, TestServer) {
            let mut initial = None;
            let deadline = Instant::now() + TEST_TIMEOUT;
            while Instant::now() < deadline {
                if let Ok(connection) = ClientConnection::connect(&endpoint, "murmur-gui-test") {
                    initial = Some(connection);
                    break;
                }
                std::thread::sleep(TEST_POLL_INTERVAL);
            }
            let initial = initial.map_or_else(
                || Err("test server did not accept a client connection".into()),
                Ok,
            );
            let view_holder = Rc::new(RefCell::new(None));
            let view_holder_for_window = view_holder.clone();
            let (_root, window) = cx.add_window_view(move |window, cx| {
                let view = cx.new(|cx| Murmur::new(endpoint, initial, window, cx));
                view_holder_for_window.borrow_mut().replace(view.clone());
                Root::new(view, window, cx)
            });
            let view = view_holder
                .borrow_mut()
                .take()
                .expect("Murmur view should be created with the Root");
            if wait_until(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .is_some_and(ServerConnection::can_mutate)
                })
            }) {
                return (view, window, server);
            }
            panic!("GUI did not acquire control from the test server");
        }

        fn terminal_selector(pane_id: PaneId) -> &'static str {
            Box::leak(format!("terminal-pane-{}", pane_id.as_u64()).into_boxed_str())
        }

        fn tab_selector(tab_id: TabId) -> &'static str {
            Box::leak(format!("tab-{}", tab_id.as_u64()).into_boxed_str())
        }

        fn sidebar_workspace_selector(workspace_id: WorkspaceId) -> &'static str {
            Box::leak(format!("workspace-1-{}", workspace_id.as_u64()).into_boxed_str())
        }

        #[cfg(target_os = "linux")]
        fn sidebar_agent_selector(pane_id: PaneId) -> &'static str {
            Box::leak(format!("agent-1-{}", pane_id.as_u64()).into_boxed_str())
        }

        struct TestDirectory(std::path::PathBuf);

        impl TestDirectory {
            fn new(label: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "murmur-gui-{label}-{}-{}",
                    std::process::id(),
                    NEXT_TEST_SERVER_ID.fetch_add(1, Ordering::Relaxed),
                ));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
        }

        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        fn run_git<I, S>(cwd: &std::path::Path, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<std::ffi::OsStr>,
        {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(cwd)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "Git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn submit_text_dialog(window: &mut VisualTestContext, value: &str) {
            window.run_until_parked();
            window.update(|window, cx| _ = window.draw(cx));
            assert!(window.update(|window, cx| window.has_active_dialog(cx)));
            assert!(window.update(|window, cx| window.has_focused_input(cx)));
            window.simulate_input(value);
            window.update(|window, cx| _ = window.draw(cx));
            let cancel = window
                .debug_bounds("dialog-cancel")
                .expect("text dialog should render Cancel");
            let primary = window
                .debug_bounds("dialog-primary-action")
                .expect("text dialog should render its primary action");
            assert!(
                cancel.left() < primary.left(),
                "use the default button order"
            );
            assert!(
                primary.left() - cancel.right() <= px(12.),
                "keep text dialog actions compact"
            );
            window.simulate_click(primary.center(), Modifiers::default());
        }

        fn wait_until(
            window: &mut VisualTestContext,
            mut predicate: impl FnMut(&mut VisualTestContext) -> bool,
        ) -> bool {
            let deadline = Instant::now() + TEST_TIMEOUT;
            while Instant::now() < deadline {
                window.executor().advance_clock(Duration::from_millis(20));
                window.run_until_parked();
                if predicate(window) {
                    return true;
                }
                std::thread::sleep(TEST_POLL_INTERVAL);
            }
            false
        }

        fn wait_until_event_driven(
            window: &mut VisualTestContext,
            mut predicate: impl FnMut(&mut VisualTestContext) -> bool,
        ) -> bool {
            let deadline = Instant::now() + TEST_TIMEOUT;
            while Instant::now() < deadline {
                window.run_until_parked();
                if predicate(window) {
                    return true;
                }
                std::thread::sleep(TEST_POLL_INTERVAL);
            }
            false
        }

        #[test]
        fn default_window_options_create_1280_by_720_window() {
            let app = TestAppContext::single();
            let handle = app.update(|cx| {
                cx.open_window(default_window_options(cx), |_, cx| cx.new(|_| gpui::Empty))
                    .unwrap()
            });
            let window = VisualTestContext::from_window(*handle.deref(), &app).into_mut();
            let bounds = window.update(|window, _| window.bounds());

            assert_eq!(bounds.size, DEFAULT_WINDOW_SIZE);
            window.quit();
        }

        #[test]
        fn server_workspace_button_and_only_tab_close_round_trip() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);

            let add_server = window
                .debug_bounds("add-server")
                .expect("Add Server button should be rendered");
            window.simulate_click(add_server.center(), Modifiers::default());
            assert!(window.update(|window, cx| window.has_active_dialog(cx)));
            window.update(|window, cx| window.close_dialog(cx));
            window.run_until_parked();
            assert!(!window.update(|window, cx| window.has_active_dialog(cx)));

            let new_workspace = window
                .debug_bounds("new-workspace-server-1")
                .expect("Local server should expose New Workspace");
            window.simulate_click(new_workspace.center(), Modifiers::default());
            assert!(window.did_prompt_for_paths());
            window.simulate_path_prompt_response(|options| {
                assert!(!options.files);
                assert!(options.directories);
                assert!(!options.multiple);
                assert_eq!(options.prompt.as_deref(), Some("New Workspace on Local"));
                Some(vec![std::env::temp_dir()])
            });

            let mut tab_id = None;
            assert!(
                wait_until(window, |window| {
                    tab_id = window.read(|app| {
                        view.read(app)
                            .active_session()
                            .and_then(|session| Some(session.active_workspace()?.active_tab().id()))
                    });
                    tab_id.is_some()
                }),
                "server-scoped button should create a Workspace"
            );
            let tab_id = tab_id.unwrap();
            assert!(
                window.debug_bounds("close-tab").is_none(),
                "Tab row should not render a close button"
            );

            let tab = window
                .debug_bounds(tab_selector(tab_id))
                .expect("the only Tab should be rendered");
            window.simulate_mouse_down(tab.center(), MouseButton::Right, Modifiers::default());
            window.run_until_parked();
            window.update(|window, cx| {
                _ = window.draw(cx);
            });
            window.simulate_keystrokes("down enter");
            window.run_until_parked();
            assert!(window.update(|window, cx| window.has_active_dialog(cx)));
            window.update(|window, cx| {
                window.close_dialog(cx);
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::CloseTab { tab_id });
                });
            });

            let empty = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .active_session()
                        .is_some_and(|session| session.workspaces().is_empty())
                })
            });
            assert!(empty, "closing the only Tab should close its Workspace");
        }

        #[test]
        fn stale_connection_result_cannot_replace_the_current_attempt() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);

            window.update(|window, cx| {
                view.update(cx, |this, cx| {
                    let connection = this.connection_mut(1).unwrap();
                    let original_endpoint = connection.endpoint.clone();
                    let original_generation = connection.connect_generation;
                    connection.connect_generation = original_generation.wrapping_add(1);
                    connection.status = ConnectionStatus::Connecting;

                    this.handle_connection_result(
                        ConnectionResult {
                            key: 1,
                            generation: original_generation,
                            endpoint: Endpoint::tcp("127.0.0.1:9".parse().unwrap()),
                            result: Err("stale failure".into()),
                        },
                        window,
                        cx,
                    );

                    let connection = this.connection_mut(1).unwrap();
                    assert_eq!(connection.endpoint, original_endpoint);
                    assert!(connection.status == ConnectionStatus::Connecting);
                    assert!(connection.error.is_none());
                    connection.connect_generation = original_generation;
                    connection.status = ConnectionStatus::Connected;
                });
            });
        }

        #[test]
        fn denied_replacement_connection_retries_after_the_controller_releases() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);
            let endpoint = window.read(|app| {
                view.read(app)
                    .connection(1)
                    .expect("local connection exists")
                    .endpoint
                    .clone()
            });

            window.update(|_, cx| {
                view.update(cx, |this, cx| {
                    this.disconnect_server(1);
                    cx.notify();
                });
            });

            let contender = ClientConnection::connect(&endpoint, "control-contender").unwrap();
            let session_id = contender.bootstrap().session_id;
            let mut contender_stream = contender.into_stream();
            let deadline = Instant::now() + TEST_TIMEOUT;
            loop {
                murmur_core::protocol::write_message(
                    &mut contender_stream,
                    &ClientMessage::AcquireControl { session_id },
                )
                .unwrap();
                match murmur_core::protocol::read_message(&mut contender_stream).unwrap() {
                    ServerMessage::ControlGranted { .. } => break,
                    ServerMessage::ControlDenied { .. } => {
                        assert!(
                            Instant::now() < deadline,
                            "the disconnected GUI never released control"
                        );
                        std::thread::sleep(TEST_POLL_INTERVAL);
                    }
                    message => panic!("unexpected control response: {message:?}"),
                }
            }

            window.update(|_, cx| {
                view.update(cx, |this, cx| {
                    this.start_connect(1);
                    cx.notify();
                });
            });
            assert!(wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).connection(1).is_some_and(|connection| {
                        connection.status == ConnectionStatus::Connected
                            && !connection.controlling
                            && connection.error.as_deref() == Some(CONTROL_BUSY_REASON)
                    })
                })
            }));

            murmur_core::protocol::write_message(
                &mut contender_stream,
                &ClientMessage::ReleaseControl { session_id },
            )
            .unwrap();
            assert!(matches!(
                murmur_core::protocol::read_message(&mut contender_stream).unwrap(),
                ServerMessage::ControlReleased { .. }
            ));
            assert!(
                wait_until(window, |window| {
                    window.read(|app| {
                        view.read(app)
                            .connection(1)
                            .is_some_and(ServerConnection::can_mutate)
                    })
                }),
                "replacement connection did not retry control acquisition"
            );
        }

        #[test]
        fn server_disconnect_reconnect_and_remove_preserve_runtime() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);
            let identity = window.read(|app| {
                let connection = view.read(app).connection(1).unwrap();
                (
                    connection.server_id,
                    connection.runtime_epoch,
                    connection.session_id,
                )
            });

            window.update(|_, cx| {
                view.update(cx, |this, cx| {
                    this.disconnect_server(1);
                    cx.notify();
                });
            });
            assert!(window.read(|app| {
                view.read(app).connection(1).is_some_and(|connection| {
                    connection.status == ConnectionStatus::Disconnected && !connection.can_mutate()
                })
            }));

            window.update(|_, cx| {
                view.update(cx, |this, cx| {
                    this.start_connect(1);
                    let generation = this.connection(1).unwrap().connect_generation;
                    this.start_connect(1);
                    assert_eq!(
                        this.connection(1).unwrap().connect_generation,
                        generation,
                        "a duplicate reconnect must share the in-flight attempt"
                    );
                    cx.notify();
                });
            });
            let reconnected = wait_until_event_driven(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .is_some_and(ServerConnection::can_mutate)
                })
            });
            assert!(reconnected, "Server did not reconnect");
            assert_eq!(
                window.read(|app| {
                    let connection = view.read(app).connection(1).unwrap();
                    (
                        connection.server_id,
                        connection.runtime_epoch,
                        connection.session_id,
                    )
                }),
                identity,
                "reconnect should retain the Server runtime"
            );

            window.update(|window, cx| {
                view.update(cx, |this, cx| this.remove_server(1, window, cx));
            });
            assert!(window.read(|app| view.read(app).connections.is_empty()));
            window.quit();
        }

        #[test]
        fn replacement_server_restores_structure_with_fresh_terminal_state() {
            let directory = TestDirectory::new("persistent-restart");
            let workspace_root = directory.0.join("workspace");
            std::fs::create_dir_all(&workspace_root).unwrap();
            let endpoint = Endpoint::local(directory.0.join("server.sock"));
            let snapshot_path = directory.0.join("session.snapshot");
            let server = start_server_with_config(
                ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
            );

            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, mut server) =
                connected_murmur_with(&mut cx, server, endpoint.clone());
            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::CreateWorkspace {
                        root_directory: workspace_root.clone(),
                    });
                });
            });
            assert!(wait_until_event_driven(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session.active_workspace().is_some_and(|workspace| {
                            workspace.root_directory() == workspace_root.as_path()
                        })
                    })
                })
            }));

            let (server_id, runtime_epoch, expected_snapshot, pane_id) = window.read(|app| {
                let murmur = view.read(app);
                let connection = murmur.connection(1).unwrap();
                let pane_id = murmur
                    .active_session()
                    .unwrap()
                    .active_workspace()
                    .unwrap()
                    .active_tab()
                    .focused_pane()
                    .id();
                (
                    connection.server_id,
                    connection.runtime_epoch,
                    connection.snapshot.clone(),
                    pane_id,
                )
            });
            let old_terminal_marker = "MURMUR_PH6_OLD_TERMINAL_STATE";
            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.terminal_command(
                        1,
                        pane_id,
                        TerminalCommand::Text(old_terminal_marker.into()),
                    );
                });
            });
            assert!(wait_until_event_driven(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .and_then(|connection| connection.terminals.get(&pane_id))
                        .is_some_and(|terminal| {
                            terminal
                                .view
                                .cells
                                .iter()
                                .map(|cell| cell.text.as_str())
                                .collect::<String>()
                                .contains(old_terminal_marker)
                        })
                })
            }));

            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    let connection = this.connection_mut(1).unwrap();
                    connection.send(ClientMessage::StopServer {
                        server_id: connection.server_id.unwrap(),
                    });
                });
            });
            assert!(wait_until_event_driven(window, |window| {
                window.read(|app| {
                    view.read(app).connection(1).is_some_and(|connection| {
                        connection.status == ConnectionStatus::Disconnected
                    })
                })
            }));
            server.stop();

            let _replacement = start_server_with_config(
                ServerConfig::new(endpoint).with_snapshot_path(snapshot_path),
            );
            window.update(|_, cx| {
                view.update(cx, |this, cx| {
                    this.start_connect(1);
                    cx.notify();
                });
            });
            assert!(wait_until_event_driven(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .is_some_and(ServerConnection::can_mutate)
                })
            }));

            window.read(|app| {
                let murmur = view.read(app);
                let connection = murmur.connection(1).unwrap();
                assert_eq!(connection.server_id, server_id);
                assert_ne!(connection.runtime_epoch, runtime_epoch);
                assert_eq!(connection.snapshot, expected_snapshot);
                assert_eq!(connection.terminals.len(), 1);
                assert!(connection.agents.is_empty());
                assert!(
                    !connection.terminals[&pane_id]
                        .view
                        .cells
                        .iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                        .contains(old_terminal_marker)
                );
                assert_eq!(
                    murmur
                        .active_session()
                        .unwrap()
                        .active_workspace()
                        .unwrap()
                        .root_directory(),
                    workspace_root.as_path()
                );
            });
            window.update(|window, cx| _ = window.draw(cx));
            assert!(
                window.debug_bounds(terminal_selector(pane_id)).is_some(),
                "restored Pane should remain visible after reconnecting to the replacement Server"
            );
            window.quit();
        }

        #[test]
        fn corrupt_snapshot_connects_to_an_operable_start_page() {
            let directory = TestDirectory::new("corrupt-snapshot");
            let endpoint = Endpoint::local(directory.0.join("server.sock"));
            let snapshot_path = directory.0.join("session.snapshot");
            std::fs::write(&snapshot_path, b"not a Murmur snapshot").unwrap();
            let server = start_server_with_config(
                ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path),
            );

            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur_with(&mut cx, server, endpoint);
            assert!(window.read(|app| {
                view.read(app)
                    .active_session()
                    .is_some_and(|session| session.workspaces().is_empty())
            }));
            assert!(window.read(|app| {
                view.read(app)
                    .connection(1)
                    .is_some_and(|connection| connection.terminals.is_empty())
            }));
            window.update(|window, cx| _ = window.draw(cx));
            let new_workspace = window
                .debug_bounds("new-terminal-workspace")
                .expect("Start Page should offer New Workspace after a corrupt snapshot");
            window.simulate_click(new_workspace.center(), Modifiers::default());
            assert!(window.did_prompt_for_paths());
            window.simulate_path_prompt_response(|_| None);
            window.quit();
        }

        #[test]
        fn text_dialog_actions_are_compact_and_submit() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);

            window.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.prompt_rename_server_on(1, "Local".into(), window, cx)
                });
            });
            submit_text_dialog(window, "Build Server");

            assert_eq!(
                window.read(|app| view.read(app).connection(1).unwrap().label.clone()),
                "Build Server"
            );
        }

        #[test]
        fn server_events_wake_gui_without_polling_clock() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);

            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::CreateWorkspace {
                        root_directory: std::env::temp_dir(),
                    });
                });
            });

            assert!(
                wait_until_event_driven(window, |window| {
                    window.read(|app| {
                        view.read(app)
                            .active_session()
                            .is_some_and(|session| session.active_workspace().is_some())
                    })
                }),
                "server events should wake GPUI without a timer tick"
            );
        }

        #[test]
        fn terminal_drag_selection_updates_locally() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);

            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::CreateWorkspace {
                        root_directory: std::env::temp_dir(),
                    });
                });
            });

            let mut pane_id = None;
            assert!(wait_until(window, |window| {
                pane_id = window.read(|app| {
                    view.read(app)
                        .active_session()?
                        .active_workspace()
                        .map(|workspace| workspace.active_tab().focused_pane().id())
                });
                pane_id.is_some_and(|pane_id| {
                    window.debug_bounds(terminal_selector(pane_id)).is_some()
                })
            }));
            let pane_id = pane_id.unwrap();
            let render_cache = window.read(|app| {
                view.read(app)
                    .panels
                    .get(&(1, pane_id))
                    .unwrap()
                    .read(app)
                    .render_cache
                    .clone()
            });
            let mut last_revision = None;
            let mut stable_since = Instant::now();
            assert!(wait_until(window, |window| {
                window.update(|window, cx| _ = window.draw(cx));
                let (revision, resize_pending) = window.read(|app| {
                    let murmur = view.read(app);
                    (
                        murmur.terminal(1, pane_id).unwrap().view.revision,
                        murmur.pending_sizes.contains_key(&(1, pane_id)),
                    )
                });
                if last_revision != Some(revision) {
                    last_revision = Some(revision);
                    stable_since = Instant::now();
                }
                !resize_pending
                    && render_cache.borrow().shaped_cells() > 0
                    && stable_since.elapsed() >= Duration::from_millis(50)
            }));
            let shaped_before_drag = render_cache.borrow().shaped_cells();
            let terminal = window.debug_bounds(terminal_selector(pane_id)).unwrap();
            let start = point(
                terminal.left() + terminal.size.width * 0.25,
                terminal.top() + terminal.size.height * 0.35,
            );
            let end = point(
                terminal.left() + terminal.size.width * 0.75,
                terminal.top() + terminal.size.height * 0.65,
            );

            window.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
            window.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
            let dragging = window.read(|app| view.read(app).terminal_selection.unwrap());
            assert!(dragging.dragging);
            assert_eq!((dragging.connection_key, dragging.pane_id), (1, pane_id));
            assert_ne!(dragging.range.start, dragging.range.end);
            window.update(|window, cx| _ = window.draw(cx));
            assert_eq!(
                render_cache.borrow().shaped_cells(),
                shaped_before_drag,
                "selection-only frames must reuse shaped terminal cells"
            );

            window.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
            let completed = window.read(|app| view.read(app).terminal_selection.unwrap());
            assert!(!completed.dragging);
            assert_eq!(completed.range, dragging.range);
        }

        #[test]
        fn terminal_double_click_and_ctrl_c_copy_a_word() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);

            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::CreateWorkspace {
                        root_directory: std::env::temp_dir(),
                    });
                });
            });

            let mut pane_id = None;
            assert!(wait_until(window, |window| {
                pane_id = window.read(|app| {
                    view.read(app)
                        .active_session()?
                        .active_workspace()
                        .map(|workspace| workspace.active_tab().focused_pane().id())
                });
                pane_id.is_some_and(|pane_id| {
                    window.debug_bounds(terminal_selector(pane_id)).is_some()
                })
            }));
            let pane_id = pane_id.unwrap();

            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.terminal_command(
                        1,
                        pane_id,
                        TerminalCommand::Text("echo MURMUR_COPY_WORD\r".into()),
                    );
                });
            });

            let word = "MURMUR_COPY_WORD";
            let word_chars = word.chars().collect::<Vec<_>>();
            let mut word_cell = None;
            assert!(wait_until_event_driven(window, |window| {
                word_cell = window.read(|app| {
                    let terminal = &view.read(app).terminal(1, pane_id)?.view;
                    for row in 0..terminal.size.rows {
                        for column in 0..terminal.size.columns {
                            let matches =
                                word_chars.iter().enumerate().all(|(offset, expected)| {
                                    let Ok(offset) = u16::try_from(offset) else {
                                        return false;
                                    };
                                    terminal
                                        .cell(row, column.saturating_add(offset))
                                        .and_then(|cell| cell.text.chars().next())
                                        == Some(*expected)
                                });
                            if matches {
                                return Some((row, column));
                            }
                        }
                    }
                    None
                });
                word_cell.is_some()
            }));
            let (row, column) = word_cell.unwrap();
            let click = window.read(|app| {
                let geometry = view
                    .read(app)
                    .terminal_geometry
                    .get(&(1, pane_id))
                    .copied()
                    .unwrap();
                point(
                    geometry.bounds.left() + geometry.cell_size.width * (f32::from(column) + 0.5),
                    geometry.bounds.top() + geometry.cell_size.height * (f32::from(row) + 0.5),
                )
            });

            window.simulate_event(MouseDownEvent {
                button: MouseButton::Left,
                position: click,
                modifiers: Modifiers::default(),
                click_count: 2,
                first_mouse: false,
            });
            window.simulate_event(MouseUpEvent {
                button: MouseButton::Left,
                position: click,
                modifiers: Modifiers::default(),
                click_count: 2,
            });

            let selection = window.read(|app| view.read(app).terminal_selection.unwrap());
            assert!(!selection.dragging);
            assert_eq!(selection.range.start.column, column);
            assert_eq!(
                selection.range.end.column,
                column + u16::try_from(word_chars.len()).unwrap() - 1
            );

            window.simulate_keystrokes("ctrl-c");
            assert!(window.read(|app| view.read(app).terminal_selection.is_none()));
            assert!(wait_until_event_driven(window, |window| {
                window
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .is_some_and(|text| text == word)
            }));
        }

        #[test]
        fn rename_dialogs_commit_server_workspace_and_tab_names() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);

            window.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.prompt_rename_server_on(1, "Local".into(), window, cx)
                });
            });
            submit_text_dialog(window, "Build Server");
            assert_eq!(
                window.read(|app| view.read(app).connection(1).unwrap().label.clone()),
                "Build Server"
            );

            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::CreateWorkspace {
                        root_directory: std::env::temp_dir(),
                    });
                });
            });
            assert!(wait_until(window, |window| {
                window
                    .read(|app| {
                        let session = view.read(app).active_session()?;
                        let workspace = session.active_workspace()?;
                        Some((workspace.id(), workspace.active_tab().id()))
                    })
                    .is_some()
            }));
            let (workspace_id, tab_id) = window
                .read(|app| {
                    let session = view.read(app).active_session()?;
                    let workspace = session.active_workspace()?;
                    Some((workspace.id(), workspace.active_tab().id()))
                })
                .unwrap();

            window.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.prompt_rename_workspace_on(
                        1,
                        workspace_id,
                        "Workspace 1".into(),
                        window,
                        cx,
                    )
                });
            });
            submit_text_dialog(window, "Build Workspace");
            let workspace_renamed = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session
                            .workspace(workspace_id)
                            .is_some_and(|workspace| workspace.name() == "Build Workspace")
                    })
                })
            });
            assert!(
                workspace_renamed,
                "Workspace rename did not reach the server"
            );

            window.update(|window, cx| {
                view.update(cx, |this, cx| {
                    this.prompt_rename_tab_on(1, tab_id, "Tab 1".into(), window, cx)
                });
            });
            submit_text_dialog(window, "Build Tab");
            let tab_renamed = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session
                            .tab(tab_id)
                            .is_some_and(|tab| tab.name() == "Build Tab")
                    })
                })
            });
            assert!(tab_renamed, "Tab rename did not reach the server");
        }

        #[test]
        fn worktree_actions_use_the_workspace_context_and_real_server() {
            let temp = TestDirectory::new("worktree-ui");
            let repository = temp.0.join("repository");
            let worktree = temp.0.join("existing-worktree");
            let created_branch = format!(
                "feature/ui-created-{}-{}",
                std::process::id(),
                NEXT_TEST_SERVER_ID.fetch_add(1, Ordering::Relaxed)
            );
            std::fs::create_dir_all(&repository).unwrap();
            run_git(&repository, ["init"]);
            run_git(&repository, ["config", "user.name", "Murmur Tests"]);
            run_git(
                &repository,
                ["config", "user.email", "murmur@example.invalid"],
            );
            std::fs::write(repository.join("README.md"), "murmur\n").unwrap();
            run_git(&repository, ["add", "README.md"]);
            run_git(&repository, ["commit", "-m", "initial"]);
            run_git(
                &repository,
                [
                    std::ffi::OsString::from("worktree"),
                    std::ffi::OsString::from("add"),
                    std::ffi::OsString::from("-b"),
                    std::ffi::OsString::from("feature/ui"),
                    worktree.as_os_str().to_os_string(),
                ],
            );

            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);
            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::CreateWorkspace {
                        root_directory: repository.clone(),
                    });
                });
            });
            let mut parent_workspace_id = None;
            assert!(wait_until(window, |window| {
                window.read(|app| {
                    let murmur = view.read(app);
                    parent_workspace_id = murmur
                        .active_session()
                        .and_then(|session| session.active_workspace_id());
                    parent_workspace_id.is_some()
                        && murmur
                            .connection(1)
                            .is_some_and(|connection| !connection.workspace_git.is_empty())
                })
            }));
            window.update(|window, cx| _ = window.draw(cx));

            let workspace = window
                .debug_bounds(sidebar_workspace_selector(parent_workspace_id.unwrap()))
                .expect("Git Workspace should render in the sidebar");
            window.simulate_mouse_down(
                workspace.center(),
                MouseButton::Right,
                Modifiers::default(),
            );
            window.run_until_parked();
            window.update(|window, cx| _ = window.draw(cx));
            window.simulate_keystrokes("down down enter");
            window.run_until_parked();
            assert!(
                window.update(|window, cx| window.has_active_dialog(cx)),
                "Create Worktree should open its branch dialog"
            );
            submit_text_dialog(window, &created_branch);

            let mut managed_workspace = None;
            assert!(wait_until(window, |window| {
                window.read(|app| {
                    let murmur = view.read(app);
                    managed_workspace = murmur.active_session().and_then(|session| {
                        session.workspaces().iter().find_map(|workspace| {
                            workspace
                                .worktree()
                                .is_some_and(|association| association.is_managed())
                                .then(|| (workspace.id(), workspace.root_directory().to_path_buf()))
                        })
                    });
                    managed_workspace.as_ref().is_some_and(|(workspace_id, _)| {
                        murmur
                            .connection(1)
                            .and_then(|connection| connection.workspace_git.get(workspace_id))
                            .and_then(|git| git.branch.as_deref())
                            == Some(created_branch.as_str())
                    })
                })
            }));
            let (managed_workspace_id, managed_root) = managed_workspace.unwrap();
            std::fs::write(managed_root.join("untracked.txt"), "keep me\n").unwrap();

            window.update(|window, cx| _ = window.draw(cx));
            let managed = window
                .debug_bounds(sidebar_workspace_selector(managed_workspace_id))
                .expect("managed worktree branch should render in the sidebar");
            window.simulate_mouse_down(managed.center(), MouseButton::Right, Modifiers::default());
            window.run_until_parked();
            window.update(|window, cx| _ = window.draw(cx));
            window.simulate_keystrokes("down down enter");
            window.run_until_parked();
            assert!(window.update(|window, cx| window.has_active_dialog(cx)));
            window.simulate_keystrokes("enter");
            assert!(wait_until(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .and_then(|connection| connection.error.as_deref())
                        .is_some_and(|error| error.contains("modified or untracked"))
                })
            }));
            assert!(managed_root.exists());

            std::fs::remove_file(managed_root.join("untracked.txt")).unwrap();
            window.update(|window, cx| _ = window.draw(cx));
            let managed = window
                .debug_bounds(sidebar_workspace_selector(managed_workspace_id))
                .unwrap();
            window.simulate_mouse_down(managed.center(), MouseButton::Right, Modifiers::default());
            window.run_until_parked();
            window.update(|window, cx| _ = window.draw(cx));
            window.simulate_keystrokes("down down enter");
            window.run_until_parked();
            assert!(window.update(|window, cx| window.has_active_dialog(cx)));
            window.simulate_keystrokes("enter");
            assert!(wait_until(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .active_session()
                        .is_some_and(|session| session.workspace(managed_workspace_id).is_none())
                })
            }));
            assert!(!managed_root.exists());
            run_git(
                &repository,
                [
                    std::ffi::OsString::from("show-ref"),
                    std::ffi::OsString::from("--verify"),
                    std::ffi::OsString::from(format!("refs/heads/{created_branch}")),
                ],
            );

            window.update(|window, cx| _ = window.draw(cx));
            let workspace = window
                .debug_bounds(sidebar_workspace_selector(parent_workspace_id.unwrap()))
                .unwrap();
            window.simulate_mouse_down(
                workspace.center(),
                MouseButton::Right,
                Modifiers::default(),
            );
            window.run_until_parked();
            window.update(|window, cx| _ = window.draw(cx));
            window.simulate_keystrokes("down down down enter");
            window.run_until_parked();
            assert!(window.did_prompt_for_paths());
            let selected_worktree = worktree.clone();
            window.simulate_path_prompt_response(move |options| {
                assert!(!options.files);
                assert!(options.directories);
                assert!(!options.multiple);
                Some(vec![selected_worktree])
            });

            assert!(wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session.workspaces().iter().any(|workspace| {
                            workspace.root_directory() == worktree
                                && workspace
                                    .worktree()
                                    .is_some_and(|association| !association.is_managed())
                        })
                    })
                })
            }));
            assert!(window.read(|app| {
                view.read(app)
                    .connection(1)
                    .unwrap()
                    .workspace_git
                    .values()
                    .any(|git| git.branch.as_deref() == Some("feature/ui"))
            }));
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn detected_agent_sidebar_item_activates_its_real_pty_pane() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, _server) = connected_murmur(&mut cx);
            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::CreateWorkspace {
                        root_directory: std::env::temp_dir(),
                    });
                });
            });
            let mut agent_pane = None;
            assert!(wait_until(window, |window| {
                agent_pane = window.read(|app| {
                    view.read(app)
                        .active_session()?
                        .active_workspace()
                        .map(|workspace| workspace.active_tab().focused_pane().id())
                });
                agent_pane.is_some()
            }));
            let agent_pane = agent_pane.unwrap();
            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.send_layout(LayoutCommand::SplitPane {
                        pane_id: agent_pane,
                        direction: SplitDirection::Horizontal,
                    });
                });
            });
            let mut other_pane = None;
            assert!(wait_until(window, |window| {
                other_pane = window.read(|app| {
                    let session = view.read(app).active_session()?;
                    let tab = session.active_workspace()?.active_tab();
                    (tab.panes().len() == 2).then(|| tab.focused_pane().id())
                });
                other_pane.is_some_and(|pane_id| pane_id != agent_pane)
            }));

            window.update(|_, cx| {
                view.update(cx, |this, _| {
                    this.terminal_command(
                        1,
                        agent_pane,
                        TerminalCommand::Text(
                            "exec -a codex /bin/bash -c \"echo '◦ Working (1s - esc to interrupt)'; sleep 1; printf '\\033[2J\\033[H›\\n'; sleep 30 & wait\"\r"
                                .into(),
                        ),
                    );
                });
            });
            let working = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .and_then(|connection| connection.agents.get(&agent_pane))
                        .is_some_and(|agent| agent.state == murmur_core::AgentState::Working)
                })
            });
            assert!(
                working,
                "working Agent was not detected; terminal={:?}; error={:?}",
                window.read(|app| {
                    view.read(app).terminal(1, agent_pane).map(|terminal| {
                        terminal
                            .view
                            .cells
                            .iter()
                            .map(|cell| cell.text.as_str())
                            .collect::<String>()
                    })
                }),
                window.read(|app| view.read(app).connection(1).unwrap().error.clone()),
            );
            assert!(wait_until(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .and_then(|connection| connection.agent_trackers.get(&agent_pane))
                        .is_some_and(|tracker| tracker.display_state().label() == "done")
                })
            }));
            window.update(|window, cx| _ = window.draw(cx));
            let agent = window
                .debug_bounds(sidebar_agent_selector(agent_pane))
                .expect("detected Agent should render below its Workspace");
            window.simulate_click(agent.center(), Modifiers::default());

            assert!(wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session.active_workspace().is_some_and(|workspace| {
                            workspace.active_tab().focused_pane().id() == agent_pane
                        })
                    })
                })
            }));
            assert_eq!(
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .unwrap()
                        .agent_trackers
                        .get(&agent_pane)
                        .unwrap()
                        .display_state()
                        .label()
                }),
                "idle"
            );
        }

        #[test]
        fn new_workspace_round_trip_updates_gui_from_real_server() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (view, window, server) = connected_murmur(&mut cx);
            let button = window
                .debug_bounds("new-terminal-workspace")
                .expect("new workspace button should be rendered");
            window.simulate_click(button.center(), Modifiers::default());
            assert!(window.did_prompt_for_paths());
            let workspace_root = std::env::temp_dir();
            let selected_root = workspace_root.clone();
            window.simulate_path_prompt_response(move |_| Some(vec![selected_root]));

            let received = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .active_session()
                        .is_some_and(|session| session.active_workspace().is_some())
                })
            });
            assert!(
                received,
                "GUI did not render the workspace created by the real server; server workspaces: {}, client workspaces: {}, client sequence: {}, connection error: {:?}",
                murmur_core::Session::restore(server.handle.snapshot())
                    .unwrap()
                    .workspaces()
                    .len(),
                window.read(|app| murmur_core::Session::restore(
                    view.read(app).connection(1).unwrap().snapshot.clone()
                )
                .unwrap()
                .workspaces()
                .len()),
                window.read(|app| view.read(app).connection(1).unwrap().sequence),
                window.read(|app| view.read(app).connection(1).unwrap().error.clone()),
            );
            assert_eq!(
                window.read(|app| {
                    view.read(app)
                        .active_session()
                        .unwrap()
                        .active_workspace()
                        .unwrap()
                        .root_directory()
                        .to_path_buf()
                }),
                workspace_root
            );

            let new_tab = window
                .debug_bounds("new-tab")
                .expect("new tab button should be rendered after workspace creation");
            window.simulate_click(new_tab.center(), Modifiers::default());
            let tab_created = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session
                            .active_workspace()
                            .is_some_and(|workspace| workspace.tabs().len() == 2)
                    })
                })
            });
            assert!(
                tab_created,
                "GUI did not render the tab created by the real server"
            );

            let new_pane = window
                .read(|app| {
                    view.read(app)
                        .active_session()?
                        .active_workspace()
                        .map(|workspace| workspace.active_tab().focused_pane().id())
                })
                .unwrap();
            let new_tab_focused = wait_until(window, |window| {
                let focus = window.read(|app| {
                    view.read(app)
                        .panels
                        .get(&(1, new_pane))
                        .map(|panel| panel.read(app).focus_handle.clone())
                });
                focus.is_some_and(|focus| window.update(|window, _| focus.is_focused(window)))
            });
            assert!(
                new_tab_focused,
                "the terminal in a newly created Tab did not receive keyboard focus"
            );
            window.simulate_input("MURMUR_NEW_TAB_FOCUS");
            let new_tab_received_input = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app)
                        .connection(1)
                        .and_then(|connection| connection.terminals.get(&new_pane))
                        .is_some_and(|terminal| {
                            terminal
                                .view
                                .cells
                                .iter()
                                .map(|cell| cell.text.as_str())
                                .collect::<String>()
                                .contains("MURMUR_NEW_TAB_FOCUS")
                        })
                })
            });
            assert!(
                new_tab_received_input,
                "the terminal in a newly created Tab did not receive text input"
            );

            let active_tab = window
                .read(|app| {
                    view.read(app)
                        .active_session()?
                        .active_workspace()
                        .map(|workspace| workspace.active_tab().id())
                })
                .unwrap();
            let tab = window
                .debug_bounds(tab_selector(active_tab))
                .expect("active Tab should be rendered");
            window.simulate_mouse_down(tab.center(), MouseButton::Right, Modifiers::default());
            window.run_until_parked();
            window.update(|window, cx| {
                _ = window.draw(cx);
            });
            window.simulate_keystrokes("down enter");
            window.run_until_parked();
            assert!(
                window.update(|window, cx| window.has_active_dialog(cx)),
                "Rename Tab should open from its context menu"
            );
            window.update(|window, cx| window.close_dialog(cx));
            window.run_until_parked();

            let initial_pane = window
                .read(|app| {
                    view.read(app)
                        .active_session()?
                        .active_workspace()
                        .map(|workspace| workspace.active_tab().focused_pane().id())
                })
                .unwrap();
            let initial_terminal = terminal_selector(initial_pane);
            let terminal_ready = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).connection(1).is_some_and(|connection| {
                        connection
                            .terminals
                            .values()
                            .any(|terminal| !terminal.exited)
                    })
                }) && window.debug_bounds(initial_terminal).is_some()
            });
            assert!(terminal_ready, "GUI did not render the server PTY");

            let terminal = window.debug_bounds(initial_terminal).unwrap();
            window.simulate_mouse_down(terminal.center(), MouseButton::Right, Modifiers::default());
            window.run_until_parked();
            window.update(|window, cx| {
                _ = window.draw(cx);
            });
            window.simulate_keystrokes("down down enter");
            let split_right = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session
                            .active_workspace()
                            .is_some_and(|workspace| workspace.active_tab().panes().len() == 2)
                    })
                })
            });
            assert!(split_right, "Pane context menu did not split right");

            let (pane_to_focus, rebuilds_before_focus) = window.read(|app| {
                let murmur = view.read(app);
                let workspace = murmur.active_session().unwrap();
                let tab = workspace.active_workspace().unwrap().active_tab();
                let focused = tab.focused_pane().id();
                let other = tab
                    .panes()
                    .iter()
                    .find(|pane| pane.id() != focused)
                    .unwrap()
                    .id();
                (other, murmur.dock_rebuild_count)
            });
            let other_terminal = window
                .debug_bounds(terminal_selector(pane_to_focus))
                .expect("the other Pane should be rendered");
            window.simulate_click(other_terminal.center(), Modifiers::default());
            let pane_focused = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session.active_workspace().is_some_and(|workspace| {
                            workspace.active_tab().focused_pane().id() == pane_to_focus
                        })
                    })
                })
            });
            assert!(pane_focused, "clicking a Pane did not focus it");
            assert_eq!(
                window.read(|app| view.read(app).dock_rebuild_count),
                rebuilds_before_focus,
                "focus-only updates must not rebuild Dock"
            );

            window.simulate_keystrokes("alt-shift--");
            let split_down = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).active_session().is_some_and(|session| {
                        session
                            .active_workspace()
                            .is_some_and(|workspace| workspace.active_tab().panes().len() == 3)
                    })
                })
            });
            assert!(split_down, "Alt+Shift+- did not split down");

            window.simulate_input("printf MURMUR_E2E");
            window.simulate_keystrokes("enter");

            let output_received = wait_until(window, |window| {
                window.read(|app| {
                    view.read(app).connection(1).is_some_and(|connection| {
                        connection.terminals.values().any(|terminal| {
                            terminal
                                .view
                                .cells
                                .iter()
                                .map(|cell| cell.text.as_str())
                                .collect::<String>()
                                .contains("MURMUR_E2E")
                        })
                    })
                })
            });
            assert!(
                output_received,
                "GUI did not receive output from the server PTY"
            );

            let active_tab = window.read(|app| {
                view.read(app).active_session().and_then(|session| {
                    session
                        .active_workspace()
                        .map(|workspace| workspace.active_tab().id())
                })
            });
            window.simulate_keystrokes("ctrl-tab");
            let tab_switched = wait_until(window, |window| {
                let next_tab = window.read(|app| {
                    view.read(app).active_session().and_then(|session| {
                        session
                            .active_workspace()
                            .map(|workspace| workspace.active_tab().id())
                    })
                });
                next_tab.is_some() && next_tab != active_tab
            });
            assert!(
                tab_switched,
                "Ctrl+Tab did not switch tabs through the real server"
            );
        }

        #[test]
        fn visual_context_can_resize_murmur_window() {
            let mut cx = TestAppContext::single();
            cx.update(gpui_component::init);
            let (_, window, _server) = connected_murmur(&mut cx);
            window.simulate_resize(size(px(1280.), px(720.)));
            let bounds = window
                .debug_bounds("murmur-sidebar")
                .expect("sidebar should remain rendered");
            assert!(bounds.size.width > px(0.));
            assert!(bounds.size.height > px(0.));
            window.quit();
        }
    }
}
